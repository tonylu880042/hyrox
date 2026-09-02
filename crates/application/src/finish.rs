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

/// Marks everyone the policy says is finished, stores it, and returns their ids.
///
/// The finish is *stored*, on the roster row rather than as an interpreted event
/// (migration 0010). It was not, once, on the reasoning that a class-duration finish is
/// derived and gets re-derived after a restart -- which holds only while the class is still
/// live. Once it is over nothing ticks it again, so the replay handed back a class of people
/// still running, with times that kept growing. Writing an interpreted event instead would
/// invent a read no reader ever reported (CLAUDE.md 19); this is derived data, stored as
/// derived data (CLAUDE.md 13).
pub async fn apply_finish_policy<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    now: Instant,
) -> Result<Vec<String>, S::Error> {
    let policy = state.config.finish_policy;
    let course = state.config.course.clone();
    let clock = state.class_clock();
    let mut newly = Vec::new();
    for athlete in &mut state.athletes {
        if athlete.status == AthleteStatus::Finished {
            continue;
        }
        // Finish at the moment the rule says they stopped, not at this tick. A poll that
        // runs late -- or one that runs for the first time after a restart -- must not
        // inflate a result (CLAUDE.md 11, 17).
        if let FinishDecision::Finished { at } =
            policy.evaluate(athlete, clock, now, course.as_ref())
        {
            domain::finish(athlete, at);
            newly.push((athlete.athlete_id.clone(), at));
        }
    }

    // Only for those who just finished: the common tick finishes nobody and writes nothing.
    for (athlete_id, at) in &newly {
        store.save_athlete_finish(&state.session.id, athlete_id, Some(*at)).await?;
    }
    Ok(newly.into_iter().map(|(id, _)| id).collect())
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
    // Same reason as the rule's own finishes: the coach's decision is not a read, so nothing
    // replays it (migration 0010).
    for athlete_id in &finished {
        store
            .save_athlete_finish(&state.session.id, athlete_id, Some(cmd.at))
            .await
            .map_err(OperatorError::Storage)?;
    }

    let before = status_name(state.session.status);
    state.session.complete().map_err(OperatorError::Session)?;
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
