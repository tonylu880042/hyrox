//! The application-level ACK protocol (CLAUDE.md 15, 16, 24).
//!
//! The rule under test is "do not ACK before persistent storage commit succeeds". The API
//! is shaped so that an `Ack` can only be obtained from a `Commit`, and a `Commit` can only
//! be minted by `ingest` after the port returned `Ok` — these tests pin the behaviour, the
//! type system pins the shape.

use contract::{
    ingest, ingest_payload, AckPayload, AckStatus, CommitOutcome, EdgeEvent, EventId, EventStore,
    IngestError, ReceivedEvent, WireError,
};
use std::collections::HashMap;
use std::sync::Mutex;

const CANONICAL: &str = r#"{"device_id":"a4cf128b3d91","reader_id":"rfid-02",
    "boot_id":18,"sequence":10382,"tag_id":["E280117000001234"],
    "detected_at":1787734821382,"uptime_ms":382912}"#;

fn event() -> EdgeEvent {
    EdgeEvent::decode(CANONICAL.as_bytes()).unwrap()
}

fn received(e: EdgeEvent) -> ReceivedEvent {
    let at = e.detected_at + 120;
    ReceivedEvent::new(e, at)
}

/// A stand-in for the real SQLite store. It commits into a map keyed by the idempotency
/// key, which is exactly what `crates/storage` does with `device_id + boot_id + sequence`.
#[derive(Default)]
struct FakeStore {
    committed: Mutex<HashMap<EventId, ReceivedEvent>>,
    calls: Mutex<u32>,
    fail: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct DiskFull;

impl EventStore for FakeStore {
    type Error = DiskFull;

    async fn commit(&self, event: &ReceivedEvent) -> Result<CommitOutcome, DiskFull> {
        *self.calls.lock().unwrap() += 1;
        if self.fail {
            return Err(DiskFull);
        }
        let key = EventId::of(event.event());
        let mut map = self.committed.lock().unwrap();
        if map.contains_key(&key) {
            return Ok(CommitOutcome::AlreadyStored);
        }
        map.insert(key, event.clone());
        Ok(CommitOutcome::Stored)
    }
}

impl FakeStore {
    fn stored(&self) -> usize {
        self.committed.lock().unwrap().len()
    }
    fn calls(&self) -> u32 {
        *self.calls.lock().unwrap()
    }
}

#[tokio::test]
async fn a_committed_event_is_acked() {
    let store = FakeStore::default();
    let ack = ingest(&store, &received(event())).await.unwrap();

    assert_eq!(store.stored(), 1);
    let p = ack.payload();
    assert_eq!(p.device_id.as_str(), "a4cf128b3d91");
    assert_eq!(p.boot_id, 18);
    assert_eq!(p.sequence, 10382);
    assert_eq!(p.status, AckStatus::Stored);
}

#[tokio::test]
async fn a_failed_commit_produces_no_ack() {
    // The whole point: a storage failure must leave the edge holding the event
    // (CLAUDE.md 15, 18, 31).
    let store = FakeStore {
        fail: true,
        ..Default::default()
    };
    let err = ingest(&store, &received(event())).await.unwrap_err();
    assert!(matches!(err, IngestError::Storage(DiskFull)));
    assert_eq!(store.stored(), 0);
}

#[tokio::test]
async fn duplicate_delivery_is_acked_but_committed_once() {
    // Duplicate delivery is allowed; duplicate business processing is not (CLAUDE.md 16).
    let store = FakeStore::default();
    let ev = received(event());

    let first = ingest(&store, &ev).await.unwrap();
    let second = ingest(&store, &ev).await.unwrap();
    let third = ingest(&store, &ev).await.unwrap();

    assert_eq!(store.stored(), 1, "one event, one row");
    assert_eq!(
        store.calls(),
        3,
        "every delivery reaches the store, which decides"
    );
    assert_eq!(first.payload().status, AckStatus::Stored);
    assert_eq!(second.payload().status, AckStatus::Duplicate);
    assert_eq!(third.payload().status, AckStatus::Duplicate);
}

#[tokio::test]
async fn a_duplicate_is_still_acked_so_the_edge_can_release_it() {
    // If a redelivery went unacked the edge would resend it for ever.
    let store = FakeStore::default();
    let ev = received(event());
    ingest(&store, &ev).await.unwrap();
    let ack = ingest(&store, &ev).await.unwrap();
    assert_eq!(EventId::from(ack.payload()), EventId::of(ev.event()));
}

#[tokio::test]
async fn the_ack_addresses_exactly_the_event_that_was_committed() {
    let store = FakeStore::default();
    let mut other = event();
    other.sequence = 10383;
    let a = ingest(&store, &received(event())).await.unwrap();
    let b = ingest(&store, &received(other.clone())).await.unwrap();

    assert_eq!(EventId::from(a.payload()), EventId::of(&event()));
    assert_eq!(EventId::from(b.payload()), EventId::of(&other));
    assert_ne!(EventId::from(a.payload()), EventId::from(b.payload()));
}

#[tokio::test]
async fn a_malformed_payload_is_rejected_before_it_reaches_storage() {
    let store = FakeStore::default();
    let err = ingest_payload(&store, b"{\"device_id\":\"nope\"}", 1)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        IngestError::Malformed(WireError::Malformed(_))
    ));
    assert_eq!(store.calls(), 0);
}

#[tokio::test]
async fn ingesting_a_payload_records_arrival_without_touching_official_time() {
    let store = FakeStore::default();
    let arrived = 1_787_734_899_999;
    ingest_payload(&store, CANONICAL.as_bytes(), arrived)
        .await
        .unwrap();

    let map = store.committed.lock().unwrap();
    let stored = map.values().next().unwrap();
    assert_eq!(stored.official_time(), 1_787_734_821_382);
    assert_eq!(stored.received_at(), arrived);
}

#[test]
fn the_ack_wire_form_round_trips_for_the_edge_to_read() {
    let raw = r#"{"device_id":"a4cf128b3d91","boot_id":18,"sequence":10382,"status":"STORED"}"#;
    let p = AckPayload::decode(raw.as_bytes()).unwrap();
    assert_eq!(p.status, AckStatus::Stored);
    assert_eq!(AckPayload::decode(&p.encode()).unwrap(), p);

    let dup = r#"{"device_id":"a4cf128b3d91","boot_id":18,"sequence":1,"status":"DUPLICATE"}"#;
    assert_eq!(
        AckPayload::decode(dup.as_bytes()).unwrap().status,
        AckStatus::Duplicate
    );
}
