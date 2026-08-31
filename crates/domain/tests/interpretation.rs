//! Behaviour required by CLAUDE.md 24 for this slice: session lifecycle, the competition
//! start rule, ENTRY/EXIT and TOGGLE interpretation, and transition timing.

use domain::*;

const T0: i64 = 1_787_734_800_000;

fn armed(mode: SessionMode) -> Session {
    let mut s = Session::new_draft("s1", "Test", mode);
    s.mark_ready().unwrap();
    s.start().unwrap();
    s
}

fn reader(station: &str, mode: ReaderMode) -> ReaderBinding {
    ReaderBinding { station: station.into(), mode }
}

fn at(offset_ms: i64) -> Instant {
    Instant(T0 + offset_ms)
}

// --- session lifecycle (ADR 0001 D2) -------------------------------------------------

#[test]
fn draft_session_rejects_events_and_arming_opens_it() {
    let mut s = Session::new_draft("s1", "Test", SessionMode::Training);
    assert!(!s.accepts_events());
    s.mark_ready().unwrap();
    s.start().unwrap();
    assert!(s.accepts_events());
    assert_eq!(s.status, SessionStatus::Running);
}

#[test]
fn armed_can_return_to_draft_only_while_nothing_has_been_interpreted() {
    let mut s = armed(SessionMode::Training);
    s.back_to_draft().unwrap();
    assert_eq!(s.status, SessionStatus::Draft);

    let mut s = armed(SessionMode::Training);
    s.interpreted_event_count = 1;
    assert_eq!(s.back_to_draft(), Err(SessionError::HasInterpretedEvents));
    assert_eq!(s.status, SessionStatus::Running, "a rejected transition must not mutate");
}

#[test]
fn completed_session_can_be_reopened() {
    // Deliberate: a mis-tap on a busy floor must not force a new session (D2).
    let mut s = armed(SessionMode::Training);
    s.complete().unwrap();
    assert_eq!(s.status, SessionStatus::Completed);
    s.reopen().unwrap();
    assert_eq!(s.status, SessionStatus::Running);
}

#[test]
fn draft_cannot_be_completed_directly() {
    let mut s = Session::new_draft("s1", "Test", SessionMode::Training);
    assert!(matches!(s.complete(), Err(SessionError::IllegalTransition { .. })));
}

// --- start rule (CLAUDE.md 11) -------------------------------------------------------

#[test]
fn first_valid_event_after_arming_starts_timing_at_detected_at() {
    let s = armed(SessionMode::Competition);
    let mut a = AthleteState::ready("a1", "Chen");
    assert_eq!(a.status, AthleteStatus::Ready);

    let r = interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(0), &s);
    assert!(matches!(r, Interpreted::Entered { started_timing: true, .. }));
    assert_eq!(a.status, AthleteStatus::Active);
    assert_eq!(a.started_at, Some(at(0)), "started_at must be detected_at, not arrival time");
}

#[test]
fn only_the_first_event_starts_the_clock() {
    let s = armed(SessionMode::Competition);
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(0), &s);
    let r = interpret(&mut a, &reader("SKIERG", ReaderMode::Exit), at(60_000), &s);
    assert!(matches!(r, Interpreted::Exited { .. }));
    assert_eq!(a.started_at, Some(at(0)), "the clock must not restart");
}

#[test]
fn events_before_arming_are_recorded_as_exceptions_not_dropped() {
    let s = Session::new_draft("s1", "Test", SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");
    let r = interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(0), &s);
    assert_eq!(
        r,
        Interpreted::Exception { reason: ExceptionReason::SessionNotArmed, at: at(0) }
    );
    assert_eq!(a.status, AthleteStatus::Ready, "a rejected read must not mutate state");
}

// --- dedicated ENTRY / EXIT readers (CLAUDE.md 10.1) ---------------------------------

#[test]
fn entry_then_exit_moves_outside_inside_outside() {
    let s = armed(SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");
    assert_eq!(a.station_state, StationState::Outside);

    interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(0), &s);
    assert_eq!(a.station_state, StationState::Inside);
    assert_eq!(a.current_station.as_deref(), Some("SKIERG"));

    interpret(&mut a, &reader("SKIERG", ReaderMode::Exit), at(90_000), &s);
    assert_eq!(a.station_state, StationState::Outside);
    assert_eq!(a.current_station, None);
    assert_eq!(a.runs[0].exited_at, Some(at(90_000)));
}

#[test]
fn entry_while_already_inside_is_an_exception() {
    let s = armed(SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(0), &s);
    let r = interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(1_000), &s);
    assert_eq!(
        r,
        Interpreted::Exception { reason: ExceptionReason::ImpossibleTransition, at: at(1_000) }
    );
    assert_eq!(a.runs.len(), 1, "an exception must not open a second run");
}

#[test]
fn exit_from_a_station_the_athlete_is_not_inside_is_an_exception() {
    let s = armed(SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(0), &s);
    let r = interpret(&mut a, &reader("ROWING", ReaderMode::Exit), at(1_000), &s);
    assert_eq!(
        r,
        Interpreted::Exception { reason: ExceptionReason::ImpossibleTransition, at: at(1_000) }
    );
    assert_eq!(a.station_state, StationState::Inside, "state must be unchanged");
}

// --- shared TOGGLE reader (CLAUDE.md 10.2) -------------------------------------------

#[test]
fn toggle_uses_athlete_state_not_scan_parity() {
    let s = armed(SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");

    let r1 = interpret(&mut a, &reader("SLED PUSH", ReaderMode::Toggle), at(0), &s);
    assert!(matches!(r1, Interpreted::Entered { .. }));

    let r2 = interpret(&mut a, &reader("SLED PUSH", ReaderMode::Toggle), at(45_000), &s);
    assert!(matches!(r2, Interpreted::Exited { .. }));

    let r3 = interpret(&mut a, &reader("SLED PUSH", ReaderMode::Toggle), at(80_000), &s);
    assert!(matches!(r3, Interpreted::Entered { .. }), "third scan re-enters");
}

#[test]
fn toggle_for_a_different_station_while_inside_is_an_exception() {
    let s = armed(SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &reader("SLED PUSH", ReaderMode::Toggle), at(0), &s);
    let r = interpret(&mut a, &reader("ROWING", ReaderMode::Toggle), at(10_000), &s);
    assert_eq!(
        r,
        Interpreted::Exception { reason: ExceptionReason::ImpossibleTransition, at: at(10_000) }
    );
}

// --- transition / ROX time (CLAUDE.md 13) --------------------------------------------

#[test]
fn transition_is_next_entry_minus_previous_exit() {
    let s = armed(SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");

    interpret(&mut a, &reader("RUN", ReaderMode::Entry), at(0), &s);
    interpret(&mut a, &reader("RUN", ReaderMode::Exit), at(920_500), &s); // 10:15:20.500
    let r = interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(936_800), &s); // +16.300s

    match r {
        Interpreted::Entered { transition, .. } => {
            assert_eq!(transition, Some(Duration(16_300)), "CLAUDE.md 13 worked example");
        }
        other => panic!("expected Entered, got {other:?}"),
    }
    assert_eq!(a.runs[1].transition_from_prev, Some(Duration(16_300)));
}

#[test]
fn first_station_has_no_transition() {
    let s = armed(SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");
    let r = interpret(&mut a, &reader("SKIERG", ReaderMode::Entry), at(0), &s);
    match r {
        Interpreted::Entered { transition, .. } => assert_eq!(transition, None),
        other => panic!("expected Entered, got {other:?}"),
    }
}

// --- training accepts any order (CLAUDE.md 9.2) --------------------------------------

#[test]
fn training_accepts_stations_in_any_order() {
    let s = armed(SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");
    // Deliberately not the official HYROX order.
    for (i, st) in ["WALL BALLS", "SKIERG", "ROWING"].iter().enumerate() {
        let t = (i as i64) * 100_000;
        let r = interpret(&mut a, &reader(st, ReaderMode::Entry), at(t), &s);
        assert!(matches!(r, Interpreted::Entered { .. }), "{st} should be accepted as-is");
        interpret(&mut a, &reader(st, ReaderMode::Exit), at(t + 50_000), &s);
    }
    assert_eq!(a.runs.len(), 3);
}

// --- derived readouts the live screen needs ------------------------------------------

#[test]
fn elapsed_and_current_leg_track_the_athlete() {
    let s = armed(SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");
    interpret(&mut a, &reader("ROWING", ReaderMode::Entry), at(0), &s);

    assert_eq!(a.elapsed(at(30_000)), Some(Duration(30_000)));
    assert_eq!(a.current_leg(at(30_000)), Some(Duration(30_000)), "inside: time in station");

    interpret(&mut a, &reader("ROWING", ReaderMode::Exit), at(60_000), &s);
    assert_eq!(a.current_leg(at(75_000)), Some(Duration(15_000)), "outside: time in transition");
}

// --- replay (CLAUDE.md 21, 24) --------------------------------------------------------

/// Drives one athlete through two full stations and returns (live state, the events).
fn run_two_stations() -> (AthleteState, Vec<Interpreted>) {
    let s = armed(SessionMode::Training);
    let mut a = AthleteState::ready("a1", "Chen");
    let mut log = Vec::new();
    for (station, mode, t) in [
        ("SKIERG", ReaderMode::Entry, 0),
        ("SKIERG", ReaderMode::Exit, 110_000),
        ("SLED PUSH", ReaderMode::Entry, 128_000),
        ("SLED PUSH", ReaderMode::Exit, 210_000),
    ] {
        log.push(interpret(&mut a, &reader(station, mode), at(t), &s));
    }
    (a, log)
}

#[test]
fn replaying_interpreted_events_rebuilds_identical_state() {
    let (live, log) = run_two_stations();
    let rebuilt = replay("a1", "Chen", &log);

    assert_eq!(rebuilt.status, live.status);
    assert_eq!(rebuilt.station_state, live.station_state);
    assert_eq!(rebuilt.current_station, live.current_station);
    assert_eq!(rebuilt.started_at, live.started_at);
    assert_eq!(rebuilt.last_event_at, live.last_event_at);
    assert_eq!(rebuilt.last_exit_at, live.last_exit_at);
    assert_eq!(rebuilt.runs.len(), live.runs.len());
    for (r, l) in rebuilt.runs.iter().zip(live.runs.iter()) {
        assert_eq!(r.station, l.station);
        assert_eq!(r.entered_at, l.entered_at);
        assert_eq!(r.exited_at, l.exited_at);
        assert_eq!(r.transition_from_prev, l.transition_from_prev);
    }
}

#[test]
fn replay_is_idempotent_across_repeated_rebuilds() {
    let (_, log) = run_two_stations();
    let once = replay("a1", "Chen", &log);
    let twice = replay("a1", "Chen", &log);
    assert_eq!(once.runs.len(), twice.runs.len());
    assert_eq!(once.started_at, twice.started_at);
    assert_eq!(once.last_exit_at, twice.last_exit_at);
}

#[test]
fn exceptions_in_the_log_do_not_advance_state_on_replay() {
    let (_, mut log) = run_two_stations();
    log.insert(
        2,
        Interpreted::Exception { reason: ExceptionReason::ImpossibleTransition, at: at(115_000) },
    );
    let rebuilt = replay("a1", "Chen", &log);
    assert_eq!(rebuilt.runs.len(), 2, "an exception must not open a run");
    assert_eq!(rebuilt.last_exit_at, Some(at(210_000)));
}

#[test]
fn voiding_an_interpreted_event_changes_the_rebuilt_state() {
    // How an operator correction reaches the derived values (CLAUDE.md 20): the raw event
    // stays, the interpreted one is excluded from replay, and everything recomputes.
    let (_, log) = run_two_stations();
    let kept: Vec<_> = log
        .iter()
        .filter(|e| !matches!(e, Interpreted::Entered { station, .. } if station == "SLED PUSH"))
        .cloned()
        .collect();
    let rebuilt = replay("a1", "Chen", &kept);
    assert_eq!(rebuilt.runs.len(), 1, "the voided station must be gone");
    assert_eq!(rebuilt.station_state, StationState::Outside);
}
