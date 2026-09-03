//! The athlete stage read model (workout brief §10, §27 scenarios C and D).

use application::{current_stage, stages, StageStatus};
use domain::{AthleteState, AthleteStatus, Course, CourseStep, Instant, StationRun};

const NOW: Instant = Instant(1_000_000);

fn course() -> Course {
    Course::new(
        "Engine",
        vec![
            CourseStep::new("RUN"),
            CourseStep::new("ROWING"),
            CourseStep::new("RUN"),
            CourseStep::new("WALL BALLS"),
        ],
    )
}

fn run(station: &str, entered: i64, exited: Option<i64>) -> StationRun {
    StationRun {
        station: station.to_string(),
        entered_at: Instant(entered),
        exited_at: exited.map(Instant),
        transition_from_prev: None,
    }
}

#[test]
fn an_athlete_who_has_not_started_has_one_ready_stage_and_the_rest_pending() {
    let a = AthleteState::ready("a1", "TONY");

    let s = stages(&course(), &a, NOW);

    assert_eq!(s.len(), 4);
    assert_eq!(s[0].status, StageStatus::Ready);
    assert_eq!(s[1].status, StageStatus::Pending);
    assert_eq!(s[3].status, StageStatus::Pending);
    assert_eq!(current_stage(&s), Some(1));
}

#[test]
fn the_stage_an_athlete_is_inside_is_active() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.status = AthleteStatus::Active;
    a.current_station = Some("RUN".into());
    a.runs.push(run("RUN", 1_000, None));

    let s = stages(&course(), &a, Instant(4_000));

    assert_eq!(s[0].status, StageStatus::Active);
    assert_eq!(s[0].started_at, Some(1_000));
    assert_eq!(s[0].elapsed_ms, Some(3_000), "an open stage keeps counting");
    assert_eq!(
        s[1].status,
        StageStatus::Pending,
        "not READY while they are still inside"
    );
    assert_eq!(current_stage(&s), Some(1));
}

/// Scenario D: the rower reports, the stage completes, and the next one becomes READY.
#[test]
fn completing_a_stage_makes_the_next_one_ready() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.status = AthleteStatus::Active;
    a.runs.push(run("RUN", 1_000, Some(4_000)));

    let s = stages(&course(), &a, NOW);

    assert_eq!(s[0].status, StageStatus::Completed);
    assert_eq!(s[0].completed_at, Some(4_000));
    assert_eq!(
        s[0].elapsed_ms,
        Some(3_000),
        "a closed stage is frozen at its exit"
    );
    assert_eq!(s[1].status, StageStatus::Ready);
    assert_eq!(current_stage(&s), Some(2));
}

#[test]
fn a_repeated_station_is_two_separate_stages() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.status = AthleteStatus::Active;
    a.runs.push(run("RUN", 1_000, Some(2_000)));
    a.runs.push(run("ROWING", 3_000, Some(4_000)));
    a.runs.push(run("RUN", 5_000, Some(6_000)));

    let s = stages(&course(), &a, NOW);

    assert_eq!(s[0].status, StageStatus::Completed);
    assert_eq!(
        s[2].status,
        StageStatus::Completed,
        "the second run is its own stage"
    );
    assert_eq!(s[2].started_at, Some(5_000));
    assert_eq!(s[3].status, StageStatus::Ready);
}

/// Training records what actually happens and must not warn on a different order
/// (CLAUDE.md 9.2). A stepped-over stage is labelled, not rejected.
#[test]
fn a_station_the_athlete_went_past_is_skipped() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.status = AthleteStatus::Active;
    a.runs.push(run("RUN", 1_000, Some(2_000)));
    a.runs.push(run("WALL BALLS", 3_000, Some(4_000)));

    let s = stages(&course(), &a, NOW);

    assert_eq!(s[0].status, StageStatus::Completed);
    assert_eq!(
        s[1].status,
        StageStatus::Skipped,
        "the rower was passed over"
    );
    assert_eq!(s[2].status, StageStatus::Skipped);
    assert_eq!(s[3].status, StageStatus::Completed);
}

#[test]
fn the_transition_before_a_stage_is_carried_on_it() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.status = AthleteStatus::Active;
    a.runs.push(run("RUN", 1_000, Some(2_000)));
    let mut second = run("ROWING", 18_300, Some(20_000));
    second.transition_from_prev = Some(domain::Duration(16_300));
    a.runs.push(second);

    let s = stages(&course(), &a, NOW);

    assert_eq!(s[1].transition_ms, Some(16_300));
    assert_eq!(s[0].transition_ms, None, "nothing precedes the first stage");
}

/// A group class ends when its time is up and most athletes are short of the last station.
/// That is the normal outcome, not an error (CLAUDE.md 12, settled 2026-08-27).
#[test]
fn stages_the_class_ended_before_are_dnf_not_pending() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.status = AthleteStatus::Finished;
    a.finished_at = Some(Instant(9_000));
    a.runs.push(run("RUN", 1_000, Some(2_000)));

    let s = stages(&course(), &a, NOW);

    assert_eq!(s[0].status, StageStatus::Completed);
    assert_eq!(s[1].status, StageStatus::Dnf);
    assert_eq!(s[3].status, StageStatus::Dnf);
    assert_eq!(
        current_stage(&s),
        None,
        "a finished athlete is not on a stage"
    );
}

/// An athlete caught inside a station when the class ended keeps that run open: no reader
/// reported them leaving, and inventing an exit would fabricate a split (CLAUDE.md 19).
#[test]
fn an_athlete_stopped_mid_station_shows_that_stage_as_dnf() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.status = AthleteStatus::Finished;
    a.finished_at = Some(Instant(9_000));
    a.runs.push(run("RUN", 1_000, None));

    let s = stages(&course(), &a, NOW);

    assert_eq!(s[0].status, StageStatus::Dnf);
    assert_eq!(
        s[0].elapsed_ms,
        Some(8_000),
        "frozen at the class end, not still counting"
    );
}

#[test]
fn a_class_with_no_course_has_no_stages() {
    let a = AthleteState::ready("a1", "TONY");
    assert!(stages(&Course::new("none", vec![]), &a, NOW).is_empty());
}

// --- expectation (workout brief §11, §20, scenario C) ---------------------------------------

use application::current_expectation;
use domain::Expectation;

/// Scenario C: the athlete's plan says the rower next, and a ROW station reads their band.
#[test]
fn arriving_where_the_plan_says_is_expected() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.status = AthleteStatus::Active;
    a.runs.push(run("RUN", 1_000, Some(2_000)));
    a.runs.push(run("ROWING", 3_000, None));
    a.current_station = Some("ROWING".into());

    assert_eq!(
        current_expectation(&course(), &a),
        Some(Expectation::Expected)
    );
}

/// The other half of scenario C: the same athlete walking into wall balls instead. Recorded,
/// never a disqualification (brief §20).
#[test]
fn arriving_further_down_the_plan_is_out_of_order() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.status = AthleteStatus::Active;
    a.runs.push(run("RUN", 1_000, Some(2_000)));
    a.runs.push(run("WALL BALLS", 3_000, None));
    a.current_station = Some("WALL BALLS".into());

    assert_eq!(
        current_expectation(&course(), &a),
        Some(Expectation::OutOfOrder)
    );
}

#[test]
fn arriving_somewhere_the_plan_never_mentions_is_unexpected() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.status = AthleteStatus::Active;
    a.runs.push(run("SLED PUSH", 1_000, None));
    a.current_station = Some("SLED PUSH".into());

    assert_eq!(
        current_expectation(&course(), &a),
        Some(Expectation::Unexpected)
    );
}

#[test]
fn an_athlete_between_stations_has_nothing_to_judge() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.runs.push(run("RUN", 1_000, Some(2_000)));

    assert_eq!(current_expectation(&course(), &a), None);
}

/// Training must not warn on a different order (CLAUDE.md 9.2). With no plan there is no
/// answer, and "no answer" is not "wrong".
#[test]
fn a_class_with_no_plan_judges_nothing() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.current_station = Some("ROWING".into());

    assert_eq!(
        current_expectation(&Course::new("none", vec![]), &a),
        Some(Expectation::Unknown)
    );
}
