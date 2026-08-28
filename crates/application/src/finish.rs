//! Applying the session's finish policy (CLAUDE.md 12).
//!
//! `domain::FinishPolicy` decides; this module is what actually asks it, on every tick, and
//! marks the athletes it names. Without this the training answer from 2026-08-27 -- a group
//! class ends when its time is up -- was written down but never enforced.
//!
//! Competition is untouched. Its rule is `NotConfigured`, which evaluates to `Undetermined`,
//! and `Undetermined` is treated here as "no answer": nobody is finished, and nobody is
//! declared unfinished either (CLAUDE.md 12, 28).

use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, HubStore};
use crate::session::status_name;
use domain::{AthleteStatus, FinishDecision, FinishPolicy, Instant};

/// Marks everyone the policy says is finished, and returns their ids.
///
/// Deliberately not persisted. Being finished by a class-duration rule is *derived* from
/// the class clock and the athlete's events, so a restart re-derives it from the replayed
/// state at the next tick (CLAUDE.md 21). Writing a FINISHED row instead would invent an
/// event no reader ever reported.
pub fn apply_finish_policy(state: &mut LiveSession, now: Instant) -> Vec<String> {
    let elapsed = state.class_elapsed(now);
    let policy = state.config.finish_policy;
    let mut newly = Vec::new();
    for athlete in &mut state.athletes {
        if athlete.status == AthleteStatus::Finished {
            continue;
        }
        if policy.evaluate(athlete, elapsed) == FinishDecision::Finished {
            domain::finish(athlete, now);
            newly.push(athlete.athlete_id.clone());
        }
    }
    newly
}

/// The coach ends the class by hand: everyone still running is finished, and the session
/// closes (ADR 0001 D2, CLAUDE.md 12).
///
/// Refused when no finish rule is configured. That is the competition case, and a button
/// that stopped every competitor's clock would be exactly the invented rule CLAUDE.md 28
/// forbids.
pub async fn end_class<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    cmd: &OperatorCommand,
) -> Result<Vec<String>, OperatorError<S::Error>> {
    if state.config.finish_policy == FinishPolicy::NotConfigured {
        return Err(OperatorError::NoFinishRule);
    }

    let mut finished = Vec::new();
    for athlete in &mut state.athletes {
        if athlete.status == AthleteStatus::Active {
            domain::finish(athlete, cmd.at);
            finished.push(athlete.athlete_id.clone());
        }
    }

    let before = status_name(state.session.status);
    state.session.close().map_err(OperatorError::Session)?;
    store
        .save_session(&state.session, state.class_start)
        .await
        .map_err(OperatorError::Storage)?;
    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "CLASS_END".to_string(),
            subject: state.session.id.clone(),
            reason: cmd.stated_reason().map(str::to_string),
            before: Some(before.to_string()),
            after: Some(format!("CLOSED, {} athletes finished", finished.len())),
        })
        .await
        .map_err(OperatorError::Storage)?;
    Ok(finished)
}
