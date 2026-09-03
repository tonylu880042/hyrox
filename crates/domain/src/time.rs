//! Epoch-millisecond instants and durations.
//!
//! Newtypes rather than bare i64 so an instant can never be handed to something
//! expecting a duration. Official timing is always `detected_at` (CLAUDE.md 11, 17).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct Instant(pub i64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct Duration(pub i64);

impl Instant {
    /// Elapsed time from `earlier` to self. Negative if the events arrived out of order,
    /// which the caller must decide how to treat rather than silently clamping here.
    pub fn since(self, earlier: Instant) -> Duration {
        Duration(self.0 - earlier.0)
    }
}

impl Duration {
    pub fn millis(self) -> i64 {
        self.0
    }
    pub fn is_negative(self) -> bool {
        self.0 < 0
    }
}

/// The class clock, with pause accounting (ADR 0008).
///
/// A paused class is not timing anybody, so paused wall time must not count towards a
/// duration-based finish rule (CLAUDE.md 12). Keeping the accounting here rather than
/// subtracting at each call site means every consumer -- the live screen, the finish
/// policy, the results page -- reads the same number.
///
/// Serialisable because it is part of the session row: a hub that restarts mid-pause must
/// come back still paused, with the same accumulated total (CLAUDE.md 21).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ClassClock {
    /// The wall-clock origin. For a class this is when the session row was created.
    pub started_at: Instant,
    /// Wall time already spent paused, excluding any pause still open.
    pub paused_total: Duration,
    /// When the open pause began, if the clock is paused right now.
    pub paused_since: Option<Instant>,
}

impl ClassClock {
    pub fn started_at(started_at: Instant) -> Self {
        Self {
            started_at,
            paused_total: Duration(0),
            paused_since: None,
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused_since.is_some()
    }

    /// Time the class has actually been running. Frozen while paused.
    pub fn elapsed(self, now: Instant) -> Duration {
        let open = self
            .paused_since
            .map(|since| now.since(since).millis())
            .unwrap_or(0);
        Duration(now.since(self.started_at).millis() - self.paused_total.millis() - open)
    }

    /// The wall-clock moment at which the class had been running for `elapsed`.
    ///
    /// This is what turns a class-duration rule into a result: the athlete stopped when the
    /// clock reached the limit, not when a background tick noticed (CLAUDE.md 11, 17). An
    /// open pause is deliberately not added -- while paused that moment is not yet knowable,
    /// and the rule cannot have fired anyway because the clock is frozen short of it.
    pub fn instant_at_elapsed(self, elapsed: Duration) -> Instant {
        Instant(self.started_at.0 + elapsed.millis() + self.paused_total.millis())
    }

    /// Idempotent: pausing an already-paused clock keeps the earlier pause, so a double tap
    /// on the operator screen cannot silently discard the time between them.
    pub fn pause(&mut self, at: Instant) {
        if self.paused_since.is_none() {
            self.paused_since = Some(at);
        }
    }

    /// Idempotent for the same reason: resuming a running clock does nothing rather than
    /// subtracting a pause that never happened.
    pub fn resume(&mut self, at: Instant) {
        if let Some(since) = self.paused_since.take() {
            self.paused_total = Duration(self.paused_total.millis() + at.since(since).millis());
        }
    }
}
