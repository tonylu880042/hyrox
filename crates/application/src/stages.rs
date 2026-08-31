//! The athlete's progress through the class, stage by stage (workout brief §10, §27).
//!
//! **Derived, never stored.** The brief asks for `AthleteSession` and `AthleteStage` rows;
//! this hub already rebuilds every athlete by replaying its non-voided interpreted events
//! (CLAUDE.md 21), so stage rows would be a second source of truth that an operator's void
//! would silently fail to update. The same data is projected here instead, from the
//! session's snapshot course and the athlete's replayed runs -- so a correction changes the
//! stage list for free, and nothing can drift.

use domain::{AthleteState, AthleteStatus, Course, Expectation, Instant, StationRun};
use serde::Serialize;

/// Where one stage stands (brief §10).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StageStatus {
    /// Further down the plan.
    Pending,
    /// The next thing to do. Exactly one stage is READY at a time, and only while the
    /// athlete is between stations.
    Ready,
    /// The athlete is inside this station now.
    Active,
    Completed,
    /// Passed over: the athlete went on to a later station without doing this one.
    Skipped,
    /// The class ended before the athlete reached it. Not a judgement -- a group class
    /// ending with most athletes short of the last station is the normal outcome
    /// (CLAUDE.md 12, settled 2026-08-27).
    Dnf,
}

#[derive(Clone, Debug, Serialize)]
pub struct StageView {
    /// Position in the compiled course, from 1.
    pub sequence: usize,
    pub station: String,
    pub status: StageStatus,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    /// Time inside the station. Still climbing while ACTIVE.
    pub elapsed_ms: Option<i64>,
    /// The ROX / transition gap before this stage began (CLAUDE.md 13).
    pub transition_ms: Option<i64>,
}

/// The athlete's stage list for one class.
///
/// Pairs the plan with what actually happened, walking both in order. A run whose station
/// is not the next planned one is not an error: the plan is a plan, and training records
/// what happens (CLAUDE.md 9.2). It marks the stages that were stepped over as SKIPPED and
/// carries on.
pub fn stages(course: &Course, athlete: &AthleteState, now: Instant) -> Vec<StageView> {
    let mut views: Vec<StageView> = Vec::with_capacity(course.len());
    let mut runs = athlete.runs.iter().peekable();
    let finished = athlete.status == AthleteStatus::Finished;

    for (index, step) in course.steps.iter().enumerate() {
        // Does a still-unconsumed run match this step? Look ahead only as far as a run that
        // matches some *later* step, which is what tells us this step was stepped over.
        let matched = match runs.peek() {
            Some(run) if run.station == step.station => runs.next(),
            _ => None,
        };

        let view = match matched {
            Some(run) => StageView {
                sequence: index + 1,
                station: step.station.clone(),
                status: match run.exited_at {
                    Some(_) => StageStatus::Completed,
                    None if finished => StageStatus::Dnf,
                    None => StageStatus::Active,
                },
                started_at: Some(run.entered_at.0),
                completed_at: run.exited_at.map(|i| i.0),
                elapsed_ms: Some(leg_ms(run, athlete, now)),
                transition_ms: run.transition_from_prev.map(|d| d.millis()),
            },
            None => StageView {
                sequence: index + 1,
                station: step.station.clone(),
                // Anything still unvisited once the athlete has moved past it was skipped;
                // anything after the point they reached is pending, or DNF if the class is
                // over for them.
                status: StageStatus::Pending,
                started_at: None,
                completed_at: None,
                elapsed_ms: None,
                transition_ms: None,
            },
        };
        views.push(view);
    }

    mark_unvisited(&mut views, athlete, finished);
    views
}

/// Second pass over the stages nothing matched. It has to be a second pass: whether an
/// unvisited stage was *skipped* or is merely *pending* depends on how far the athlete got
/// overall, which is not known while walking forwards.
fn mark_unvisited(views: &mut [StageView], athlete: &AthleteState, finished: bool) {
    let reached = views.iter().rposition(|v| v.started_at.is_some());
    let mut next_up = !matches!(athlete.status, AthleteStatus::Finished);
    for (index, view) in views.iter_mut().enumerate() {
        if view.started_at.is_some() {
            continue;
        }
        let behind = reached.is_some_and(|r| index < r);
        view.status = if behind {
            StageStatus::Skipped
        } else if finished {
            StageStatus::Dnf
        } else if next_up && athlete.current_station.is_none() {
            // Exactly one READY, and only while the athlete is between stations: a stage
            // cannot be "next" while they are still inside the previous one.
            next_up = false;
            StageStatus::Ready
        } else {
            StageStatus::Pending
        };
        if !behind {
            next_up = false;
        }
    }
}

fn leg_ms(run: &StationRun, athlete: &AthleteState, now: Instant) -> i64 {
    let end = run.exited_at.or(athlete.finished_at).unwrap_or(now);
    end.since(run.entered_at).millis()
}

/// Which stage the athlete is on, from 1. `None` before they start and after they finish
/// the plan -- the brief's `current_stage_index`, derived rather than stored.
pub fn current_stage(stages: &[StageView]) -> Option<usize> {
    stages
        .iter()
        .find(|s| matches!(s.status, StageStatus::Active | StageStatus::Ready))
        .map(|s| s.sequence)
}

/// How the station the athlete is standing in compares with the plan (brief §11, §20).
///
/// `None` when they are between stations -- there is nothing to judge. Recorded, never
/// enforced: training records what happens and must not warn on a different order
/// (CLAUDE.md 9.2), and no athlete is disqualified by this.
pub fn current_expectation(course: &Course, athlete: &AthleteState) -> Option<Expectation> {
    let station = athlete.current_station.as_deref()?;
    // Judged against the plan as it stood *before* this station was entered, which is what
    // "did they come to the right place?" actually asks.
    let mut before = athlete.clone();
    before.runs.retain(|r| r.exited_at.is_some());
    Some(domain::expectation(course, &before, station))
}
