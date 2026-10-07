//! The venue layout of ADR 0016: one start/finish mat (1S = 9N) that also closes the last
//! station, dedicated ENTRY/EXIT gates elsewhere, and missed reads that must not jam an
//! athlete for the rest of the race.

use domain::*;

const T0: i64 = 1_787_734_800_000;

fn armed() -> Session {
    let mut s = Session::new_draft("s1", "Race", SessionMode::Competition);
    s.mark_ready().unwrap();
    s.start().unwrap();
    s
}

fn reader(station: &str, mode: ReaderMode) -> ReaderBinding {
    ReaderBinding {
        station: station.into(),
        mode,
    }
}

fn mat() -> ReaderBinding {
    reader("WALL_BALLS", ReaderMode::StartFinish)
}

fn at(offset_ms: i64) -> Instant {
    Instant(T0 + offset_ms)
}

// --- the start/finish mat --------------------------------------------------------------

#[test]
fn crossing_the_mat_before_anything_else_starts_the_clock() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");

    let r = interpret(&mut a, &mat(), at(0), &s);

    assert_eq!(r, Interpreted::Started { at: at(0) });
    assert_eq!(a.status, AthleteStatus::Active);
    assert_eq!(a.started_at, Some(at(0)));
    assert_eq!(a.station_state, StationState::Outside);
    assert!(a.runs.is_empty(), "the mat is not a station visit");
}

#[test]
fn the_first_run_is_measured_from_the_start_mat() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &mat(), at(0), &s);

    let r = interpret(
        &mut a,
        &reader("SKIERG", ReaderMode::Entry),
        at(240_000),
        &s,
    );

    assert_eq!(
        r,
        Interpreted::Entered {
            station: "SKIERG".into(),
            at: at(240_000),
            transition: Some(Duration(240_000)),
            started_timing: false,
        },
        "run 1 = 2S - 1S, and the clock was already running"
    );
}

#[test]
fn the_mat_closes_the_station_it_is_registered_against() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &mat(), at(0), &s);
    interpret(
        &mut a,
        &reader("WALL_BALLS", ReaderMode::Entry),
        at(10_000),
        &s,
    );

    let r = interpret(&mut a, &mat(), at(70_000), &s);

    assert_eq!(
        r,
        Interpreted::Exited {
            station: "WALL_BALLS".into(),
            at: at(70_000)
        }
    );
    assert_eq!(a.runs[0].exited_at, Some(at(70_000)));
}

#[test]
fn passing_the_mat_mid_race_is_recorded_and_changes_nothing() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &mat(), at(0), &s);
    interpret(
        &mut a,
        &reader("SKIERG", ReaderMode::Entry),
        at(240_000),
        &s,
    );
    interpret(&mut a, &reader("SKIERG", ReaderMode::Exit), at(500_000), &s);
    let before = format!("{a:?}");

    let r = interpret(&mut a, &mat(), at(600_000), &s);

    assert_eq!(
        r,
        Interpreted::Passed {
            station: "WALL_BALLS".into(),
            at: at(600_000)
        },
        "the route passes the mat; that is not an exception"
    );
    assert_eq!(format!("{a:?}"), before, "a pass must not move the athlete");
}

#[test]
fn a_finished_athlete_lingering_on_the_mat_is_a_pass_not_an_exception() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &mat(), at(0), &s);
    finish(&mut a, at(3_600_000));

    let r = interpret(&mut a, &mat(), at(3_610_000), &s);

    assert!(matches!(r, Interpreted::Passed { .. }), "got {r:?}");
}

#[test]
fn the_mat_before_the_session_is_armed_is_still_an_exception() {
    let s = Session::new_draft("s1", "Race", SessionMode::Competition);
    let mut a = AthleteState::ready("a1", "Chen");

    let r = interpret(&mut a, &mat(), at(0), &s);

    assert_eq!(
        r,
        Interpreted::Exception {
            reason: ExceptionReason::SessionNotArmed,
            at: at(0)
        }
    );
}

// --- missed exits ----------------------------------------------------------------------

#[test]
fn entering_another_station_while_inside_one_records_the_entry_and_flags_the_missed_exit() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &mat(), at(0), &s);
    interpret(
        &mut a,
        &reader("SKIERG", ReaderMode::Entry),
        at(240_000),
        &s,
    );
    // 2N is never read.

    let r = interpret(
        &mut a,
        &reader("SLED_PUSH", ReaderMode::Entry),
        at(800_000),
        &s,
    );

    assert_eq!(
        r,
        Interpreted::Entered {
            station: "SLED_PUSH".into(),
            at: at(800_000),
            transition: None,
            started_timing: false,
        },
        "no exit time is known, so there is nothing to subtract"
    );
    assert_eq!(a.current_station.as_deref(), Some("SLED_PUSH"));
    assert!(a.runs[0].exit_missed);
    assert_eq!(a.runs[0].exited_at, None, "no exit time is invented");
    assert!(!a.runs[1].exit_missed);
}

#[test]
fn a_toggle_at_another_station_while_inside_is_also_a_missed_exit() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &reader("SLED_PUSH", ReaderMode::Toggle), at(0), &s);

    let r = interpret(
        &mut a,
        &reader("ROWING", ReaderMode::Toggle),
        at(10_000),
        &s,
    );

    assert!(matches!(r, Interpreted::Entered { ref station, .. } if station == "ROWING"));
    assert!(a.runs[0].exit_missed);
}

#[test]
fn timing_resumes_normally_after_a_missed_exit() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(0), &s);
    interpret(
        &mut a,
        &reader("SLED_PUSH", ReaderMode::Entry),
        at(500_000),
        &s,
    );
    interpret(
        &mut a,
        &reader("SLED_PUSH", ReaderMode::Exit),
        at(600_000),
        &s,
    );

    let r = interpret(
        &mut a,
        &reader("SLED_PULL", ReaderMode::Entry),
        at(840_000),
        &s,
    );

    assert!(
        matches!(r, Interpreted::Entered { transition: Some(d), .. } if d == Duration(240_000))
    );
}

#[test]
fn replay_rebuilds_a_missed_exit_exactly() {
    let s = armed();
    let mut live = AthleteState::ready("a1", "Chen");
    let log: Vec<Interpreted> = [
        (mat(), 0),
        (reader("SKIERG", ReaderMode::Entry), 240_000),
        (reader("SLED_PUSH", ReaderMode::Entry), 800_000),
    ]
    .iter()
    .map(|(r, t)| interpret(&mut live, r, at(*t), &s))
    .collect();

    let replayed = replay("a1", "Chen", &log);

    assert_eq!(format!("{replayed:?}"), format!("{live:?}"));
}

// --- re-deriving a stored log (CLAUDE.md 20, 21) ---------------------------------------

#[test]
fn rederiving_drops_a_second_entry_into_the_same_station() {
    let mut a = AthleteState::ready("a1", "Chen");
    let first = Interpreted::Entered {
        station: "SKIERG".into(),
        at: at(0),
        transition: None,
        started_timing: true,
    };
    apply(&mut a, &first);

    let again = Interpreted::Entered {
        station: "SKIERG".into(),
        at: at(1_000),
        transition: None,
        started_timing: false,
    };
    assert_eq!(rederive(&a, again), None);
}

#[test]
fn rederiving_recomputes_transition_from_the_corrected_prefix() {
    let mut a = AthleteState::ready("a1", "Chen");
    apply(&mut a, &Interpreted::Started { at: at(0) });

    let stale = Interpreted::Entered {
        station: "SKIERG".into(),
        at: at(240_000),
        transition: Some(Duration(1)),
        started_timing: true,
    };
    assert_eq!(
        rederive(&a, stale),
        Some(Interpreted::Entered {
            station: "SKIERG".into(),
            at: at(240_000),
            transition: Some(Duration(240_000)),
            started_timing: false,
        })
    );
}

#[test]
fn rederiving_a_start_that_is_no_longer_first_drops_it() {
    let mut a = AthleteState::ready("a1", "Chen");
    apply(
        &mut a,
        &Interpreted::Entered {
            station: "SKIERG".into(),
            at: at(0),
            transition: None,
            started_timing: true,
        },
    );
    assert_eq!(rederive(&a, Interpreted::Started { at: at(1_000) }), None);
}

// --- finishing (CourseComplete) --------------------------------------------------------

fn race() -> Course {
    Course::new(
        "RACE",
        vec![
            CourseStep::new("SKIERG"),
            CourseStep::new("SLED_PUSH"),
            CourseStep::new("WALL_BALLS"),
        ],
    )
}

fn evaluate(a: &AthleteState) -> FinishDecision {
    FinishPolicy::CourseComplete.evaluate(
        a,
        ClassClock::started_at(at(0)),
        at(9_999_999),
        Some(&race()),
    )
}

#[test]
fn the_whole_layout_finishes_on_the_mat() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    for (r, t) in [
        (mat(), 0),
        (reader("SKIERG", ReaderMode::Entry), 240_000),
        (reader("SKIERG", ReaderMode::Exit), 500_000),
        (mat(), 600_000), // the route passes the mat
        (reader("SLED_PUSH", ReaderMode::Entry), 740_000),
        (reader("SLED_PUSH", ReaderMode::Exit), 900_000),
        (reader("WALL_BALLS", ReaderMode::Entry), 1_140_000),
    ] {
        interpret(&mut a, &r, at(t), &s);
    }
    assert_eq!(evaluate(&a), FinishDecision::NotFinished);

    interpret(&mut a, &mat(), at(1_500_000), &s);

    assert_eq!(evaluate(&a), FinishDecision::Finished { at: at(1_500_000) });
}

#[test]
fn a_missed_exit_does_not_stop_the_athlete_finishing() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    for (r, t) in [
        (mat(), 0),
        (reader("SKIERG", ReaderMode::Entry), 240_000),
        // 2N missed
        (reader("SLED_PUSH", ReaderMode::Entry), 740_000),
        (reader("SLED_PUSH", ReaderMode::Exit), 900_000),
        (reader("WALL_BALLS", ReaderMode::Entry), 1_140_000),
        (mat(), 1_500_000),
    ] {
        interpret(&mut a, &r, at(t), &s);
    }

    assert_eq!(evaluate(&a), FinishDecision::Finished { at: at(1_500_000) });
}

#[test]
fn a_missed_exit_counts_as_a_completed_step_for_expectation() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(0), &s);
    interpret(
        &mut a,
        &reader("SLED_PUSH", ReaderMode::Entry),
        at(500_000),
        &s,
    );
    interpret(
        &mut a,
        &reader("SLED_PUSH", ReaderMode::Exit),
        at(600_000),
        &s,
    );

    assert_eq!(
        expectation(&race(), &a, "WALL_BALLS"),
        Expectation::Expected
    );
}
