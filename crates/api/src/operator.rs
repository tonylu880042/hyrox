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
    ChangePinRequest, ConfigureRequest, CreateClassRequest, DeviceView, DuplicateTemplateRequest,
    EndClassResponse, ExceptionView, ExceptionsResponse, OverviewResponse, ReadersResponse,
    ReasonRequest, RegisterReaderRequest, ReinterpretRequest, SaveTemplateRequest,
    TemplatesResponse, VerifyPinRequest, VerifyPinResponse,
};
use application::HubStore;
use axum::extract::{Path, State};
use axum::routing::{delete, get, post, put};
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
        // The non-destructive half of the pair (ADR 0001 D4).
        .route("/exceptions/{interpreted_event_id}/accept", post(accept))
        // The corrective action (ADR 0001 D4).
        .route(
            "/exceptions/{interpreted_event_id}/reinterpret",
            post(reinterpret),
        )
        // The nightly window asks for this rather than copying the file itself: the hub is
        // the only process allowed to touch the database (ADR 0009, 0012).
        .route("/backup", post(backup))
        // The settings screen's two additions (M6): what the machine can be asked to do,
        // and which readers are still waiting to be told what they are.
        .route("/power", post(power))
        .route("/settings", put(settings))
        .route("/pin/verify", post(verify_pin))
        .route("/pin/change", post(change_pin))
        .route("/logo", post(upload_logo).delete(remove_logo))
        // Demo data (M6 follow-up). Present on every build; whether it does anything is
        // the machine's answer, not this router's.
        .route("/demo", post(load_demo).delete(clear_demo))
        .route("/readers/unregistered", get(unregistered))
        // Taking one off the wall (ADR 0007 §7, amended). In the path rather than the body
        // because it names a thing that exists, and DELETE has no body worth parsing --
        // except the reason, which every destructive action carries.
        .route("/readers/{device_id}/{reader_id}", delete(remove_reader))
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
        .route(
            "/templates/{template_id}",
            put(save_template_at).delete(delete_template),
        )
        .route(
            "/templates/{template_id}/duplicate",
            post(duplicate_template),
        )
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
/// Destructive, so the use case demands a reason and answers 422 without one.
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

/// Accepts one exception as it stands: out of the inbox, still in the log (ADR 0001 D4).
///
/// A reason is optional here, unlike voiding. Nothing is removed and no result changes, so
/// requiring one would buy a trail of "ok" rather than a trail worth reading.
async fn accept<S>(
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
    operator
        .accept_exception(interpreted_event_id, &cmd)
        .await?;
    exceptions(State(operator)).await
}

/// Reinterprets one exception into a valid station reading (ADR 0001 D4; CLAUDE.md 20).
async fn reinterpret<S>(
    State(operator): State<Operator<S>>,
    Path(interpreted_event_id): Path<i64>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<ReinterpretRequest>,
) -> Result<Json<ExceptionsResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let now = operator.read().now();
    let cmd = command(device, now, request.reason);
    let spec = application::ReinterpretSpec {
        station: request.station,
        mode: request.mode,
        athlete_id: request.athlete_id,
        at: request.at.map(domain::Instant),
    };
    operator
        .reinterpret_exception(interpreted_event_id, spec, &cmd)
        .await?;
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
    operator
        .save_template(WorkoutTemplate::from(request), &cmd)
        .await?;
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
    operator
        .create_class(&request.template_id, new, &cmd)
        .await?;
    session_response(operator).await
}

async fn templates_response<S>(operator: Operator<S>) -> Result<Json<TemplatesResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let read = operator.read();
    let templates = read.templates().await.map_err(crate::error::storage)?;
    Ok(Json(TemplatesResponse {
        freshness: freshness(read).await,
        templates,
    }))
}

/// Takes a backup of the whole database and says where it went (ADR 0012).
///
/// A write, so it carries the operator's device name and is audited like any other -- the
/// caller is usually the nightly maintenance unit, and "who asked for this file" is worth
/// having when somebody restores it months later.
///
/// The hub does the copying because it is the only process that may touch the database
/// (ADR 0009). A shell script running `cp` on a live SQLite file is the classic way to
/// produce a backup that is missing the transactions in the `-wal`, or is simply corrupt.
async fn backup<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<crate::wire::BackupRequest>,
) -> Result<Json<crate::wire::BackupResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let now = operator.read().now();
    let reason = request.reason.clone();
    let cmd = command(device, now, request.reason);
    let path = operator.backup(&cmd, reason).await?;
    Ok(Json(crate::wire::BackupResponse {
        freshness: crate::read::freshness(operator.read()).await,
        path: path.display().to_string(),
        at: cmd.at.0,
    }))
}

/// Switch the machine off, restart it, or restart the hub's own service (M6).
///
/// Guarded by the same question the nightly window asks: a class on the floor wins. The
/// guard lives in [`Operator::power`] rather than here, so it holds for any caller.
async fn power<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<crate::wire::PowerRequest>,
) -> Result<Json<crate::wire::PowerResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let now = operator.read().now();
    let reason = request.reason.clone();
    let cmd = command(device, now, request.reason);
    // Checked here rather than in the port: the reason is for the audit row, and the audit
    // row is written before the machine is asked to do anything.
    if reason.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err(ApiError::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "REASON_REQUIRED",
            "switching the machine off stops a venue's evening, so it needs a reason",
        ));
    }
    operator.power(request.action, &cmd, reason).await?;
    Ok(Json(crate::wire::PowerResponse {
        freshness: crate::read::freshness(operator.read()).await,
        action: request.action,
        at: now.0,
    }))
}

/// Readers the hub has heard from and cannot resolve, most recently tapped first (M6).
///
/// This is how a venue is installed: tap an antenna with any band, and it appears here.
/// Nobody copies a MAC address off a sticker.
async fn unregistered<S>(
    State(operator): State<Operator<S>>,
) -> Result<Json<crate::wire::UnregisteredReadersResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let readers = operator.unregistered_readers().await?;
    Ok(Json(crate::wire::UnregisteredReadersResponse {
        freshness: crate::read::freshness(operator.read()).await,
        readers: readers
            .into_iter()
            .map(|r| crate::wire::UnregisteredReader {
                device_id: r.device_id,
                reader_id: r.reader_id,
                last_seen: r.last_seen.0,
                reads: r.reads,
            })
            .collect(),
    }))
}

/// Change one of the venue's own numbers (M6 follow-up).
///
/// A write like any other: it carries the operator's device name and lands in the audit
/// trail. Only the settings this build defines are accepted -- an unknown key is a typo,
/// and a stored typo is a setting somebody will swear they changed.
async fn settings<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<crate::wire::SettingsRequest>,
) -> Result<Json<crate::wire::SettingsResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), None);
    if let Some(ms) = request.live_page_ms {
        operator
            .save_setting(application::LIVE_PAGE_MS, &ms.to_string(), &cmd)
            .await?;
    }
    if let Some(size) = request.live_page_size {
        operator
            .save_setting(application::LIVE_PAGE_SIZE, &size.to_string(), &cmd)
            .await?;
    }
    let settings = operator
        .read()
        .venue_settings()
        .await
        .map_err(crate::error::storage)?;
    Ok(Json(crate::wire::SettingsResponse {
        freshness: crate::read::freshness(operator.read()).await,
        live_page_ms: settings.live_page_ms,
        live_page_size: settings.live_page_size,
        demo_available: operator.read().demo_available(),
        page_layouts: crate::wire::layouts(),
    }))
}

/// Upload the venue's logo (M6 follow-up).
///
/// Raw bytes rather than a JSON envelope or a multipart form: one file, one request, and
/// nothing to parse before the only check that matters -- what the bytes actually are.
async fn upload_logo<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    bytes: axum::body::Bytes,
) -> Result<Json<crate::wire::LogoResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), None);
    let asset = operator.save_logo(bytes.to_vec(), &cmd).await?;
    Ok(Json(crate::wire::LogoResponse {
        freshness: crate::read::freshness(operator.read()).await,
        media_type: asset.media_type,
        bytes: asset.bytes.len(),
    }))
}

/// Remove it. The screens go back to leading with the class, which is what they did before.
async fn remove_logo<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
) -> Result<Json<crate::wire::LogoResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), None);
    operator.remove_logo(&cmd).await?;
    Ok(Json(crate::wire::LogoResponse {
        freshness: crate::read::freshness(operator.read()).await,
        media_type: String::new(),
        bytes: 0,
    }))
}

/// Forget one reader's mapping (ADR 0007 §7, amended 2026-09-02).
///
/// Nothing already recorded moves: `raw_events` keeps the device and reader behind every
/// read, and an interpretation names the station rather than the reader. What changes is
/// what happens next -- reads from this antenna become `UNKNOWN_READER` exceptions, which
/// land in the inbox rather than disappearing.
async fn remove_reader<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Path((device_id, reader_id)): Path<(String, String)>,
    Body(request): Body<crate::wire::ReaderRemovalRequest>,
) -> Result<Json<crate::wire::ReadersResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let key = domain::ReaderKey::parse(&device_id, &reader_id)
        .map_err(|e| ApiError::invalid_body(format!("{device_id} {reader_id}: {e:?}")))?;
    let cmd = command(device, operator.read().now(), request.reason);
    operator.unregister_reader(&key, &cmd).await?;
    Ok(Json(crate::wire::ReadersResponse {
        freshness: crate::read::freshness(operator.read()).await,
        readers: operator.read().readers().await,
    }))
}

/// Fill the hub with a venue's worth of demo data and start producing reads (M6 follow-up).
///
/// Guarded like the power buttons: **a class on the floor wins**. Loading twelve invented
/// athletes into somebody's evening would be indistinguishable from a bug.
async fn load_demo<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
) -> Result<Json<crate::wire::DemoResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), None);
    operator.load_demo(&cmd).await?;
    Ok(Json(crate::wire::DemoResponse {
        freshness: crate::read::freshness(operator.read()).await,
        loaded: true,
    }))
}

/// Stop the invented reads and stand the demo class down.
///
/// Allowed at any time, unlike loading: a demo that has gone wrong is exactly when somebody
/// needs the off switch, and stopping invented reads cannot damage a real class.
async fn clear_demo<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
) -> Result<Json<crate::wire::DemoResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let cmd = command(device, operator.read().now(), None);
    operator.clear_demo(&cmd).await?;
    Ok(Json(crate::wire::DemoResponse {
        freshness: crate::read::freshness(operator.read()).await,
        loaded: false,
    }))
}

/// Verifies whether the candidate PIN matches the venue's active PIN.
async fn verify_pin<S>(
    State(operator): State<Operator<S>>,
    Body(request): Body<VerifyPinRequest>,
) -> Result<Json<VerifyPinResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let ok = operator
        .verify_pin(&request.pin)
        .await
        .map_err(crate::error::storage)?;
    if !ok {
        return Err(ApiError::pin_invalid());
    }
    Ok(Json(VerifyPinResponse { ok: true }))
}

/// Changes the venue's PIN after validating knowledge of the current PIN.
async fn change_pin<S>(
    State(operator): State<Operator<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<ChangePinRequest>,
) -> Result<Json<VerifyPinResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let ok = operator
        .verify_pin(&request.current_pin)
        .await
        .map_err(crate::error::storage)?;
    if !ok {
        return Err(ApiError::pin_invalid());
    }
    let cmd = command(device, operator.read().now(), None);
    operator
        .save_setting(application::SECURITY_PIN, &request.new_pin, &cmd)
        .await?;
    Ok(Json(VerifyPinResponse { ok: true }))
}
