//! Per-session configuration: the plan and the policies (CLAUDE.md 9, 12).
//!
//! Kept beside `Session` rather than inside it. `Session` is the lifecycle record the
//! storage layer reads and writes row-by-row (DRAFT/ARMED/CLOSED); configuration is a
//! nested document with a different edit cadence, and folding it in would also change
//! `Session`'s construction shape for every existing caller.
//!
//! Round-trips through serde because a resumed session must come back with the rule it was
//! armed under, never the rule the caller happened to supply (CLAUDE.md 21; ADR 0004).

use crate::course::Course;
use crate::finish::FinishPolicy;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionConfig {
    pub session_id: String,
    /// A plan, never a constraint: training accepts any actual order (CLAUDE.md 9.2).
    /// None for a drop-in session with no course at all.
    pub course: Option<Course>,
    /// Unresolved product rule, isolated behind configuration per CLAUDE.md 28.
    pub finish_policy: FinishPolicy,
}

impl SessionConfig {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            course: None,
            finish_policy: FinishPolicy::default(),
        }
    }

    pub fn with_course(mut self, course: Course) -> Self {
        self.course = Some(course);
        self
    }

    pub fn with_finish_policy(mut self, policy: FinishPolicy) -> Self {
        self.finish_policy = policy;
        self
    }
}
