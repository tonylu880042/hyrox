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
use crate::ports::{AuditEntry, AuditedWrite, FinishOrigin, HubStore};
use crate::session::status_name;
use domain::{AthleteState, AthleteStatus, Course, FinishDecision, FinishPolicy, Instant};

/// Applies the replay-derived CourseComplete answer without writing roster state. Callers
/// that rebuild from the store (startup, results and correction replay) use this so a final
/// exit is reflected even when no periodic tick happened yet.
pub(crate) fn apply_course_completion(athletes: &mut [AthleteState], course: Option<&Course>) {
    let Some(course) = course else { return };
    let clock = domain::ClassClock::started_at(Instant(0));
    for athlete in athletes {
        if let FinishDecision::Finished { at } =
            FinishPolicy::CourseComplete.evaluate(athlete, clock, Instant(0), Some(course))
        {
            domain::finish(athlete, at);
        }
    }
}

/// Marks everyone the policy says is finished, stores it, and returns their ids.
///
/// ClassDuration has no event behind it, so its result is stored on the roster row. Course
/// completion is different: its answer is the final exit in the interpreted log, so it stays
/// derived in memory and is re-derived after a void/restart. Manual end-class decisions are
/// explicitly stored (migration 0013).
pub async fn apply_finish_policy<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    now: Instant,
) -> Result<Vec<String>, S::Error> {
    let policy = state.config.finish_policy;
    let course = state.config.course.clone();
    let clock = state.class_clock();
    let mut newly = Vec::new();
    for athlete in &state.athletes {
        if athlete.status == AthleteStatus::Finished {
            continue;
        }
        // Finish at the moment the rule says they stopped, not at this tick. A poll that
        // runs late -- or one that runs for the first time after a restart -- must not
        // inflate a result (CLAUDE.md 11, 17).
        if let FinishDecision::Finished { at } =
            policy.evaluate(athlete, clock, now, course.as_ref())
        {
            newly.push((athlete.athlete_id.clone(), at));
        }
    }

    // Persist finishes FIRST. If saving fails, the athletes remain unfinished in memory,
    // and the next tick will safely retry saving them. CourseComplete is derived from the
    // event log and deliberately has no roster write.
    if policy != FinishPolicy::CourseComplete {
        store
            .save_athlete_finishes(&state.session.id, &newly, FinishOrigin::Policy)
            .await?;
    }

    // Only after durable persistence succeeds do we fold the finish into memory.
    for (athlete_id, at) in &newly {
        if let Some(athlete) = state.athlete_mut(athlete_id) {
            domain::finish(athlete, *at);
        }
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

    // Reconcile an ACKed-but-uninterpreted read before closing the class. Otherwise a failed
    // ENTRY followed by this command could leave the durable sequence split across a closed
    // session and an in-memory state that never saw the entry.
    crate::ingest::reconcile_pending(state, store)
        .await
        .map_err(OperatorError::Storage)?;

    let finished: Vec<String> = state
        .athletes
        .iter()
        .filter(|a| a.status == AthleteStatus::Active)
        .map(|a| a.athlete_id.clone())
        .collect();

    let before = status_name(state.session.status);
    let mut candidate = state.session.clone();
    candidate.complete().map_err(OperatorError::Session)?;

    // Validate the lifecycle transition before any roster write. A refused command must not
    // leave a completed/invalid session with only some athletes manually finished.
    let finish_rows: Vec<_> = finished
        .iter()
        .map(|athlete_id| (athlete_id.clone(), cmd.at))
        .collect();
    store
        .apply_audited(AuditedWrite::EndClass {
            session: candidate.clone(),
            created_at: state.class_start,
            finishes: finish_rows,
            audit: AuditEntry {
                at: cmd.at,
                operator: cmd.operator.clone(),
                action: "CLASS_END".to_string(),
                subject: state.session.id.clone(),
                reason: cmd.stated_reason().map(str::to_string),
                before: Some(before.to_string()),
                after: Some(format!("CLOSED, {} athletes finished", finished.len())),
            },
        })
        .await
        .map_err(OperatorError::Storage)?;

    // All persistence succeeded: commit to in-memory session and athletes.
    state.session = candidate;
    state.session_save_dirty = false;
    for athlete_id in &finished {
        if let Some(athlete) = state.athlete_mut(athlete_id) {
            domain::finish(athlete, cmd.at);
        }
    }
    Ok(finished)
}
