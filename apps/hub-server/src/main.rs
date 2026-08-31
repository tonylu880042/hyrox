//! HYROX Central Hub server: the composition root, and nothing else.
//!
//! It opens the store, recovers the interrupted session, provisions the development venue,
//! subscribes to the broker, starts the tick loop, and serves the router `crates/api`
//! builds. It holds no route, no handler and no rule.
//!
//! Every business decision -- what a read means, when a class ends, what may be
//! acknowledged, which surface may write -- belongs to `application`, `domain` and `api`
//! (CLAUDE.md 3, 29; ADR 0007). This file is the only place that may see every layer at
//! once, which is exactly why nothing in it may grow a rule.
//!
//! ```text
//! ESP32 --MQTT--> mqtt::run --> application::ingest_read --> storage
//!                     |                                        |
//!                     +------------- ACK <---------------------+   (only after COMMIT)
//!
//! browser ---HTTP/WS---> api::router --> api capability state --> application use cases
//! ```
//!
//! `HYROX_DB`, `HYROX_MQTT_HOST`, `HYROX_MQTT_PORT`, `HYROX_MQTT_CLIENT_ID` and `HYROX_SIM`
//! configure it; see `README.md`.

mod feeder;
mod mqtt;

/// The emulated collector that keeps `/live` moving on a developer's machine, publishing
/// over the real broker so nothing short-circuits ingestion (CLAUDE.md 25; ADR 0006).
/// `--no-default-features` leaves it out of a venue build entirely.
#[cfg(feature = "dev-simulator")]
mod sim;

use api::{Clock, Hub};
use application::{
    apply_finish_policy, checkin::bind_tag, register_reader, snapshot, OperatorCommand,
    Recovery, RosterEntry, SessionPlan,
};
use axum::{
    response::{Html, Redirect},
    routing::get,
    Router,
};
use domain::{
    Duration, ExerciseLibrary, FinishPolicy, Instant, Session, SessionConfig, SessionMode,
    WorkoutTemplate,
};
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use storage::Store;
use transport::MqttConfig;

/// Development clock speed. The class script covers ~20 minutes of real training;
/// running it faster keeps the screen moving while iterating on the UI.
/// ponytail: dev-only knob. Real ingestion uses detected_at straight from the edge.
const SPEED: i64 = 12;

/// The hub's MQTT client id. Stable on purpose: with `clean_session = false` the broker
/// holds this client's QoS 1 subscription and its queued messages while the hub is down, so
/// events published during a restart are delivered afterwards rather than lost
/// (CLAUDE.md 15, 21). A per-run id would throw that queue away every restart.
const DEFAULT_CLIENT_ID: &str = "hyrox-hub";

/// The dev class runs for twenty (virtual) minutes. A session's finish rule is
/// configuration, never code (CLAUDE.md 12): this is the value for the demo script, not a
/// product decision.
const DEV_CLASS_LENGTH: Duration = Duration(20 * 60 * 1000);

/// How often the read model is re-derived and pushed. Published to every screen in the
/// freshness readout, so a client can tell "quiet" from "the socket died" without inventing
/// a timeout of its own (ADR 0001 D5).
const PUSH_INTERVAL_MS: i64 = 250;

/// Snapshot fan-out depth. A screen further behind than this is dropped rather than served
/// stale frames: on a live screen an old snapshot is worse than a visibly closed socket.
const SNAPSHOT_CHANNEL_CAPACITY: usize = 16;

const TRAINING_HTML: &str = include_str!("../static/training.html");
const WORKOUT_HTML: &str = include_str!("../static/workout.html");
/// The interface dictionary both screens read their labels from (roadmap M7). Served from
/// the hub, never a CDN: a venue with no internet must still get its own language.
const I18N_JS: &str = include_str!("../static/i18n.js");

/// The development clock: the class script's time, run fast.
///
/// Dev-only, and deliberately not on the ingestion path -- official timing is the
/// `detected_at` the edge stamps (CLAUDE.md 17). This decides when the emulated venue
/// *presents a tag*, and when the class's own duration is up; it never converts an arrival
/// into a result.
///
/// It is also the clock `crates/api` reads, through [`api::Clock`]. One clock for the
/// screens and the script, so an age shown on a screen means what the events mean.
#[derive(Clone, Copy)]
struct VirtualClock {
    /// Where the class script's clock starts. Only the emulated collector needs it, so a
    /// build without `dev-simulator` has no reader for it.
    #[cfg_attr(not(feature = "dev-simulator"), allow(dead_code))]
    class_start: Instant,
    /// Where the clock resumes: the class start, plus how far the stored events already got.
    base_ms: i64,
    started_wall_ms: i64,
    speed: i64,
}

impl VirtualClock {
    fn start(class_start: Instant, resume_offset: i64, speed: i64) -> Self {
        Self {
            class_start,
            base_ms: class_start.0 + resume_offset,
            started_wall_ms: wall_clock_ms(),
            speed,
        }
    }

    #[cfg_attr(not(feature = "dev-simulator"), allow(dead_code))]
    fn class_start(&self) -> Instant {
        self.class_start
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> Instant {
        Instant(self.base_ms + (wall_clock_ms() - self.started_wall_ms) * self.speed)
    }
}

fn wall_clock_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_millis() as i64
}

#[tokio::main]
async fn main() {
    let db = std::env::var("HYROX_DB").unwrap_or_else(|_| "sqlite://hyrox.db".to_string());
    let store = Store::open(&db)
        .await
        .unwrap_or_else(|e| panic!("cannot open {db}: {e}"));

    // First-run content: the exercise library and the starter templates (workout brief
    // §3, §16). Idempotent -- both are keyed writes, so a hub that has been started before
    // re-writes the same rows rather than accumulating copies, and a coach's edits to a
    // template of their own are never touched because presets are SYSTEM and have their own
    // stable ids.
    seed_workout_library(&store).await;

    let class_start = Instant(wall_clock_ms());
    let plan = SessionPlan {
        session: Session::new_draft(
            format!("s-{}", class_start.0),
            "THURSDAY 19:00 HYROX CLASS",
            SessionMode::Training,
        ),
        config: SessionConfig::new(format!("s-{}", class_start.0))
            .with_course(feeder::course())
            .with_finish_policy(FinishPolicy::ClassDuration { limit: DEV_CLASS_LENGTH }),
        roster: feeder::athletes()
            .iter()
            .enumerate()
            .map(|(i, name)| RosterEntry {
                athlete_id: feeder::athlete_id(i),
                display_name: (*name).to_string(),
            })
            .collect(),
        class_start,
    };

    // Resume an interrupted session rather than starting a new one (CLAUDE.md 21).
    let (mut state, recovery) = application::resume_or_start(&store, plan)
        .await
        .expect("recovery failed");
    match recovery {
        Recovery::Resumed => println!(
            "resumed session {} ({:?}) with {} athletes, {} events already interpreted",
            state.session.id,
            state.config.finish_policy,
            state.athletes.len(),
            state.session.interpreted_event_count
        ),
        // Said out loud rather than swallowed: the class is running under configuration that
        // may not be the configuration it was armed with (ADR 0004).
        Recovery::ResumedWithoutStoredConfig => eprintln!(
            "WARNING: resumed session {} has no stored configuration; \
             using the startup plan's course and finish rule",
            state.session.id
        ),
        Recovery::Started => println!("started new session {}", state.session.id),
    }

    // Dev venue provisioning. The reader map and the bands are recovered from the store by
    // `resume_or_start`; these calls only fill in what a fresh database does not have yet.
    // Registering an unchanged reader is a no-op, and a band already bound is left alone.
    let session_id = state.session.id.clone();
    let provisioning = OperatorCommand::new("DEV FEEDER", state.class_start);
    for registration in feeder::readers() {
        register_reader(&mut state, &store, &registration, &provisioning)
            .await
            .expect("dev reader registration");
    }
    for (tag, athlete_id) in feeder::bands() {
        // Two reasons to leave a band alone, and both are the hub being right rather than
        // the hub being in the way:
        //
        // * the athlete is not on this session's roster -- once a coach builds their own
        //   class from a template, the roster is theirs, not the dev script's (ADR 0001 D4);
        // * the band is already on somebody's wrist. Tag uniqueness is checked across
        //   *every* session, not within one (migration 0003), so a band handed out in
        //   yesterday's class is still bound today until someone unbinds it.
        //
        // Neither is an error worth refusing to boot over. A venue machine that will not
        // start because of demo data is a far worse failure than a missing demo band.
        let on_roster = state.athlete(&athlete_id).is_some();
        let already_worn = state.bindings.active().any(|b| b.tag_id == tag);
        if on_roster && !already_worn {
            if let Err(e) = bind_tag(&mut state, &store, &tag, &athlete_id, &provisioning).await {
                eprintln!("dev band {tag:?} not bound: {e:?}");
            }
        }
    }

    // Dev virtual clock only: pick up where the stored events left off instead of replaying
    // the class from zero. Not a business value.
    let resume_offset = store
        .max_detected_at(&session_id)
        .await
        .expect("max detected_at lookup failed")
        .map(|t| t.0 - state.class_start.0)
        .unwrap_or(0);

    let clock = VirtualClock::start(state.class_start, resume_offset, SPEED);
    let hub = Hub::new(
        state,
        Arc::new(store),
        Arc::new(clock),
        PUSH_INTERVAL_MS,
        SNAPSHOT_CHANNEL_CAPACITY,
        // The shipped artefact's version, which is what `/api/health` is asked for
        // (ADR 0009) -- not any library crate's.
        env!("CARGO_PKG_VERSION"),
    );

    // Real ingestion: everything that reaches the screen from here on arrived over the
    // broker (CLAUDE.md 15, 16).
    let broker = broker_config();

    #[cfg(feature = "dev-simulator")]
    if std::env::var("HYROX_SIM").as_deref() != Ok("off") {
        // A separate client id: the emulated collector is a different MQTT client from the
        // hub, exactly as a real ESP32 is.
        let device = MqttConfig {
            client_id: format!("hyrox-sim-{}", feeder::DEVICE_MAC.replace(':', "")),
            ..broker.clone()
        };
        tokio::spawn(sim::run(clock, device));
    }

    tokio::spawn(mqtt::run(hub.clone(), broker));
    tokio::spawn(tick_loop(hub.clone()));

    // The two static routes are the app's own; every API route comes from `crates/api`,
    // which owns the read/write split (ADR 0001, 0007). Merging rather than re-declaring
    // means this file cannot quietly add a write surface of its own.
    let app = Router::new()
        .route("/", get(|| async { Redirect::temporary("/live") }))
        .route("/live", get(|| async { Html(TRAINING_HTML) }))
        .route("/workout", get(|| async { Html(WORKOUT_HTML) }))
        .route(
            "/i18n.js",
            get(|| async {
                ([(axum::http::header::CONTENT_TYPE, "text/javascript; charset=utf-8")], I18N_JS)
            }),
        )
        .merge(api::router(hub));

    let addr = bind_address();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {addr}: {e}"));
    println!("HYROX Central Hub listening on http://{addr}/live");
    println!("  workout builder: http://{addr}/workout");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server stopped");
    println!("stopped");
}

/// Where the HTTP surface listens. `127.0.0.1:8730` unless `HYROX_BIND` says otherwise.
///
/// Loopback by default on purpose (ADR 0009 §5): running `cargo run` on a laptop must not
/// put an unauthenticated write surface on the café's wifi. The appliance opts in through
/// its unit file, and the network boundary there is the deployment layer's job -- there is
/// no login, and ADR 0001 D1 accepted that trade deliberately.
fn bind_address() -> SocketAddr {
    match std::env::var("HYROX_BIND") {
        Ok(value) => value
            .parse()
            .unwrap_or_else(|e| panic!("HYROX_BIND {value:?} is not an address: {e}")),
        Err(_) => SocketAddr::from(([127, 0, 0, 1], 8730)),
    }
}

/// Stops serving on SIGTERM (systemd) or Ctrl-C, letting in-flight requests finish.
///
/// Nothing is flushed here and nothing needs to be: every RFID event is durable before it
/// is acknowledged (ADR 0002), and `synchronous = FULL` makes that survive a pulled plug as
/// well as a signal (ADR 0009 §7). A read committed but not yet acknowledged is redelivered
/// by the edge and skipped by its idempotency key, so an abrupt stop costs nothing but a
/// duplicate delivery -- which the protocol is built to expect (CLAUDE.md 16).
///
/// Deciding *whether* it is a good moment to stop is not this function's business:
/// `GET /api/health` answers that, and the maintenance window asks before it sends anything
/// (ADR 0009 §6).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("cannot listen for Ctrl-C");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("cannot listen for SIGTERM")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => println!("interrupted, stopping"),
        () = terminate => println!("SIGTERM, stopping"),
    }
}

/// Writes the shipped exercise library and preset templates (workout brief §3, §16).
///
/// Runs on every start, not just the first: the writes are keyed on `code` and on the
/// preset ids, so this brings a hub up to date with a build that added an exercise without
/// duplicating anything. It never touches a coach's own templates -- those have different
/// ids and are never SYSTEM.
async fn seed_workout_library(store: &Store) {
    for exercise in ExerciseLibrary::preset().iter() {
        store
            .save_exercise(exercise)
            .await
            .unwrap_or_else(|e| panic!("cannot seed exercise {}: {e}", exercise.code));
    }
    for template in WorkoutTemplate::presets() {
        store
            .save_template(&template)
            .await
            .unwrap_or_else(|e| panic!("cannot seed template {}: {e}", template.id));
    }
}

/// Where the broker is. Defaults to a broker on this machine, which is the Phase 1 layout:
/// hub, broker and SQLite on one box on the venue LAN (CLAUDE.md 5).
fn broker_config() -> MqttConfig {
    let client_id =
        std::env::var("HYROX_MQTT_CLIENT_ID").unwrap_or_else(|_| DEFAULT_CLIENT_ID.to_string());
    let mut config = MqttConfig::local(client_id);
    if let Ok(host) = std::env::var("HYROX_MQTT_HOST") {
        config.host = host;
    }
    if let Ok(port) = std::env::var("HYROX_MQTT_PORT") {
        config.port = port.parse().unwrap_or_else(|e| panic!("HYROX_MQTT_PORT: {e}"));
    }
    config
}

/// Applies the session's finish rule on the class clock and broadcasts the read model.
///
/// Interpretation is not done here and never was: reads arrive over MQTT and go through
/// `application::ingest_read` (see [`mqtt`]). This loop only re-derives what the screen
/// shows, which is why it is the one thing outside `crates/api` that still touches the live
/// session -- it needs the mutable access `Hub::lock` gives the composition root and gives
/// nobody else.
async fn tick_loop(hub: Hub<Store>) {
    let interval = std::time::Duration::from_millis(PUSH_INTERVAL_MS as u64);
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        let now = hub.now();
        let mut state = hub.lock().await;

        // A class ends when its time is up (CLAUDE.md 12, as configured on the session).
        apply_finish_policy(&mut state, now);

        let payload =
            serde_json::to_string(&snapshot(&state, now)).expect("snapshot must serialise");
        drop(state);

        hub.publish(payload);
    }
}
