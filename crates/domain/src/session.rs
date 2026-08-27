//! Session lifecycle. Three states only (docs/decisions/0001-ui-operation-rules.md D2):
//! per-athlete READY/ACTIVE/FINISHED lives on AthleteState, not duplicated here.

use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SessionMode {
    Competition,
    Training,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SessionStatus {
    Draft,
    Armed,
    Closed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionError {
    /// ARMED -> DRAFT is only legal while nothing has been interpreted yet (D2).
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
    /// Gates ARMED -> DRAFT. Incremented by the engine, never by the UI.
    pub interpreted_event_count: u64,
}

impl Session {
    pub fn new_draft(id: impl Into<String>, name: impl Into<String>, mode: SessionMode) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            mode,
            status: SessionStatus::Draft,
            interpreted_event_count: 0,
        }
    }

    pub fn accepts_events(&self) -> bool {
        self.status == SessionStatus::Armed
    }

    pub fn arm(&mut self) -> Result<(), SessionError> {
        match self.status {
            // CLOSED -> ARMED is deliberately allowed: a mis-tap on a busy gym floor must not
            // force a new session. The audit record carries the reason (D2).
            SessionStatus::Draft | SessionStatus::Closed => {
                self.status = SessionStatus::Armed;
                Ok(())
            }
            SessionStatus::Armed => Ok(()),
        }
    }

    pub fn close(&mut self) -> Result<(), SessionError> {
        match self.status {
            SessionStatus::Armed | SessionStatus::Closed => {
                self.status = SessionStatus::Closed;
                Ok(())
            }
            SessionStatus::Draft => Err(SessionError::IllegalTransition {
                from: SessionStatus::Draft,
                to: SessionStatus::Closed,
            }),
        }
    }

    pub fn back_to_draft(&mut self) -> Result<(), SessionError> {
        if self.status != SessionStatus::Armed {
            return Err(SessionError::IllegalTransition {
                from: self.status,
                to: SessionStatus::Draft,
            });
        }
        if self.interpreted_event_count > 0 {
            return Err(SessionError::HasInterpretedEvents);
        }
        self.status = SessionStatus::Draft;
        Ok(())
    }
}
