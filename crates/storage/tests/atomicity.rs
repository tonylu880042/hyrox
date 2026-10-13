//! Transaction boundaries for the durable handoffs and derived finish writes.

use application::{
    AuditEntry, AuditedInterpretedWrite, AuditedWrite, FinishOrigin, RawInterpretationContext,
};
use domain::{ExceptionReason, Instant, Interpreted, Session, SessionMode};
use sqlx::Row;
use storage::{RawEvent, Store};

const NOW: Instant = Instant(1_787_734_800_000);

fn raw(sequence: i64) -> RawEvent {
    RawEvent {
        device_id: "a4cf128b3d91".into(),
        reader_id: "rfid-01".into(),
        boot_id: 7,
        sequence,
        tag_id: format!("TAG-{sequence}"),
        detected_at: NOW,
        received_at: Instant(NOW.0 + 100),
        presence: domain::Presence::Arrived,
    }
}

#[tokio::test]
async fn raw_and_context_roll_back_together_when_context_write_fails() {
    let store = Store::open_in_memory().await.unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_raw_context
         BEFORE INSERT ON raw_interpretation_context
         BEGIN SELECT RAISE(ABORT, 'injected context failure'); END",
    )
    .execute(store.pool())
    .await
    .unwrap();

    let context = RawInterpretationContext {
        session_id: "s1".into(),
        athlete_id: Some("a1".into()),
        bound_session_id: Some("s1".into()),
        on_roster: true,
        reader: None,
        planned: None,
    };
    let result = store.save_raw_with_context(&raw(1), &context).await;
    assert!(result.is_err());
    assert_eq!(store.raw_event_count().await.unwrap(), 0);
    let context_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM raw_interpretation_context")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(context_rows, 0);
}

#[tokio::test]
async fn a_failure_in_the_middle_of_finish_batch_rolls_back_earlier_rows() {
    let store = Store::open_in_memory().await.unwrap();
    let session = Session::new_draft("s1", "Class", SessionMode::Training);
    store.save_session(&session, NOW).await.unwrap();
    store.save_athlete("s1", "a1", "A", 1, None).await.unwrap();
    store.save_athlete("s1", "a2", "B", 2, None).await.unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_second_finish
         BEFORE UPDATE OF finished_at ON session_athletes
         WHEN NEW.athlete_id = 'a2'
         BEGIN SELECT RAISE(ABORT, 'injected finish failure'); END",
    )
    .execute(store.pool())
    .await
    .unwrap();

    let result = store
        .save_athlete_finishes(
            "s1",
            &[
                ("a1".into(), Instant(NOW.0 + 1)),
                ("a2".into(), Instant(NOW.0 + 2)),
            ],
            FinishOrigin::Policy,
        )
        .await;
    assert!(result.is_err());

    let rows = sqlx::query(
        "SELECT athlete_id, finished_at, finish_origin
         FROM session_athletes WHERE session_id = 's1' ORDER BY athlete_id",
    )
    .fetch_all(store.pool())
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert!(row.get::<Option<i64>, _>("finished_at").is_none());
        assert!(row.get::<Option<String>, _>("finish_origin").is_none());
    }
}

#[tokio::test]
async fn end_class_rolls_back_finishes_and_session_when_audit_fails() {
    let store = Store::open_in_memory().await.unwrap();
    let mut session = Session::new_draft("s1", "Class", SessionMode::Training);
    session.mark_ready().unwrap();
    session.start().unwrap();
    store.save_session(&session, NOW).await.unwrap();
    store.save_athlete("s1", "a1", "A", 1, None).await.unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_audit_end
         BEFORE INSERT ON audit_log
         BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END",
    )
    .execute(store.pool())
    .await
    .unwrap();

    let mut completed = session.clone();
    completed.complete(Instant(NOW.0 + 1)).unwrap();
    let result = store
        .apply_audited(AuditedWrite::EndClass {
            session: completed,
            created_at: NOW,
            finishes: vec![("a1".into(), Instant(NOW.0 + 1))],
            audit: AuditEntry {
                at: NOW,
                operator: "COACH".into(),
                action: "CLASS_END".into(),
                subject: "s1".into(),
                reason: Some("close".into()),
                before: Some("RUNNING".into()),
                after: Some("COMPLETED".into()),
            },
        })
        .await;
    assert!(result.is_err());

    let stored = store.session("s1").await.unwrap().unwrap();
    assert_eq!(stored.status, domain::SessionStatus::Running);
    let finish: Option<i64> = sqlx::query_scalar(
        "SELECT finished_at FROM session_athletes
         WHERE session_id = 's1' AND athlete_id = 'a1'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert!(finish.is_none());
    let audits: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(audits, 0);
}

#[tokio::test]
async fn reinterpret_rolls_back_the_void_and_replacement_when_audit_fails() {
    let store = Store::open_in_memory().await.unwrap();
    let mut session = Session::new_draft("s1", "Class", SessionMode::Training);
    session.mark_ready().unwrap();
    session.start().unwrap();
    store.save_session(&session, NOW).await.unwrap();
    let exception = Interpreted::Exception {
        reason: ExceptionReason::UnknownReader,
        at: Instant(NOW.0 + 1),
    };
    store
        .save_interpreted("s1", "a1", None, &exception)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_audit_reinterpret
         BEFORE INSERT ON audit_log
         BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END",
    )
    .execute(store.pool())
    .await
    .unwrap();

    let result = store
        .apply_audited(AuditedWrite::ExceptionReinterpret {
            session_id: "s1".into(),
            interpreted_event_id: 1,
            at: Instant(NOW.0 + 2),
            operator: "COACH".into(),
            reason: "reader fixed".into(),
            replacement: AuditedInterpretedWrite {
                session_id: "s1".into(),
                athlete_id: "a1".into(),
                raw_event_id: None,
                event: Interpreted::Entered {
                    station: "SKIERG".into(),
                    at: Instant(NOW.0 + 1),
                    transition: None,
                    started_timing: true,
                },
            },
            audit: AuditEntry {
                at: Instant(NOW.0 + 2),
                operator: "COACH".into(),
                action: "EVENT_REINTERPRET".into(),
                subject: "1".into(),
                reason: Some("reader fixed".into()),
                before: Some("EXCEPTION".into()),
                after: Some("ENTERED".into()),
            },
        })
        .await;
    assert!(result.is_err());

    let row = sqlx::query("SELECT voided_at FROM interpreted_events WHERE id = 1")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert!(row.get::<Option<i64>, _>("voided_at").is_none());
    let interpreted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM interpreted_events")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(interpreted, 1);
    let audits: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_log")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(audits, 0);
}
