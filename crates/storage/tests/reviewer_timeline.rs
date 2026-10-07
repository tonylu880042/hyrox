//! Independent review: transport order must not change a result on replay.
use application::{HubStore, LiveSession, OperatorCommand};
use domain::{Instant, ReaderMode, Session, SessionConfig, SessionMode, TagId};
use storage::Store;

#[tokio::test]
async fn every_three_read_arrival_permutation_matches_durable_replay() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let store = Store::open_in_memory().await.unwrap();
        let mut session = Session::new_draft("review", "Review", SessionMode::Training);
        session.mark_ready().unwrap();
        session.start().unwrap();
        store.save_session(&session, Instant(0)).await.unwrap();
        let config = SessionConfig::new("review");
        store.save_session_config(&config).await.unwrap();
        store
            .save_athlete("review", "a", "Alice", 1, None)
            .await
            .unwrap();
        let mut live = LiveSession::new(session, config, Instant(0))
            .with_athletes(store.rebuild_athletes("review").await.unwrap());
        let cmd = OperatorCommand::new("REVIEW", Instant(0));
        application::checkin::bind_tag(
            &mut live,
            &store,
            &TagId::parse("BAND").unwrap(),
            "a",
            &cmd,
        )
        .await
        .unwrap();
        // Different collectors, so a transport's per-device sequence ordering cannot hide
        // the cross-device arrival race.
        for (device, mode) in [
            ("a4cf128b3d91", ReaderMode::Entry),
            ("a4cf128b3d92", ReaderMode::Exit),
        ] {
            let reader = domain::ReaderRegistration::new(
                domain::ReaderKey::parse(device, "reader").unwrap(),
                "RUN",
                mode,
            );
            application::register_reader(&mut live, &store, &reader, &cmd)
                .await
                .unwrap();
        }
        let events = [
            ("a4cf128b3d91", 100),
            ("a4cf128b3d92", 200),
            ("a4cf128b3d91", 300),
        ];
        for index in order {
            let (device, at) = events[index];
            let event = contract::EdgeEvent {
                device_id: contract::DeviceId::from_mac_str(device).unwrap(),
                reader_id: contract::ReaderId::parse("reader").unwrap(),
                boot_id: 1,
                sequence: index as i64 + 1,
                tag_id: vec!["BAND".into()],
                detected_at: at,
                uptime_ms: at,
            };
            application::ingest_read(
                &mut live,
                &store,
                &contract::ReceivedEvent::new(event, 1_000),
            )
            .await
            .unwrap();
        }
        let expected = serde_json::to_value(&live.athletes).unwrap();
        assert_eq!(
            expected,
            serde_json::to_value(store.rebuild_athletes("review").await.unwrap()).unwrap(),
            "order {order:?}"
        );
        for run in &live.athlete("a").unwrap().runs {
            assert!(run.exited_at.is_none_or(|exit| exit >= run.entered_at));
            assert!(run.transition_from_prev.is_none_or(|gap| gap.millis() >= 0));
        }
        let raw_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM raw_events")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(raw_count, 3, "late exceptions must retain all evidence");
        assert!(HubStore::pending_raw_reads(&store, "review")
            .await
            .unwrap()
            .is_empty());
        let (recovered, _) = application::resume_or_start(
            &store,
            application::SessionPlan {
                session: Session::new_draft("fallback", "Fallback", SessionMode::Training),
                config: SessionConfig::new("fallback"),
                roster: vec![],
                class_start: Instant(2_000),
                start_now: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            expected,
            serde_json::to_value(&recovered.athletes).unwrap(),
            "restart order {order:?}"
        );
    }
}
