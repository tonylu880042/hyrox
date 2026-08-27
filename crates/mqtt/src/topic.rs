//! Topic scheme.
//!
//! Two branches, so a subscription can never confuse the two directions:
//!
//! ```text
//! hyrox/v1/edge/<device>/events   edge → hub   RFID events        QoS 1
//! hyrox/v1/edge/<device>/status   edge → hub   health / warnings  (CLAUDE.md 18)
//! hyrox/v1/hub/<device>/ack       hub  → edge  application ACK    (CLAUDE.md 15)
//! hyrox/v1/hub/time               hub  → edge  time sync          (CLAUDE.md 17)
//! ```
//!
//! The `v1` segment exists so a contract change is a new topic rather than a silent
//! reinterpretation of the old one (CLAUDE.md 30).

use crate::DeviceId;

const ROOT: &str = "hyrox/v1";
const EDGE: &str = "hyrox/v1/edge";
const HUB: &str = "hyrox/v1/hub";

const EVENTS_LEAF: &str = "events";
const STATUS_LEAF: &str = "status";
const ACK_LEAF: &str = "ack";

/// Every device's events, in one subscription.
pub const ALL_EVENTS: &str = "hyrox/v1/edge/+/events";
/// Every device's health, in one subscription.
pub const ALL_STATUS: &str = "hyrox/v1/edge/+/status";
/// The hub is the local time authority and broadcasts to all devices (CLAUDE.md 17).
pub const TIME_SYNC: &str = "hyrox/v1/hub/time";

pub fn events(device: &DeviceId) -> String {
    format!("{EDGE}/{device}/{EVENTS_LEAF}")
}

pub fn status(device: &DeviceId) -> String {
    format!("{EDGE}/{device}/{STATUS_LEAF}")
}

pub fn ack(device: &DeviceId) -> String {
    format!("{HUB}/{device}/{ACK_LEAF}")
}

/// The device an arriving event topic belongs to, or `None` if the topic is not one of
/// ours. Returning `None` rather than guessing keeps a stray publisher from being
/// mistaken for a registered device.
pub fn device_of_events(topic: &str) -> Option<DeviceId> {
    device_of(topic, EVENTS_LEAF)
}

pub fn device_of_status(topic: &str) -> Option<DeviceId> {
    device_of(topic, STATUS_LEAF)
}

fn device_of(topic: &str, leaf: &str) -> Option<DeviceId> {
    let rest = topic.strip_prefix(EDGE)?.strip_prefix('/')?;
    let (device, tail) = rest.split_once('/')?;
    (tail == leaf).then(|| device.parse().ok())?
}

/// The namespace both directions share, for logging and for broker ACL configuration.
pub fn root() -> &'static str {
    ROOT
}
