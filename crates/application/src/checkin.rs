//! Check-in: putting a member on the roster and a band on their wrist (ADR 0001 D3).
//!
//! `/checkin` is the narrow write surface -- it may bind tags and nothing else, so a
//! check-in tablet handed to a helper can never touch timing.

use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, HubStore};
use domain::{AthleteState, MemberRef, TagId};

/// Adds a member to this session's roster.
///
/// Membership status is carried for display and is deliberately not checked: confirmed with
/// the user on 2026-08-27, if 健身管 returns the member they may be timed. A gate here would
/// stop someone's clock over a billing detail (CLAUDE.md 31).
pub async fn admit<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    member: &MemberRef,
) -> Result<(), OperatorError<S::Error>> {
    if state.athlete(&member.member_id).is_some() {
        return Ok(()); // the tablet double-tapped; one person, one roster line
    }
    let bib = state.athletes.len() as i64 + 1;
    store
        .save_athlete(&state.session.id, &member.member_id, &member.display_name, bib)
        .await
        .map_err(OperatorError::Storage)?;
    state
        .athletes
        .push(AthleteState::ready(&member.member_id, &member.display_name));
    Ok(())
}

/// Binds a tag to an athlete, clearing it from the pending list if it was read before
/// anyone claimed it (ADR 0001 D3).
///
/// Reads that arrived while the tag was unbound are still in the raw store; re-interpreting
/// them retroactively is not done here (see the crate docs).
pub async fn bind_tag<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    tag: &TagId,
    athlete_id: &str,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    if state.athlete(athlete_id).is_none() {
        return Err(OperatorError::UnknownAthlete(athlete_id.to_string()));
    }
    state
        .bindings
        .bind(&state.session.id, tag, athlete_id, cmd.at)
        .map_err(OperatorError::Binding)?;
    state.clear_pending_tag(tag);

    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "TAG_BIND".to_string(),
            subject: tag.to_string(),
            reason: cmd.stated_reason().map(str::to_string),
            before: None,
            after: Some(athlete_id.to_string()),
        })
        .await
        .map_err(OperatorError::Storage)?;
    Ok(())
}
