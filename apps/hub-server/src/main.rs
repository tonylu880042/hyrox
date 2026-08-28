//! HYROX Central Hub server.
//!
//! Transport and wiring only: it opens the store, builds the session, serves HTTP and
//! WebSocket, and pushes the read model. Every business decision -- what a read means, when
//! a class ends, what may be acknowledged -- belongs to `application` and `domain`
//! (CLAUDE.md 3, 29). Nothing in this file may grow a rule.

mod feeder;

use application::{
    apply_finish_policy, checkin::bind_tag, ingest_read, register_reader, snapshot, LiveSession,
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
use mqtt::ReceivedEvent;
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use storage::Store;
use tokio::sync::{broadcast, Mutex};

/// Development clock speed. The class script covers ~20 minutes of real training;
/// running it faster keeps the screen moving while iterating on the UI.
/// ponytail: dev-only knob. Real ingestion uses detected_at straight from the edge.
const SPEED: i64 = 12;

/// The dev class runs for twenty (virtual) minutes. A session's finish rule is
/// configuration, never code (CLAUDE.md 12): this is the value for the demo script, not a
/// product decision.
const DEV_CLASS_LENGTH: Duration = Duration(20 * 60 * 1000);

const TRAINING_HTML: &str = include_str!("../static/training.html");

struct Hub {
    state: LiveSession,
    /// Virtual-clock offset so a resumed session continues instead of rewinding.
    resume_offset: i64,
    script: Vec<feeder::ScriptedRead>,
    cursor: usize,
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

    let script = feeder::script(state.class_start);
    let store = Arc::new(store);
    let hub = Arc::new(Mutex::new(Hub { state, resume_offset, script, cursor: 0 }));

    let (tx, _) = broadcast::channel(16);
    let app_state = AppState { hub, store, tx };

    tokio::spawn(tick_loop(app_state.clone()));

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

/// Advances the virtual clock, hands any due reads to the ingestion use case, and
/// broadcasts the resulting snapshot. No interpretation happens here.
async fn tick_loop(app: AppState) {
    let started_wall = wall_clock_ms();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        ticker.tick().await;
        let mut hub = app.hub.lock().await;
        let now = Instant(
            hub.state.class_start.0 + hub.resume_offset + (wall_clock_ms() - started_wall) * SPEED,
        );

        while hub.cursor < hub.script.len() && hub.script[hub.cursor].at <= now {
            let event = hub.script[hub.cursor].event.clone();
            hub.cursor += 1;
            let received = ReceivedEvent::new(event, wall_clock_ms());
            match ingest_read(&mut hub.state, &*app.store, &received).await {
                // The ACK is dropped rather than published: there is no broker in the dev
                // feeder. Dropping it is safe -- an unacknowledged event is resent -- while
                // publishing one without a commit would not be (ADR 0002).
                Ok(_) => {}
                Err(e) => eprintln!("ingestion failed: {e}"),
            }
        }

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
