//! MQTT delivery for the edge ↔ hub contract (CLAUDE.md 5, 15).
//!
//! Everything that *decides* anything — the event contract, the idempotency key, the ACK
//! rule — is in `crates/contract`. What is left here is transport: the topic scheme, the
//! device health payload that rides on it, the classification of an arriving message
//! ([`inbound`]) and the rumqttc client. A delivery mechanism
//! must be replaceable without touching a business rule (CLAUDE.md 3), so nothing in this
//! crate may grow one (CLAUDE.md 29).
//!
//! ```text
//! hyrox/v1/edge/<device>/events   edge → hub   contract::EdgeEvent
//! hyrox/v1/edge/<device>/status   edge → hub   DeviceStatus
//! hyrox/v1/hub/<device>/ack       hub  → edge  contract::Ack
//! ```
//!
//! The broker itself lives behind the `broker` feature, so the topic scheme and the status
//! payloads compile and test with no MQTT anywhere in the build (CLAUDE.md 24).

pub mod inbound;
pub mod status;
pub mod topic;

#[cfg(feature = "broker")]
pub mod client;

pub use inbound::{classify, payload_excerpt, Inbound};
pub use status::{DeviceStatus, DeviceWarning};

#[cfg(feature = "broker")]
pub use client::{MqttConfig, QOS};
