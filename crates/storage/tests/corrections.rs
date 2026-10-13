//! Historical corrections and session scoping.

use application::{
    accept, ingest_read, register_reader, reinterpret, void, LiveSession, OperatorCommand,
    ReinterpretSpec,
};
use contract::{EdgeEvent, ReceivedEvent};
use domain::{
    ExceptionReason, Instant, Interpreted, ReaderMode, Session, SessionConfig, SessionMode,
};
use storage::Store;

const START: Instant = Instant(1_000_000);

async fn running(store: &Store, id: &str) -> (Session, SessionConfig) {
    let mut session = Session::new_draft(id, id, SessionMode::Training);
    session.mark_ready().unwrap();
    session.start().unwrap();
    store.save_session(&session, START).await.unwrap();
    let config = SessionConfig::new(id);
    store.save_session_config(&config).await.unwrap();
    store
        .save_athlete(id, "a1", "Alice", 1, None)
        .await
        .unwrap();
    (session, config)
}

fn command(at: i64) -> OperatorCommand {
    OperatorCommand::new("REVIEW", Instant(at)).with_reason("corrected reader map")
}

#[tokio::test]
async fn historical_reinterpretation_rebuilds_the_affected_timeline() {
    let store = Store::open_in_memory().await.unwrap();
    let (session, config) = running(&store, "s1").await;

    let exception = store
        .save_interpreted(
            "s1",
            "a1",
            None,
            &Interpreted::Exception {
                reason: ExceptionReason::UnknownReader,
                at: Instant(100),
            },
        )
        .await
        .unwrap();
    store
        .save_interpreted(
            "s1",
            "a1",
            None,
            &Interpreted::Exited {
                station: "SKIERG".into(),
                at: Instant(200),
            },
        )
        .await
        .unwrap();
    store
        .save_interpreted(
            "s1",
            "a1",
            None,
            &Interpreted::Entered {
                station: "RUN".into(),
                at: Instant(300),
                transition: None,
                started_timing: true,
            },
        )
        .await
        .unwrap();
    store
        .save_interpreted(
            "s1",
            "a1",
            None,
            &Interpreted::Exited {
                station: "RUN".into(),
                at: Instant(400),
            },
        )
        .await
        .unwrap();

    let athletes = store.rebuild_athletes("s1").await.unwrap();
    let mut live = LiveSession::new(session, config, START).with_athletes(athletes);
    let replacement = reinterpret(
        &mut live,
        &store,
        exception,
        ReinterpretSpec {
            station: "SKIERG".into(),
            mode: ReaderMode::Entry,
            athlete_id: None,
            at: None,
        },
        &command(500),
    )
    .await
    .unwrap();
    assert_ne!(replacement, exception);

    let athlete = &live.athletes[0];
    assert_eq!(athlete.started_at, Some(Instant(100)));
    assert_eq!(
        athlete.runs.len(),
        2,
        "the corrected suffix should replay once"
    );
    assert_eq!(athlete.runs[0].station, "SKIERG");
    assert_eq!(athlete.runs[0].entered_at, Instant(100));
    assert_eq!(athlete.runs[0].exited_at, Some(Instant(200)));
    assert_eq!(athlete.runs[0].transition_from_prev, None);
    assert_eq!(athlete.runs[1].station, "RUN");
    assert_eq!(athlete.runs[1].entered_at, Instant(300));
    assert_eq!(athlete.runs[1].exited_at, Some(Instant(400)));

    let restarted = store.rebuild_athletes("s1").await.unwrap();
    assert_eq!(
        serde_json::to_value(&restarted[0]).unwrap(),
        serde_json::to_value(athlete).unwrap()
    );
}

#[tokio::test]
async fn contradictory_historical_reinterpretation_is_rejected_before_mutation() {
    let store = Store::open_in_memory().await.unwrap();
    let (session, config) = running(&store, "s1").await;
    let exception = store
        .save_interpreted(
            "s1",
            "a1",
            None,
            &Interpreted::Exception {
                reason: ExceptionReason::UnknownReader,
                at: Instant(100),
            },
        )
        .await
        .unwrap();
    store
        .save_interpreted(
            "s1",
            "a1",
            None,
            // Entering the SAME station the correction would put the athlete in. A different
            // station would be a missed exit, which is not a contradiction (ADR 0016).
            &Interpreted::Entered {
                station: "SKIERG".into(),
                at: Instant(200),
                transition: None,
                started_timing: true,
            },
        )
        .await
        .unwrap();

    let before_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM interpreted_events")
        .fetch_one(store.pool())
        .await
        .unwrap();
    let mut live = LiveSession::new(session, config, START)
        .with_athletes(store.rebuild_athletes("s1").await.unwrap());
    let result = reinterpret(
        &mut live,
        &store,
        exception,
        ReinterpretSpec {
            station: "SKIERG".into(),
            mode: ReaderMode::Entry,
            athlete_id: None,
            at: None,
        },
        &command(500),
    )
    .await;
    assert!(matches!(
        result,
        Err(application::OperatorError::CorrectionConflict(_))
    ));

    let after_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM interpreted_events")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(after_count, before_count);
    let voided: Option<i64> =
        sqlx::query_scalar("SELECT voided_at FROM interpreted_events WHERE id = ?1")
            .bind(exception)
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert!(voided.is_none());
}

#[tokio::test]
async fn accept_and_void_require_the_current_session_scope() {
    let store = Store::open_in_memory().await.unwrap();
    let (s1, c1) = running(&store, "s1").await;
    let (s2, c2) = running(&store, "s2").await;
    let exception = Interpreted::Exception {
        reason: ExceptionReason::UnknownReader,
        at: Instant(100),
    };
    let event_id = store
        .save_interpreted("s1", "a1", None, &exception)
        .await
        .unwrap();

    let mut state =
        LiveSession::new(s2, c2, START).with_athletes(store.rebuild_athletes("s2").await.unwrap());
    let result = accept(&mut state, &store, event_id, &command(500)).await;
    assert!(matches!(result, Err(application::OperatorError::UnknownEvent(id)) if id == event_id));
    let result = void(&mut state, &store, event_id, &command(501)).await;
    assert!(matches!(result, Err(application::OperatorError::UnknownEvent(id)) if id == event_id));

    let mut s1_state =
        LiveSession::new(s1, c1, START).with_athletes(store.rebuild_athletes("s1").await.unwrap());
    accept(&mut s1_state, &store, event_id, &command(502))
        .await
        .unwrap();
    assert_eq!(store.exceptions("s1").await.unwrap().len(), 0);
}

#[tokio::test]
async fn a_failed_claim_replays_its_durable_intent_after_restart() {
    let store = Store::open_in_memory().await.unwrap();
    let (session, config) = running(&store, "s1").await;
    let raw = storage::RawEvent {
        device_id: "a4cf128b3d91".into(),
        reader_id: "rfid-01".into(),
        boot_id: 1,
        sequence: 1,
        tag_id: "TAG-1".into(),
        detected_at: Instant(START.0 + 100),
        received_at: Instant(START.0 + 150),
        presence: domain::Presence::Arrived,
        uptime_ms: 0,
    };
    store
        .save_raw_with_context(
            &raw,
            &application::RawInterpretationContext {
                session_id: "s1".into(),
                athlete_id: None,
                bound_session_id: None,
                on_roster: false,
                reader: Some(domain::ReaderBinding::new("RUN", ReaderMode::Entry)),
                planned: None,
            },
        )
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_claim
         BEFORE INSERT ON interpreted_events
         BEGIN SELECT RAISE(ABORT, 'injected claim failure'); END",
    )
    .execute(store.pool())
    .await
    .unwrap();

    let mut live = LiveSession::new(session, config, START)
        .with_athletes(store.rebuild_athletes("s1").await.unwrap());
    let tag = domain::TagId::parse("TAG-1").unwrap();
    let failed =
        application::checkin::bind_tag(&mut live, &store, &tag, "a1", &command(START.0 + 500))
            .await;
    assert!(failed.is_err());
    let context = store.raw_interpretation_context(1).await.unwrap().unwrap();
    assert_eq!(context.athlete_id.as_deref(), Some("a1"));
    assert_eq!(context.bound_session_id.as_deref(), Some("s1"));
    assert!(context.planned.is_none());

    sqlx::query("DROP TRIGGER fail_claim")
        .execute(store.pool())
        .await
        .unwrap();
    let future_raw = storage::RawEvent {
        device_id: "a4cf128b3d91".into(),
        reader_id: "rfid-01".into(),
        boot_id: 1,
        sequence: 2,
        tag_id: "TAG-1".into(),
        detected_at: Instant(START.0 + 600),
        received_at: Instant(START.0 + 650),
        presence: domain::Presence::Arrived,
        uptime_ms: 0,
    };
    store
        .save_raw_with_context(
            &future_raw,
            &application::RawInterpretationContext {
                session_id: "s1".into(),
                athlete_id: None,
                bound_session_id: None,
                on_roster: false,
                reader: Some(domain::ReaderBinding::new("RUN", ReaderMode::Entry)),
                planned: None,
            },
        )
        .await
        .unwrap();
    let plan = application::SessionPlan {
        session: Session::new_draft("boot", "boot", SessionMode::Training),
        config: SessionConfig::new("boot"),
        roster: vec![],
        class_start: START,
        start_now: false,
    };
    let (mut recovered, _) = application::resume_or_start(&store, plan).await.unwrap();
    assert_eq!(
        recovered.athlete("a1").unwrap().started_at,
        Some(Instant(START.0 + 100))
    );
    let pending = store.pending_raw_reads("s1").await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].raw_event_id, 2);

    // A repeated check-in after recovery must not reinterpret the same raw row.  The durable
    // claim marker is consumed by reconcile_pending before rebind starts.
    let retried = application::checkin::rebind_tag(
        &mut recovered,
        &store,
        &tag,
        "a1",
        &command(START.0 + 501),
    )
    .await
    .unwrap();
    assert!(retried.is_empty());
    assert_eq!(store.interpreted_event_count("s1").await.unwrap(), 1);
    let pending = store.pending_raw_reads("s1").await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].raw_event_id, 2);
}

#[tokio::test]
async fn late_reads_are_explicit_exceptions_and_live_matches_replay() {
    let store = Store::open_in_memory().await.unwrap();
    let (session, config) = running(&store, "s1").await;
    let mut live = LiveSession::new(session, config, START)
        .with_athletes(store.rebuild_athletes("s1").await.unwrap());
    let device = "a4cf128b3d91";
    for (reader, mode) in [("entry", ReaderMode::Entry), ("exit", ReaderMode::Exit)] {
        let registration = domain::ReaderRegistration::new(
            domain::ReaderKey::parse(device, reader).unwrap(),
            "RUN",
            mode,
        );
        register_reader(&mut live, &store, &registration, &command(0))
            .await
            .unwrap();
    }
    application::checkin::bind_tag(
        &mut live,
        &store,
        &domain::TagId::parse("TAG-1").unwrap(),
        "a1",
        &command(0),
    )
    .await
    .unwrap();

    for (sequence, detected_at, reader) in [
        (3, START.0 + 300, "entry"),
        (2, START.0 + 200, "exit"),
        (1, START.0 + 100, "entry"),
    ] {
        let event = EdgeEvent {
            device_id: contract::DeviceId::from_mac_str(device).unwrap(),
            reader_id: contract::ReaderId::parse(reader).unwrap(),
            boot_id: 1,
            sequence,
            tag_id: vec!["TAG-1".into()],
            detected_at,
            uptime_ms: detected_at,
            presence: contract::Presence::Arrived,
        };
        ingest_read(
            &mut live,
            &store,
            &ReceivedEvent::new(event, START.0 + 1_000),
        )
        .await
        .unwrap();
    }

    let live_json = serde_json::to_value(live.athlete("a1").unwrap()).unwrap();
    let replay_json =
        serde_json::to_value(&store.rebuild_athletes("s1").await.unwrap()[0]).unwrap();
    assert_eq!(live_json, replay_json);
    assert_eq!(live.athlete("a1").unwrap().runs.len(), 1);
    assert_eq!(store.exceptions("s1").await.unwrap().len(), 2);
}

#[tokio::test]
async fn same_timestamp_arrival_orders_are_each_stable_across_restart() {
    for (first, second) in [("entry", "exit"), ("exit", "entry")] {
        let store = Store::open_in_memory().await.unwrap();
        let (session, config) = running(&store, "s1").await;
        let mut live = LiveSession::new(session, config, START)
            .with_athletes(store.rebuild_athletes("s1").await.unwrap());
        let device = "a4cf128b3d91";
        for (reader, mode) in [("entry", ReaderMode::Entry), ("exit", ReaderMode::Exit)] {
            let registration = domain::ReaderRegistration::new(
                domain::ReaderKey::parse(device, reader).unwrap(),
                "RUN",
                mode,
            );
            register_reader(&mut live, &store, &registration, &command(START.0))
                .await
                .unwrap();
        }
        application::checkin::bind_tag(
            &mut live,
            &store,
            &domain::TagId::parse("TAG-1").unwrap(),
            "a1",
            &command(START.0),
        )
        .await
        .unwrap();

        for (sequence, reader) in [(1_i64, first), (2_i64, second)] {
            let event = EdgeEvent {
                device_id: contract::DeviceId::from_mac_str(device).unwrap(),
                reader_id: contract::ReaderId::parse(reader).unwrap(),
                boot_id: 1,
                sequence,
                tag_id: vec!["TAG-1".into()],
                detected_at: START.0 + 100,
                uptime_ms: 100,
                presence: contract::Presence::Arrived,
            };
            ingest_read(&mut live, &store, &ReceivedEvent::new(event, START.0 + 200))
                .await
                .unwrap();
        }

        let live_json = serde_json::to_value(live.athlete("a1").unwrap()).unwrap();
        let replay_json =
            serde_json::to_value(&store.rebuild_athletes("s1").await.unwrap()[0]).unwrap();
        assert_eq!(live_json, replay_json, "arrival order {first}/{second}");
    }
}
