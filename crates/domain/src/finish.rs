//! Finish rules (CLAUDE.md 12).
//!
//! Training was settled with the user on 2026-08-27: a group class ends when its time is
//! up, and most athletes will not have completed all eight stations by then. A coach must
//! also be able to end a class by hand. Competition remains UNDECIDED (CLAUDE.md 28) and
//! is deliberately still `NotConfigured` -- guessing it would stop clocks at a wrong moment.

use crate::athlete::{AthleteState, AthleteStatus};
use crate::course::Course;
use crate::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
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
    /// Competition: finishing is completing the configured course. Settled with the user
    /// 2026-08-28 -- the exit of the final station is the result's recording point, so the
    /// format (full course, half course) is expressed as a different course, not as another
    /// rule here. Adding a format must not mean adding a variant.
    CourseComplete,
}

/// Three-valued on purpose. A two-valued answer would force `NotConfigured` to report
/// `false`, which reads as "checked, and not finished" -- a rule smuggled in by omission.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FinishDecision {
    /// Carries WHEN, not just whether. The moment an athlete stopped is their result, and
    /// it is not the moment a background tick happened to notice: a tick is up to a poll
    /// interval late, and after a restart it can be minutes late (CLAUDE.md 11, 17).
    Finished { at: Instant },
    NotFinished,
    Undetermined,
}

impl FinishPolicy {
    /// Whether this athlete should now be considered finished, given how long the class
    /// has been running. Pure: deciding and applying stay separate, as everywhere else in
    /// this crate, so a replay can re-derive the same answer.
    pub fn evaluate(
        &self,
        state: &AthleteState,
        class_start: Instant,
        now: Instant,
        course: Option<&Course>,
    ) -> FinishDecision {
        if let (AthleteStatus::Finished, Some(at)) = (state.status, state.finished_at) {
            return FinishDecision::Finished { at };
        }
        match self {
            // Nothing to say, and saying NotFinished here would be an answer we do not have.
            FinishPolicy::NotConfigured => FinishDecision::Undetermined,
            // A known rule whose answer is "not automatically": the coach's action decides.
            FinishPolicy::CoachDecides => FinishDecision::NotFinished,

            FinishPolicy::ClassDuration { limit } => {
                if now.since(class_start) < *limit {
                    return FinishDecision::NotFinished;
                }
                match state.status {
                    // Never scanned in: the class ended, but this athlete did not take part.
                    AthleteStatus::Ready => FinishDecision::NotFinished,
                    // The clock ran out at the limit, whenever we got around to looking.
                    _ => FinishDecision::Finished { at: Instant(class_start.0 + limit.millis()) },
                }
            }

            FinishPolicy::CourseComplete => {
                let Some(course) = course else {
                    // A course-completion rule without a course cannot answer, and
                    // NotFinished would read as an answer (CLAUDE.md 28).
                    return FinishDecision::Undetermined;
                };
                let finished_runs: Vec<_> =
                    state.runs.iter().filter_map(|r| r.exited_at.map(|at| (&r.station, at))).collect();
                let Some(last_step) = course.steps.last() else {
                    return FinishDecision::Undetermined;
                };
                if finished_runs.len() < course.steps.len() {
                    return FinishDecision::NotFinished;
                }
                match finished_runs.last() {
                    // The exit of the final station is the recording point, so that exit's
                    // own timestamp is the result -- not now.
                    Some((station, at)) if **station == last_step.station => {
                        FinishDecision::Finished { at: *at }
                    }
                    _ => FinishDecision::NotFinished,
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
