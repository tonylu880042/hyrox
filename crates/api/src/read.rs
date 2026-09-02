//! The read-only surfaces: `/coach`, `/live`, `/result/{id}` (ADR 0001).
//!
//! Three things make this module structurally read-only, and a reviewer needs none of the
//! handler bodies to check any of them:
//!
//! 1. Its state is [`ReadOnly`], whose store and live session are private and have no
//!    accessor. There is no expression in this file that can reach `HubStore`'s writes or
//!    `&mut LiveSession`; a write attempted here does not compile.
//! 2. The only routing verb imported is `get`. `post`, `put` and `delete` are not in scope.
//! 3. The routes live at paths the write routers do not touch, so a read-only screen's URL
//!    space contains no mutating route at all -- a `POST` to one of these paths is a 405
//!    from axum's own method router, before any code of ours runs.
//!
//! Why it matters (ADR 0001): read-only screens get handed to coaches and athletes on their
//! own phones. The fewer entrances that can change data, the smaller the surface anyone has
//! to audit after a confusing evening.

use crate::error::{storage, ApiError};
use crate::state::ReadOnly;
use crate::wire::{
    AthleteStages, CoachResponse, EntryResponse, ExercisesResponse, Freshness, LiveResponse,
    SettingsResponse,
    ResultResponse, LeaderboardResponse, SessionResponse, StagesResponse, TemplateResponse,
    TemplatesResponse,
};
use domain::EntryCode;
use application::Health;
use application::HubStore;
use axum::extract::{
    ws::{Message, WebSocket, WebSocketUpgrade},
    Path, State,
};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use std::fmt::Display;

/// Where the live snapshot stream lives. One constant because it is both a route and a
/// value published in every [`Freshness`], and the two must not drift.
pub const WEBSOCKET_PATH: &str = "/ws";

pub fn router<S>(state: ReadOnly<S>) -> Router
where
    S: HubStore + Send + Sync + 'static,
    S::Error: Display + Send,
{
    Router::new()
        .route(WEBSOCKET_PATH, get(websocket))
        .route("/api/live", get(live))
        .route("/api/coach", get(coach))
        .route("/api/session", get(session))
        .route("/api/result/{session_id}", get(result))
        // The workout library is read here and written under /api/operator, so the
        // read/write split stays structural: these paths carry no mutating verb at all
        // (ADR 0007 §5).
        .route("/api/exercises", get(exercises))
        .route("/api/workout-templates", get(templates))
        .route("/api/workout-templates/{template_id}", get(template))
        .route("/api/stages", get(stages))
        // Read by the appliance's maintenance window, and by the operator screen's update
        // badge (ADR 0009). Read-only, like everything else on this surface.
        .route("/api/health", get(health))
        .route("/api/leaderboard", get(leaderboard))
        // An entrant's own two reads: who am I, and how did I do (ADR 0011). They are read
        // from a phone the venue does not control, so they live here and nowhere else.
        // Read by the projector itself, which carries no operator name: it is a screen.
        .route("/api/settings", get(settings))
        // The venue's own logo. Read by every screen, including the projector, which has
        // no operator identity -- it is a wall.
        .route("/api/logo", get(logo))
        .route("/api/entry/{code}", get(entry))
        .route("/api/entry/{code}/qr.svg", get(entry_qr))
        .with_state(state)
}

/// Builds the mandatory freshness readout (ADR 0001 D5).
///
/// Shared by every surface, read and write alike, so the number a coach sees and the number
/// an operator sees are the same number.
pub async fn freshness<S>(read: &ReadOnly<S>) -> Freshness {
    Freshness {
        now: read.now().0,
        last_event_age_ms: read.last_event_age_ms().await,
        websocket_path: WEBSOCKET_PATH,
        push_interval_ms: read.push_interval_ms(),
        subscribers: read.subscribers(),
    }
}

/// The same freshness, when the caller already holds the snapshot it would be derived from.
pub fn freshness_from<S>(read: &ReadOnly<S>, last_event_age_ms: Option<i64>) -> Freshness {
    Freshness {
        now: read.now().0,
        last_event_age_ms,
        websocket_path: WEBSOCKET_PATH,
        push_interval_ms: read.push_interval_ms(),
        subscribers: read.subscribers(),
    }
}

async fn live<S>(State(read): State<ReadOnly<S>>) -> Json<LiveResponse> {
    let snapshot = read.snapshot().await;
    Json(LiveResponse {
        freshness: freshness_from(&read, snapshot.last_event_age_ms),
        snapshot,
    })
}

/// CLAUDE.md 23's coach view: the athlete rows, plus the device and reader warnings the
/// same section asks for.
async fn coach<S>(State(read): State<ReadOnly<S>>) -> Json<CoachResponse> {
    let snapshot = read.snapshot().await;
    Json(CoachResponse {
        freshness: freshness_from(&read, snapshot.last_event_age_ms),
        readers: read.readers().await,
        snapshot,
    })
}

async fn session<S>(State(read): State<ReadOnly<S>>) -> Json<SessionResponse> {
    let (session, config, class_elapsed_ms) = read.session().await;
    Json(SessionResponse {
        freshness: freshness(&read).await,
        config_editable: session.accepts_config_edits(),
        session,
        config,
        class_elapsed_ms,
    })
}

/// Results for one stored session, running or long finished.
///
/// 404 when the store has no such session, which is a different answer from a session that
/// exists and has no rows yet -- the second is a class where nobody has scanned in.
async fn result<S>(
    State(read): State<ReadOnly<S>>,
    Path(session_id): Path<String>,
) -> Result<Json<ResultResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let results = read
        .results(&session_id)
        .await
        .map_err(storage)?
        .ok_or_else(|| {
            ApiError::not_found("UNKNOWN_SESSION", format!("no session {session_id:?}"))
        })?;
    Ok(Json(ResultResponse { freshness: freshness(&read).await, results }))
}

/// The live snapshot stream every screen listens on.
///
/// Push, never poll (CLAUDE.md 23). The frames are `application::Snapshot` documents,
/// already serialised by whoever produced them, so a hundred connected phones cost one
/// serialisation rather than a hundred.
async fn websocket<S: Send + Sync + 'static>(
    ws: WebSocketUpgrade,
    State(read): State<ReadOnly<S>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| push_snapshots(socket, read))
}

async fn push_snapshots<S>(mut socket: WebSocket, read: ReadOnly<S>) {
    let mut rx = read.subscribe();
    // A slow client that lagged past the channel's capacity is dropped rather than served
    // stale frames: on a live screen, an old snapshot is worse than a visibly closed socket
    // (ADR 0001 D5).
    while let Ok(payload) = rx.recv().await {
        if socket.send(Message::Text(payload.into())).await.is_err() {
            break; // the client went away
        }
    }
}

// --- the workout library (ADR 0008) -----------------------------------------------------

/// The exercises a builder screen may choose from (workout brief §3, §17).
async fn exercises<S>(State(read): State<ReadOnly<S>>) -> Result<Json<ExercisesResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let exercises = read.exercises().await.map_err(storage)?;
    Ok(Json(ExercisesResponse {
        freshness: freshness(&read).await,
        exercises: exercises.iter().cloned().collect(),
    }))
}

/// Every template, system ones first (workout brief §13).
async fn templates<S>(State(read): State<ReadOnly<S>>) -> Result<Json<TemplatesResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let templates = read.templates().await.map_err(storage)?;
    Ok(Json(TemplatesResponse { freshness: freshness(&read).await, templates }))
}

async fn template<S>(
    State(read): State<ReadOnly<S>>,
    Path(template_id): Path<String>,
) -> Result<Json<TemplateResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let template = read
        .template(&template_id)
        .await
        .map_err(storage)?
        .ok_or_else(|| {
            ApiError::not_found("UNKNOWN_TEMPLATE", format!("no template with id {template_id:?}"))
        })?;
    Ok(Json(TemplateResponse { freshness: freshness(&read).await, template }))
}

/// Every athlete's progress through the class, stage by stage (workout brief §10).
///
/// Derived on each request from the snapshot course and the replayed runs, never stored, so
/// an operator's void changes it for free (CLAUDE.md 21).
async fn stages<S>(State(read): State<ReadOnly<S>>) -> Json<StagesResponse>
where
    S: HubStore,
{
    let athletes = read
        .stages()
        .await
        .into_iter()
        .map(|p| AthleteStages {
            athlete_id: p.athlete_id,
            name: p.name,
            current_stage: application::current_stage(&p.stages),
            expectation: p.expectation,
            stages: p.stages,
        })
        .collect();
    Json(StagesResponse { freshness: freshness(&read).await, athletes })
}

/// Whether this machine may be stopped right now (ADR 0009 §6).
///
/// The nightly maintenance window asks this before it updates and powers the machine off,
/// and does nothing on a `false`. Deliberately outside the `freshness` envelope every other
/// read carries: this is consumed by a shell script, and `safe_to_stop` must be the whole
/// answer without a client having to understand the rest of the API.
async fn health<S>(State(read): State<ReadOnly<S>>) -> Json<Health>
where
    S: HubStore,
{
    Json(read.health().await)
}

/// The running session's results, ranked where the finish rule allows it (ADR 0010).
///
/// A separate route from `/api/result/{id}` so the leaderboard screen does not have to ask
/// which session is on before it can show anything -- on a competition floor that is one
/// more round trip between a finish and the name appearing.
async fn leaderboard<S>(State(read): State<ReadOnly<S>>) -> Json<LeaderboardResponse>
where
    S: HubStore,
{
    Json(LeaderboardResponse {
        results: read.leaderboard().await,
        freshness: freshness(&read).await,
    })
}

/// One entrant's own view of themselves: their name, their bib, and their row of today's
/// results (ADR 0011).
///
/// The code in the path is the athlete id, so no lookup table is needed and nothing can
/// drift out of step. Parsed rather than compared raw, because somebody typing it off a
/// phone types `O` for `0`.
///
/// Scoped to the class on now: an entry code is issued for a session, and a code that is
/// not on today's roster is a `404` rather than an empty answer that reads like "you have
/// no result yet".
async fn entry<S>(
    State(read): State<ReadOnly<S>>,
    Path(code): Path<String>,
) -> Result<Json<EntryResponse>, ApiError>
where
    S: HubStore,
{
    let code = parse_code(&code)?;
    let results = read.leaderboard().await;
    let row = results
        .rows
        .iter()
        .find(|r| r.athlete_id == code.as_str())
        .cloned()
        .ok_or_else(|| {
            ApiError::not_found("UNKNOWN_ENTRY", format!("no entrant {code} in this class"))
        })?;
    Ok(Json(EntryResponse {
        freshness: freshness(&read).await,
        code: code.to_string(),
        session_name: results.session_name.clone(),
        session_status: results.status,
        ordering: results.ordering,
        course_length: results.course.len(),
        row,
    }))
}

/// The entrant's QR, drawn by the hub.
///
/// It carries the six characters and nothing else -- not a URL. A desk scanner is a
/// keyboard: scanning has to type a code into a search box, and a scanner that types a
/// whole URL would be useless there (ADR 0011).
///
/// SVG rather than PNG so it stays sharp on any phone, and served from the hub like every
/// other asset, because a venue with no internet must still be able to hand out entries.
async fn entry_qr(Path(code): Path<String>) -> Result<impl IntoResponse, ApiError> {
    let code = parse_code(code.trim_end_matches(".svg"))?;
    let svg = qrcode::QrCode::new(code.as_str().as_bytes())
        .map_err(|e| ApiError::invalid_body(format!("cannot encode {code}: {e}")))?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .build();
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        svg,
    ))
}

fn parse_code(raw: &str) -> Result<EntryCode, ApiError> {
    EntryCode::parse(raw).map_err(|e| ApiError::invalid_body(format!("{raw:?}: {e}")))
}

/// What this venue has chosen for itself (M6 follow-up).
///
/// On the read surface because the live screen is the main reader and it has no operator
/// identity -- it is a screen on a wall. Writing one is `PUT /api/operator/settings`.
async fn settings<S>(State(read): State<ReadOnly<S>>) -> Result<Json<SettingsResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let settings = read.venue_settings().await.map_err(storage)?;
    Ok(Json(SettingsResponse {
        freshness: freshness(&read).await,
        live_page_ms: settings.live_page_ms,
        live_page_size: settings.live_page_size,
        demo_available: read.demo_available(),
        page_layouts: crate::wire::layouts(),
    }))
}

/// The venue's logo, or a plain 404 where none was uploaded.
///
/// 404 rather than a blank image: a screen has to be able to tell "this gym has no logo"
/// from "the logo failed to load", and lay itself out accordingly.
///
/// Served with the type that was recorded when it was accepted, never one guessed here, and
/// with `nosniff` so a browser cannot decide it is something more interesting than a
/// picture.
async fn logo<S>(State(read): State<ReadOnly<S>>) -> Result<impl IntoResponse, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let asset = read
        .venue_asset(application::VENUE_LOGO)
        .await
        .map_err(storage)?
        .ok_or_else(|| ApiError::not_found("NO_LOGO", "this venue has not uploaded a logo"))?;
    Ok((
        [
            (axum::http::header::CONTENT_TYPE, asset.media_type),
            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            // Re-read rather than cached: a logo changed on a tablet has to reach the
            // projector, and the file is tens of kilobytes on a local network.
            (axum::http::header::CACHE_CONTROL, "no-cache".to_string()),
        ],
        asset.bytes,
    ))
}
