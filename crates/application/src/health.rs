//! Whether the hub may be stopped right now (ADR 0009 §6).
//!
//! The appliance runs a nightly maintenance window that updates the machine and powers it
//! off. It asks this first, and does nothing on a `false`. Getting this wrong means a class
//! ending mid-station because a timer fired at 23:30, so the question is answered from the
//! hub's own state rather than guessed at from uptime or a quiet period.
//!
//! Derived on each call, stored nowhere. Both inputs already exist: the session's lifecycle
//! and the edge devices' own journal reports.

use crate::live_session::LiveSession;
use domain::Instant;
use serde::Serialize;

/// A reason the hub should not be stopped. Listed rather than collapsed into a boolean, so
/// an operator screen and a maintenance log can both say *what* is in the way.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Blocker {
    /// The class is READY, RUNNING or PAUSED. A paused class is a coffee break, not an
    /// ending, and a READY one is a coach about to press start.
    ClassRunning,
    /// An edge device still reports unacknowledged events in its journal (CLAUDE.md 18).
    ///
    /// Stopping now is survivable -- the journal exists for exactly this, and the events
    /// arrive when the hub returns (CLAUDE.md 15, 21). But there is no reason to do it
    /// deliberately when tomorrow will do.
    DeviceBacklog,
}

#[derive(Clone, Debug, Serialize)]
pub struct Health {
    /// The running binary's version, for answering "what is on that machine" without
    /// a terminal.
    pub version: String,
    pub session_status: String,
    pub class_live: bool,
    pub devices_with_backlog: usize,
    /// How stale the newest event is. `null` means none exists yet, never zero.
    pub last_event_age_ms: Option<i64>,
    /// The one field the maintenance script reads.
    pub safe_to_stop: bool,
    /// Every reason, not just the first: fixing one must not hide the other.
    pub blocked_by: Vec<Blocker>,
}

pub fn health(state: &LiveSession, now: Instant) -> Health {
    health_with_version(state, now, env!("CARGO_PKG_VERSION"))
}

/// The composition root supplies the shipped binary's version; this crate's own would be a
/// different number the moment the workspace stops moving in lockstep.
pub fn health_with_version(state: &LiveSession, now: Instant, version: &str) -> Health {
    let class_live = state.session.is_live();
    let devices_with_backlog = state
        .devices()
        .iter()
        .filter(|d| d.report.as_ref().is_some_and(|r| r.pending_events > 0))
        .count();

    let mut blocked_by = Vec::new();
    if class_live {
        blocked_by.push(Blocker::ClassRunning);
    }
    if devices_with_backlog > 0 {
        blocked_by.push(Blocker::DeviceBacklog);
    }

    Health {
        version: version.to_string(),
        session_status: state.session.status.name().to_string(),
        class_live,
        devices_with_backlog,
        last_event_age_ms: crate::live::last_event_age_ms(state, now),
        safe_to_stop: blocked_by.is_empty(),
        blocked_by,
    }
}
