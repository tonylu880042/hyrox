//! The ESP32 ↔ Central Hub wire layer (CLAUDE.md 15, 16, 17).
//!
//! This crate is the contract and nothing else. It does not depend on `domain` or on
//! `storage`: an edge collector knows no business meaning (CLAUDE.md 8), and the layers
//! above depend on the contract rather than the contract depending on them (CLAUDE.md 3).
//!
//! Three rules the rest of the system leans on:
//!
//! * `detected_at` is official timing; `received_at` is diagnostics (CLAUDE.md 17).
//! * `device_id + boot_id + sequence` identifies an event. Duplicate delivery is allowed,
//!   duplicate processing is not (CLAUDE.md 16).
//! * Nothing is acknowledged before it is durable (CLAUDE.md 15) — see [`ack`].
//!
//! The broker itself lives behind the `broker` feature, so all of the above compiles and
//! tests with no MQTT anywhere in the build (CLAUDE.md 24).

pub mod ack;
pub mod event;
pub mod id;
pub mod status;
pub mod topic;

#[cfg(feature = "broker")]
pub mod client;

pub use ack::{
    ingest, ingest_payload, Ack, AckPayload, AckStatus, Commit, CommitOutcome, EventStore,
    IngestError,
};
pub use event::{EdgeEvent, ReceivedEvent, WireError};
pub use id::{DeviceId, EventId, IdError, ReaderId};
pub use status::{DeviceStatus, DeviceWarning};

#[cfg(feature = "broker")]
pub use client::{MqttConfig, QOS};
