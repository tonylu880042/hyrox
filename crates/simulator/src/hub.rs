//! An in-memory stand-in for the Central Hub's store.
//!
//! It implements [`mqtt::EventStore`] with the one behaviour the real SQLite store must
//! also have: idempotency on `device_id + boot_id + sequence` (CLAUDE.md 16). That is
//! enough to exercise the whole edge → link → hub → ACK loop with no broker, no database
//! and no hardware (CLAUDE.md 24).
//!
//! It is a test double. The real store is `crates/storage`, and wiring the two together is
//! a separate piece of work.

use mqtt::{CommitOutcome, EventId, EventStore, ReceivedEvent};
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HubError {
    /// Simulates a store that cannot commit. No commit means no ACK (CLAUDE.md 15).
    #[error("persistent storage is unavailable")]
    StorageUnavailable,
}

#[derive(Default)]
struct HubState {
    committed: BTreeMap<EventId, ReceivedEvent>,
    /// Distinct events in the order they first arrived — deliberately not the order they
    /// happened, so a test can tell the two apart.
    arrival: Vec<EventId>,
    commit_calls: usize,
    failing: bool,
}

#[derive(Default)]
pub struct InMemoryHub {
    state: Mutex<HubState>,
}

impl InMemoryHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Turns storage failure on or off mid-scenario.
    pub fn set_failing(&self, failing: bool) {
        self.state.lock().unwrap().failing = failing;
    }

    /// Distinct events made durable.
    pub fn committed_count(&self) -> usize {
        self.state.lock().unwrap().committed.len()
    }

    /// Deliveries that reached the store, duplicates included. The gap between this and
    /// `committed_count` is the deduplication doing its job.
    pub fn commit_calls(&self) -> usize {
        self.state.lock().unwrap().commit_calls
    }

    pub fn arrival_order(&self) -> Vec<EventId> {
        self.state.lock().unwrap().arrival.clone()
    }

    pub fn contains(&self, key: &EventId) -> bool {
        self.state.lock().unwrap().committed.contains_key(key)
    }

    /// The stored `detected_at` — the only timestamp a result may be computed from.
    pub fn official_time(&self, key: &EventId) -> Option<i64> {
        self.state
            .lock()
            .unwrap()
            .committed
            .get(key)
            .map(|e| e.official_time())
    }

    /// Diagnostics: how late the event was, which must never affect anything.
    pub fn arrival_lag_ms(&self, key: &EventId) -> Option<i64> {
        self.state
            .lock()
            .unwrap()
            .committed
            .get(key)
            .map(|e| e.arrival_lag_ms())
    }
}

impl EventStore for InMemoryHub {
    type Error = HubError;

    async fn commit(&self, event: &ReceivedEvent) -> Result<CommitOutcome, HubError> {
        let mut state = self.state.lock().unwrap();
        state.commit_calls += 1;
        if state.failing {
            return Err(HubError::StorageUnavailable);
        }
        let key = event.id();
        if state.committed.contains_key(&key) {
            return Ok(CommitOutcome::AlreadyStored);
        }
        state.committed.insert(key.clone(), event.clone());
        state.arrival.push(key);
        Ok(CommitOutcome::Stored)
    }
}
