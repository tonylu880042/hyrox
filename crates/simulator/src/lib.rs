//! Emulated ESP32 edge collectors (CLAUDE.md 25).
//!
//! Everything the venue can do to the system — a tag held at an antenna, a dropped link, a
//! power cut, a duplicated or reordered delivery, a lost ACK — is a deterministic function
//! call here, so all of it is testable with no RFID hardware and no MQTT broker
//! (CLAUDE.md 24).
//!
//! Two rules this crate exists to pin down:
//!
//! * Suppression is **presence and re-arm**, never a fixed window and never a station's
//!   duration (CLAUDE.md 14) — see [`presence`].
//! * An event leaves the edge only when the hub has said it is durable (CLAUDE.md 18) —
//!   see [`journal`].
//!
//! It depends on `contract` for what it says and on `transport` for how it says it, and on
//! nothing else: a real edge collector carries no business meaning (CLAUDE.md 8), so this one
//! must not be able to either (ADR 0005).

pub mod bench;
pub mod device;
pub mod error;
pub mod hub;
pub mod journal;
pub mod link;
pub mod presence;

pub use bench::{Bench, FlushReport};
pub use device::{DeviceConfig, RfOutcome, SimDevice};
pub use error::{ConfigError, DeviceError, JournalError};
pub use hub::{HubError, InMemoryHub};
pub use journal::{AckResult, Journal, JournalConfig};
pub use link::{AckDelivery, Duplication, Link, LinkFaults, Ordering};
pub use presence::{AbsentTimeout, PresenceDecision, ReaderConfig, TagPresence};
