//! `Store` seen through the application's ports (ADR 0003), including the two things the
//! port added to this crate: the audit log (CLAUDE.md 20) and the roster exception reason.

use application::{AuditEntry, HubStore, InterpretedWrite, RawRead};
use domain::{ExceptionReason, Instant, Interpreted, Session, SessionMode};
use contract::CommitOutcome;
use storage::Store;

const T0: i64 = 1_787_734_800_000;

fn raw(seq: i64) -> RawRead {
    RawRead {
        device_id: "esp32-a4cf128b3d91".into(),
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
    session.arm().unwrap();
    HubStore::save_session(&store, &session, Instant(T0)).await.unwrap();
    HubStore::save_athlete(&store, "s1", "a1", "CHEN YU-TING", 1).await.unwrap();
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
            before: Some("CLOSED".into()),
            after: Some("ARMED".into()),
        })
        .await
        .expect("audit write");
}
