//! Wire identities: the device, the reader, and the idempotency key.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdError {
    #[error("{0:?} is not a 12-nibble base MAC address")]
    NotAMac(String),
    #[error("{0:?} is not a canonical device id (expected esp32-<12 hex>)")]
    NotADeviceId(String),
    #[error("reader id must not be empty")]
    EmptyReaderId,
}

/// The prefix that makes a device id self-describing in logs and topics (CLAUDE.md 7.3).
const DEVICE_PREFIX: &str = "esp32-";
const MAC_NIBBLES: usize = 12;

/// An edge collector, identified by its ESP32 base MAC (CLAUDE.md 7.3).
///
/// A random UUID would be re-issued on a reflash and would silently split one device's
/// history in two, so identity is derived from the hardware and nothing else.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeviceId(String);

impl DeviceId {
    /// Accepts the separator styles a technician is likely to read off a label
    /// (`A4:CF:12:8B:3D:91`, `a4-cf-12-8b-3d-91`, `a4cf128b3d91`) and normalises them all
    /// to one canonical id, so the same box is never two devices.
    pub fn from_mac(mac: &str) -> Result<Self, IdError> {
        let hex: String = mac
            .chars()
            .filter(|c| !matches!(c, ':' | '-' | '.' | ' '))
            .collect();
        if hex.len() != MAC_NIBBLES || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(IdError::NotAMac(mac.to_string()));
        }
        Ok(Self(format!("{DEVICE_PREFIX}{}", hex.to_ascii_lowercase())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The MAC nibbles without the prefix, for display next to hardware labels.
    pub fn mac_hex(&self) -> &str {
        &self.0[DEVICE_PREFIX.len()..]
    }
}

impl FromStr for DeviceId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, IdError> {
        let rest = s
            .strip_prefix(DEVICE_PREFIX)
            .ok_or_else(|| IdError::NotADeviceId(s.to_string()))?;
        Self::from_mac(rest).map_err(|_| IdError::NotADeviceId(s.to_string()))
    }
}

impl TryFrom<String> for DeviceId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, IdError> {
        s.parse()
    }
}

impl From<DeviceId> for String {
    fn from(d: DeviceId) -> String {
        d.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({})", self.0)
    }
}

/// A reader attached to a device. Deliberately separate from `DeviceId`: one ESP32 may
/// carry several readers (CLAUDE.md 7.3), and the reader is what the hub maps to a
/// station and an event role (CLAUDE.md 8).
///
/// Case is folded because CLAUDE.md writes both `RFID-02` and `rfid-02`; two spellings
/// must not become two rows in the reader map.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ReaderId(String);

impl ReaderId {
    pub fn new(id: &str) -> Result<Self, IdError> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(IdError::EmptyReaderId);
        }
        Ok(Self(trimmed.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ReaderId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, IdError> {
        Self::new(s)
    }
}

impl TryFrom<String> for ReaderId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, IdError> {
        Self::new(&s)
    }
}

impl From<ReaderId> for String {
    fn from(r: ReaderId) -> String {
        r.0
    }
}

impl fmt::Display for ReaderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ReaderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReaderId({})", self.0)
    }
}

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
        Self { device_id, boot_id, sequence }
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
