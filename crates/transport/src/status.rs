//! Edge health published on the status topic (CLAUDE.md 18).
//!
//! A device whose journal is filling up is about to start losing RFID events, which is the
//! one failure the system may never have (CLAUDE.md 31). It must be able to say so before
//! it happens, and the hub must be able to surface it to an operator.

use contract::WireError;
use domain::DeviceId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStatus {
    pub device_id: DeviceId,
    pub boot_id: i64,
    /// Events written but not yet acknowledged by the hub.
    pub pending_events: u64,
    pub journal_capacity: u64,
    /// `None` while the device is healthy.
    pub warning: Option<DeviceWarning>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeviceWarning {
    /// Past the configured warning threshold: still recording, but an operator should look.
    JournalNearlyFull,
    /// No reclaimable space left. The next RF read cannot be journalled.
    JournalFull,
}

impl DeviceStatus {
    pub fn decode(payload: &[u8]) -> Result<Self, WireError> {
        serde_json::from_slice(payload).map_err(|e| WireError::Malformed(e.to_string()))
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("DeviceStatus is always serialisable")
    }
}
