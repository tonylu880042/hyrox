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
    assert_eq!(a.next_station.as_deref(), Some("SLED PUSH"));
    assert_eq!(a.next_plan.as_deref(), Some("25 M"));
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

#[test]
fn repeated_stations_use_the_current_round_and_preview_its_next_target() {
    let course = Course::new(
        "ROUNDS",
        vec![
            CourseStep::new("SKIERG").with_target(StationTarget::Distance { meters: 500 }),
            CourseStep::new("SKIERG").with_target(StationTarget::Distance { meters: 250 }),
            CourseStep::new("SLED PUSH").with_target(StationTarget::Distance { meters: 30 }),
        ],
    );
    let mut state = session();
    state.config.course = Some(course);
    let a = state.athlete_mut("a1").unwrap();
    domain::apply(
        a,
        &Interpreted::Entered {
            station: "SKIERG".into(),
            at: START,
            transition: None,
            started_timing: true,
        },
    );
    domain::apply(
        a,
        &Interpreted::Exited {
            station: "SKIERG".into(),
            at: Instant(START.0 + 1000),
        },
    );
    domain::apply(
        a,
        &Interpreted::Entered {
            station: "SKIERG".into(),
            at: Instant(START.0 + 2000),
            transition: Some(Duration(1000)),
            started_timing: false,
        },
    );
    let snap = snapshot(&state, Instant(START.0 + 3000));
    let a = &snap.athletes[0];
    assert_eq!(a.station_index, Some(1));
    assert_eq!(a.plan.as_deref(), Some("250 M"));
    assert_eq!(a.next_station.as_deref(), Some("SLED PUSH"));
    assert_eq!(a.next_plan.as_deref(), Some("30 M"));
}

#[test]
fn finished_athletes_have_no_next_action_even_when_class_ends_early() {
    let mut state = session();
    state.athlete_mut("a1").unwrap().status = AthleteStatus::Finished;
    let snap = snapshot(&state, START);
    assert_eq!(snap.athletes[0].next_station, None);
    assert_eq!(snap.athletes[0].next_plan, None);
}

/// ADR 0016: a station whose exit was never read is still done. The screens must count it,
/// and must say why its split is blank rather than leave a coach guessing.
#[test]
fn a_missed_exit_counts_as_done_and_is_flagged_on_its_split() {
    let mut state = session();
    let athlete = state.athlete_mut("a1").unwrap();
    for event in [
        Interpreted::Entered {
            station: "SKIERG".into(),
            at: START,
            transition: None,
            started_timing: true,
        },
        // The SKIERG exit was never read.
        Interpreted::Entered {
            station: "SLED PUSH".into(),
            at: Instant(START.0 + 300_000),
            transition: None,
            started_timing: false,
        },
    ] {
        domain::apply(athlete, &event);
    }

    let snap = snapshot(&state, Instant(START.0 + 310_000));
    let a = &snap.athletes[0];

    assert_eq!(a.completed, 1);
    assert_eq!(a.station_index, Some(1));
    assert_eq!(a.next_station.as_deref(), Some("WALL BALLS"));
    assert!(a.splits[0].exit_missed);
    assert_eq!(a.splits[0].work_ms, None);
    assert!(!a.splits[1].exit_missed);

    let results = application::live_results(&state);
    assert_eq!(results.rows[0].stations_completed, 1);
    assert!(results.rows[0].splits[0].exit_missed);
}

/// Per-head freshness (CLAUDE.md 23 "Device / Reader Warnings"): one board can carry the
/// S and N gates, and the board answering says nothing about which antenna is reading.
#[test]
fn each_reader_reports_when_it_last_produced_a_read() {
    let mut state = session();
    let key = ReaderKey::parse("a4cf128b3d91", "rfid-01").unwrap();
    let silent = ReaderKey::parse("a4cf128b3d91", "rfid-02").unwrap();
    state.readers.register(ReaderRegistration::new(
        silent.clone(),
        "SKIERG",
        ReaderMode::Exit,
    ));

    application::note_reader_read(&mut state, &key, Instant(START.0 + 5_000));

    let views = application::reader_views(&state, Instant(START.0 + 7_000));
    let view = |id: &str| views.iter().find(|v| v.reader_id == id).unwrap();
    assert_eq!(view("rfid-01").last_read_age_ms, Some(2_000));
    assert_eq!(
        view("rfid-01").last_seen_age_ms,
        Some(2_000),
        "a read is also liveness"
    );
    assert_eq!(
        view("rfid-02").last_read_age_ms,
        None,
        "never read is not zero, and must not be drawn as fresh"
    );
}

/// A status is retained on the broker, so the hub is handed every device's last one each
/// time it (re)connects -- including boards that lost power an hour ago. What it says is
/// worth showing; that it arrived is not evidence the board is alive (firmware protocol
/// v1.1 section 5.4).
#[test]
fn a_retained_status_carries_the_report_but_is_not_liveness() {
    let mut state = session();
    let device = domain::DeviceId::parse("a4cf128b3d91").unwrap();
    let report = |pending_events| application::DeviceReport {
        device_id: device.clone(),
        boot_id: 8,
        pending_events,
        journal_capacity: 1024,
        warning: Some(domain::DeviceWarning::JournalFull),
    };

    application::note_retained_status(&mut state, report(1024));
    let known = state.device(&device).expect("the report is kept");
    assert_eq!(known.last_seen, None, "never heard from live");
    assert_eq!(known.warning(), Some(domain::DeviceWarning::JournalFull));

    application::note_device_status(&mut state, report(3), Instant(START.0 + 5_000));
    application::note_retained_status(&mut state, report(7));
    let known = state.device(&device).unwrap();
    assert_eq!(
        known.last_seen,
        Some(Instant(START.0 + 5_000)),
        "a replay does not make the board any fresher"
    );
    assert_eq!(
        known.report.as_ref().unwrap().pending_events,
        7,
        "the broker's retained copy is the board's last word"
    );
}
