//! Everything one running session needs in memory, in one place.
//!
//! The hub is a modular monolith with a single active session (CLAUDE.md 4), so this is an
//! aggregate rather than a repository per part. Nothing here is authoritative: session,
//! roster and interpreted events all come back from the store on restart (CLAUDE.md 21),
//! and this struct is the working copy the use cases advance.

use crate::devices::DeviceHealth;
use domain::{
    AthleteState, BindingLedger, ClassClock, DeviceId, Instant, ReaderRegistry, Session,
    SessionConfig, TagId,
};

pub struct LiveSession {
    pub session: Session,
    /// The plan and the policies. Never a constraint in training (CLAUDE.md 9.2).
    pub config: SessionConfig,
    /// `(device_id, reader_id) -> station / mode` (CLAUDE.md 8).
    pub readers: ReaderRegistry,
    /// Tag ↔ athlete (CLAUDE.md 7.2; ADR 0001 D3).
    pub bindings: BindingLedger,
    pub athletes: Vec<AthleteState>,
    /// When the class clock started. The left-hand side of every class-duration decision.
    pub class_start: Instant,
    /// Tags read by a reader that belong to nobody yet. Not errors: they are the work list
    /// for `/checkin` (ADR 0001 D3), which is why they are kept apart from the exception
    /// count below.
    pending_tags: Vec<TagId>,
    /// How many interpreted events were exceptions, for the operator's inbox badge (D4).
    pub exception_count: usize,
    /// When each edge device was last heard from (ADR 0001 D5). In memory only, and
    /// deliberately so -- see [`crate::devices`].
    devices: Vec<DeviceHealth>,
}

impl LiveSession {
    pub fn new(session: Session, config: SessionConfig, class_start: Instant) -> Self {
        Self {
            session,
            config,
            readers: ReaderRegistry::new(),
            bindings: BindingLedger::new(),
            athletes: Vec::new(),
            class_start,
            pending_tags: Vec::new(),
            exception_count: 0,
            devices: Vec::new(),
        }
    }

    pub fn with_athletes(mut self, athletes: Vec<AthleteState>) -> Self {
        self.athletes = athletes;
        self
    }

    pub fn with_readers(mut self, readers: ReaderRegistry) -> Self {
        self.readers = readers;
        self
    }

    pub fn with_bindings(mut self, bindings: BindingLedger) -> Self {
        self.bindings = bindings;
        self
    }

    pub fn athlete(&self, athlete_id: &str) -> Option<&AthleteState> {
        self.athletes.iter().find(|a| a.athlete_id == athlete_id)
    }

    pub fn athlete_mut(&mut self, athlete_id: &str) -> Option<&mut AthleteState> {
        self.athletes.iter_mut().find(|a| a.athlete_id == athlete_id)
    }

    /// How long the class has been running, which is what a duration-based finish rule
    /// compares against (CLAUDE.md 12). Frozen while the session is paused (ADR 0008).
    pub fn class_elapsed(&self, now: Instant) -> domain::Duration {
        self.class_clock().elapsed(now)
    }

    /// The class clock: the stored origin plus the session's own pause accounting. Derived
    /// rather than stored, so there is one place a pause can be recorded -- the session.
    pub fn class_clock(&self) -> ClassClock {
        self.session.clock(self.class_start)
    }

    /// Tags waiting to be claimed on `/checkin`, oldest first.
    pub fn pending_tags(&self) -> &[TagId] {
        &self.pending_tags
    }

    /// Remember an unbound tag. Idempotent: a band scanned five times is one line on the
    /// check-in screen, not five.
    pub fn note_pending_tag(&mut self, tag: TagId) {
        if !self.pending_tags.contains(&tag) {
            self.pending_tags.push(tag);
        }
    }

    pub fn clear_pending_tag(&mut self, tag: &TagId) {
        self.pending_tags.retain(|t| t != tag);
    }

    /// Edge devices the hub has heard from since it started, in the order it first heard
    /// them. A device missing from here has said nothing this run (ADR 0001 D5).
    pub fn devices(&self) -> &[DeviceHealth] {
        &self.devices
    }

    pub fn device(&self, device_id: &DeviceId) -> Option<&DeviceHealth> {
        self.devices.iter().find(|d| &d.device_id == device_id)
    }

    pub(crate) fn device_mut(&mut self, device_id: &DeviceId) -> Option<&mut DeviceHealth> {
        self.devices.iter_mut().find(|d| &d.device_id == device_id)
    }

    pub(crate) fn push_device(&mut self, health: DeviceHealth) {
        self.devices.push(health);
    }
}
