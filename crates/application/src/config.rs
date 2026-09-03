//! Editing a session's configuration (ADR 0001 D2).
//!
//! Configuration is the course and the policies: what the class is meant to do, and what
//! ends it. It may only be edited while the session is DRAFT, which is the whole reason a
//! resumed session can trust what it reads back (ADR 0004) -- a running class must not have
//! its finish rule changed underneath it.
//!
//! Nothing here validates the *content* of a course. Training records what actually happens
//! and must not warn on a different order (CLAUDE.md 9.2), so a plan is a plan: repeated
//! stations, missing targets and unfamiliar station names are all legal.

use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, HubStore};
use domain::{Course, FinishPolicy, SessionConfig};

/// Replaces the live session's course and finish rule, in the store first and then in
/// memory.
///
/// The session id comes from the session, never from the caller: an operator screen must
/// not be able to write a configuration onto some other session by naming it.
pub async fn configure<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    course: Option<Course>,
    finish_policy: FinishPolicy,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    if !state.session.accepts_config_edits() {
        return Err(OperatorError::NotEditable {
            status: state.session.status,
        });
    }

    let mut config = SessionConfig::new(&state.session.id);
    config.course = course;
    config.finish_policy = finish_policy;

    store
        .save_session_config(&config)
        .await
        .map_err(OperatorError::Storage)?;
    let before = describe(&state.config);
    let after = describe(&config);
    state.config = config;

    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "SESSION_CONFIGURE".to_string(),
            subject: state.session.id.clone(),
            reason: cmd.stated_reason().map(str::to_string),
            before: Some(before),
            after: Some(after),
        })
        .await
        .map_err(OperatorError::Storage)?;
    Ok(())
}

/// A one-line summary for the audit trail. The full course would be a JSON document in an
/// audit column that nobody reads; what an operator needs later is "which plan, how long,
/// which rule" (CLAUDE.md 20).
fn describe(config: &SessionConfig) -> String {
    match &config.course {
        Some(course) => format!(
            "{:?}, course {:?} ({} steps)",
            config.finish_policy,
            course.name,
            course.len()
        ),
        None => format!("{:?}, no course", config.finish_policy),
    }
}
