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
    ConfigureRequest, CreateClassRequest, DeviceView, DuplicateTemplateRequest, EndClassResponse,
    ExceptionView, ExceptionsResponse, OverviewResponse, ReadersResponse, ReasonRequest,
    RegisterReaderRequest, SaveTemplateRequest, TemplatesResponse,
};
use application::HubStore;
use axum::extract::{Path, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use domain::{Instant, ReaderKey, ReaderRegistration, WorkoutTemplate};
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
        .route("/session/ready", post(mark_ready))
        .route("/session/start", post(start))
        .route("/session/pause", post(pause))
        .route("/session/resume", post(resume))
        .route("/session/complete", post(complete))
        .route("/session/cancel", post(cancel))
        .route("/session/reopen", post(reopen))
        .route("/session/draft", post(return_to_draft))
        .route("/session/end-class", post(end_class))
        // The workout library's writes. Reads of the same resources live on the read-only
        // surface at /api/workout-templates (ADR 0007 §5).
        .route("/templates", post(save_template))
        .route("/templates/{template_id}", put(save_template_at).delete(delete_template))
        .route("/templates/{template_id}/duplicate", post(duplicate_template))
        .route("/class", post(create_class))
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

/// The session lifecycle routes (ADR 0008). Every one of them is the same four lines --
/// take the operator's identity, build the command, call the use case, answer with the new
/// session view -- so they are generated rather than copied eight times. The *rules* are in
/// `domain::Session` and the *audit* is in `application::session`; a handler that grew a
/// difference from its siblings would be a rule in an HTTP handler (CLAUDE.md 29).
macro_rules! session_route {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        async fn $name<S>(
            State(operator): State<Operator<S>>,
            OperatorDevice(device): OperatorDevice,
            Body(request): Body<ReasonRequest>,
        ) -> Result<Json<crate::wire::SessionResponse>, ApiError>
        where
            S: HubStore,
            S::Error: Display,
        {
            let cmd = command(device, operator.read().now(), request.reason);
            operator.$name(&cmd).await?;
            session_response(operator).await
        }
    };
}

session_route!(
    /// DRAFT -> READY. The class is built; it stays editable until it starts.
    mark_ready
);
session_route!(
    /// READY -> RUNNING. From here the first valid read starts an athlete's clock.
    start
);
session_route!(
    /// RUNNING -> PAUSED. The class clock freezes and reads become exceptions (ADR 0008).
    pause
);
session_route!(resume);
session_route!(complete);
session_route!(
    /// The class did not happen. Destructive, so `422 REASON_REQUIRED` without a reason.
    cancel
);
session_route!(
    /// COMPLETED -> RUNNING, deliberately allowed and deliberately requiring a reason
    /// (ADR 0001 D2). A mis-tap on a busy floor must not force a new session, and there is
    /// no time window on it -- a window would be a magic constant nobody validated
    /// (CLAUDE.md 29). A CANCELLED session is not reopened: `409 ILLEGAL_TRANSITION`.
    reopen
);
session_route!(
    /// Back to DRAFT, only while nothing has been interpreted (ADR 0001 D2). 409 otherwise.
    return_to_draft
);

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

// --- the workout library (ADR 0008) -----------------------------------------------------

/// Creates or replaces a coach's template (workout brief §14).
///
/// The version is not taken from the request: the use case reads what is stored and moves it
/// on. A client cannot pin, rewind or skip a version, which is what makes "which plan did
/// Friday's class run?" answerable.
async fn save_template<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<SaveTemplateRequest>,
) -> Result<Json<TemplatesResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), request.reason.clone());
    operator.save_template(WorkoutTemplate::from(request), &cmd).await?;
    templates_response(operator).await
}

/// The same write addressed by path. `PUT /templates/{id}` is what a builder screen saving
/// an open template naturally sends; the id in the path wins over the one in the body, so a
/// mismatched payload cannot write to a template the URL did not name.
async fn save_template_at<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Path(template_id): Path<String>,
    Body(request): Body<SaveTemplateRequest>,
) -> Result<Json<TemplatesResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), request.reason.clone());
    let mut template = WorkoutTemplate::from(request);
    template.id = template_id;
    operator.save_template(template, &cmd).await?;
    templates_response(operator).await
}

/// A coach's own copy of any template, system ones included (workout brief §4, scenario A).
async fn duplicate_template<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Path(template_id): Path<String>,
    Body(request): Body<DuplicateTemplateRequest>,
) -> Result<Json<TemplatesResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), request.reason);
    operator
        .duplicate_template(
            &template_id,
            &request.new_id,
            &request.name,
            request.owner_id.as_deref(),
            &cmd,
        )
        .await?;
    templates_response(operator).await
}

/// Deletes a coach's template. Destructive, so `422 REASON_REQUIRED` without a reason, and
/// system templates are refused outright (workout brief §13).
async fn delete_template<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Path(template_id): Path<String>,
    Body(request): Body<ReasonRequest>,
) -> Result<Json<TemplatesResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), request.reason);
    operator.delete_template(&template_id, &cmd).await?;
    templates_response(operator).await
}

/// Template -> compiled course -> snapshot -> DRAFT class (workout brief §15).
///
/// The class comes back DRAFT, not running: today's tweaks go through
/// `PUT /api/operator/config` next, and they land on this class's own snapshot.
async fn create_class<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<CreateClassRequest>,
) -> Result<Json<crate::wire::SessionResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), request.reason);
    let new = application::NewClass {
        session_id: request.session_id,
        name: request.name,
        mode: request.mode,
        coach_id: request.coach_id,
        scheduled_at: request.scheduled_at.map(Instant),
        finish_policy: request.finish_policy,
        roster: request
            .athletes
            .into_iter()
            .map(|a| application::RosterEntry {
                athlete_id: a.athlete_id,
                display_name: a.display_name,
            })
            .collect(),
        created_at: operator.read().now(),
    };
    operator.create_class(&request.template_id, new, &cmd).await?;
    session_response(operator).await
}

async fn templates_response<S>(operator: Operator<S>) -> Result<Json<TemplatesResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let read = operator.read();
    let templates = read.templates().await.map_err(crate::error::storage)?;
    Ok(Json(TemplatesResponse { freshness: freshness(read).await, templates }))
}
