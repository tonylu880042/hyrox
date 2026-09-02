//! The development fleet: an emulated ESP32 publishing the class script over the real
//! broker (CLAUDE.md 25).
//!
//! This is `crates/simulator`'s device — the same journal, the same presence/re-arm
//! suppression (CLAUDE.md 14), the same "release nothing until the hub says it is durable"
//! rule — attached to Mosquitto instead of to the in-process link. So the dev screen is fed
//! by the real ingestion path: publish → subscribe → decode → commit → ACK. Nothing
//! short-circuits it.
//!
//! Behind the `dev-simulator` feature (on by default) and, at run time, the demo button on
//! the settings screen (`HYROX_DEMO=1`), so a venue build carries no emulated hardware at
//! all and a venue machine never starts any.
//!
//! Two tasks rather than one `select!`: rumqttc's `EventLoop::poll` is not cancel-safe, and
//! cancelling it at a tick boundary could drop an acknowledgement mid-flight.

use crate::{feeder, VirtualClock};
// The dev clock's `now` comes from the API's clock port, which is the same clock the
// screens read (ADR 0007).
use api::Clock;
use simulator::{
    mqtt::MqttDevice, AbsentTimeout, DeviceConfig, ReaderConfig, RfOutcome, SimDevice,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use transport::{client, Inbound, MqttConfig};

/// How often the emulated venue is stepped. Reads are stamped with their *scripted*
/// detection time, not with the tick, so this only controls how promptly they are published
/// -- it can never move a result (CLAUDE.md 17).
const STEP: std::time::Duration = std::time::Duration::from_millis(100);

/// How long an event may sit unacknowledged before the device resends it. Firmware needs
/// such a timer: an ACK can go missing without the link ever dropping, and a lost ACK must
/// cost a redelivery rather than an event (CLAUDE.md 18). The value is a dev knob, not a
/// contract -- the real one belongs with the firmware team.
const RESEND_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

/// Boots the device, publishes the class, and applies the hub's acknowledgements.
/// Publishing ends when `running` goes false, and the ACK poller is dropped with it: the
/// demo button needs an off switch (M6 follow-up).
pub async fn run_until(
    clock: VirtualClock,
    config: MqttConfig,
    running: Arc<std::sync::atomic::AtomicBool>,
) {
    let device = match boot(clock) {
        Ok(device) => device,
        Err(e) => {
            eprintln!("dev simulator: cannot boot the emulated collector: {e}");
            return;
        }
    };
    println!(
        "dev simulator: emulated collector {} publishing to {}:{}",
        device.device_id(),
        config.host,
        config.port
    );

    let (device, eventloop) = MqttDevice::attach(device, &config);
    let device = Arc::new(Mutex::new(device));
    let poller = tokio::spawn(acks(Arc::clone(&device), eventloop));
    publish_class(device, clock, running).await;
    poller.abort();
}

/// One collector carrying every reader in the dev venue -- the layout CLAUDE.md 7.3 allows
/// (one ESP32, several readers) and the one `feeder` describes.
fn boot(clock: VirtualClock) -> Result<SimDevice, simulator::ConfigError> {
    let mut config = DeviceConfig::new(feeder::DEVICE_MAC)?;
    for registration in feeder::readers() {
        // The absent timeout is per reader and configurable (CLAUDE.md 14); the dev venue
        // has no measurements of its own, so every reader takes the documented default
        // rather than a number invented here.
        config = config.with_reader(ReaderConfig::new(
            registration.key.reader_id.as_str(),
            AbsentTimeout::default(),
        )?);
    }
    SimDevice::boot(config, clock.now().0)
}

/// Presents each scripted tag to its reader as the virtual clock reaches it, then publishes
/// whatever the journal is owed an ACK for.
async fn publish_class(
    device: Arc<Mutex<MqttDevice>>,
    clock: VirtualClock,
    running: Arc<std::sync::atomic::AtomicBool>,
) {
    let script = feeder::script(clock.class_start());
    let mut cursor = 0usize;
    let mut ticker = tokio::time::interval(STEP);
    let mut last_resend = tokio::time::Instant::now();

    while running.load(std::sync::atomic::Ordering::SeqCst) {
        ticker.tick().await;
        let now = clock.now();
        let mut guard = device.lock().await;

        while cursor < script.len() && script[cursor].at <= now {
            let read = &script[cursor];
            cursor += 1;
            // Stamped with the scripted detection time, not with "now": that is what makes a
            // resumed run republish byte-identical events, which the hub then deduplicates
            // on device + boot + sequence instead of timing the class twice (CLAUDE.md 16).
            match guard.device_mut().rf_read(&read.reader_id, &read.tag_id, read.at.0) {
                Ok(RfOutcome::Emitted(_)) | Ok(RfOutcome::Suppressed) => {}
                Err(e) => eprintln!("dev simulator: RF read rejected: {e}"),
            }
        }

        let due_for_resend = last_resend.elapsed() >= RESEND_AFTER
            && guard.device().pending_count() > 0;
        let published = if due_for_resend {
            last_resend = tokio::time::Instant::now();
            guard.resend_pending().await
        } else {
            guard.publish_new().await
        };
        if let Err(e) = published {
            // Nothing is lost: the journal still holds every unacknowledged event, and the
            // resend timer or the next connection will send them again (CLAUDE.md 18).
            eprintln!("dev simulator: publish failed: {e}");
        }
    }
}

/// Polls the connection and releases journal entries the hub has acknowledged.
async fn acks(device: Arc<Mutex<MqttDevice>>, mut eventloop: client::EventLoop) {
    loop {
        match client::next_inbound(&mut eventloop).await {
            Ok(Some(Inbound::Connected { .. })) => {
                let mut guard = device.lock().await;
                // A reconnect resends the whole unacknowledged backlog -- publishing
                // released nothing, only an ACK does (CLAUDE.md 15, 18).
                guard.on_reconnect();
                if let Err(e) = guard.subscribe_acks().await {
                    eprintln!("dev simulator: ack subscribe failed: {e}");
                }
                if let Err(e) = guard.publish_status().await {
                    eprintln!("dev simulator: status publish failed: {e}");
                }
            }
            Ok(Some(Inbound::Ack(ack))) => {
                device.lock().await.on_ack(&ack);
            }
            Ok(Some(_)) | Ok(None) => {}
            Err(e) => {
                eprintln!("dev simulator: connection error: {e} -- retrying");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    }
}
