//! The ESP32 → Central Hub event contract (CLAUDE.md 16).
//!
//! `device_id` and `reader_id` decode straight into the `domain` identity types
//! (CLAUDE.md 7.3): the wire lands on the very type the reader registry is keyed by, so
//! there is no conversion step between them to drift out of step (ADR 0005).

use crate::{DeviceId, EventId, ReaderId};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("payload is not a valid edge event: {0}")]
    Malformed(String),
    /// Counters come from monotonic hardware; a negative one means a corrupt or spoofed
    /// payload, and storing it would poison the idempotency key.
    #[error("{field} must not be negative (got {value})")]
    NegativeCounter { field: &'static str, value: i64 },
    #[error("{0} must not be empty")]
    EmptyField(&'static str),
}

/// Exactly the fields an ESP32 publishes (CLAUDE.md 16). Nothing here carries business
/// meaning: the edge does not know what a station is (CLAUDE.md 8).
///
/// Changing this struct changes the contract with deployed firmware, which CLAUDE.md 30
/// forbids doing silently.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeEvent {
    pub device_id: DeviceId,
    pub reader_id: ReaderId,
    pub boot_id: i64,
    pub sequence: i64,
    pub tag_id: String,
    /// Epoch milliseconds on the edge clock. This is the **official** timing source
    /// (CLAUDE.md 11, 17) and the only timestamp a result may ever be computed from.
    pub detected_at: i64,
    /// Milliseconds since this boot. Diagnostics: it is what lets an operator tell a clock
    /// jump from a genuinely late event.
    pub uptime_ms: i64,
}

impl EdgeEvent {
    pub fn decode(payload: &[u8]) -> Result<Self, WireError> {
        let event: EdgeEvent =
            serde_json::from_slice(payload).map_err(|e| WireError::Malformed(e.to_string()))?;
        event.validate()?;
        Ok(event)
    }

    pub fn encode(&self) -> Vec<u8> {
        // Serialising our own struct cannot fail.
        serde_json::to_vec(self).expect("EdgeEvent is always serialisable")
    }

    pub fn id(&self) -> EventId {
        EventId::of(self)
    }

    fn validate(&self) -> Result<(), WireError> {
        for (field, value) in [
            ("boot_id", self.boot_id),
            ("sequence", self.sequence),
            ("detected_at", self.detected_at),
            ("uptime_ms", self.uptime_ms),
        ] {
            if value < 0 {
                return Err(WireError::NegativeCounter { field, value });
            }
        }
        if self.tag_id.trim().is_empty() {
            return Err(WireError::EmptyField("tag_id"));
        }
        Ok(())
    }
}

/// An `EdgeEvent` plus the hub's own arrival stamp (CLAUDE.md 16).
///
/// `received_at` is kept separate and is never merged into the event, so no later layer can
/// mistake arrival for detection. MQTT latency is not competition timing (CLAUDE.md 17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceivedEvent {
    event: EdgeEvent,
    received_at: i64,
}

impl ReceivedEvent {
    pub fn new(event: EdgeEvent, received_at: i64) -> Self {
        Self { event, received_at }
    }

    pub fn event(&self) -> &EdgeEvent {
        &self.event
    }

    pub fn into_event(self) -> EdgeEvent {
        self.event
    }

    pub fn id(&self) -> EventId {
        self.event.id()
    }

    /// The timestamp any result must be computed from (CLAUDE.md 11, 17).
    pub fn official_time(&self) -> i64 {
        self.event.detected_at
    }

    /// Diagnostics only.
    pub fn received_at(&self) -> i64 {
        self.received_at
    }

    /// How long the event spent in the journal and on the wire. Useful for spotting a
    /// flapping link or a device replaying its backlog; never useful for timing.
    pub fn arrival_lag_ms(&self) -> i64 {
        self.received_at - self.event.detected_at
    }
}
