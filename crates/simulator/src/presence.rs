//! RF-level suppression by tag presence and re-arm (CLAUDE.md 14).
//!
//! ```text
//! first_seen                        → SEND
//! same tag continuously visible     → suppress
//! absent longer than absent_timeout → re-arm
//! seen again after re-arm           → SEND
//! ```
//!
//! There is no periodic window here, and deliberately so: CLAUDE.md 14 rules out a fixed
//! 60-second suppression, and forbids using a station's duration as a suppression
//! duration. The only knob is `absent_timeout`, which belongs to the reader.
//!
//! This is RF-level suppression only. Business-level deduplication is the hub's job.

use crate::ConfigError;
use std::collections::BTreeMap;

/// How long a tag must be *out of sight* before its next appearance counts as new.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct AbsentTimeout(i64);

impl AbsentTimeout {
    /// The middle of the 3–5 s target in CLAUDE.md 14. A starting point for venue tuning,
    /// not a rule: every reader may override it, and the real value has to be measured on
    /// site with the actual antennas.
    pub const DEFAULT_MS: i64 = 4_000;

    pub fn from_millis(ms: i64) -> Result<Self, ConfigError> {
        if ms <= 0 {
            return Err(ConfigError::NonPositiveTimeout(ms));
        }
        Ok(Self(ms))
    }

    pub fn millis(self) -> i64 {
        self.0
    }
}

impl Default for AbsentTimeout {
    fn default() -> Self {
        Self(Self::DEFAULT_MS)
    }
}

/// One reader's configuration. The timeout lives here rather than on the device because
/// CLAUDE.md 14 asks for it per reader: a doorway antenna and a station antenna do not see
/// the same dwell behaviour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderConfig {
    pub reader_id: mqtt::ReaderId,
    pub absent_timeout: AbsentTimeout,
}

impl ReaderConfig {
    pub fn new(reader_id: &str, absent_timeout: AbsentTimeout) -> Result<Self, ConfigError> {
        Ok(Self {
            reader_id: mqtt::ReaderId::new(reader_id)?,
            absent_timeout,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenceDecision {
    /// A new appearance: publish an event.
    Emit,
    /// Still the same continuous presence: the RF read goes no further.
    Suppressed,
}

/// Per-reader presence state. Lives in RAM on the real ESP32, which is why
/// [`TagPresence::forget_all`] is what a reboot does to it.
#[derive(Clone, Debug)]
pub struct TagPresence {
    timeout: AbsentTimeout,
    /// Tag → the instant it was last seen. Presence is "last seen recently", so every read
    /// extends it; a tag held at the antenna for a whole station never re-arms.
    last_seen: BTreeMap<String, i64>,
}

impl TagPresence {
    pub fn new(timeout: AbsentTimeout) -> Self {
        Self { timeout, last_seen: BTreeMap::new() }
    }

    pub fn timeout(&self) -> AbsentTimeout {
        self.timeout
    }

    /// One RF read at `now_ms`. Idempotent in the sense that matters: repeated reads of an
    /// already-present tag change nothing but the presence clock.
    pub fn observe(&mut self, tag: &str, now_ms: i64) -> PresenceDecision {
        let decision = match self.last_seen.get(tag) {
            // Strictly greater: an absence of exactly the timeout is not yet a re-arm, so
            // the boundary is one documented place instead of an accident.
            Some(&seen) if now_ms - seen > self.timeout.millis() => PresenceDecision::Emit,
            Some(_) => PresenceDecision::Suppressed,
            None => PresenceDecision::Emit,
        };
        self.last_seen.insert(tag.to_string(), now_ms);
        decision
    }

    /// Drops tags that have been away long enough to re-arm anyway. Bounds the table on a
    /// device that sees thousands of tags in a day; changes no decision.
    pub fn prune(&mut self, now_ms: i64) {
        let timeout = self.timeout.millis();
        self.last_seen.retain(|_, seen| now_ms - *seen <= timeout);
    }

    /// What a power cycle does: the radio comes back knowing nothing.
    pub fn forget_all(&mut self) {
        self.last_seen.clear();
    }

    pub fn tracked_tags(&self) -> usize {
        self.last_seen.len()
    }
}
