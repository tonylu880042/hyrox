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
    session.arm().unwrap();
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
    let mut a = raw(1, 0);
    let mut b = raw(1, 0);
    b.boot_id = 19; // sequence numbers restart after a reboot
    store.save_raw(&mut a).await.unwrap();
    store.save_raw(&mut b).await.unwrap();
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
