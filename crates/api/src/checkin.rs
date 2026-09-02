//! `/checkin`: the narrow write surface (ADR 0001).
//!
//! It may put a band on a wrist and nothing else. The state type is [`CheckIn`], which has
//! exactly two write methods -- bind and rebind -- and reaches reads only through
//! [`CheckIn::read`]. There is no expression in this module that can arm a session, close
//! one, edit a course or void an event, because the state it is given has no such method.
//!
//! That is the point of the surface: a check-in tablet gets handed to whoever is on the
//! door, and the worst thing they can do with it is bind the wrong band or add a name --
//! both recoverable, both audited.
//!
//! ADR 0010 widened it by one verb. A competition takes entries from people the gym has
//! never seen, and putting them on the roster is literally what checking in *is*; refusing
//! it here would have meant handing the door an operator tablet that can also stop the
//! clock. The surface still has no method that touches timing.

use crate::error::ApiError;
use crate::identity::{Body, OperatorDevice};
use crate::read::freshness;
use crate::state::CheckIn;
use crate::wire::{
    BindRequest, BindResponse, CheckInResponse, EnteredResponse, EnterRequest, SignupRequest,
    SignupResponse,
};
use application::{Entrant, HubStore, OperatorCommand};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use domain::TagId;
use std::fmt::Display;

pub fn router<S>(state: CheckIn<S>) -> Router
where
    S: HubStore + Send + Sync + 'static,
    S::Error: Display + Send,
{
    Router::new()
        .route("/", get(pending))
        .route("/entrants", post(enter))
        // Self sign-up (ADR 0011): the one route on this hub a member of the public reaches.
        // It is here rather than under /api/operator because it does exactly what the door
        // does -- put a name on the roster -- and the surface's other guarantees hold for it.
        .route("/signup", post(signup))
        .route("/bind", post(bind))
        .route("/rebind", post(rebind))
        .with_state(state)
}

/// The bands waiting to be claimed, and who on the roster still has none (ADR 0001 D3).
async fn pending<S>(State(checkin): State<CheckIn<S>>) -> Json<CheckInResponse> {
    let read = checkin.read();
    Json(CheckInResponse {
        freshness: freshness(read).await,
        view: read.checkin().await,
    })
}

/// Binds a band to an athlete, and claims whatever that band was already read doing.
///
/// The reads that happened before anyone owned the band are interpreted as part of this
/// (ADR 0001 D3), which is why the response says what was claimed: on a busy floor a band
/// is often scanned before it is handed out, and the operator needs to see that the time
/// was not lost.
async fn bind<S>(
    State(checkin): State<CheckIn<S>>,
    OperatorDevice(operator): OperatorDevice,
    Body(request): Body<BindRequest>,
) -> Result<Json<BindResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let now = checkin.read().now();
    let tag = parse_tag(&request.tag_id)?;
    let cmd = command(operator, now, request.reason);
    let claimed = checkin.bind(&tag, &request.athlete_id, &cmd).await?;
    Ok(Json(BindResponse { freshness: freshness(checkin.read()).await, claimed }))
}

/// Moves an athlete onto a different band. Destructive, so the use case requires a reason
/// (CLAUDE.md 20) and answers 422 without one.
async fn rebind<S>(
    State(checkin): State<CheckIn<S>>,
    OperatorDevice(operator): OperatorDevice,
    Body(request): Body<BindRequest>,
) -> Result<Json<BindResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let now = checkin.read().now();
    let tag = parse_tag(&request.tag_id)?;
    let cmd = command(operator, now, request.reason);
    let claimed = checkin.rebind(&tag, &request.athlete_id, &cmd).await?;
    Ok(Json(BindResponse { freshness: freshness(checkin.read()).await, claimed }))
}

fn parse_tag(raw: &str) -> Result<TagId, ApiError> {
    TagId::parse(raw).map_err(|e| ApiError::invalid_body(format!("tag_id: {e:?}")))
}

/// Assembles the operator command. The identity is whatever the header carried -- the
/// extractor has already refused a request that carried none (ADR 0001 D1).
pub(crate) fn command(
    operator: String,
    at: domain::Instant,
    reason: Option<String>,
) -> OperatorCommand {
    let cmd = OperatorCommand::new(operator, at);
    match reason {
        Some(reason) => cmd.with_reason(reason),
        None => cmd,
    }
}

/// Puts somebody on the roster: a member by their 健身管 id, or a walk-in with nothing but a
/// name (ADR 0010).
///
/// Idempotent for a member -- a door tablet's double tap is one roster line, and the same
/// athlete id comes back so the helper is not told a different number the second time.
async fn enter<S>(
    State(checkin): State<CheckIn<S>>,
    OperatorDevice(device): OperatorDevice,
    Body(request): Body<EnterRequest>,
) -> Result<Json<EnteredResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let read = checkin.read();
    let cmd = command(device, read.now(), request.reason);
    let entrant = Entrant {
        member_id: request.member_id,
        display_name: request.display_name,
        bib: request.bib,
    };
    let athlete_id = checkin.enter(entrant, &cmd).await?;

    let view = read.checkin().await;
    Ok(Json(EnteredResponse {
        freshness: freshness(read).await,
        athlete_id,
        pending: view.pending,
        athletes: view.athletes,
    }))
}

/// Somebody entering a mock race on their own phone (ADR 0011).
///
/// The only unauthenticated write on the hub, and it is deliberately the narrowest one
/// there is:
///
/// * **No operator header.** There is no device to name, and an audit row naming a tablet
///   that did not do it would be a lie. The row says `SELF SIGN-UP`, which is the fact.
/// * **A name and nothing else.** No bib -- printed numbers are the desk's to hand out, and
///   letting the public claim one invites two people wearing 07. No member id -- claiming
///   somebody else's membership from an unauthenticated route is not a thing we will offer.
/// * **Nothing else on this surface moves.** Binding a band still requires the desk's
///   header, so an entry is inert until a helper hands over a wristband.
///
/// The entry code that comes back is the whole point: it is their athlete id, the QR they
/// carry, and the number they look their result up with afterwards.
async fn signup<S>(
    State(checkin): State<CheckIn<S>>,
    Body(request): Body<SignupRequest>,
) -> Result<Json<SignupResponse>, ApiError>
where
    S: HubStore,
    S::Error: Display,
{
    let read = checkin.read();
    let cmd = OperatorCommand::new(SELF_SIGNUP, read.now());
    let entrant = Entrant::walk_in(request.display_name);
    let code = checkin.enter(entrant, &cmd).await?;

    let view = read.checkin().await;
    let athlete = view.athletes.iter().find(|a| a.athlete_id == code);
    Ok(Json(SignupResponse {
        freshness: freshness(read).await,
        display_name: athlete.map(|a| a.name.clone()).unwrap_or_default(),
        bib: athlete.map(|a| a.bib),
        code,
    }))
}

/// What the audit trail calls an entrant who registered themselves. Not a device name: no
/// device did it (CLAUDE.md 20; ADR 0001 D1).
const SELF_SIGNUP: &str = "SELF SIGN-UP";
