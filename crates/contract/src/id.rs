//! The idempotency key.
//!
//! Device and reader identity live in `domain` (CLAUDE.md 7.3) and are re-exported by this
//! crate's root. Only the key that identifies *an event* is defined here, because it is the
//! wire contract that gives it its parts (CLAUDE.md 16).

use crate::DeviceId;
use std::fmt;

/// The idempotency key: `device_id + boot_id + sequence` (CLAUDE.md 16).
///
/// A real type rather than a loose tuple, because the three parts are meaningless apart
/// and because both the store and the ACK path must agree on what "the same event" means.
/// `boot_id` is what keeps a post-reboot `sequence` restart from colliding with the events
/// of the previous boot.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId {
    device_id: DeviceId,
    boot_id: i64,
    sequence: i64,
}

impl EventId {
    pub fn new(device_id: DeviceId, boot_id: i64, sequence: i64) -> Self {
        Self {
            device_id,
            boot_id,
            sequence,
        }
    }

    pub fn of(event: &crate::EdgeEvent) -> Self {
        Self::new(event.device_id.clone(), event.boot_id, event.sequence)
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }
    pub fn boot_id(&self) -> i64 {
        self.boot_id
    }
    pub fn sequence(&self) -> i64 {
        self.sequence
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.device_id, self.boot_id, self.sequence)
    }
}

impl fmt::Debug for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EventId({self})")
    }
}
