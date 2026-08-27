//! Epoch-millisecond instants and durations.
//!
//! Newtypes rather than bare i64 so an instant can never be handed to something
//! expecting a duration. Official timing is always `detected_at` (CLAUDE.md 11, 17).

use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
pub struct Instant(pub i64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize)]
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
