//! The exception inbox (ADR 0001 D4; CLAUDE.md 20).
//!
//! An exception is an interpretation like any other: it is stored, it belongs to an athlete,
//! and it is never a dropped read (CLAUDE.md 31 principle 1). What makes it an inbox item is
//! that the hub could not turn it into progress -- an unknown reader, an impossible
//! transition, a band belonging to somebody not on this roster.
//!
//! The actions from ADR 0001 D4 live here: `list` fills the operator's inbox,
//! `accept` clears an exception without modifying historical interpretations or recomputing,
//! and `void` marks an invalid read voided and triggers a rebuild from the log.

use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, HubStore, InterpretedWrite, StoredException};
use domain::{AthleteStatus, Instant, Interpreted, ReaderMode};

/// What the operator wants to reinterpret an exception into (ADR 0001 D4; CLAUDE.md 20).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReinterpretSpec {
    pub station: String,
    pub mode: ReaderMode,
    /// If None, keeps the athlete the original exception was attributed to.
    pub athlete_id: Option<String>,
    /// If None, keeps the original exception's detected_at timestamp.
    pub at: Option<Instant>,
}

/// The session's live exceptions, oldest first.
///
/// Read from the store rather than from memory, for the same reason the badge is
/// (CLAUDE.md 21): the inbox must survive the process that produced it.
///
/// Takes the session id, not the session: the caller can then let go of the live session
/// before waiting on the disk. This query filters a table that grows all season, and the
/// settings screen asks for it every five seconds -- with the session locked, that is a
/// growing wait between a reader's tap and its ACK.
pub async fn list<S: HubStore>(
    session_id: &str,
    store: &S,
) -> Result<Vec<StoredException>, OperatorError<S::Error>> {
    store
        .exceptions(session_id)
        .await
        .map_err(OperatorError::Storage)
}

/// Accepts one exception as it stands: it leaves the inbox, and nothing else changes.
///
/// The other half of D4's pair. `void` is for a reading that should never have counted;
/// this is for one that is a true record and simply needs no action -- a band that brushed
/// an antenna twice, a read taken while a reader was being moved. Nothing is removed from
/// the log and no replay changes, so unlike voiding it takes no reason. Demanding one would
/// only teach an operator to type "ok" thirty times an evening, and a trail full of "ok" is
/// worse than a trail that says an operator cleared it and when.
pub async fn accept<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    interpreted_event_id: i64,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    let accepted = store
        .acknowledge_interpreted(
            interpreted_event_id,
            cmd.at,
            &cmd.operator,
            cmd.stated_reason(),
        )
        .await
        .map_err(OperatorError::Storage)?;
    if !accepted {
        return Err(OperatorError::UnknownEvent(interpreted_event_id));
    }

    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "EXCEPTION_ACCEPT".to_string(),
            subject: interpreted_event_id.to_string(),
            reason: cmd.stated_reason().map(str::to_string),
            before: None,
            after: Some("ACCEPTED".to_string()),
        })
        .await
        .map_err(OperatorError::Storage)?;

    // Only the badge moves. Athlete state is not rebuilt, because nothing that replays has
    // changed -- and rebuilding the whole class to clear one notification would be the
    // expensive way to do nothing.
    state.exception_count = store
        .exception_count(&state.session.id)
        .await
        .map_err(OperatorError::Storage)?;
    Ok(())
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

/// Reinterprets one exception: voids the old exception, commits a new corrected interpretation,
/// and recomputes athlete state from the log (ADR 0001 D4; CLAUDE.md 20).
pub async fn reinterpret<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    interpreted_event_id: i64,
    spec: ReinterpretSpec,
    cmd: &OperatorCommand,
) -> Result<i64, OperatorError<S::Error>> {
    let Some(reason) = cmd.stated_reason() else {
        return Err(OperatorError::ReasonRequired);
    };

    let exceptions = store
        .exceptions(&state.session.id)
        .await
        .map_err(OperatorError::Storage)?;
    let exception = exceptions
        .iter()
        .find(|e| e.interpreted_event_id == interpreted_event_id)
        .ok_or(OperatorError::UnknownEvent(interpreted_event_id))?;

    let target_athlete_id = spec.athlete_id.as_deref().unwrap_or(&exception.athlete_id);

    let athlete = state
        .athlete(target_athlete_id)
        .ok_or_else(|| OperatorError::UnknownAthlete(target_athlete_id.to_string()))?;

    let effective_at = spec.at.unwrap_or(exception.at);

    let started_timing = athlete.status == AthleteStatus::Ready && spec.mode != ReaderMode::Exit;
    let transition = if spec.mode == ReaderMode::Entry {
        athlete.last_exit_at.map(|prev| effective_at.since(prev))
    } else {
        None
    };

    let new_event = match spec.mode {
        ReaderMode::Exit => Interpreted::Exited {
            station: spec.station.clone(),
            at: effective_at,
        },
        _ => Interpreted::Entered {
            station: spec.station.clone(),
            at: effective_at,
            transition,
            started_timing,
        },
    };

    let voided = store
        .void_interpreted(interpreted_event_id, cmd.at, &cmd.operator, reason)
        .await
        .map_err(OperatorError::Storage)?;
    if !voided {
        return Err(OperatorError::UnknownEvent(interpreted_event_id));
    }

    let new_id = store
        .commit_interpreted(InterpretedWrite {
            session_id: &state.session.id,
            athlete_id: target_athlete_id,
            raw_event_id: exception.raw_event_id,
            event: &new_event,
        })
        .await
        .map_err(OperatorError::Storage)?;

    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "EVENT_REINTERPRET".to_string(),
            subject: interpreted_event_id.to_string(),
            reason: Some(reason.to_string()),
            before: Some(format!("EXCEPTION: {:?}", exception.reason)),
            after: Some(format!(
                "INTERPRETED: {new_id}, {target_athlete_id}, {}, {:?}",
                spec.station, spec.mode
            )),
        })
        .await
        .map_err(OperatorError::Storage)?;

    recalculate(state, store).await?;
    Ok(new_id)
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
