//! One emulated ESP32 edge collector (CLAUDE.md 25).
//!
//! It knows about radios, counters and its own journal, and nothing about stations,
//! athletes or sessions — the edge must not carry business meaning (CLAUDE.md 8). Every
//! method takes the time explicitly: there is no hidden clock, so a scenario replays
//! identically every run (CLAUDE.md 29).

use crate::{
    AbsentTimeout, AckResult, ConfigError, DeviceError, Journal, JournalConfig, PresenceDecision,
    ReaderConfig, TagPresence,
};
use contract::{AckPayload, AckStatus, DeviceId, EdgeEvent, EventId, ReaderId};
use transport::DeviceStatus;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct DeviceConfig {
    pub device_id: DeviceId,
    pub readers: Vec<ReaderConfig>,
    pub journal: JournalConfig,
}

impl DeviceConfig {
    /// `mac` is the ESP32 base MAC in any of the usual spellings; it is the device's
    /// identity (CLAUDE.md 7.3).
    pub fn new(mac: &str) -> Result<Self, ConfigError> {
        Ok(Self {
            device_id: DeviceId::from_mac_str(mac)?,
            readers: Vec::new(),
            journal: JournalConfig::default(),
        })
    }

    pub fn with_reader(mut self, reader: ReaderConfig) -> Self {
        self.readers.retain(|r| r.reader_id != reader.reader_id);
        self.readers.push(reader);
        self
    }

    /// Attaches a reader at the default absent timeout (CLAUDE.md 14).
    pub fn with_default_reader(self, reader_id: &str) -> Result<Self, ConfigError> {
        let reader = ReaderConfig::new(reader_id, AbsentTimeout::default())?;
        Ok(self.with_reader(reader))
    }

    pub fn with_journal(mut self, journal: JournalConfig) -> Self {
        self.journal = journal;
        self
    }
}

/// What an RF read did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RfOutcome {
    /// A new appearance: journalled and awaiting an ACK.
    Emitted(EventId),
    /// Still the same continuous presence (CLAUDE.md 14).
    Suppressed,
}

pub struct SimDevice {
    device_id: DeviceId,
    boot_id: i64,
    next_sequence: i64,
    booted_at: i64,
    readers: BTreeMap<ReaderId, TagPresence>,
    journal: Journal,
    online: bool,
}

impl SimDevice {
    /// First power-on. `boot_id` starts at 1 so that 0 can never be mistaken for "unset"
    /// in the idempotency key.
    pub fn boot(config: DeviceConfig, now_ms: i64) -> Result<Self, ConfigError> {
        if config.readers.is_empty() {
            return Err(ConfigError::NoReaders);
        }
        let readers = config
            .readers
            .iter()
            .map(|r| (r.reader_id.clone(), TagPresence::new(r.absent_timeout)))
            .collect();
        Ok(Self {
            device_id: config.device_id,
            boot_id: 1,
            next_sequence: 1,
            booted_at: now_ms,
            readers,
            journal: Journal::new(config.journal),
            online: true,
        })
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    pub fn boot_id(&self) -> i64 {
        self.boot_id
    }

    /// The sequence the next emitted event will carry.
    pub fn next_sequence(&self) -> i64 {
        self.next_sequence
    }

    pub fn reader_ids(&self) -> Vec<ReaderId> {
        self.readers.keys().cloned().collect()
    }

    /// One RF read of `tag` at `reader`: an inventory round that saw exactly one tag.
    ///
    /// A suppressed read consumes no sequence number, and neither does a read that could
    /// not be journalled -- a gap in the sequence would look like a lost event to anyone
    /// auditing the log (CLAUDE.md 16).
    pub fn rf_read(
        &mut self,
        reader: &ReaderId,
        tag: &str,
        now_ms: i64,
    ) -> Result<RfOutcome, DeviceError> {
        self.rf_inventory(reader, &[tag], now_ms)
    }

    /// One UHF inventory round: every tag the reader saw in the same instant.
    ///
    /// Anti-collision means a reader reports several tags at once, so the round is the unit
    /// on the wire (ADR 0014): the tags that count as new sightings travel in **one** event,
    /// under one `sequence`, released by one ACK. Tags still continuously present are
    /// suppressed out of it, and a round where every tag is suppressed publishes nothing and
    /// consumes no sequence number.
    ///
    /// Presence is per tag, not per reader, so a crowd standing in the field never holds a
    /// departed tag's re-arm open.
    pub fn rf_inventory(
        &mut self,
        reader: &ReaderId,
        tags: &[&str],
        now_ms: i64,
    ) -> Result<RfOutcome, DeviceError> {
        let fresh: Vec<String> = {
            let presence = self
                .readers
                .get_mut(reader)
                .ok_or_else(|| DeviceError::UnknownReader(reader.clone()))?;
            tags.iter()
                .filter(|tag| presence.observe(tag, now_ms) == PresenceDecision::Emit)
                .map(|tag| tag.to_string())
                .collect()
        };
        if fresh.is_empty() {
            return Ok(RfOutcome::Suppressed);
        }

        let event = EdgeEvent {
            device_id: self.device_id.clone(),
            reader_id: reader.clone(),
            boot_id: self.boot_id,
            sequence: self.next_sequence,
            tag_id: fresh,
            // Official timing is the moment of detection on the edge clock (CLAUDE.md 17).
            detected_at: now_ms,
            // Clamped: an emulated device cannot have been running for negative time. A
            // caller winding the clock back is a test bug, not a wire-level condition.
            uptime_ms: (now_ms - self.booted_at).max(0),
        };
        let key = event.id();
        self.journal.append(event)?;
        self.next_sequence += 1;
        Ok(RfOutcome::Emitted(key))
    }

    /// A power cycle. `boot_id` advances and `sequence` restarts, which together keep the
    /// new events from colliding with the old ones (CLAUDE.md 16); the journal survives,
    /// which is the point of it being persistent (CLAUDE.md 18); presence is lost, because
    /// it lived in RAM; and the link drops, because it did.
    pub fn reboot(&mut self, now_ms: i64) {
        self.boot_id += 1;
        self.next_sequence = 1;
        self.booted_at = now_ms;
        for presence in self.readers.values_mut() {
            presence.forget_all();
        }
        self.online = false;
    }

    pub fn is_online(&self) -> bool {
        self.online
    }

    pub fn disconnect(&mut self) {
        self.online = false;
    }

    pub fn reconnect(&mut self) {
        self.online = true;
    }

    /// Everything still owed an ACK, oldest first. Recording continues while offline; only
    /// delivery stops (CLAUDE.md 31).
    pub fn pending(&self) -> Vec<EdgeEvent> {
        self.journal.pending()
    }

    pub fn pending_count(&self) -> usize {
        self.journal.pending_count()
    }

    /// What the device would publish right now. A reconnect resends the whole backlog
    /// (CLAUDE.md 18) — publishing releases nothing, only an ACK does.
    pub fn publish_batch(&self) -> Vec<EdgeEvent> {
        if self.online {
            self.journal.pending()
        } else {
            Vec::new()
        }
    }

    pub fn on_ack(&mut self, ack: &AckPayload) -> AckResult {
        self.acknowledge(&EventId::from(ack), ack.status)
    }

    /// Both ACK statuses release the event: `Duplicate` means the hub already has it
    /// durably, which is exactly as good as having just stored it (CLAUDE.md 16).
    pub fn acknowledge(&mut self, key: &EventId, status: AckStatus) -> AckResult {
        match status {
            // Both statuses are proof of durability, so both release the entry.
            AckStatus::Stored | AckStatus::Duplicate => self.journal.ack(key),
        }
    }

    /// Health for the status topic (CLAUDE.md 18).
    pub fn status(&self) -> DeviceStatus {
        DeviceStatus {
            device_id: self.device_id.clone(),
            boot_id: self.boot_id,
            pending_events: self.journal.pending_count() as u64,
            journal_capacity: self.journal.capacity() as u64,
            warning: self.journal.warning(),
        }
    }
}
