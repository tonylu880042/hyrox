//! `/operator`: the write surface (ADR 0001).
//!
//! Session control, configuration, the reader map and the exception inbox. Every route here
//! takes an [`OperatorDevice`], so no write can reach the audit trail without a name on it
//! (D1), and every handler is the same three steps: parse the request, call one use case,
//! turn its answer into a status code. The rules -- which transitions are legal, when a
//! reason is required, whether a class may be ended by hand -- are all below this layer
//! (CLAUDE.md 29).

use crate::checkin::command;
use crate::error::ApiError;
use crate::identity::{Body, OperatorDevice};
use crate::read::freshness;
use crate::state::Operator;
use crate::wire::{
    ConfigureRequest, DeviceView, EndClassResponse, ExceptionView, ExceptionsResponse,
    OverviewResponse, ReadersResponse, ReasonRequest, RegisterReaderRequest,
};
use application::HubStore;
use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use domain::{ReaderKey, ReaderRegistration};
use std::fmt::Display;

pub fn router<S>(state: Operator<S>) -> Router
where
    S: HubStore + Send + Sync + 'static,
    S::Error: Display + Send,
{
    Router::new()
        .route("/", get(overview))
        .route("/readers", get(readers).post(register_reader))
        .route("/config", put(configure))
        .route("/exceptions", get(exceptions))
        .route("/exceptions/{interpreted_event_id}/void", post(void))
        .route("/session/arm", post(arm))
        .route("/session/close", post(close))
        .route("/session/reopen", post(reopen))
        .route("/session/draft", post(return_to_draft))
        .route("/session/end-class", post(end_class))
        .with_state(state)
}

/// Everything the operator screen keeps on display: the session, its configuration, the
/// reader map with per-reader freshness (ADR 0001 D5), device health, and the two badges.
async fn overview<S>(State(operator): State<Operator<S>>) -> Json<OverviewResponse> {
    let read = operator.read();
    let now = read.now();
    let snapshot = read.snapshot().await;
    let (session, config, class_elapsed_ms) = read.session().await;
    Json(OverviewResponse {
        freshness: crate::read::freshness_from(read, snapshot.last_event_age_ms),
        config_editable: session.accepts_config_edits(),
        session,
        config,
        class_elapsed_ms,
        readers: read.readers().await,
        devices: read
            .devices()
            .await
            .iter()
            .map(|h| DeviceView::of(h, now))
            .collect(),
        exceptions: snapshot.exceptions,
        pending_tags: snapshot.pending_tags,
    })
}

async fn readers<S>(State(operator): State<Operator<S>>) -> Json<ReadersResponse> {
    let read = operator.read();
    Json(ReadersResponse {
        freshness: freshness(read).await,
        readers: read.readers().await,
    })
}

/// Registers or repoints one reader (CLAUDE.md 8).
///
/// There is deliberately no removal route. No use case deletes a reader, and inventing one
/// here would mean deciding what becomes of the events already attributed through it --
/// which is a product rule nobody has made (CLAUDE.md 28). Re-registering the same
/// `(device_id, reader_id)` replaces its mapping, which is what repointing a reader is.
async fn register_reader<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<RegisterReaderRequest>,
) -> Result<Json<ReadersResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let now = operator.read().now();
    let key = ReaderKey::parse(&request.device_id, &request.reader_id)
        .map_err(|e| ApiError::invalid_body(format!("reader key: {e:?}")))?;
    let mut registration = ReaderRegistration::new(key, request.station, request.mode);
    registration.zone = request.zone;

    let cmd = command(device, now, request.reason);
    operator.register_reader(&registration, &cmd).await?;
    Ok(Json(ReadersResponse {
        freshness: freshness(operator.read()).await,
        readers: operator.read().readers().await,
    }))
}

/// Edits the course and the finish rule. DRAFT only (ADR 0001 D2) -- the use case refuses
/// anything else, and the refusal comes back as 409, never 500.
async fn configure<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<ConfigureRequest>,
) -> Result<Json<crate::wire::SessionResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let now = operator.read().now();
    let cmd = command(device, now, request.reason);
    operator
        .configure(request.course, request.finish_policy, &cmd)
        .await?;

    let (session, config, class_elapsed_ms) = operator.read().session().await;
    Ok(Json(crate::wire::SessionResponse {
        freshness: freshness(operator.read()).await,
        config_editable: session.accepts_config_edits(),
        session,
        config,
        class_elapsed_ms,
    }))
}

/// The exception inbox (ADR 0001 D4). Read from the store, so it survives the process that
/// produced it.
async fn exceptions<S>(
    State(operator): State<Operator<S>>,
) -> Result<Json<ExceptionsResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let now = operator.read().now();
    let stored = operator.exceptions().await?;
    Ok(Json(ExceptionsResponse {
        freshness: freshness(operator.read()).await,
        exceptions: stored.iter().map(|e| ExceptionView::of(e, now)).collect(),
    }))
}

/// Voids one interpretation and lets everything derived from it be recomputed
/// (CLAUDE.md 20). The raw read is untouched (CLAUDE.md 19).
///
/// Destructive, so the use case demands a reason and answers 422 without one. *Accept
/// as-is* and *reinterpret*, D4's other two actions, have no use case yet: see
/// `docs/open-issues.md`. They are missing rather than half-built here.
async fn void<S>(
    State(operator): State<Operator<S>>,
    Path(interpreted_event_id): Path<i64>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<ReasonRequest>,
) -> Result<Json<ExceptionsResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let now = operator.read().now();
    let cmd = command(device, now, request.reason);
    operator.void_exception(interpreted_event_id, &cmd).await?;
    exceptions(State(operator)).await
}

async fn arm<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<ReasonRequest>,
) -> Result<Json<crate::wire::SessionResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), request.reason);
    operator.arm(&cmd).await?;
    session_response(operator).await
}

async fn close<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<ReasonRequest>,
) -> Result<Json<crate::wire::SessionResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), request.reason);
    operator.close(&cmd).await?;
    session_response(operator).await
}

/// CLOSED -> ARMED, deliberately allowed and deliberately requiring a reason (ADR 0001 D2).
/// A mis-tap on a busy floor must not force a new session, and there is no time window on
/// it -- a window would be a magic constant nobody validated (CLAUDE.md 29).
async fn reopen<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<ReasonRequest>,
) -> Result<Json<crate::wire::SessionResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), request.reason);
    operator.reopen(&cmd).await?;
    session_response(operator).await
}

/// ARMED -> DRAFT, only while nothing has been interpreted (ADR 0001 D2). 409 otherwise.
async fn return_to_draft<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<ReasonRequest>,
) -> Result<Json<crate::wire::SessionResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), request.reason);
    operator.return_to_draft(&cmd).await?;
    session_response(operator).await
}

/// The coach ends the class by hand: everyone still running stops, and the session closes.
///
/// 409 `NO_FINISH_RULE` where the session has no finish rule, which is competition
/// (CLAUDE.md 12, 28). A button that stopped every competitor's clock would be exactly the
/// invented rule the project forbids.
async fn end_class<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<ReasonRequest>,
) -> Result<Json<EndClassResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), request.reason);
    let finished = operator.end_class(&cmd).await?;
    Ok(Json(EndClassResponse {
        freshness: freshness(operator.read()).await,
        finished,
    }))
}

/// The session as it now stands, which is what every lifecycle write answers with: the
/// screen that pressed the button never has to re-derive the new state.
async fn session_response<S>(
    operator: Operator<S>,
) -> Result<Json<crate::wire::SessionResponse>, ApiError> {
    let read = operator.read();
    let (session, config, class_elapsed_ms) = read.session().await;
    Ok(Json(crate::wire::SessionResponse {
        freshness: freshness(read).await,
        config_editable: session.accepts_config_edits(),
        session,
        config,
        class_elapsed_ms,
    }))
}
