//! The persistent edge journal (CLAUDE.md 18).
//!
//! An append-only log with an ACK cursor, in front of a ring buffer that only ever reclaims
//! *acknowledged* space. The invariant the whole reliability story rests on:
//!
//! > an event leaves the journal only when the hub has said it is durable.
//!
//! So a lost ACK costs a redelivery (harmless — the hub deduplicates on
//! `device_id + boot_id + sequence`), while dropping an unacked event would cost a result.
//!
//! This is an in-memory model of flash, not a flash driver: it exists so the *semantics*
//! are testable without hardware (CLAUDE.md 24) and so firmware has a specification to
//! match.

use crate::{ConfigError, JournalError};
use contract::{EdgeEvent, EventId};
use transport::DeviceWarning;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalConfig {
    pub capacity: usize,
    /// Percentage of capacity at which the device starts warning (CLAUDE.md 18).
    pub warn_at_percent: u8,
    /// How many acknowledged entries are reclaimed at once. Batching is required: erasing
    /// flash after every ACK would burn the part out.
    pub reclaim_batch: usize,
}

impl JournalConfig {
    /// CLAUDE.md 18's minimum design target of 10,000 events per ESP32.
    pub const DEFAULT_CAPACITY: usize = 10_000;
    /// Leaves a fifth of the journal as headroom to get an operator's attention.
    pub const DEFAULT_WARN_AT_PERCENT: u8 = 80;
    /// One erase block's worth, in events.
    pub const DEFAULT_RECLAIM_BATCH: usize = 256;

    pub fn new(
        capacity: usize,
        warn_at_percent: u8,
        reclaim_batch: usize,
    ) -> Result<Self, ConfigError> {
        if capacity == 0 {
            return Err(ConfigError::ZeroCapacity);
        }
        if warn_at_percent == 0 || warn_at_percent > 100 {
            return Err(ConfigError::WarnThreshold(warn_at_percent));
        }
        if reclaim_batch == 0 {
            return Err(ConfigError::ZeroReclaimBatch);
        }
        Ok(Self { capacity, warn_at_percent, reclaim_batch })
    }
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            capacity: Self::DEFAULT_CAPACITY,
            warn_at_percent: Self::DEFAULT_WARN_AT_PERCENT,
            reclaim_batch: Self::DEFAULT_RECLAIM_BATCH,
        }
    }
}

/// What an arriving ACK did. `Unknown` and `AlreadyReleased` are both fine: an ACK may
/// arrive twice, or for an entry already reclaimed, and neither may corrupt the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AckResult {
    Released,
    AlreadyReleased,
    Unknown,
}

#[derive(Clone, Debug)]
struct Entry {
    event: EdgeEvent,
    acked: bool,
}

#[derive(Clone, Debug)]
pub struct Journal {
    config: JournalConfig,
    entries: VecDeque<Entry>,
}

impl Journal {
    pub fn new(config: JournalConfig) -> Self {
        Self { config, entries: VecDeque::new() }
    }

    pub fn config(&self) -> JournalConfig {
        self.config
    }

    pub fn capacity(&self) -> usize {
        self.config.capacity
    }

    /// Entries occupying space, acknowledged or not.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn pending_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.acked).count()
    }

    /// Everything still owed an ACK, oldest first — exactly what a reconnect resends.
    pub fn pending(&self) -> Vec<EdgeEvent> {
        self.entries
            .iter()
            .filter(|e| !e.acked)
            .map(|e| e.event.clone())
            .collect()
    }

    pub fn contains(&self, key: &EventId) -> bool {
        self.entries.iter().any(|e| e.event.id() == *key)
    }

    /// Records an event, reclaiming acknowledged space first if the journal is full.
    pub fn append(&mut self, event: EdgeEvent) -> Result<(), JournalError> {
        if self.entries.len() >= self.config.capacity {
            self.reclaim();
        }
        if self.entries.len() >= self.config.capacity {
            return Err(JournalError::Full {
                pending: self.pending_count(),
                capacity: self.config.capacity,
            });
        }
        self.entries.push_back(Entry { event, acked: false });
        Ok(())
    }

    /// Marks one event acknowledged. The entry stays put — space is reclaimed later, in
    /// batches, and only when it is needed.
    pub fn ack(&mut self, key: &EventId) -> AckResult {
        match self.entries.iter_mut().find(|e| e.event.id() == *key) {
            Some(entry) if entry.acked => AckResult::AlreadyReleased,
            Some(entry) => {
                entry.acked = true;
                AckResult::Released
            }
            None => AckResult::Unknown,
        }
    }

    /// Health for the status topic (CLAUDE.md 18). `JournalFull` means the next RF read
    /// cannot be recorded, which is a critical, operator-visible condition.
    pub fn warning(&self) -> Option<DeviceWarning> {
        let used = self.entries.len();
        if used >= self.config.capacity && self.reclaimable() == 0 {
            return Some(DeviceWarning::JournalFull);
        }
        let threshold = self.config.capacity * self.config.warn_at_percent as usize;
        (used * 100 >= threshold).then_some(DeviceWarning::JournalNearlyFull)
    }

    /// Leading acknowledged entries — the only space that may ever be taken back.
    fn reclaimable(&self) -> usize {
        self.entries.iter().take_while(|e| e.acked).count()
    }

    fn reclaim(&mut self) {
        for _ in 0..self.config.reclaim_batch {
            match self.entries.front() {
                Some(entry) if entry.acked => {
                    self.entries.pop_front();
                }
                _ => break,
            }
        }
    }
}
