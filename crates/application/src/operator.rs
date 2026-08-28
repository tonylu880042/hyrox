//! What every operator-initiated command carries, and how it can fail.
//!
//! There is no login (ADR 0001 D1): the identity is the device's own name, taken once when
//! the tablet first opens `/operator` or `/checkin`. Destructive actions still need a
//! reason, offered as quick reason keys rather than free typing, so the audit trail
//! required by CLAUDE.md 20 survives a fast gym floor.

use domain::{BindingError, Instant, SessionError, SessionStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorCommand {
    /// The device name, which is the audit `operator` (ADR 0001 D1). Traceability is to a
    /// device, not a person -- a trade accepted deliberately for zero friction.
    pub operator: String,
    /// Required for anything that changes recorded data.
    pub reason: Option<String>,
    pub at: Instant,
}

impl OperatorCommand {
    pub fn new(operator: impl Into<String>, at: Instant) -> Self {
        Self { operator: operator.into(), reason: None, at }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// A reason that is present and not just whitespace. A blank string would satisfy the
    /// type and tell a later reader nothing.
    pub(crate) fn stated_reason(&self) -> Option<&str> {
        self.reason.as_deref().map(str::trim).filter(|r| !r.is_empty())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OperatorError<E> {
    #[error("session transition rejected: {0:?}")]
    Session(SessionError),
    #[error("binding rejected: {0:?}")]
    Binding(BindingError),
    /// The action changes recorded data, so CLAUDE.md 20 requires a reason for the audit
    /// trail. Reopening a closed session is the common case (ADR 0001 D2).
    #[error("this action needs a reason")]
    ReasonRequired,
    /// Ending a class by hand is only meaningful where a finish rule exists. Competition's
    /// rule is undecided (CLAUDE.md 12, 28) and must not be invented by a button.
    #[error("no finish rule is configured for this session")]
    NoFinishRule,
    #[error("athlete {0:?} is not in this session")]
    UnknownAthlete(String),
    /// Configuration may only be edited while the session is DRAFT (ADR 0001 D2). Not a
    /// `SessionError`: nothing was asked of the state machine, so it has nothing to say.
    #[error("configuration cannot be edited while the session is {status:?}")]
    NotEditable { status: SessionStatus },
    /// No interpreted event has that id, so there was nothing to correct. Reported rather
    /// than silently succeeding: an operator who voided nothing must not be told they did.
    #[error("no interpreted event with id {0}")]
    UnknownEvent(i64),
    #[error("store write failed")]
    Storage(E),
}
