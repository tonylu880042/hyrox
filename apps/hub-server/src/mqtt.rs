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

use crate::{wall_clock_ms, AppState};
use application::{ingest_read, IngestError};
use contract::ReceivedEvent;
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

/// Subscribes to the edge topics and ingests for as long as the process lives.
///
/// Never returns: an ingestion loop that gave up would leave the hub running with a live
/// screen and no events, which is worse than a visible crash.
pub async fn run(app: AppState, config: MqttConfig) {
    println!(
        "MQTT ingestion: connecting to {}:{} as \"{}\"",
        config.host, config.port, config.client_id
    );
    let (client, mut eventloop) = client::connect(&config);
    // Diagnostics, deliberately counted rather than merely logged: "the hub has been
    // dropping payloads all evening" is a question an operator should be able to answer.
    let mut undecodable_total: u64 = 0;

    loop {
        match client::next_inbound(&mut eventloop).await {
            Ok(Some(Inbound::Connected { session_present })) => {
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
            Ok(Some(Inbound::Event(event))) => ingest(&app, &client, *event).await,
            Ok(Some(Inbound::Status(status))) => report_status(&status),
            Ok(Some(Inbound::Undecodable { topic, error, payload })) => {
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
                eprintln!("MQTT connection error: {e} -- retrying");
                tokio::time::sleep(RECONNECT_BACKOFF).await;
            }
        }
    }
}

/// Hands one decoded read to the ingestion use case and publishes whatever ACK comes back.
async fn ingest(app: &AppState, client: &AsyncClient, event: contract::EdgeEvent) {
    // `received_at` is stamped here and is diagnostics only. The official time is the
    // `detected_at` the edge put in the payload (CLAUDE.md 17), and this function never
    // touches it.
    let received = ReceivedEvent::new(event, wall_clock_ms());
    let key = received.id();

    let ack = {
        let mut hub = app.hub.lock().await;
        match ingest_read(&mut hub.state, &*app.store, &received).await {
            Ok(ingested) => Some(ingested.ack),
            // The raw read IS durable, so the ACK is earned and the edge may release its
            // copy; the interpretation is missing and an operator has to add it
            // (CLAUDE.md 20).
            Err(IngestError::Interpretation { ack, source }) => {
                eprintln!("{key}: raw read stored but not interpreted: {source}");
                Some(ack)
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

/// Surfaces device health (CLAUDE.md 18). A journal filling up is the last warning before
/// RFID events start being lost, which is the one failure the system may never have.
fn report_status(status: &DeviceStatus) {
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
