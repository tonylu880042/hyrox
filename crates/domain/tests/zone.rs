//! ADR 0017: one reader per station whose antenna covers the whole station area. The edge
//! reports a tag arriving and, once it has been gone for `absent_timeout`, departing; the
//! hub turns the pair into a station visit, and ignores visits too short to be real.

use domain::*;

const T0: i64 = 1_787_734_800_000;

fn armed() -> Session {
    let mut s = Session::new_draft("s1", "Race", SessionMode::Competition);
    s.mark_ready().unwrap();
    s.start().unwrap();
    s
}

fn zone(station: &str) -> ReaderBinding {
    ReaderBinding::new(station, ReaderMode::Zone)
}

fn line() -> ReaderBinding {
    ReaderBinding::new("FINISH", ReaderMode::StartFinish)
}

fn at(offset_ms: i64) -> Instant {
    Instant(T0 + offset_ms)
}

fn arrive(a: &mut AthleteState, r: &ReaderBinding, t: i64, s: &Session) -> Interpreted {
    interpret_read(a, r, Presence::Arrived, at(t), s)
}

fn depart(a: &mut AthleteState, r: &ReaderBinding, t: i64, s: &Session) -> Interpreted {
    interpret_read(a, r, Presence::Departed, at(t), s)
}

// --- a station visit from arrival and departure ----------------------------------------

#[test]
fn arriving_enters_and_departing_exits_at_the_last_sighting() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(&mut a, &line(), 0, &s);

    let r1 = arrive(&mut a, &zone("SKIERG"), 240_000, &s);
    let r2 = depart(&mut a, &zone("SKIERG"), 500_000, &s);

    assert!(
        matches!(r1, Interpreted::Entered { transition: Some(d), .. } if d == Duration(240_000))
    );
    assert_eq!(
        r2,
        Interpreted::Exited {
            station: "SKIERG".into(),
            at: at(500_000)
        }
    );
    assert_eq!(a.station_state, StationState::Outside);
    assert_eq!(a.runs[0].exited_at, Some(at(500_000)));
}

#[test]
fn a_departure_means_nothing_to_a_reader_that_is_not_a_zone() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(
        &mut a,
        &ReaderBinding::new("SKIERG", ReaderMode::Toggle),
        0,
        &s,
    );

    let r = depart(
        &mut a,
        &ReaderBinding::new("SKIERG", ReaderMode::Toggle),
        60_000,
        &s,
    );

    assert!(matches!(r, Interpreted::Passed { .. }), "got {r:?}");
    assert_eq!(a.station_state, StationState::Inside);
}

#[test]
fn a_departure_from_a_zone_the_athlete_is_not_in_is_recorded_not_interpreted() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(&mut a, &line(), 0, &s);

    let r = depart(&mut a, &zone("SKIERG"), 60_000, &s);

    assert!(matches!(r, Interpreted::Passed { .. }), "got {r:?}");
}

#[test]
fn arriving_at_the_next_zone_while_still_inside_one_is_a_missed_departure() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(&mut a, &zone("SKIERG"), 0, &s);

    arrive(&mut a, &zone("SLED_PUSH"), 500_000, &s);

    assert!(a.runs[0].exit_missed);
    assert_eq!(a.current_station.as_deref(), Some("SLED_PUSH"));
}

// --- flicker guards (default 15 s, per reader) -----------------------------------------

#[test]
fn a_visit_shorter_than_the_guard_is_withdrawn_as_a_pass_by() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(&mut a, &line(), 0, &s);
    arrive(&mut a, &zone("SKIERG"), 240_000, &s);
    arrive(&mut a, &zone("SKIERG"), 240_000, &s); // duplicate arrival is not a second visit
    assert_eq!(a.runs.len(), 1);

    // The route brushed the edge of the ROWING zone for 6 s.
    arrive(&mut a, &zone("ROWING"), 600_000, &s);
    let r = depart(&mut a, &zone("ROWING"), 606_000, &s);

    assert_eq!(
        r,
        Interpreted::VisitWithdrawn {
            station: "ROWING".into(),
            at: at(606_000)
        }
    );
    assert_eq!(a.runs.len(), 1, "the brush is not a station");
    assert_eq!(
        a.current_station.as_deref(),
        Some("SKIERG"),
        "withdrawing the visit undoes the missed exit it implied"
    );
    assert!(!a.runs[0].exit_missed);
}

#[test]
fn a_pass_by_before_anything_else_does_not_leave_the_clock_running() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(&mut a, &zone("ROWING"), 0, &s);

    depart(&mut a, &zone("ROWING"), 5_000, &s);

    assert_eq!(a.status, AthleteStatus::Ready);
    assert_eq!(a.started_at, None);
}

#[test]
fn coming_back_within_the_guard_after_a_departure_reopens_the_same_visit() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(&mut a, &line(), 0, &s);
    arrive(&mut a, &zone("SKIERG"), 240_000, &s);
    // Hidden behind the erg for longer than absent_timeout, then seen again.
    depart(&mut a, &zone("SKIERG"), 300_000, &s);

    let r = arrive(&mut a, &zone("SKIERG"), 310_000, &s);

    assert_eq!(
        r,
        Interpreted::ExitWithdrawn {
            station: "SKIERG".into(),
            at: at(310_000)
        }
    );
    assert_eq!(a.runs.len(), 1, "one visit, not two");
    assert_eq!(a.station_state, StationState::Inside);
    assert_eq!(a.runs[0].exited_at, None);

    depart(&mut a, &zone("SKIERG"), 500_000, &s);
    arrive(&mut a, &zone("SLED_PUSH"), 740_000, &s);
    assert_eq!(a.runs[0].exited_at, Some(at(500_000)));
    assert_eq!(
        a.runs[1].transition_from_prev,
        Some(Duration(240_000)),
        "the run is measured from the real departure"
    );
}

#[test]
fn coming_back_after_the_guard_is_a_new_visit() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(&mut a, &zone("SKIERG"), 0, &s);
    depart(&mut a, &zone("SKIERG"), 300_000, &s);

    let r = arrive(&mut a, &zone("SKIERG"), 316_000, &s);

    assert!(matches!(r, Interpreted::Entered { .. }), "got {r:?}");
    assert_eq!(a.runs.len(), 2);
}

#[test]
fn the_guard_is_set_per_reader() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    let strict = zone("SKIERG").with_min_visit(Duration(2_000));
    arrive(&mut a, &strict, 0, &s);

    let r = depart(&mut a, &strict, 6_000, &s);

    assert!(matches!(r, Interpreted::Exited { .. }), "got {r:?}");
}

#[test]
fn replay_rebuilds_withdrawals_exactly() {
    let s = armed();
    let mut live = AthleteState::ready("a1", "Chen");
    let log: Vec<Interpreted> = vec![
        arrive(&mut live, &line(), 0, &s),
        arrive(&mut live, &zone("SKIERG"), 240_000, &s),
        arrive(&mut live, &zone("ROWING"), 300_000, &s),
        depart(&mut live, &zone("ROWING"), 304_000, &s),
        depart(&mut live, &zone("SKIERG"), 400_000, &s),
        arrive(&mut live, &zone("SKIERG"), 405_000, &s),
        depart(&mut live, &zone("SKIERG"), 500_000, &s),
    ];

    let replayed = replay("a1", "Chen", &log);

    assert_eq!(format!("{replayed:?}"), format!("{live:?}"));
}

#[test]
fn a_withdrawal_that_no_longer_fits_the_corrected_history_is_dropped() {
    let a = AthleteState::ready("a1", "Chen");
    let orphan = Interpreted::VisitWithdrawn {
        station: "ROWING".into(),
        at: at(0),
    };
    assert_eq!(rederive(&a, orphan), None);
    let orphan = Interpreted::ExitWithdrawn {
        station: "ROWING".into(),
        at: at(0),
    };
    assert_eq!(rederive(&a, orphan), None);
}

// --- the finish line -------------------------------------------------------------------

fn race() -> Course {
    Course::new(
        "RACE",
        vec![CourseStep::new("SKIERG"), CourseStep::new("WALL_BALLS")],
    )
}

fn evaluate(a: &AthleteState) -> FinishDecision {
    FinishPolicy::FinishLine.evaluate(
        a,
        ClassClock::started_at(at(0)),
        at(9_999_999),
        Some(&race()),
    )
}

#[test]
fn crossing_the_line_after_the_last_station_is_the_result() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(&mut a, &line(), 0, &s);
    arrive(&mut a, &zone("SKIERG"), 240_000, &s);
    depart(&mut a, &zone("SKIERG"), 500_000, &s);
    let mid = arrive(&mut a, &line(), 600_000, &s); // the route crosses the line mid-race
    assert_eq!(mid, Interpreted::Crossed { at: at(600_000) });
    arrive(&mut a, &zone("WALL_BALLS"), 800_000, &s);
    assert_eq!(evaluate(&a), FinishDecision::NotFinished);

    // The departure is published absent_timeout after the athlete left, so it reaches the
    // hub after the crossing even though it happened first.
    arrive(&mut a, &line(), 1_210_000, &s);
    assert_eq!(evaluate(&a), FinishDecision::NotFinished, "still inside");
    depart(&mut a, &zone("WALL_BALLS"), 1_200_000, &s);

    assert_eq!(evaluate(&a), FinishDecision::Finished { at: at(1_210_000) });
}

#[test]
fn completing_the_course_without_crossing_the_line_is_not_finishing() {
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(&mut a, &line(), 0, &s);
    arrive(&mut a, &zone("SKIERG"), 240_000, &s);
    depart(&mut a, &zone("SKIERG"), 500_000, &s);
    arrive(&mut a, &zone("WALL_BALLS"), 800_000, &s);
    depart(&mut a, &zone("WALL_BALLS"), 1_200_000, &s);

    assert_eq!(evaluate(&a), FinishDecision::NotFinished);
}

#[test]
fn a_crossing_does_not_count_as_a_read_for_ordering() {
    // The late departure must not be rejected as out of order behind the crossing.
    let s = armed();
    let mut a = AthleteState::ready("a1", "Chen");
    arrive(&mut a, &zone("SKIERG"), 0, &s);
    arrive(&mut a, &line(), 300_000, &s);
    assert_eq!(a.last_event_at, Some(at(0)));
}
