//! Session lifecycle use cases: DRAFT -> ARMED -> CLOSED, and back (ADR 0001 D2).
//!
//! The invariants themselves live in `domain::Session`. What is added here is the part that
//! is not a rule about states: persisting the new status and writing the audit record that
//! CLAUDE.md 20 requires, in that order, so a transition that was not stored was also not
//! claimed to have happened.

use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, HubStore};
use domain::{Session, SessionError, SessionStatus};

/// DRAFT -> ARMED. From here the first valid read starts each athlete's clock
/// (CLAUDE.md 11); the session itself does not change status again on its own.
///
/// Refuses a CLOSED session: bringing one back is a correction and goes through
/// [`reopen`], which insists on a reason.
pub async fn arm<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    if state.session.status == SessionStatus::Closed {
        return Err(OperatorError::ReasonRequired);
    }
    transition(state, store, cmd, "SESSION_ARM", Session::arm).await
}

pub async fn close<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    transition(state, store, cmd, "SESSION_CLOSE", Session::close).await
}

/// CLOSED -> ARMED. Allowed on purpose (ADR 0001 D2): a mis-tap on a busy floor must not
/// force a new session, and there is deliberately no time window on it -- a window would be
/// a magic constant nobody validated (CLAUDE.md 29).
pub async fn reopen<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    if cmd.stated_reason().is_none() {
        return Err(OperatorError::ReasonRequired);
    }
    transition(state, store, cmd, "SESSION_REOPEN", Session::arm).await
}

/// ARMED -> DRAFT, only while nothing has been interpreted (ADR 0001 D2). `domain::Session`
/// enforces that; the session is otherwise editable again and the roster may change.
pub async fn return_to_draft<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    transition(state, store, cmd, "SESSION_BACK_TO_DRAFT", Session::back_to_draft).await
}

/// Applies a domain transition, persists it, then records it. Persisting first means a
/// crash can leave an unaudited transition, never an audited one that did not happen.
async fn transition<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
    action: &str,
    change: fn(&mut Session) -> Result<(), SessionError>,
) -> Result<(), OperatorError<S::Error>> {
    let before = status_name(state.session.status);
    change(&mut state.session).map_err(OperatorError::Session)?;
    let after = status_name(state.session.status);

    store
        .save_session(&state.session, state.class_start)
        .await
        .map_err(OperatorError::Storage)?;
    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: action.to_string(),
            subject: state.session.id.clone(),
            reason: cmd.stated_reason().map(str::to_string),
            before: Some(before.to_string()),
            after: Some(after.to_string()),
        })
        .await
        .map_err(OperatorError::Storage)?;
    Ok(())
}

pub(crate) fn status_name(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Draft => "DRAFT",
        SessionStatus::Armed => "ARMED",
        SessionStatus::Closed => "CLOSED",
    }
}
