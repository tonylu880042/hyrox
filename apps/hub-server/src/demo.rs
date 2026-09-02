//! Demo data on demand (M6 follow-up).
//!
//! The platform-specific half of [`api::Demo`]: a fixture venue (`crate::feeder`) loaded
//! into the *running* session through the ordinary use cases, plus an emulated collector
//! (`crate::sim`) publishing over the **real broker**. So an integration test exercises the
//! whole pipeline -- publish, subscribe, decode, commit, ACK, interpret, screen -- and not a
//! shortcut into the read model.
//!
//! Off unless `HYROX_DEMO=1`. This replaces the old behaviour, where *every* hub provisioned
//! twelve invented athletes at startup: `--no-default-features` removed the emulated reads
//! but never the fixture, so a customer's first boot showed a roster of people who do not
//! exist (ADR 0009 amended).

use crate::{feeder, VirtualClock, DEV_CLASS_LENGTH};
use api::{CheckIn, Demo, Hub, Operator};
use application::{checkin::Entrant, OperatorCommand};
use domain::FinishPolicy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use storage::Store;
use transport::MqttConfig;

/// The environment variable that turns any of this on.
pub const ENABLED: &str = "HYROX_DEMO";

pub fn enabled() -> bool {
    matches!(std::env::var(ENABLED).as_deref(), Ok("1") | Ok("true") | Ok("on"))
}

pub struct DemoVenue {
    /// Set by the composition root once the hub exists. The hub owns the demo port and the
    /// demo needs the hub, so one of the two directions has to be filled in afterwards.
    hub: OnceLock<Hub<Store>>,
    clock: VirtualClock,
    broker: MqttConfig,
    /// Whether an emulated collector is currently publishing. The collector reads this and
    /// stops: a task that cannot be asked to stop outlives the button that started it.
    running: Arc<AtomicBool>,
}

impl DemoVenue {
    pub fn new(clock: VirtualClock, broker: MqttConfig) -> Self {
        Self { hub: OnceLock::new(), clock, broker, running: Arc::new(AtomicBool::new(false)) }
    }

    pub fn attach(&self, hub: Hub<Store>) {
        let _ = self.hub.set(hub);
    }
}

impl Demo for DemoVenue {
    fn available(&self) -> bool {
        enabled()
    }

    fn load(&self) -> Result<(), String> {
        let hub = self.hub.get().ok_or("the hub is not attached yet")?.clone();
        if self.running.swap(true, Ordering::SeqCst) {
            // Loading twice would hand out the same bands again and leave two collectors
            // publishing the same sequence numbers.
            return Err("demo data is already running".to_string());
        }
        let (clock, broker, running) =
            (self.clock, self.broker.clone(), Arc::clone(&self.running));
        tokio::spawn(async move {
            if let Err(e) = provision(&hub, clock).await {
                eprintln!("demo: cannot provision the fixture venue: {e}");
                running.store(false, Ordering::SeqCst);
                return;
            }
            // A separate client id: the emulated collector is a different MQTT client from
            // the hub, exactly as a real board is.
            let device = MqttConfig {
                client_id: format!("hyrox-demo-{}", feeder::DEVICE_MAC.replace(':', "")),
                ..broker
            };
            crate::sim::run_until(clock, device, running).await;
        });
        Ok(())
    }

    fn clear(&self) -> Result<(), String> {
        // Only the reads stop. What was already recorded stays: `raw_events` is immutable
        // (CLAUDE.md 19), and the class is ended from the training screen like any other.
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

/// The fixture venue: a course, a roster, a reader map, bands, and an armed class.
///
/// Written through the ordinary use cases rather than into the tables, so the demo cannot
/// produce a state the product itself could not.
async fn provision(hub: &Hub<Store>, clock: VirtualClock) -> Result<(), String> {
    let operator = Operator::new(hub.clone());
    let door = CheckIn::new(hub.clone());
    let cmd = OperatorCommand::new("DEMO DATA", clock.class_start());
    let fail = |what: &str, e: String| format!("{what}: {e}");

    operator
        .configure(
            Some(feeder::course()),
            FinishPolicy::ClassDuration { limit: DEV_CLASS_LENGTH },
            &cmd,
        )
        .await
        .map_err(|e| fail("course", e.to_string()))?;

    for registration in feeder::readers() {
        operator
            .register_reader(&registration, &cmd)
            .await
            .map_err(|e| fail("reader", e.to_string()))?;
    }

    // The roster is entered at the door like anyone else, so the ids are real entry codes
    // and the bands are bound to them -- the script only cares which tag passes which
    // reader, never which athlete wears it.
    for ((tag, _), name) in feeder::bands().into_iter().zip(feeder::athletes()) {
        let athlete_id = door
            .enter(Entrant::walk_in(name), &cmd)
            .await
            .map_err(|e| fail("entrant", e.to_string()))?;
        door.bind(&tag, &athlete_id, &cmd)
            .await
            .map_err(|e| fail("band", e.to_string()))?;
    }

    operator.mark_ready(&cmd).await.map_err(|e| fail("arming", e.to_string()))?;
    operator.start(&cmd).await.map_err(|e| fail("start", e.to_string()))?;
    Ok(())
}
