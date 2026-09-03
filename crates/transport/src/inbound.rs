//! What arrived on a subscription, before anyone has decided what it means.
//!
//! Splitting classification out of the broker client buys two things. It is pure — topic in,
//! bytes in, verdict out — so every case including the ones a broker is awkward to provoke
//! (a foreign topic, a truncated payload) is testable with no MQTT in the build at all
//! (CLAUDE.md 24). And it keeps the subscriber loop down to *decode, hand off, publish the
//! ACK it was given*: the loop chooses nothing, so no business rule can settle here
//! (CLAUDE.md 29).
//!
//! One thing is deliberately **not** decided here: a payload whose `device_id` disagrees
//! with the device segment of the topic it arrived on is still returned as an event. The
//! payload is the contract (CLAUDE.md 16) and the idempotency key is built from it; the
//! topic is only an address, and the ACK is published back to the device the *payload*
//! names, so the two cannot drift apart. Rejecting on a mismatch would mean dropping a
//! real read over an addressing mistake (CLAUDE.md 31 principle 1).

use crate::{topic, DeviceStatus};
use contract::{AckPayload, EdgeEvent, WireError};

/// One message off the wire, classified but not interpreted.
#[derive(Debug)]
pub enum Inbound {
    /// The connection came up. Emitted by the broker client, never by [`classify`]; the
    /// subscriber re-subscribes on it, because a broker that restarted has forgotten the
    /// subscriptions its `clean_session = false` clients were relying on.
    Connected {
        /// Whether the broker still had this client's QoS 1 session.
        session_present: bool,
    },
    /// A valid edge event (CLAUDE.md 16). Needs a `received_at` stamp and an ingest.
    Event(Box<EdgeEvent>),
    /// A device health report (CLAUDE.md 18).
    Status(Box<DeviceStatus>),
    /// An acknowledgement on the downlink branch (CLAUDE.md 15). Only an edge collector
    /// subscribes to these; the hub sees them classified but never receives one.
    Ack(Box<AckPayload>),
    /// Arrived on one of our topics but could not be decoded. Kept whole rather than
    /// dropped: it is the only evidence that a device is publishing something the hub
    /// cannot read, and a broken device must not be able to silence itself
    /// (CLAUDE.md 31 principle 1).
    Undecodable {
        topic: String,
        error: WireError,
        payload: Vec<u8>,
    },
    /// Not one of our topics. Another publisher shares the broker; that is not a fault.
    Foreign { topic: String },
}

/// Decides which of our topics a message arrived on and decodes it accordingly.
///
/// Never panics and never fails: an undecodable payload is a variant, not an error, because
/// the subscriber has to keep running (a broken device must not stop a class).
pub fn classify(topic_name: &str, payload: &[u8]) -> Inbound {
    if topic::device_of_events(topic_name).is_some() {
        return match EdgeEvent::decode(payload) {
            Ok(event) => Inbound::Event(Box::new(event)),
            Err(error) => undecodable(topic_name, error, payload),
        };
    }
    if topic::device_of_status(topic_name).is_some() {
        return match DeviceStatus::decode(payload) {
            Ok(status) => Inbound::Status(Box::new(status)),
            Err(error) => undecodable(topic_name, error, payload),
        };
    }
    if topic::device_of_ack(topic_name).is_some() {
        return match AckPayload::decode(payload) {
            Ok(ack) => Inbound::Ack(Box::new(ack)),
            Err(error) => undecodable(topic_name, error, payload),
        };
    }
    Inbound::Foreign {
        topic: topic_name.to_string(),
    }
}

fn undecodable(topic_name: &str, error: WireError, payload: &[u8]) -> Inbound {
    Inbound::Undecodable {
        topic: topic_name.to_string(),
        error,
        payload: payload.to_vec(),
    }
}

/// A bounded, printable rendering of a payload nobody could decode, for the operator log.
///
/// Bounded because the record must not be a way to flood the log, and lossy because the
/// bytes are by definition not trustworthy UTF-8.
pub fn payload_excerpt(payload: &[u8], max_bytes: usize) -> String {
    let head = &payload[..payload.len().min(max_bytes)];
    let text = String::from_utf8_lossy(head).escape_debug().to_string();
    if payload.len() > max_bytes {
        format!("{text}… ({} bytes total)", payload.len())
    } else {
        text
    }
}
