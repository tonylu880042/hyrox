//! The live read model the screens render (CLAUDE.md 22, 23; ADR 0001 D5).

use application::{course_view, snapshot, LiveSession};
use domain::{
    AthleteState, AthleteStatus, Course, CourseStep, Duration, Instant, Interpreted, ReaderKey,
    ReaderMode, ReaderRegistration, ReaderRegistry, Session, SessionConfig, SessionMode,
    StationState, StationTarget,
};

const START: Instant = Instant(1_000_000);

fn course() -> Course {
    Course::new(
        "HYROX CLASS",
        vec![
            CourseStep::new("SKIERG").with_target(StationTarget::Distance { meters: 500 }),
            CourseStep::new("SLED PUSH").with_target(StationTarget::Distance { meters: 25 }),
            CourseStep::new("WALL BALLS").with_target(StationTarget::Repetitions { count: 50 }),
        ],
    )
}

fn session() -> LiveSession {
    let mut s = Session::new_draft("s1", "THU 19:00 HYROX CLASS", SessionMode::Training);
    s.mark_ready().expect("arm");
    s.start().expect("arm");
    let mut readers = ReaderRegistry::new();
    readers.register(ReaderRegistration::new(
        ReaderKey::parse("a4cf128b3d91", "rfid-01").unwrap(),
        "SKIERG",
        ReaderMode::Entry,
    ));
    LiveSession::new(s, SessionConfig::new("s1").with_course(course()), START)
        .with_athletes(vec![AthleteState::ready("a1", "CHEN YU-TING")])
        .with_readers(readers)
}

#[test]
fn the_course_carries_a_stable_key_and_a_printable_plan() {
    let view = course_view(Some(&course()));

    assert_eq!(view[0].key, "skierg");
    assert_eq!(view[0].plan, "500 M");
    // Spaces collapse so a screen's icon lookup survives a station rename.
    assert_eq!(view[1].key, "sled_push");
    assert_eq!(view[2].plan, "50 REPS");
}

#[test]
fn a_duration_target_is_shown_as_a_clock() {
    let course = Course::new(
        "TABATA",
        vec![
            CourseStep::new("BIKE").with_target(StationTarget::Duration {
                duration: Duration(150_000),
            }),
        ],
    );

    assert_eq!(course_view(Some(&course))[0].plan, "2:30 MIN");
}

#[test]
fn a_session_with_no_course_still_renders() {
    let mut state = session();
    state.config.course = None;

    let snap = snapshot(&state, START);

    assert!(snap.course.is_empty());
    assert_eq!(snap.athletes.len(), 1);
}

#[test]
fn an_athlete_inside_a_station_shows_the_station_and_its_leg_time() {
    let mut state = session();
    domain::apply(
        state.athlete_mut("a1").unwrap(),
        &Interpreted::Entered {
            station: "SKIERG".into(),
            at: START,
            transition: None,
            started_timing: true,
        },
    );

    let snap = snapshot(&state, Instant(START.0 + 30_000));
    let a = &snap.athletes[0];

    assert_eq!(a.status, AthleteStatus::Active);
    assert_eq!(a.station_state, StationState::Inside);
    assert_eq!(a.station.as_deref(), Some("SKIERG"));
    assert_eq!(a.station_key.as_deref(), Some("skierg"));
    assert_eq!(a.station_index, Some(0));
    assert_eq!(a.leg_ms, Some(30_000));
    assert_eq!(a.elapsed_ms, Some(30_000));
    assert_eq!(a.next_station, None);
}

#[test]
fn an_athlete_between_stations_shows_the_transition_and_what_is_next() {
    let mut state = session();
    let athlete = state.athlete_mut("a1").unwrap();
    domain::apply(
        athlete,
        &Interpreted::Entered {
            station: "SKIERG".into(),
            at: START,
            transition: None,
            started_timing: true,
        },
    );
    domain::apply(
        athlete,
        &Interpreted::Exited {
            station: "SKIERG".into(),
            at: Instant(START.0 + 100_000),
        },
    );

    let snap = snapshot(&state, Instant(START.0 + 110_000));
    let a = &snap.athletes[0];

    assert_eq!(a.completed, 1);
    assert_eq!(a.next_station.as_deref(), Some("SLED PUSH"));
    // Time in transition, which the competition screen calls ROX Zone (CLAUDE.md 13).
    assert_eq!(a.leg_ms, Some(10_000));
}

#[test]
fn the_snapshot_reports_freshness_and_the_exception_badge() {
    let mut state = session();
    domain::apply(
        state.athlete_mut("a1").unwrap(),
        &Interpreted::Entered {
            station: "SKIERG".into(),
            at: START,
            transition: None,
            started_timing: true,
        },
    );
    state.exception_count = 2;
    state.note_pending_tag(domain::TagId::parse("TAG-X").unwrap());

    let snap = snapshot(&state, Instant(START.0 + 4_000));

    // Without this a still screen and a dead link look identical (ADR 0001 D5).
    assert_eq!(snap.last_event_age_ms, Some(4_000));
    assert_eq!(snap.exceptions, 2);
    assert_eq!(snap.pending_tags, 1);
    assert_eq!(snap.readers_online, 1);
    assert_eq!(snap.mode, "TRAINING");
    assert_eq!(snap.status, "RUNNING");
    assert_eq!(snap.class_elapsed_ms, 4_000);
}

#[test]
fn a_session_where_nothing_has_happened_has_no_freshness_reading() {
    let snap = snapshot(&session(), START);

    assert_eq!(snap.last_event_age_ms, None);
    assert_eq!(snap.in_class, 1);
    assert_eq!(snap.finished, 0);
}
