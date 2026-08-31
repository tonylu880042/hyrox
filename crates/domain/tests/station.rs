//! Physical stations and expectation (workout brief §11, §12, §20).

use domain::{
    AthleteState, Course, CourseStep, Expectation, ExerciseLibrary, PhysicalStation, StationMap,
    expectation,
};

fn map() -> StationMap {
    StationMap::new(vec![
        PhysicalStation::new("ROW_01", "ROWERG", "Rower 1"),
        PhysicalStation::new("ROW_02", "ROWERG", "Rower 2"),
        PhysicalStation::new("ROW_03", "ROWERG", "Rower 3"),
        PhysicalStation::new("WB_01", "WALL_BALL", "Wall Ball 1"),
    ])
}

// --- an exercise is not a station (brief §12) -------------------------------------------

#[test]
fn one_exercise_can_be_served_by_several_physical_stations() {
    let m = map();
    let ids: Vec<&str> = m.serving("ROWERG").map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["ROW_01", "ROW_02", "ROW_03"]);
}

#[test]
fn a_station_declares_the_one_exercise_it_can_serve() {
    let m = map();
    assert!(m.can_serve("ROW_02", "ROWERG"));
    assert!(!m.can_serve("ROW_02", "WALL_BALL"), "a rower is not a wall ball target");
}

#[test]
fn an_unknown_station_serves_nothing() {
    assert!(!map().can_serve("ROW_99", "ROWERG"));
}

/// The station key the course carries is the exercise's, not the machine's. Resolving a
/// physical station to that key is what lets ROW_02 satisfy a "ROWING" step.
#[test]
fn a_physical_station_resolves_to_the_course_station_key() {
    let lib = ExerciseLibrary::preset();
    let m = map();
    assert_eq!(m.station_key("ROW_02", &lib).as_deref(), Some("ROWING"));
    assert_eq!(m.station_key("WB_01", &lib).as_deref(), Some("WALL BALLS"));
    assert_eq!(m.station_key("NOPE", &lib), None);
}

// --- expectation (brief §11, §20) --------------------------------------------------------

fn course() -> Course {
    Course::new(
        "Engine",
        vec![
            CourseStep::new("RUN"),
            CourseStep::new("ROWING"),
            CourseStep::new("WALL BALLS"),
        ],
    )
}

/// Scenario C from the brief: the athlete's current stage is the rower, a ROW station reads
/// their band, and the hub calls it EXPECTED.
#[test]
fn arriving_at_the_station_the_plan_calls_for_is_expected() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.runs.push(finished_run("RUN"));

    assert_eq!(expectation(&course(), &a, "ROWING"), Expectation::Expected);
}

#[test]
fn the_first_station_of_the_course_is_expected_for_an_athlete_who_has_done_nothing() {
    let a = AthleteState::ready("a1", "TONY");
    assert_eq!(expectation(&course(), &a, "RUN"), Expectation::Expected);
}

/// Also scenario C: the same athlete walking into wall balls instead. Recorded, never a
/// disqualification -- the rule engine that might care does not exist yet (brief §20).
#[test]
fn skipping_ahead_in_the_course_is_out_of_order_not_unexpected() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.runs.push(finished_run("RUN"));

    assert_eq!(expectation(&course(), &a, "WALL BALLS"), Expectation::OutOfOrder);
}

#[test]
fn going_back_to_a_station_already_completed_is_out_of_order() {
    let mut a = AthleteState::ready("a1", "TONY");
    a.runs.push(finished_run("RUN"));
    a.runs.push(finished_run("ROWING"));

    assert_eq!(expectation(&course(), &a, "RUN"), Expectation::OutOfOrder);
}

#[test]
fn a_station_the_course_never_mentions_is_unexpected() {
    let a = AthleteState::ready("a1", "TONY");
    assert_eq!(expectation(&course(), &a, "SLED PUSH"), Expectation::Unexpected);
}

/// Past the end of the plan there is no next station to compare against, so nothing about
/// the read is out of order -- it is simply not in the plan.
#[test]
fn a_read_after_the_last_step_is_unexpected() {
    let mut a = AthleteState::ready("a1", "TONY");
    for station in ["RUN", "ROWING", "WALL BALLS"] {
        a.runs.push(finished_run(station));
    }
    assert_eq!(expectation(&course(), &a, "RUN"), Expectation::OutOfOrder);
    assert_eq!(expectation(&course(), &a, "SLED PUSH"), Expectation::Unexpected);
}

/// Training records what actually happens and must never warn on a different order
/// (CLAUDE.md 9.2). Expectation is a label a screen may show, not a gate -- a course with
/// no plan cannot judge anything.
#[test]
fn a_session_with_no_course_judges_nothing() {
    let a = AthleteState::ready("a1", "TONY");
    let empty = Course::new("none", vec![]);
    assert_eq!(expectation(&empty, &a, "ROWING"), Expectation::Unknown);
}

fn finished_run(station: &str) -> domain::StationRun {
    domain::StationRun {
        station: station.to_string(),
        entered_at: domain::Instant(0),
        exited_at: Some(domain::Instant(1)),
        transition_from_prev: None,
    }
}
