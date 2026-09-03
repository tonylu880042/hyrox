//! Session lifecycle (ADR 0001 D2; ADR 0008).
//!
//! Six states, one per thing an operator can meaningfully do to a class:
//!
//! ```text
//! DRAFT -> READY -> RUNNING <-> PAUSED
//!                      |          |
//!                      +----------+--> COMPLETED
//!
//! DRAFT | READY | RUNNING | PAUSED --> CANCELLED
//! ```
//!
//! RUNNING is what earlier builds spelled ARMED and COMPLETED is what they spelled CLOSED;
//! migration 0004 rewrites the stored values. The split of DRAFT into DRAFT + READY is what
//! lets a class be built from a template and then tweaked for today without being open to
//! edits once it is timing anybody (ADR 0008).
//!
//! Per-athlete READY/ACTIVE/FINISHED lives on `AthleteState`, not here.

use crate::time::{ClassClock, Duration, Instant};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SessionMode {
    Competition,
    Training,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SessionStatus {
    /// Being built. The course and the policies are open to edits.
    Draft,
    /// Built, not started. Still editable -- this is where today's tweaks happen.
    Ready,
    /// Timing. The only state that accepts reader events.
    Running,
    /// Stopped without ending. The class clock is frozen (see [`ClassClock`]).
    Paused,
    Completed,
    Cancelled,
}

impl SessionStatus {
    pub fn name(self) -> &'static str {
        match self {
            SessionStatus::Draft => "DRAFT",
            SessionStatus::Ready => "READY",
            SessionStatus::Running => "RUNNING",
            SessionStatus::Paused => "PAUSED",
            SessionStatus::Completed => "COMPLETED",
            SessionStatus::Cancelled => "CANCELLED",
        }
    }

    /// Whether the class is over. A finished class is never restarted by an ordinary
    /// transition; reopening a completed one is a correction (see [`Session::reopen`]).
    pub fn is_terminal(self) -> bool {
        matches!(self, SessionStatus::Completed | SessionStatus::Cancelled)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionError {
    /// RUNNING -> DRAFT is only legal while nothing has been interpreted yet (D2).
    HasInterpretedEvents,
    IllegalTransition {
        from: SessionStatus,
        to: SessionStatus,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub mode: SessionMode,
    pub status: SessionStatus,
    /// Gates RUNNING -> DRAFT. Incremented by the engine, never by the UI.
    pub interpreted_event_count: u64,
    /// Wall time this class has spent paused, excluding an open pause (ADR 0008).
    pub paused_total: Duration,
    /// When the open pause began, if it is paused right now. Persisted, so a hub that
    /// restarts mid-pause comes back paused rather than silently resuming (CLAUDE.md 21).
    pub paused_since: Option<Instant>,
}

impl Session {
    pub fn new_draft(id: impl Into<String>, name: impl Into<String>, mode: SessionMode) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            mode,
            status: SessionStatus::Draft,
            interpreted_event_count: 0,
            paused_total: Duration(0),
            paused_since: None,
        }
    }

    /// The class clock, given the moment the class began (the session row's `created_at`).
    ///
    /// Composed rather than stored whole: the origin belongs to the row that created the
    /// session, the pause accounting belongs to the lifecycle, and keeping them apart means
    /// `new_draft` does not have to invent a time it does not know yet.
    pub fn clock(&self, started_at: Instant) -> ClassClock {
        ClassClock {
            started_at,
            paused_total: self.paused_total,
            paused_since: self.paused_since,
        }
    }

    /// Only RUNNING. A paused class is not timing anybody, so a read arriving during a pause
    /// is an exception rather than a split (ADR 0008) -- it is recorded, never dropped.
    pub fn accepts_events(&self) -> bool {
        self.status == SessionStatus::Running
    }

    /// Whether the course and the finish rule may still be changed (D2).
    ///
    /// DRAFT and READY. Once a class is running it keeps the rule it started under, which is
    /// what makes a resumed session trustworthy (ADR 0004) -- and it is stated here rather
    /// than restated in every caller, so an operator screen and the use case that refuses
    /// the edit are reading the same sentence.
    pub fn accepts_config_edits(&self) -> bool {
        matches!(self.status, SessionStatus::Draft | SessionStatus::Ready)
    }

    /// Whether a restart should pick this session back up (CLAUDE.md 21). A class that was
    /// paused when the hub died is still today's class.
    pub fn is_live(&self) -> bool {
        matches!(
            self.status,
            SessionStatus::Ready | SessionStatus::Running | SessionStatus::Paused
        )
    }

    /// DRAFT -> READY. The class is built; from here it can be started.
    pub fn mark_ready(&mut self) -> Result<(), SessionError> {
        match self.status {
            SessionStatus::Draft | SessionStatus::Ready => {
                self.status = SessionStatus::Ready;
                Ok(())
            }
            from => Err(SessionError::illegal(from, SessionStatus::Ready)),
        }
    }

    /// READY -> RUNNING. From here the first valid read starts each athlete's clock
    /// (CLAUDE.md 11).
    ///
    /// Refuses a DRAFT session: a class that was never marked ready has not been looked at,
    /// and refuses a terminal one -- bringing a completed class back is [`Session::reopen`],
    /// which the application layer will not run without a stated reason.
    pub fn start(&mut self) -> Result<(), SessionError> {
        match self.status {
            SessionStatus::Ready | SessionStatus::Running => {
                self.status = SessionStatus::Running;
                Ok(())
            }
            from => Err(SessionError::illegal(from, SessionStatus::Running)),
        }
    }

    pub fn pause(&mut self, at: Instant) -> Result<(), SessionError> {
        match self.status {
            SessionStatus::Running | SessionStatus::Paused => {
                let mut clock = self.clock(at);
                clock.pause(at);
                self.absorb(clock);
                self.status = SessionStatus::Paused;
                Ok(())
            }
            from => Err(SessionError::illegal(from, SessionStatus::Paused)),
        }
    }

    pub fn resume(&mut self, at: Instant) -> Result<(), SessionError> {
        match self.status {
            SessionStatus::Paused | SessionStatus::Running => {
                let mut clock = self.clock(at);
                clock.resume(at);
                self.absorb(clock);
                self.status = SessionStatus::Running;
                Ok(())
            }
            from => Err(SessionError::illegal(from, SessionStatus::Running)),
        }
    }

    /// RUNNING or PAUSED -> COMPLETED. A paused class need not be resumed to be ended: the
    /// coach who paused it is the one ending it.
    ///
    /// An open pause is left open on purpose. Closing it here would add the pause's wall
    /// time to `paused_total` and shift every derived instant, and the class is not running
    /// any more anyway -- the clock reads the same either way.
    pub fn complete(&mut self) -> Result<(), SessionError> {
        match self.status {
            SessionStatus::Running | SessionStatus::Paused | SessionStatus::Completed => {
                self.status = SessionStatus::Completed;
                Ok(())
            }
            from => Err(SessionError::illegal(from, SessionStatus::Completed)),
        }
    }

    /// Abandoned rather than finished. Legal from every live state, because the reason a
    /// class is cancelled is rarely known in advance.
    pub fn cancel(&mut self) -> Result<(), SessionError> {
        match self.status {
            SessionStatus::Completed => {
                Err(SessionError::illegal(self.status, SessionStatus::Cancelled))
            }
            _ => {
                self.status = SessionStatus::Cancelled;
                Ok(())
            }
        }
    }

    /// COMPLETED -> RUNNING. A correction, not an ordinary transition (ADR 0001 D2): a
    /// mis-tap on a busy floor must not force a new session, and there is deliberately no
    /// time window on it -- a window would be a magic constant nobody validated.
    ///
    /// A CANCELLED session is not reopened. Cancelling says the class did not happen; the
    /// honest repair is a new session, not resurrecting one that was written off.
    pub fn reopen(&mut self) -> Result<(), SessionError> {
        match self.status {
            SessionStatus::Completed | SessionStatus::Running => {
                self.status = SessionStatus::Running;
                Ok(())
            }
            from => Err(SessionError::illegal(from, SessionStatus::Running)),
        }
    }

    /// Back to editing. Free from READY; from RUNNING only while nothing has been
    /// interpreted (D2), because a class with splits in it has a history to answer for.
    pub fn back_to_draft(&mut self) -> Result<(), SessionError> {
        match self.status {
            SessionStatus::Ready | SessionStatus::Draft => {
                self.status = SessionStatus::Draft;
                Ok(())
            }
            SessionStatus::Running | SessionStatus::Paused => {
                if self.interpreted_event_count > 0 {
                    return Err(SessionError::HasInterpretedEvents);
                }
                self.status = SessionStatus::Draft;
                Ok(())
            }
            from => Err(SessionError::illegal(from, SessionStatus::Draft)),
        }
    }

    fn absorb(&mut self, clock: ClassClock) {
        self.paused_total = clock.paused_total;
        self.paused_since = clock.paused_since;
    }
}

impl SessionError {
    fn illegal(from: SessionStatus, to: SessionStatus) -> Self {
        SessionError::IllegalTransition { from, to }
    }
}
