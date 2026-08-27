//! Reader registry: `(device_id, reader_id) -> station / zone / mode` (CLAUDE.md 8).
//!
//! The ESP32 publishes hardware identity only. Every business meaning is attached here,
//! which is what lets the venue layout be rewired without touching firmware. Reader
//! layout is still an open issue (CLAUDE.md 28), so the mapping is data, not code.

use crate::athlete::{ReaderBinding, ReaderMode};
use crate::device::{DeviceId, DeviceIdError, ReaderId, ReaderIdError};
use serde::Serialize;

/// The pair the ESP32 sends. Both halves are required: the same `reader_id` on two boards
/// is two different readers (CLAUDE.md 7.3).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize)]
pub struct ReaderKey {
    pub device_id: DeviceId,
    pub reader_id: ReaderId,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReaderKeyError {
    Device(DeviceIdError),
    Reader(ReaderIdError),
}

impl ReaderKey {
    pub fn new(device_id: DeviceId, reader_id: ReaderId) -> Self {
        Self { device_id, reader_id }
    }

    /// Parse both halves straight off an incoming MQTT payload (CLAUDE.md 16).
    pub fn parse(device_id: &str, reader_id: &str) -> Result<Self, ReaderKeyError> {
        Ok(Self {
            device_id: DeviceId::parse(device_id).map_err(ReaderKeyError::Device)?,
            reader_id: ReaderId::parse(reader_id).map_err(ReaderKeyError::Reader)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReaderRegistration {
    pub key: ReaderKey,
    pub station: String,
    /// Optional: the venue zone layout is unresolved (CLAUDE.md 28), and a reader is
    /// usable without one.
    pub zone: Option<String>,
    /// The event role from CLAUDE.md 8 -- ENTRY/EXIT/TOGGLE/CHECKPOINT/PASSAGE.
    pub mode: ReaderMode,
}

impl ReaderRegistration {
    pub fn new(key: ReaderKey, station: impl Into<String>, mode: ReaderMode) -> Self {
        Self { key, station: station.into(), zone: None, mode }
    }

    pub fn with_zone(mut self, zone: impl Into<String>) -> Self {
        self.zone = Some(zone.into());
        self
    }

    /// The slice of the mapping the interpreter needs (CLAUDE.md 10).
    pub fn binding(&self) -> ReaderBinding {
        ReaderBinding { station: self.station.clone(), mode: self.mode }
    }
}

/// A read arrived from hardware the hub has no mapping for. Returned, never panicked:
/// an unregistered reader is an operator exception, not a crash (ADR 0001 D4).
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct UnknownReader {
    pub device_id: DeviceId,
    pub reader_id: ReaderId,
}

/// Reader configuration. A Vec rather than a map: a venue has tens of readers, lookup is
/// not hot, and insertion order is what the operator screen wants to show.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ReaderRegistry {
    registrations: Vec<ReaderRegistration>,
}

impl ReaderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or reconfigure a reader. Returns the mapping that was displaced, if any,
    /// so a re-registration is visible to the caller instead of silently overwriting.
    pub fn register(&mut self, registration: ReaderRegistration) -> Option<ReaderRegistration> {
        match self.registrations.iter_mut().find(|r| r.key == registration.key) {
            Some(existing) => Some(std::mem::replace(existing, registration)),
            None => {
                self.registrations.push(registration);
                None
            }
        }
    }

    pub fn resolve(&self, key: &ReaderKey) -> Result<&ReaderRegistration, UnknownReader> {
        self.registrations.iter().find(|r| &r.key == key).ok_or_else(|| UnknownReader {
            device_id: key.device_id.clone(),
            reader_id: key.reader_id.clone(),
        })
    }

    /// Every reader configured on one board, for device health screens (CLAUDE.md 23).
    pub fn readers_on<'a>(
        &'a self,
        device_id: &'a DeviceId,
    ) -> impl Iterator<Item = &'a ReaderRegistration> {
        self.registrations.iter().filter(move |r| &r.key.device_id == device_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ReaderRegistration> {
        self.registrations.iter()
    }

    pub fn len(&self) -> usize {
        self.registrations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.registrations.is_empty()
    }
}
