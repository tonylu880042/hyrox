//! Edge device liveness and health (CLAUDE.md 18, 23; ADR 0001 D5).
//!
//! D5 is a safety rule, not a decoration: without per-reader freshness a quiet operator
//! screen means both "nobody is running" and "that collector has been offline for ten
//! minutes", and the venue cannot tell which. The hub therefore remembers when it last heard
//! from each device, and what that device last said about itself.
//!
//! Deliberately in memory only. Freshness is a statement about *now*, and a hub that has
//! just restarted genuinely does not know when it last heard from a device -- reloading a
//! stored `last_seen` from the previous run would present an old fact as a current one,
//! which is exactly the confusion D5 exists to prevent. After a restart every device is
//! simply unknown until it speaks.

use crate::live_session::LiveSession;
use domain::{DeviceId, DeviceWarning, Instant};

/// What a device last reported on the status topic (CLAUDE.md 18).
///
/// An application-level type rather than `transport::DeviceStatus`: the use cases must not
/// depend on the delivery mechanism (CLAUDE.md 3; ADR 0005), and this is the same facts
/// without the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceReport {
    pub device_id: DeviceId,
    pub boot_id: i64,
    pub pending_events: u64,
    pub journal_capacity: u64,
    /// The device's own assessment. The hub shows it and never derives one.
    pub warning: Option<DeviceWarning>,
}

/// What the hub knows about one edge device right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceHealth {
    pub device_id: DeviceId,
    /// When the hub last heard anything at all from this device -- a read or a status.
    pub last_seen: Instant,
    /// `None` until the device has published a status; a device can be heard from through
    /// its reads alone, and half a fact is better than a fabricated one.
    pub report: Option<DeviceReport>,
}

impl DeviceHealth {
    pub fn warning(&self) -> Option<DeviceWarning> {
        self.report.as_ref().and_then(|r| r.warning)
    }
}

/// Records that a device was heard from at `at`, without any health detail.
///
/// Called for every arriving read. `at` is supplied by the caller rather than taken from a
/// clock in here, because the only clock the application may use is the one the composition
/// root owns -- and because official timing never comes from this path (CLAUDE.md 17).
pub fn note_device_seen(state: &mut LiveSession, device_id: &DeviceId, at: Instant) {
    match state.device_mut(device_id) {
        Some(known) => known.last_seen = at,
        None => state.push_device(DeviceHealth {
            device_id: device_id.clone(),
            last_seen: at,
            report: None,
        }),
    }
}

/// Records a status message: liveness, plus what the device said about its journal.
pub fn note_device_status(state: &mut LiveSession, report: DeviceReport, at: Instant) {
    match state.device_mut(&report.device_id) {
        Some(known) => {
            known.last_seen = at;
            known.report = Some(report);
        }
        None => state.push_device(DeviceHealth {
            device_id: report.device_id.clone(),
            last_seen: at,
            report: Some(report),
        }),
    }
}
