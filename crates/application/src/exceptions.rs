//! The exception inbox (ADR 0001 D4; CLAUDE.md 20).
//!
//! An exception is an interpretation like any other: it is stored, it belongs to an athlete,
//! and it is never a dropped read (CLAUDE.md 31 principle 1). What makes it an inbox item is
//! that the hub could not turn it into progress -- an unknown reader, an impossible
//! transition, a band belonging to somebody not on this roster.
//!
//! Two of D4's three actions live here. `void` is the one that changes anything; listing is
//! how the operator's screen is filled. *Accept as-is* and *reinterpret* are not implemented
//! -- see the note on [`void`] and `docs/open-issues.md`.

use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, HubStore, StoredException};

/// The session's live exceptions, oldest first.
///
/// Read from the store rather than from memory, for the same reason the badge is
/// (CLAUDE.md 21): the inbox must survive the process that produced it.
pub async fn list<S: HubStore>(
    state: &LiveSession,
    store: &S,
) -> Result<Vec<StoredException>, OperatorError<S::Error>> {
    store
        .exceptions(&state.session.id)
        .await
        .map_err(OperatorError::Storage)
}

/// Voids one interpreted event and recomputes everything derived from it.
///
/// The raw read is not touched (CLAUDE.md 19). The interpretation is marked voided, which
/// removes it from every replay, and the athletes are then rebuilt from the log -- CLAUDE.md
/// 20 requires the derived values to follow a correction, and rebuilding from the log is the
/// only way they cannot drift from it.
///
/// A reason is required: this is destructive, and D1 kept the requirement even after
/// dropping logins.
///
/// **Not implemented here:** *accept as-is* needs somewhere to record that a human looked at
/// an exception and left it alone, and `interpreted_events` has no such column; *reinterpret*
/// means adding an operator-authored event (a different station, a different athlete, a
/// different ENTRY/EXIT reading), which is a write path of its own. Both are listed in
/// `docs/open-issues.md` rather than half-built here.
pub async fn void<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    interpreted_event_id: i64,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    let Some(reason) = cmd.stated_reason() else {
        return Err(OperatorError::ReasonRequired);
    };

    let voided = store
        .void_interpreted(interpreted_event_id, cmd.at, &cmd.operator, reason)
        .await
        .map_err(OperatorError::Storage)?;
    if !voided {
        return Err(OperatorError::UnknownEvent(interpreted_event_id));
    }

    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "EVENT_VOID".to_string(),
            subject: interpreted_event_id.to_string(),
            reason: Some(reason.to_string()),
            before: None,
            after: Some("VOIDED".to_string()),
        })
        .await
        .map_err(OperatorError::Storage)?;

    recalculate(state, store).await
}

/// Re-derives athlete state and the inbox badge from the stored log.
///
/// Every derived value CLAUDE.md 20 names either lives on `AthleteState` (splits,
/// transitions, ROX, total time) or is projected from it on demand (`crate::live`), so
/// replacing the athletes replaces all of them at once. There is no ranking to recompute:
/// the finish rule is undecided, so none is published (CLAUDE.md 12, 28).
pub(crate) async fn recalculate<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
) -> Result<(), OperatorError<S::Error>> {
    state.athletes = store
        .rebuild_athletes(&state.session.id)
        .await
        .map_err(OperatorError::Storage)?;
    state.exception_count = store
        .exception_count(&state.session.id)
        .await
        .map_err(OperatorError::Storage)?;
    Ok(())
}
