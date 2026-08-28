//! The ESP32 ↔ Central Hub contract (CLAUDE.md 15, 16, 17).
//!
//! Two systems meet here: the edge collector and the hub. This crate is what they agree
//! on, and nothing else — no topics, no broker, no database, no use cases. That is why it
//! can sit under `application` (which interprets events) and under `simulator` (which
//! emulates the firmware that produces them) without either being able to see the other.
//!
//! Three rules the rest of the system leans on:
//!
//! * `detected_at` is official timing; `received_at` is diagnostics (CLAUDE.md 17).
//! * `device_id + boot_id + sequence` identifies an event. Duplicate delivery is allowed,
//!   duplicate processing is not (CLAUDE.md 16).
//! * Nothing is acknowledged before it is durable (CLAUDE.md 15) — see [`ack`].
//!
//! Identity (`DeviceId`, `ReaderId`) is *not* defined here. It belongs to `domain`
//! (CLAUDE.md 7.3) and is re-exported below, so the wire decodes straight into the same
//! type the reader registry is keyed by and no conversion layer can drift.
//!
//! The transport that carries all of this — topics, QoS, the rumqttc client — is
//! `crates/transport` (CLAUDE.md 3: the contract does not depend on its delivery
//! mechanism).

pub mod ack;
pub mod event;
pub mod id;

pub use ack::{
    ingest, ingest_payload, Ack, AckPayload, AckStatus, Commit, CommitOutcome, EventStore,
    IngestError,
};
pub use event::{EdgeEvent, ReceivedEvent, WireError};
pub use id::EventId;

/// The identities the wire carries, re-exported from `domain` so edge-side code — the
/// simulator, and firmware-equivalent paths — names one crate and still gets the canonical
/// types (CLAUDE.md 7.3).
pub use domain::{DeviceId, DeviceIdError, ReaderId, ReaderIdError};
