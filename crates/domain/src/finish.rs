//! Finish rules (CLAUDE.md 12).
//!
//! Training was settled with the user on 2026-08-27: a group class ends when its time is
//! up, and most athletes will not have completed all eight stations by then. A coach must
//! also be able to end a class by hand. Competition remains UNDECIDED (CLAUDE.md 28) and
//! is deliberately still `NotConfigured` -- guessing it would stop clocks at a wrong moment.

use crate::athlete::{AthleteState, AthleteStatus};
use crate::time::{Duration, Instant};
use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FinishPolicy {
    /// No rule has been chosen. Competition still sits here (CLAUDE.md 12, 28).
    #[default]
    NotConfigured,
    /// Group class: everyone still running stops when the class clock reaches `limit`.
    /// Partial course completion is the normal outcome, not an error.
    ClassDuration { limit: Duration },
    /// No automatic trigger; the class ends when the coach ends it.
    CoachDecides,
}

/// Three-valued on purpose. A two-valued answer would force `NotConfigured` to report
/// `false`, which reads as "checked, and not finished" -- a rule smuggled in by omission.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FinishDecision {
    Finished,
    NotFinished,
    Undetermined,
}

impl FinishPolicy {
    /// Whether this athlete should now be considered finished, given how long the class
    /// has been running. Pure: deciding and applying stay separate, as everywhere else in
    /// this crate, so a replay can re-derive the same answer.
    pub fn evaluate(&self, state: &AthleteState, class_elapsed: Duration) -> FinishDecision {
        if state.status == AthleteStatus::Finished {
            return FinishDecision::Finished;
        }
        match self {
            // Nothing to say, and saying NotFinished here would be an answer we do not have.
            FinishPolicy::NotConfigured => FinishDecision::Undetermined,
            // A known rule whose answer is "not automatically": the coach's action decides.
            FinishPolicy::CoachDecides => FinishDecision::NotFinished,
            FinishPolicy::ClassDuration { limit } => {
                if class_elapsed < *limit {
                    return FinishDecision::NotFinished;
                }
                match state.status {
                    // Never scanned in: the class ended, but this athlete did not take part.
                    AthleteStatus::Ready => FinishDecision::NotFinished,
                    AthleteStatus::Active => FinishDecision::Finished,
                    AthleteStatus::Finished => FinishDecision::Finished,
                }
            }
        }
    }
}

/// Marks an athlete finished. Both the class-duration trigger and the coach's manual end
/// funnel through here: they differ in what decides, not in what happens.
///
/// An athlete caught INSIDE a station keeps that run open. No reader reported them leaving,
/// and inventing an exit time would fabricate a split that nothing observed (CLAUDE.md 19).
/// The screen shows the station they were in when the class ended.
pub fn finish(state: &mut AthleteState, at: Instant) {
    state.status = AthleteStatus::Finished;
    state.finished_at = Some(at);
    state.last_event_at = Some(at);
}
