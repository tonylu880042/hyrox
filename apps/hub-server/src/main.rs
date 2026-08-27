//! HYROX Central Hub server.
//!
//! Serves the live screens and pushes state over WebSocket. All interpretation happens in
//! `domain`; this binary only ingests events, holds state and serialises it (CLAUDE.md 3, 6).

mod feeder;
mod state;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{Html, IntoResponse, Redirect},
    routing::get,
    Router,
};
use domain::{interpret, AthleteState, Instant, Interpreted, Session, SessionMode};
use storage::{RawEvent, Store};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;

/// Development clock speed. The class script covers ~20 minutes of real training;
/// running it faster keeps the screen moving while iterating on the UI.
/// ponytail: dev-only knob. Real ingestion uses detected_at straight from the edge.
const SPEED: i64 = 12;

const TRAINING_HTML: &str = include_str!("../static/training.html");

struct Hub {
    session: Session,
    athletes: Vec<AthleteState>,
    course: Vec<state::CourseStation>,
    class_start: Instant,
    /// Virtual-clock offset so a resumed session continues instead of rewinding.
    resume_offset: i64,
    exceptions: usize,
    script: Vec<feeder::ScriptedEvent>,
    cursor: usize,
}

#[derive(Clone)]
struct AppState {
    hub: Arc<Mutex<Hub>>,
    /// Outside the hub lock: the pool is already Send + Sync, and holding a mutex guard
    /// across an await would poison every writer.
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

    let course = feeder::course();

    // Resume an interrupted session rather than starting a new one (CLAUDE.md 21).
    let recovered = store.active_session().await.expect("session lookup failed");
    let (session, athletes, class_start, resume_offset) = match recovered {
        Some(s) if s.accepts_events() => {
            let class_start = store
                .session_created_at(&s.id)
                .await
                .expect("created_at lookup failed")
                .unwrap_or(Instant(wall_clock_ms()));
            let athletes = store
                .rebuild_athletes(&s.id)
                .await
                .expect("rebuild failed");
            let offset = store
                .max_detected_at(&s.id)
                .await
                .expect("max detected_at lookup failed")
                .map(|t| t.0 - class_start.0)
                .unwrap_or(0);
            println!(
                "resumed session {} with {} athletes, {} events already interpreted",
                s.id,
                athletes.len(),
                s.interpreted_event_count
            );
            (s, athletes, class_start, offset)
        }
        _ => {
            let class_start = Instant(wall_clock_ms());
            let mut s = Session::new_draft(
                format!("s-{}", class_start.0),
                "THURSDAY 19:00 HYROX CLASS",
                SessionMode::Training,
            );
            s.arm().expect("a fresh draft session must arm");
            store
                .save_session(&s, class_start)
                .await
                .expect("cannot persist session");
            let athletes: Vec<AthleteState> = feeder::athletes()
                .iter()
                .enumerate()
                .map(|(i, n)| AthleteState::ready(format!("a{}", i + 1), *n))
                .collect();
            for (i, a) in athletes.iter().enumerate() {
                store
                    .save_athlete(&s.id, &a.athlete_id, &a.display_name, i as i64 + 1)
                    .await
                    .expect("cannot persist athlete");
            }
            println!("started new session {}", s.id);
            (s, athletes, class_start, 0)
        }
    };

    let store = Arc::new(store);
    let hub = Arc::new(Mutex::new(Hub {
        session,
        athletes,
        course,
        class_start,
        resume_offset,
        exceptions: 0,
        script: feeder::script(class_start),
        cursor: 0,
    }));

    let (tx, _) = broadcast::channel(16);
    let app_state = AppState { hub: hub.clone(), store: store.clone(), tx: tx.clone() };

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

/// Advances the virtual clock, feeds any due reader events through the domain and
/// broadcasts the resulting snapshot.
async fn tick_loop(app: AppState) {
    let started_wall = wall_clock_ms();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        ticker.tick().await;
        let now = {
            let hub = app.hub.lock().expect("hub mutex poisoned");
            Instant(
                hub.class_start.0 + hub.resume_offset + (wall_clock_ms() - started_wall) * SPEED,
            )
        };

        let payload = {
            // Pull the due events out under the lock, then release it for the awaits below.
            let due: Vec<(usize, domain::ReaderBinding, Instant, i64)> = {
                let mut hub = app.hub.lock().expect("hub mutex poisoned");
                let mut out = Vec::new();
                while hub.cursor < hub.script.len() && hub.script[hub.cursor].at <= now {
                    let ev = &hub.script[hub.cursor];
                    out.push((ev.athlete, ev.binding.clone(), ev.at, hub.cursor as i64 + 1));
                    hub.cursor += 1;
                }
                out
            };

            for (athlete_idx, binding, at, sequence) in due {
                let (session_id, athlete_id) = {
                    let hub = app.hub.lock().expect("hub mutex poisoned");
                    (hub.session.id.clone(), hub.athletes[athlete_idx].athlete_id.clone())
                };
                let raw = RawEvent {
                    device_id: "esp32-devfeeder".into(),
                    reader_id: format!("rfid-{:02}", athlete_idx + 1),
                    boot_id: 1,
                    sequence,
                    tag_id: format!("TAG-{athlete_id}"),
                    detected_at: at,
                    received_at: Instant(wall_clock_ms()),
                };

                // Persist before interpreting. A redelivery returns inserted = false, which is
                // how a resumed session skips work it already did (CLAUDE.md 16).
                let (raw_id, inserted) = match app.store.save_raw(&raw).await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("raw persist failed, event not interpreted: {e}");
                        continue;
                    }
                };
                if !inserted {
                    continue;
                }

                let (event, session_snapshot) = {
                    let mut hub = app.hub.lock().expect("hub mutex poisoned");
                    let session = hub.session.clone();
                    let event = interpret(&mut hub.athletes[athlete_idx], &binding, at, &session);
                    match event {
                        Interpreted::Exception { .. } => hub.exceptions += 1,
                        _ => hub.session.interpreted_event_count += 1,
                    }
                    let snap = hub.session.clone();
                    (event, snap)
                };

                if let Err(e) = app
                    .store
                    .save_interpreted(&session_id, &athlete_id, Some(raw_id), &event)
                    .await
                {
                    eprintln!("interpreted persist failed: {e}");
                }
                let class_start = { app.hub.lock().expect("hub mutex poisoned").class_start };
                if let Err(e) = app.store.save_session(&session_snapshot, class_start).await {
                    eprintln!("session persist failed: {e}");
                }
            }

            let hub = app.hub.lock().expect("hub mutex poisoned");
            let snap = state::snapshot(
                &hub.session,
                &hub.athletes,
                &hub.course,
                now,
                hub.class_start,
                8,
                hub.exceptions,
            );
            serde_json::to_string(&snap).expect("snapshot must serialise")
        };

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
