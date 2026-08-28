//! One error shape, and the map from a refused use case to an HTTP status.
//!
//! A domain invariant that says no is an answer, not a fault: arming a session that is
//! already closed, editing a course while the class is running, voiding an event that does
//! not exist. None of those may come back as 500, because a screen cannot tell a rule from
//! an outage, and an operator on a gym floor has to know which one they are looking at.
//!
//! The mapping is the whole of this module on purpose. Deciding *whether* something is
//! refused belongs to `domain` and `application`; deciding *how to say so over HTTP* is a
//! delivery concern and belongs here (CLAUDE.md 3, 29).

use application::OperatorError;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use domain::{BindingError, SessionError};
use serde::Serialize;
use std::fmt::Display;

/// The body of every failure this API reports.
///
/// `error` is a stable machine code and is what a screen should branch on; `message` is for
/// a human reading a log. The wording of what an operator is shown is a UI decision.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: &'static str,
    pub message: String,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into() }
    }

    /// No operator device name on a write (ADR 0001 D1).
    ///
    /// Refused rather than defaulted. There is no login, so the device name is the entire
    /// audit identity of CLAUDE.md 20 -- an audit row naming nobody is worse than a
    /// rejected request, because it looks like a record and is not one.
    pub fn operator_required() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "OPERATOR_REQUIRED",
            format!(
                "every write carries the operator's device name in the {} header \
                 (ADR 0001 D1)",
                crate::identity::OPERATOR_HEADER
            ),
        )
    }

    pub fn invalid_body(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "INVALID_BODY", message)
    }

    pub fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody { error: self.code, message: self.message };
        (self.status, Json(body)).into_response()
    }
}

/// Maps a refused use case onto a status code.
///
/// `E` is the store's error and is the only variant that becomes a 500: a failed write is a
/// genuine server fault, and the read was either never durable or the caller must retry.
/// Everything above it is the hub saying no on purpose.
impl<E: Display> From<OperatorError<E>> for ApiError {
    fn from(error: OperatorError<E>) -> Self {
        match error {
            // 409: the session is in a state this transition is not legal from. The client
            // is not malformed, the world is simply not where it thought.
            OperatorError::Session(SessionError::IllegalTransition { from, to }) => ApiError::new(
                StatusCode::CONFLICT,
                "ILLEGAL_TRANSITION",
                format!("a {from:?} session cannot become {to:?} (ADR 0001 D2)"),
            ),
            OperatorError::Session(SessionError::HasInterpretedEvents) => ApiError::new(
                StatusCode::CONFLICT,
                "HAS_INTERPRETED_EVENTS",
                "this session has already interpreted events, so it cannot return to DRAFT \
                 (ADR 0001 D2)",
            ),
            OperatorError::Binding(e) => binding(e),
            // 422: the request was understood and is refusable only because it is missing
            // the reason CLAUDE.md 20 requires on the audit record.
            OperatorError::ReasonRequired => ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "REASON_REQUIRED",
                "this action changes recorded data, so it needs a reason (CLAUDE.md 20)",
            ),
            // 409, not 501: the hub is capable of ending a class, this session just has no
            // rule saying what finishing means. Competition's rule is undecided
            // (CLAUDE.md 12, 28) and no button may invent it.
            OperatorError::NoFinishRule => ApiError::new(
                StatusCode::CONFLICT,
                "NO_FINISH_RULE",
                "no finish rule is configured for this session, so it cannot be ended by \
                 hand (CLAUDE.md 12)",
            ),
            OperatorError::UnknownAthlete(id) => ApiError::not_found(
                "UNKNOWN_ATHLETE",
                format!("{id:?} is not on this session's roster"),
            ),
            OperatorError::NotEditable { status } => ApiError::new(
                StatusCode::CONFLICT,
                "SESSION_NOT_EDITABLE",
                format!(
                    "configuration may only be edited while the session is DRAFT; it is \
                     {status:?} (ADR 0001 D2)"
                ),
            ),
            OperatorError::UnknownEvent(id) => ApiError::not_found(
                "UNKNOWN_EVENT",
                format!("no interpreted event with id {id}"),
            ),
            OperatorError::Storage(e) => storage(e),
        }
    }
}

fn binding(error: BindingError) -> ApiError {
    match error {
        BindingError::TagAlreadyBound { session_id, athlete_id } => ApiError::new(
            StatusCode::CONFLICT,
            "TAG_ALREADY_BOUND",
            format!("that band is already on {athlete_id:?} in session {session_id:?}"),
        ),
        BindingError::AthleteAlreadyBound { tag_id } => ApiError::new(
            StatusCode::CONFLICT,
            "ATHLETE_ALREADY_BOUND",
            format!("that athlete already has band {tag_id}; rebind to swap it"),
        ),
        BindingError::NotBound => ApiError::new(
            StatusCode::CONFLICT,
            "NOT_BOUND",
            "there is no binding to change",
        ),
    }
}

/// The one honest 500. The message is included because the hub is a single box on a venue
/// LAN with no error-reporting service behind it: whoever is standing at the tablet is the
/// only person who will ever read it.
pub fn storage<E: Display>(error: E) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "STORAGE_FAILED",
        format!("the hub's store rejected the write: {error}"),
    )
}
