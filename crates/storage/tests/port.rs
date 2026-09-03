//! `Store` seen through the application's ports (ADR 0003), including the two things the
//! port added to this crate: the audit log (CLAUDE.md 20) and the roster exception reason.

use application::{AuditEntry, HubStore, InterpretedWrite, RawRead};
use contract::CommitOutcome;
use domain::{ExceptionReason, Instant, Interpreted, Session, SessionMode};
use storage::Store;

const T0: i64 = 1_787_734_800_000;

fn raw(seq: i64) -> RawRead {
    RawRead {
        device_id: "a4cf128b3d91".into(),
        reader_id: "rfid-02".into(),
        boot_id: 18,
        sequence: seq,
        tag_id: "E280117000001234".into(),
        detected_at: Instant(T0),
        received_at: Instant(T0 + 120),
    }
}

async fn armed_store() -> (Store, Session) {
    let store = Store::open_in_memory().await.unwrap();
    let mut session = Session::new_draft("s1", "Thursday Class", SessionMode::Training);
    session.mark_ready().unwrap();
    session.start().unwrap();
    HubStore::save_session(&store, &session, Instant(T0))
        .await
        .unwrap();
    HubStore::save_athlete(&store, "s1", "a1", "CHEN YU-TING", 1, None)
        .await
        .unwrap();
    (store, session)
}

#[tokio::test]
async fn a_redelivered_raw_event_reports_the_same_row() {
    let (store, _) = armed_store().await;

    let first = store.commit_raw(&raw(1)).await.unwrap();
    let again = store.commit_raw(&raw(1)).await.unwrap();

    // Duplicate delivery is allowed; a second row is not (CLAUDE.md 16).
    assert_eq!(first.outcome, CommitOutcome::Stored);
    assert_eq!(again.outcome, CommitOutcome::AlreadyStored);
    assert_eq!(first.raw_event_id, again.raw_event_id);
    assert_eq!(store.raw_event_count().await.unwrap(), 1);
}

#[tokio::test]
async fn a_roster_exception_survives_a_restart() {
    let (store, _) = armed_store().await;
    let committed = store.commit_raw(&raw(1)).await.unwrap();
    let event = Interpreted::Exception {
        reason: ExceptionReason::AthleteNotInSession,
        at: Instant(T0),
    };

    store
        .commit_interpreted(InterpretedWrite {
            session_id: "s1",
            athlete_id: "a1",
            raw_event_id: Some(committed.raw_event_id),
            event: &event,
        })
        .await
        .unwrap();

    // Reading it back is what proves the new reason has a stored spelling in both
    // directions; an unmapped one would come back as Corrupt.
    let rebuilt = HubStore::rebuild_athletes(&store, "s1").await.unwrap();
    assert_eq!(rebuilt.len(), 1);
    assert_eq!(rebuilt[0].status, domain::AthleteStatus::Ready);
}

/// Accepting is not voiding, and the difference has to be visible in the tables: the row
/// stays, the replay is untouched, and only the two things that mean "outstanding work" --
/// the inbox and its count -- stop naming it (ADR 0001 D4; migration 0011).
#[tokio::test]
async fn an_accepted_exception_leaves_the_inbox_but_stays_in_the_log() {
    let (store, _) = armed_store().await;
    let committed = store.commit_raw(&raw(1)).await.unwrap();
    let id = store
        .commit_interpreted(InterpretedWrite {
            session_id: "s1",
            athlete_id: "a1",
            raw_event_id: Some(committed.raw_event_id),
            event: &Interpreted::Exception {
                reason: ExceptionReason::ImpossibleTransition,
                at: Instant(T0),
            },
        })
        .await
        .unwrap();
    assert_eq!(HubStore::exception_count(&store, "s1").await.unwrap(), 1);

    let accepted = store
        .acknowledge_interpreted(
            id,
            Instant(T0 + 60_000),
            "FRONT DESK TABLET",
            Some("重複靠卡"),
        )
        .await
        .unwrap();

    assert!(accepted);
    assert!(
        HubStore::exceptions(&store, "s1").await.unwrap().is_empty(),
        "out of the inbox"
    );
    assert_eq!(
        HubStore::exception_count(&store, "s1").await.unwrap(),
        0,
        "and out of the badge"
    );
    // Still there, and still not voided: a correction that erased the evidence would be the
    // one thing CLAUDE.md 19 forbids.
    let row: (i64, Option<i64>, Option<String>) = sqlx::query_as(
        "SELECT count(*), max(acknowledged_at), max(acknowledged_by)
           FROM interpreted_events WHERE id = ?1 AND voided_at IS NULL",
    )
    .bind(id)
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(row.0, 1, "the interpretation is still in the log");
    assert_eq!(row.1, Some(T0 + 60_000));
    assert_eq!(row.2.as_deref(), Some("FRONT DESK TABLET"));
}

/// An id nobody has, or one already voided: the caller has to be able to answer 404 rather
/// than report a success that changed nothing.
#[tokio::test]
async fn accepting_something_that_is_not_an_open_exception_says_so() {
    let (store, _) = armed_store().await;

    let missing = store
        .acknowledge_interpreted(999, Instant(T0), "FRONT DESK TABLET", None)
        .await
        .unwrap();

    assert!(!missing);
}

#[tokio::test]
async fn an_audit_record_is_persisted() {
    let (store, _) = armed_store().await;

    store
        .record_audit(&AuditEntry {
            at: Instant(T0 + 5_000),
            operator: "FRONT DESK TABLET".into(),
            action: "SESSION_REOPEN".into(),
            subject: "s1".into(),
            reason: Some("誤觸".into()),
            before: Some("COMPLETED".into()),
            after: Some("RUNNING".into()),
        })
        .await
        .expect("audit write");
}
