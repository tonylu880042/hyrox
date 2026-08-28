//! HYROX Central Hub server.
//!
//! Transport and wiring only: it opens the store, builds the session, subscribes to the
//! broker, serves HTTP and WebSocket, and pushes the read model. Every business decision --
//! what a read means, when a class ends, what may be acknowledged -- belongs to
//! `application` and `domain` (CLAUDE.md 3, 29). Nothing in this file may grow a rule.
//!
//! ```text
//! ESP32 --MQTT--> mqtt::run --> application::ingest_read --> storage
//!                     |                                        |
//!                     +------------- ACK <---------------------+   (only after COMMIT)
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

use application::{
    apply_finish_policy, checkin::bind_tag, register_reader, snapshot, LiveSession,
    OperatorCommand, Recovery, RosterEntry, SessionPlan,
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{Html, IntoResponse, Redirect},
    routing::get,
    Router,
};
use domain::{Duration, FinishPolicy, Instant, Session, SessionConfig, SessionMode};
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use storage::Store;
use tokio::sync::{broadcast, Mutex};
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

const TRAINING_HTML: &str = include_str!("../static/training.html");

struct Hub {
    state: LiveSession,
}

/// The development clock: the class script's time, run fast.
///
/// Dev-only, and deliberately not on the ingestion path -- official timing is the
/// `detected_at` the edge stamps (CLAUDE.md 17). This decides when the emulated venue
/// *presents a tag*, and when the class's own duration is up; it never converts an arrival
/// into a result.
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

    fn now(&self) -> Instant {
        Instant(self.base_ms + (wall_clock_ms() - self.started_wall_ms) * self.speed)
    }

    #[cfg_attr(not(feature = "dev-simulator"), allow(dead_code))]
    fn class_start(&self) -> Instant {
        self.class_start
    }
}

#[derive(Clone)]
struct AppState {
    /// A Tokio mutex, not a std one: the ingestion use case holds the session across the
    /// store's awaits, which is exactly what keeps the ordering guarantees in one place.
    hub: Arc<Mutex<Hub>>,
    store: Arc<Store>,
    tx: broadcast::Sender<String>,
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
        if state.bindings.athlete_for_tag(&session_id, &tag).is_none() {
            bind_tag(&mut state, &store, &tag, &athlete_id, &provisioning)
                .await
                .expect("dev band binding");
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
    let store = Arc::new(store);
    let hub = Arc::new(Mutex::new(Hub { state }));

    let (tx, _) = broadcast::channel(16);
    let app_state = AppState { hub, store, tx };

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

    tokio::spawn(mqtt::run(app_state.clone(), broker));
    tokio::spawn(tick_loop(app_state.clone(), clock));

    let app = Router::new()
        .route("/", get(|| async { Redirect::temporary("/live") }))
        .route("/live", get(|| async { Html(TRAINING_HTML) }))
        .route("/ws", get(ws_handler))
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8730));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {addr}: {e}"));
    println!("HYROX Central Hub listening on http://{addr}/live");
    axum::serve(listener, app).await.expect("server stopped");
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
/// shows.
async fn tick_loop(app: AppState, clock: VirtualClock) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        ticker.tick().await;
        let now = clock.now();
        let mut hub = app.hub.lock().await;

        // A class ends when its time is up (CLAUDE.md 12, as configured on the session).
        apply_finish_policy(&mut hub.state, now);

        let payload = serde_json::to_string(&snapshot(&hub.state, now))
            .expect("snapshot must serialise");
        drop(hub);

        // Fails only when nobody is connected, which is normal.
        let _ = app.tx.send(payload);
    }
}

async fn ws_handler(ws: WebSocketUpgrade, State(app): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| push_snapshots(socket, app))
}

async fn push_snapshots(mut socket: WebSocket, app: AppState) {
    let mut rx = app.tx.subscribe();
    while let Ok(payload) = rx.recv().await {
        if socket.send(Message::Text(payload.into())).await.is_err() {
            break; // client went away
        }
    }
}
