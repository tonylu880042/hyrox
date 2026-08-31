//! Crash recovery and idempotency (CLAUDE.md 16, 20, 21, 24).

use domain::{interpret, AthleteState, Instant, ReaderBinding, ReaderMode, Session, SessionMode};
use storage::{RawEvent, Store};

const T0: i64 = 1_787_734_800_000;
fn at(ms: i64) -> Instant { Instant(T0 + ms) }

fn raw(seq: i64, detected: i64) -> RawEvent {
    RawEvent {
        device_id: "esp32-a4cf128b3d91".into(),
        reader_id: "rfid-02".into(),
        boot_id: 18,
        sequence: seq,
        tag_id: "E280117000001234".into(),
        detected_at: at(detected),
        received_at: at(detected + 120), // arrival lag must never affect timing
    }
}

fn reader(station: &str, mode: ReaderMode) -> ReaderBinding {
    ReaderBinding { station: station.into(), mode }
}

/// Ingests a short class into a fresh store and returns the live in-memory state.
async fn ingest(store: &Store) -> (Session, AthleteState) {
    let mut session = Session::new_draft("s1", "Thursday Class", SessionMode::Training);
    session.mark_ready().unwrap();
    session.start().unwrap();
    store.save_session(&session, at(0)).await.unwrap();
    store.save_athlete("s1", "a1", "CHEN YU-TING", 1).await.unwrap();

    let mut athlete = AthleteState::ready("a1", "CHEN YU-TING");
    let script = [
        ("SKIERG", ReaderMode::Entry, 0),
        ("SKIERG", ReaderMode::Exit, 110_000),
        ("SLED PUSH", ReaderMode::Entry, 128_000),
    ];
    for (i, (station, mode, t)) in script.iter().enumerate() {
        let r = raw(i as i64 + 1, *t);
        let (raw_id, inserted) = store.save_raw(&r).await.unwrap();
        assert!(inserted);
        let event = interpret(&mut athlete, &reader(station, *mode), at(*t), &session);
        store.save_interpreted("s1", "a1", Some(raw_id), &event).await.unwrap();
        session.interpreted_event_count += 1;
    }
    store.save_session(&session, at(0)).await.unwrap();
    (session, athlete)
}

#[tokio::test]
async fn state_survives_a_restart() {
    let store = Store::open_in_memory().await.unwrap();
    let (_, live) = ingest(&store).await;

    // Restart: nothing but the database survives.
    let session = store.active_session().await.unwrap().expect("session must be recoverable");
    assert_eq!(session.id, "s1");
    assert!(session.accepts_events(), "an ARMED session must resume ARMED");

    let rebuilt = store.rebuild_athletes("s1").await.unwrap();
    assert_eq!(rebuilt.len(), 1);
    let a = &rebuilt[0];
    assert_eq!(a.status, live.status);
    assert_eq!(a.station_state, live.station_state);
    assert_eq!(a.current_station, live.current_station);
    assert_eq!(a.started_at, live.started_at, "timing must survive verbatim");
    assert_eq!(a.last_exit_at, live.last_exit_at);
    assert_eq!(a.runs.len(), live.runs.len());
    assert_eq!(a.runs[1].transition_from_prev, live.runs[1].transition_from_prev);
}

#[tokio::test]
async fn redelivering_the_same_edge_event_does_not_duplicate_it() {
    // Duplicate delivery is allowed; duplicate processing is not (CLAUDE.md 16).
    let store = Store::open_in_memory().await.unwrap();
    let e = raw(1, 0);

    let (first_id, inserted) = store.save_raw(&e).await.unwrap();
    assert!(inserted);
    let (second_id, inserted_again) = store.save_raw(&e).await.unwrap();
    assert!(!inserted_again, "a redelivery must not insert a second row");
    assert_eq!(first_id, second_id);
    assert_eq!(store.raw_event_count().await.unwrap(), 1);
}

#[tokio::test]
async fn a_different_boot_id_is_a_different_event() {
    let store = Store::open_in_memory().await.unwrap();
    let a = raw(1, 0);
    let mut b = raw(1, 0);
    b.boot_id = 19; // sequence numbers restart after a reboot
    store.save_raw(&a).await.unwrap();
    store.save_raw(&b).await.unwrap();
    assert_eq!(store.raw_event_count().await.unwrap(), 2);
}

#[tokio::test]
async fn voiding_an_interpreted_event_changes_what_recovery_rebuilds() {
    let store = Store::open_in_memory().await.unwrap();
    let (_, _) = ingest(&store).await;

    let before = store.rebuild_athletes("s1").await.unwrap();
    assert_eq!(before[0].runs.len(), 2);

    // Void the SLED PUSH entry the way an operator correction would (CLAUDE.md 20).
    store.void_interpreted(3, at(200_000), "櫃檯平板", "誤刷").await.unwrap();

    let after = store.rebuild_athletes("s1").await.unwrap();
    assert_eq!(after[0].runs.len(), 1, "the voided station must disappear");
    assert_eq!(
        store.raw_event_count().await.unwrap(),
        3,
        "raw events are immutable: a correction must not delete them"
    );
}

#[tokio::test]
async fn an_unknown_reader_exception_survives_a_restart() {
    // A venue mis-configuration must not lose the read (CLAUDE.md 8, 31; ADR 0001 D4):
    // it is stored as an exception so it can be re-attributed once the mapping is fixed.
    use domain::{ExceptionReason, Interpreted};

    let store = Store::open_in_memory().await.unwrap();
    let mut session = Session::new_draft("s1", "Thursday Class", SessionMode::Training);
    session.mark_ready().unwrap();
    session.start().unwrap();
    store.save_session(&session, at(0)).await.unwrap();
    store.save_athlete("s1", "a1", "CHEN YU-TING", 1).await.unwrap();

    let event = Interpreted::Exception { reason: ExceptionReason::UnknownReader, at: at(1_000) };
    store.save_interpreted("s1", "a1", None, &event).await.unwrap();

    let rebuilt = store.rebuild_athletes("s1").await.unwrap();
    assert_eq!(rebuilt[0].runs.len(), 0, "an exception must not advance station state");
    assert_eq!(rebuilt[0].status, domain::AthleteStatus::Ready);
}

// --- configuration, reader map and bindings across a restart (ADR 0004) ----------------

mod resumed {
    use application::{
        apply_finish_policy,
        checkin::{bind_tag, rebind_tag},
        ingest_read, register_reader, resume_or_start, HubStore, OperatorCommand, Recovery,
        RosterEntry, SessionPlan,
    };
    use domain::{
        AthleteStatus, Course, CourseStep, Duration, ExceptionReason, FinishPolicy, Instant,
        Interpreted, ReaderKey, ReaderMode, ReaderRegistration, Session, SessionConfig,
        SessionMode, StationTarget, TagId,
    };
    use contract::{EdgeEvent, ReceivedEvent};
    use storage::Store;

    const T0: i64 = 1_787_734_800_000;
    const DEVICE: &str = "a4:cf:12:8b:3d:91";
    const LIMIT: Duration = Duration(60 * 60 * 1000);

    fn at(ms: i64) -> Instant {
        Instant(T0 + ms)
    }

    fn course() -> Course {
        Course::new(
            "HYROX CLASS",
            vec![
                CourseStep::new("SKIERG").with_target(StationTarget::Distance { meters: 500 }),
                CourseStep::new("WALL BALLS")
                    .with_target(StationTarget::Repetitions { count: 50 }),
            ],
        )
    }

    /// The plan a startup supplies. `policy` and `course` differ between the two calls in
    /// the restart tests, which is exactly what a resumed session must ignore.
    fn plan(policy: FinishPolicy, course: Option<Course>) -> SessionPlan {
        let mut config = SessionConfig::new("s1").with_finish_policy(policy);
        if let Some(c) = course {
            config = config.with_course(c);
        }
        SessionPlan {
            session: Session::new_draft("s1", "THU 19:00", SessionMode::Training),
            config,
            roster: vec![RosterEntry {
                athlete_id: "a1".into(),
                display_name: "CHEN YU-TING".into(),
            }],
            class_start: at(0),
        }
    }

    fn read(reader: &str, tag: &str, sequence: i64, ms: i64) -> ReceivedEvent {
        let event = EdgeEvent {
            device_id: contract::DeviceId::from_mac_str(DEVICE).expect("device id"),
            reader_id: contract::ReaderId::parse(reader).expect("reader id"),
            boot_id: 7,
            sequence,
            tag_id: tag.to_string(),
            detected_at: at(ms).0,
            uptime_ms: ms,
        };
        ReceivedEvent::new(event, at(ms + 250).0)
    }

    fn skierg_entry() -> ReaderRegistration {
        let device = format!("esp32-{}", DEVICE.replace(':', ""));
        ReaderRegistration::new(
            ReaderKey::parse(&device, "rfid-01").expect("key"),
            "SKIERG",
            ReaderMode::Entry,
        )
        .with_zone("MAIN FLOOR")
    }

    #[tokio::test]
    async fn a_resumed_class_finishes_on_the_limit_it_was_armed_with() {
        // The hole this closes: the finish policy was supplied by the caller on every start,
        // so a class armed under a one-hour limit could come back under a different rule
        // (CLAUDE.md 12, 21).
        let store = Store::open_in_memory().await.unwrap();
        let (mut state, how) = resume_or_start(
            &store,
            plan(FinishPolicy::ClassDuration { limit: LIMIT }, Some(course())),
        )
        .await
        .unwrap();
        assert_eq!(how, Recovery::Started);

        register_reader(
            &mut state,
            &store,
            &skierg_entry(),
            &OperatorCommand::new("OPERATOR TABLET", at(0)),
        )
        .await
        .unwrap();
        bind_tag(
            &mut state,
            &store,
            &TagId::parse("TAG-A1").unwrap(),
            "a1",
            &OperatorCommand::new("CHECKIN TABLET", at(0)),
        )
        .await
        .unwrap();
        ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 60_000))
            .await
            .unwrap();
        assert_eq!(state.athlete("a1").unwrap().status, AthleteStatus::Active);
        drop(state);

        // Restart. The caller offers a different rule and a different course, as a fresh
        // process would; the stored ones must win.
        let (mut resumed, how) =
            resume_or_start(&store, plan(FinishPolicy::CoachDecides, None)).await.unwrap();

        assert_eq!(how, Recovery::Resumed);
        assert_eq!(resumed.config.finish_policy, FinishPolicy::ClassDuration { limit: LIMIT });
        assert_eq!(resumed.config.course.as_ref(), Some(&course()));

        // And the rule is still enforced: nobody finishes before the limit...
        assert!(apply_finish_policy(&mut resumed, at(LIMIT.millis() - 1)).is_empty());
        // ...and the athlete who was running finishes on it (CLAUDE.md 12, answered
        // 2026-08-27). Under the caller's CoachDecides nothing would have finished at all.
        let finished = apply_finish_policy(&mut resumed, at(LIMIT.millis()));
        assert_eq!(finished, ["a1"]);
        assert_eq!(resumed.athlete("a1").unwrap().status, AthleteStatus::Finished);
    }

    #[tokio::test]
    async fn the_reader_map_resolves_the_same_way_after_a_restart() {
        let store = Store::open_in_memory().await.unwrap();
        let (mut state, _) = resume_or_start(&store, plan(FinishPolicy::CoachDecides, None))
            .await
            .unwrap();
        register_reader(
            &mut state,
            &store,
            &skierg_entry(),
            &OperatorCommand::new("OPERATOR TABLET", at(0)),
        )
        .await
        .unwrap();
        bind_tag(
            &mut state,
            &store,
            &TagId::parse("TAG-A1").unwrap(),
            "a1",
            &OperatorCommand::new("CHECKIN TABLET", at(0)),
        )
        .await
        .unwrap();
        drop(state);

        let (mut resumed, _) = resume_or_start(&store, plan(FinishPolicy::CoachDecides, None))
            .await
            .unwrap();

        let registration = resumed
            .readers
            .resolve(&skierg_entry().key)
            .expect("a registered reader must still resolve");
        assert_eq!(registration.station, "SKIERG");
        assert_eq!(registration.mode, ReaderMode::Entry);
        assert_eq!(registration.zone.as_deref(), Some("MAIN FLOOR"));

        // A registered reader attributes...
        ingest_read(&mut resumed, &store, &read("rfid-01", "TAG-A1", 1, 60_000))
            .await
            .unwrap();
        assert_eq!(resumed.athlete("a1").unwrap().current_station.as_deref(), Some("SKIERG"));

        // ...and one that was never registered is still an exception (CLAUDE.md 8; D4).
        let out = ingest_read(&mut resumed, &store, &read("rfid-99", "TAG-A1", 2, 120_000))
            .await
            .unwrap();
        assert!(matches!(
            out.outcome,
            application::IngestOutcome::Interpreted {
                event: Interpreted::Exception { reason: ExceptionReason::UnknownReader, .. },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn the_binding_ledger_survives_a_restart_with_its_closed_rows() {
        let store = Store::open_in_memory().await.unwrap();
        let (mut state, _) = resume_or_start(&store, plan(FinishPolicy::CoachDecides, None))
            .await
            .unwrap();
        let first = TagId::parse("TAG-A1").unwrap();
        let second = TagId::parse("TAG-A2").unwrap();
        bind_tag(&mut state, &store, &first, "a1", &OperatorCommand::new("CHECKIN", at(0)))
            .await
            .unwrap();
        rebind_tag(
            &mut state,
            &store,
            &second,
            "a1",
            &OperatorCommand::new("CHECKIN", at(30_000)).with_reason("腳環故障"),
        )
        .await
        .unwrap();
        drop(state);

        let (resumed, _) = resume_or_start(&store, plan(FinishPolicy::CoachDecides, None))
            .await
            .unwrap();

        assert_eq!(resumed.bindings.athlete_for_tag("s1", &second), Some("a1"));
        assert_eq!(resumed.bindings.athlete_for_tag("s1", &first), None);
        let history = resumed.bindings.history();
        assert_eq!(history.len(), 2, "the closed row is the audit trail (CLAUDE.md 20)");
        let closed = history.iter().find(|b| b.tag_id == first).expect("the old band");
        assert_eq!(closed.athlete_id, "a1", "who held it is never rewritten");
        assert_eq!(closed.unbound_at, Some(at(30_000)));
    }

    #[tokio::test]
    async fn a_band_read_before_the_restart_is_still_waiting_for_check_in() {
        let store = Store::open_in_memory().await.unwrap();
        let (mut state, _) = resume_or_start(&store, plan(FinishPolicy::CoachDecides, None))
            .await
            .unwrap();
        register_reader(
            &mut state,
            &store,
            &skierg_entry(),
            &OperatorCommand::new("OPERATOR", at(0)),
        )
        .await
        .unwrap();
        ingest_read(&mut state, &store, &read("rfid-01", "TAG-NOBODY", 1, 60_000))
            .await
            .unwrap();
        assert_eq!(state.pending_tags().len(), 1);
        drop(state);

        let (resumed, _) = resume_or_start(&store, plan(FinishPolicy::CoachDecides, None))
            .await
            .unwrap();

        // The queue used to live only in memory, so a crash lost it (ADR 0001 D3).
        assert_eq!(resumed.pending_tags().len(), 1);
        assert_eq!(resumed.pending_tags()[0].as_str(), "TAG-NOBODY");
    }

    #[tokio::test]
    async fn a_band_claimed_after_a_restart_still_gets_the_time_it_ran() {
        // ADR 0001 D3 end to end through SQLite: reads stored before anyone owned the band
        // are interpreted when it is claimed, in detected_at order.
        let store = Store::open_in_memory().await.unwrap();
        let (mut state, _) = resume_or_start(&store, plan(FinishPolicy::CoachDecides, None))
            .await
            .unwrap();
        let device = format!("esp32-{}", DEVICE.replace(':', ""));
        for (reader, mode) in [("rfid-01", ReaderMode::Entry), ("rfid-02", ReaderMode::Exit)] {
            register_reader(
                &mut state,
                &store,
                &ReaderRegistration::new(
                    ReaderKey::parse(&device, reader).expect("key"),
                    "SKIERG",
                    mode,
                ),
                &OperatorCommand::new("OPERATOR", at(0)),
            )
            .await
            .unwrap();
        }
        ingest_read(&mut state, &store, &read("rfid-01", "TAG-A1", 1, 60_000)).await.unwrap();
        ingest_read(&mut state, &store, &read("rfid-02", "TAG-A1", 2, 170_000)).await.unwrap();
        drop(state);

        let (mut resumed, _) = resume_or_start(&store, plan(FinishPolicy::CoachDecides, None))
            .await
            .unwrap();
        let claimed = bind_tag(
            &mut resumed,
            &store,
            &TagId::parse("TAG-A1").unwrap(),
            "a1",
            &OperatorCommand::new("CHECKIN TABLET", at(200_000)),
        )
        .await
        .unwrap();

        assert_eq!(claimed.len(), 2);
        let athlete = resumed.athlete("a1").unwrap();
        // The clock starts at the read, not at the binding (CLAUDE.md 11).
        assert_eq!(athlete.started_at, Some(at(60_000)));
        assert_eq!(athlete.runs.len(), 1);
        assert_eq!(athlete.runs[0].exited_at, Some(at(170_000)));

        // And the log agrees: replaying it after another restart rebuilds the same athlete.
        let rebuilt = HubStore::rebuild_athletes(&store, "s1").await.unwrap();
        assert_eq!(rebuilt[0].started_at, athlete.started_at);
        assert_eq!(rebuilt[0].runs.len(), 1);
        assert_eq!(rebuilt[0].runs[0].exited_at, athlete.runs[0].exited_at);
        assert!(resumed.pending_tags().is_empty());
    }
}
