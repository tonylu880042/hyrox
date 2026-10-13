//! MQTT ingestion (CLAUDE.md 15, 16) — composition-root wiring, and nothing else.
//!
//! The loop does three things and decides none of them:
//!
//! ```text
//! decode  (transport::classify)
//!   -> hand off  (application::ingest_read)
//!   -> publish the ACK it was handed
//! ```
//!
//! Every rule it looks like it is applying belongs to a layer below. *Acknowledge a
//! duplicate too* is `application::IngestOutcome::Duplicate` carrying an ACK; *acknowledge
//! even when the interpretation failed* is `IngestError::Interpretation` handing the ACK
//! back, because the raw read is durable and that is the guarantee the edge is waiting on;
//! *never acknowledge an uncommitted read* is the fact that `IngestError::Storage` has no
//! ACK to hand back at all (ADR 0002). There is no path here that can invent one
//! (CLAUDE.md 29).
//!
//! A device that publishes rubbish must not be able to stop a class: nothing in this loop
//! returns, panics, or stops polling on a bad payload (CLAUDE.md 31 principle 1).

use crate::wall_clock_ms;
use api::Hub;
use application::{
    ingest_read, note_device_status, note_reader_read, note_retained_status, DeviceReport,
    IngestError,
};
use contract::ReceivedEvent;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use storage::Store;
use transport::{
    client::{self, AsyncClient},
    payload_excerpt, topic, DeviceStatus, Inbound, MqttConfig,
};

/// How much of a payload nobody could decode goes into the log. Enough to recognise what
/// the device sent, bounded so a device stuck in a loop cannot drown the operator's log.
const UNDECODABLE_EXCERPT_BYTES: usize = 256;

/// Backoff after a connection error. rumqttc reconnects on the next poll by itself; this
/// only stops a down broker from turning the loop into a spin.
const RECONNECT_BACKOFF: std::time::Duration = std::time::Duration::from_secs(1);

/// How often the hub broadcasts its clock. The firmware's suggested period (protocol v1.1
/// section 7.2); it keeps the lowest-latency sample of the last five minutes, so nothing
/// rides on this being exact.
const TIME_BROADCAST_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Broadcasts the hub's clock for as long as the process lives (CLAUDE.md 17).
///
/// An edge board has no battery clock. Until it has a time source it cannot compute
/// `detected_at`, so it journals its reads and sends none of them -- and fills up. This is
/// that source, and it is the one that does not depend on what the host OS runs: an NTP
/// server is a second source on the appliance, never a requirement of the hub.
///
/// Reads the wall clock, never the class clock: the demo's virtual time must not reach a
/// real board. Silent while disconnected, because rumqttc would queue the publishes and
/// deliver a burst of stale times on reconnect.
async fn broadcast_time(client: AsyncClient, connected: Arc<AtomicBool>) {
    let mut ticker = tokio::time::interval(TIME_BROADCAST_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        if connected.load(Ordering::Relaxed) {
            // Dropped on a full queue; the next one is two seconds away.
            let _ = client::publish_time(&client, wall_clock_ms());
        }
    }
}

/// Subscribes to the edge topics and ingests for as long as the process lives.
///
/// Never returns: an ingestion loop that gave up would leave the hub running with a live
/// screen and no events, which is worse than a visible crash.
pub async fn run(hub: Hub<Store>, config: MqttConfig) {
    println!(
        "MQTT ingestion: connecting to {}:{} as \"{}\"",
        config.host, config.port, config.client_id
    );
    let (client, mut eventloop) = client::connect(&config);
    // Diagnostics, deliberately counted rather than merely logged: "the hub has been
    // dropping payloads all evening" is a question an operator should be able to answer.
    let mut undecodable_total: u64 = 0;
    let connected = Arc::new(AtomicBool::new(false));
    tokio::spawn(broadcast_time(client.clone(), connected.clone()));

    loop {
        match client::next_inbound(&mut eventloop).await {
            Ok(Some(Inbound::Connected { session_present })) => {
                connected.store(true, Ordering::Relaxed);
                // Re-subscribe on every connection: a broker that restarted has forgotten
                // the subscriptions it was holding for this session, and a hub that assumed
                // otherwise would go quiet without ever failing.
                match client::subscribe_hub(&client).await {
                    Ok(()) => println!(
                        "MQTT connected (broker kept our session: {session_present}); \
                         subscribed to {} and {}",
                        topic::ALL_EVENTS,
                        topic::ALL_STATUS
                    ),
                    Err(e) => eprintln!("MQTT subscribe failed: {e}"),
                }
            }
            Ok(Some(Inbound::Event(event))) => ingest(&hub, &client, *event).await,
            Ok(Some(Inbound::Status(status))) => report_status(&hub, &status, true).await,
            Ok(Some(Inbound::RetainedStatus(status))) => report_status(&hub, &status, false).await,
            Ok(Some(Inbound::Undecodable {
                topic,
                error,
                payload,
            })) => {
                undecodable_total += 1;
                // The record for an undecodable payload is this line, and only this line.
                // It cannot go in the raw event store: that table is keyed by
                // `device_id + boot_id + sequence` (CLAUDE.md 16), and a payload that did
                // not decode has no such key -- writing one would mean inventing it. It is
                // not acknowledged either: nothing was made durable, so under ADR 0002
                // there is nothing to acknowledge with, and the edge keeps its copy.
                eprintln!(
                    "MQTT undecodable payload #{undecodable_total} on {topic}: {error} \
                     -- not stored, not acknowledged, edge keeps it; payload: {}",
                    payload_excerpt(&payload, UNDECODABLE_EXCERPT_BYTES)
                );
            }
            // Another publisher shares the broker, or an ACK we published came back on a
            // subscription we do not hold. Neither is a fault.
            Ok(Some(Inbound::Ack(_))) | Ok(Some(Inbound::Foreign { .. })) | Ok(None) => {}
            Err(e) => {
                connected.store(false, Ordering::Relaxed);
                eprintln!("MQTT connection error: {e} -- retrying");
                tokio::time::sleep(RECONNECT_BACKOFF).await;
            }
        }
    }
}

/// Hands one decoded read to the ingestion use case and publishes whatever ACK comes back.
async fn ingest(hub: &Hub<Store>, client: &AsyncClient, event: contract::EdgeEvent) {
    // `received_at` is stamped here and is diagnostics only. The official time is the
    // `detected_at` the edge put in the payload (CLAUDE.md 17), and this function never
    // touches it.
    let received = ReceivedEvent::new(event, wall_clock_ms());
    let key = received.id();

    let ack = {
        let mut state = hub.lock().await;
        // Heard from, whatever the read turns out to mean. Freshness is a statement about
        // the link, not about interpretation, and D5 needs it for every arriving message.
        // Stamped on the hub's own clock -- the same one the screens read -- so an age
        // shown beside a reader is comparable with the ages beside the athletes.
        let event = received.event();
        let reader = domain::ReaderKey::new(event.device_id.clone(), event.reader_id.clone());
        note_reader_read(&mut state, &reader, hub.now());
        match ingest_read(&mut state, &**hub.store(), &received).await {
            Ok(ingested) => Some(ingested.ack),
            // Raw read is durable, but interpretation failed. We do NOT acknowledge to the edge
            // so the edge resends. When it resends, the duplicate handler will detect that this
            // raw read lacks an interpretation and retry interpreting it.
            Err(IngestError::Interpretation { ack: _, source }) => {
                eprintln!("{key}: raw read stored but not interpreted: {source} -- will not ACK so edge resends");
                None
            }
            Err(e) => {
                eprintln!("{key}: not durable, so not acknowledged -- the edge will resend: {e}");
                None
            }
        }
    };

    if let Some(ack) = ack {
        let device = ack.payload().device_id.clone();
        if let Err(e) = client::publish_ack(client, &device, &ack).await {
            // The event is safe; the edge simply keeps it and resends, and the hub
            // deduplicates (CLAUDE.md 16).
            eprintln!("{key}: ACK publish failed: {e}");
        }
    }
}

/// Records device health and surfaces it (CLAUDE.md 18; ADR 0001 D5). A journal filling up
/// is the last warning before RFID events start being lost, which is the one failure the
/// system may never have.
///
/// Kept, not merely logged: `/operator` shows each reader's last-seen age and its board's
/// own warning, and a log line on a headless box is not something an operator can see.
///
/// `live` is false for the broker's retained copy: its content is shown, but it is not
/// counted as hearing from the board.
async fn report_status(hub: &Hub<Store>, status: &DeviceStatus, live: bool) {
    let report = DeviceReport {
        device_id: status.device_id.clone(),
        boot_id: status.boot_id,
        pending_events: status.pending_events,
        journal_capacity: status.journal_capacity,
        // The device's own assessment, relayed. The hub never derives one of its own.
        warning: status.warning,
    };
    if live {
        let now = hub.now();
        note_device_status(&mut *hub.lock().await, report, now);
    } else {
        note_retained_status(&mut *hub.lock().await, report);
    }

    match status.warning {
        Some(warning) => eprintln!(
            "device {} (boot {}): {:?} -- {} of {} journal entries pending",
            status.device_id,
            status.boot_id,
            warning,
            status.pending_events,
            status.journal_capacity
        ),
        None => println!(
            "device {} (boot {}) healthy: {} pending",
            status.device_id, status.boot_id, status.pending_events
        ),
    }
}
