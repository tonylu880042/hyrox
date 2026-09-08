//! Application tests for the exception inbox: void, accept, and reinterpret (ADR 0001 D4; CLAUDE.md 20).

mod support;

use application::{
    accept, reinterpret, resume_or_start, void, HubStore, InterpretedWrite, OperatorCommand,
    OperatorError, ReinterpretSpec, SessionPlan,
};
use domain::{
    AthleteStatus, ExceptionReason, Instant, Interpreted, ReaderMode, Session, SessionConfig,
    SessionMode,
};
use support::FakeStore;

const START: Instant = Instant(1_000_000);
const OP: &str = "COACH TABLET";

fn running_plan() -> SessionPlan {
    SessionPlan {
        session: Session::new_draft("s1", "HYROX CLASS", SessionMode::Training),
        config: SessionConfig::new("s1"),
        roster: vec![
            application::RosterEntry {
                athlete_id: "a1".into(),
                display_name: "ALICE".into(),
            },
            application::RosterEntry {
                athlete_id: "a2".into(),
                display_name: "BOB".into(),
            },
        ],
        class_start: START,
        start_now: true,
    }
}

async fn seed_exception(store: &FakeStore, athlete_id: &str, at: Instant) -> i64 {
    store
        .commit_interpreted(InterpretedWrite {
            session_id: "s1",
            athlete_id,
            raw_event_id: Some(42),
            event: &Interpreted::Exception {
                reason: ExceptionReason::UnknownReader,
                at,
            },
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn reinterpret_voids_old_exception_and_inserts_new_interpretation() {
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, running_plan()).await.unwrap();

    let event_id = seed_exception(&store, "a1", Instant(1_000_100)).await;
    state.exception_count = 1;

    let cmd = OperatorCommand::new(OP, Instant(1_000_200)).with_reason("讀卡機站點補正");
    let spec = ReinterpretSpec {
        station: "SKIERG".to_string(),
        mode: ReaderMode::Entry,
        athlete_id: None,
        at: None,
    };

    let new_id = reinterpret(&mut state, &store, event_id, spec, &cmd)
        .await
        .expect("reinterpret should succeed");

    assert_ne!(new_id, event_id);
    assert_eq!(state.exception_count, 0, "inbox badge cleared");

    // The athlete was Ready; reinterpreting as an Entry starts their clock and marks them Active
    let athlete = state.athlete("a1").expect("athlete exists");
    assert_eq!(athlete.status, AthleteStatus::Active);
    assert_eq!(athlete.current_station.as_deref(), Some("SKIERG"));
    assert_eq!(athlete.runs.len(), 1);
    assert_eq!(athlete.runs[0].station, "SKIERG");
    assert_eq!(athlete.runs[0].entered_at, Instant(1_000_100));

    // Audit trail recorded
    let audits = store.audits();
    let entry = audits
        .iter()
        .find(|a| a.action == "EVENT_REINTERPRET")
        .expect("an audit record");
    assert_eq!(entry.operator, OP);
    assert_eq!(entry.reason.as_deref(), Some("讀卡機站點補正"));
}

#[tokio::test]
async fn reinterpret_demands_reason() {
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, running_plan()).await.unwrap();

    let event_id = seed_exception(&store, "a1", Instant(1_000_100)).await;
    let cmd = OperatorCommand::new(OP, Instant(1_000_200)); // no reason
    let spec = ReinterpretSpec {
        station: "SKIERG".to_string(),
        mode: ReaderMode::Entry,
        athlete_id: None,
        at: None,
    };

    let result = reinterpret(&mut state, &store, event_id, spec, &cmd).await;
    assert!(matches!(result, Err(OperatorError::ReasonRequired)));
}

#[tokio::test]
async fn reinterpret_can_reattribute_to_another_athlete() {
    let store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, running_plan()).await.unwrap();

    let event_id = seed_exception(&store, "a1", Instant(1_000_100)).await;
    let cmd = OperatorCommand::new(OP, Instant(1_000_200)).with_reason("手環誤戴給二號");
    let spec = ReinterpretSpec {
        station: "ROWING".to_string(),
        mode: ReaderMode::Entry,
        athlete_id: Some("a2".to_string()),
        at: Some(Instant(1_000_150)),
    };

    reinterpret(&mut state, &store, event_id, spec, &cmd)
        .await
        .expect("reinterpret should succeed");

    let a1 = state.athlete("a1").unwrap();
    assert_eq!(a1.status, AthleteStatus::Ready);
    assert_eq!(a1.runs.len(), 0);

    let a2 = state.athlete("a2").unwrap();
    assert_eq!(a2.status, AthleteStatus::Active);
    assert_eq!(a2.current_station.as_deref(), Some("ROWING"));
    assert_eq!(a2.runs[0].entered_at, Instant(1_000_150));
}

#[tokio::test]
async fn a_durable_void_with_a_failed_rebuild_is_retried_before_the_next_action() {
    let mut store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, running_plan()).await.unwrap();
    let event_id = seed_exception(&store, "a1", Instant(1_000_100)).await;
    state.exception_count = 1;

    store.fail_rebuild_athletes = true;
    let cmd = OperatorCommand::new(OP, Instant(1_000_200)).with_reason("誤讀");
    let result = void(&mut state, &store, event_id, &cmd).await;
    assert!(matches!(result, Err(OperatorError::Storage(_))));

    // The durable void is not replayed a second time. The next operator boundary first
    // repairs the dirty projection, then reports that the already-voided row is unknown.
    store.fail_rebuild_athletes = false;
    let result = void(&mut state, &store, event_id, &cmd).await;
    assert!(matches!(result, Err(OperatorError::UnknownEvent(id)) if id == event_id));
    assert_eq!(state.exception_count, 0);
}

#[tokio::test]
async fn an_accepted_exception_badge_read_failure_is_retried_without_reaccepting() {
    let mut store = FakeStore::new();
    let (mut state, _) = resume_or_start(&store, running_plan()).await.unwrap();
    let first = seed_exception(&store, "a1", Instant(1_000_100)).await;
    let second = seed_exception(&store, "a2", Instant(1_000_110)).await;
    state.exception_count = 2;

    store.fail_exception_count = true;
    let cmd = OperatorCommand::new(OP, Instant(1_000_200));
    let result = accept(&mut state, &store, first, &cmd).await;
    assert!(matches!(result, Err(OperatorError::Storage(_))));

    store.fail_exception_count = false;
    accept(&mut state, &store, second, &cmd)
        .await
        .expect("retry should rebuild the badge before accepting");
    assert_eq!(state.exception_count, 0);
}
