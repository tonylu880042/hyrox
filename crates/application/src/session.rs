//! Session lifecycle use cases: DRAFT -> READY -> RUNNING <-> PAUSED -> COMPLETED, plus
//! CANCELLED and the two corrections (ADR 0001 D2; ADR 0008).
//!
//! The invariants themselves live in `domain::Session`. What is added here is the part that
//! is not a rule about states: persisting the new status and writing the audit record that
//! CLAUDE.md 20 requires, in that order, so a transition that was not stored was also not
//! claimed to have happened.

use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, HubStore};
use domain::{Instant, Session, SessionError, SessionStatus};

/// DRAFT -> READY. The class is built and can be started; it is still editable, which is
/// where a session-specific tweak to today's plan happens (ADR 0008).
pub async fn mark_ready<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    transition(state, store, cmd, "SESSION_READY", |s, _| s.mark_ready()).await
}

/// READY -> RUNNING. From here the first valid read starts each athlete's clock
/// (CLAUDE.md 11); the session itself does not change status again on its own.
///
/// Refuses a completed session: bringing one back is a correction and goes through
/// [`reopen`], which insists on a reason.
pub async fn start<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    if state.session.status.is_terminal() {
        return Err(OperatorError::ReasonRequired);
    }
    transition(state, store, cmd, "SESSION_START", |s, _| s.start()).await
}

/// RUNNING -> PAUSED. The class clock stops here, and reads arriving during the pause are
/// recorded as exceptions rather than splits (ADR 0008).
///
/// The pause is stamped with the operator's own `at`, not with a fresh clock read, for the
/// same reason results are: the moment recorded must be the moment it happened.
pub async fn pause<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    transition(state, store, cmd, "SESSION_PAUSE", |s, at| s.pause(at)).await
}

pub async fn resume<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    transition(state, store, cmd, "SESSION_RESUME", |s, at| s.resume(at)).await
}

pub async fn complete<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    transition(state, store, cmd, "SESSION_COMPLETE", |s, _| s.complete()).await
}

/// The class did not happen. Destructive -- it writes off everything recorded under the
/// session -- so it insists on a reason (CLAUDE.md 20).
pub async fn cancel<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    if cmd.stated_reason().is_none() {
        return Err(OperatorError::ReasonRequired);
    }
    transition(state, store, cmd, "SESSION_CANCEL", |s, _| s.cancel()).await
}

/// COMPLETED -> RUNNING. Allowed on purpose (ADR 0001 D2): a mis-tap on a busy floor must
/// not force a new session, and there is deliberately no time window on it -- a window
/// would be a magic constant nobody validated (CLAUDE.md 29).
pub async fn reopen<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    if cmd.stated_reason().is_none() {
        return Err(OperatorError::ReasonRequired);
    }
    transition(state, store, cmd, "SESSION_REOPEN", |s, _| s.reopen()).await
}

/// Back to editing, only while nothing has been interpreted (ADR 0001 D2). `domain::Session`
/// enforces that; the session is otherwise editable again and the roster may change.
pub async fn return_to_draft<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    transition(state, store, cmd, "SESSION_BACK_TO_DRAFT", |s, _| {
        s.back_to_draft()
    })
    .await
}

/// Applies a domain transition, persists it, then records it. Persisting first means a
/// crash can leave an unaudited transition, never an audited one that did not happen.
async fn transition<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
    action: &str,
    change: fn(&mut Session, Instant) -> Result<(), SessionError>,
) -> Result<(), OperatorError<S::Error>> {
    let before = state.session.status.name();
    let mut candidate = state.session.clone();
    change(&mut candidate, cmd.at).map_err(OperatorError::Session)?;
    let after = candidate.status.name();

    store
        .save_session(&candidate, state.class_start)
        .await
        .map_err(OperatorError::Storage)?;
    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: action.to_string(),
            subject: candidate.id.clone(),
            reason: cmd.stated_reason().map(str::to_string),
            before: Some(before.to_string()),
            after: Some(after.to_string()),
        })
        .await
        .map_err(OperatorError::Storage)?;
    state.session = candidate;
    Ok(())
}

pub(crate) fn status_name(status: SessionStatus) -> &'static str {
    status.name()
}
