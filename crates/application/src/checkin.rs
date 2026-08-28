//! Check-in: putting a member on the roster and a band on their wrist (ADR 0001 D3).
//!
//! `/checkin` is the narrow write surface -- it may bind tags and nothing else, so a
//! check-in tablet handed to a helper can never touch timing.

use crate::ingest::attribute_read;
use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, HubStore};
use domain::{AthleteState, Instant, Interpreted, MemberRef, TagId};

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

/// Binds a tag to an athlete, clears it from the pending list, and claims the reads that
/// happened before anyone owned the band (ADR 0001 D3).
///
/// Returns the interpretations the claim produced, oldest first. Empty is the normal case:
/// a band handed out before the class starts has nothing to claim.
pub async fn bind_tag<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    tag: &TagId,
    athlete_id: &str,
    cmd: &OperatorCommand,
) -> Result<Vec<Interpreted>, OperatorError<S::Error>> {
    if state.athlete(athlete_id).is_none() {
        return Err(OperatorError::UnknownAthlete(athlete_id.to_string()));
    }
    state
        .bindings
        .bind(&state.session.id, tag, athlete_id, cmd.at)
        .map_err(OperatorError::Binding)?;
    state.clear_pending_tag(tag);
    persist_bindings(state, store).await?;

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

    claim_reads(state, store, tag, athlete_id).await
}

/// Moves an athlete onto a different band: the old binding is closed, a new one opened, and
/// both are audited (ADR 0001 D3). The closed row stays, in memory and in the store.
///
/// Reads already claimed under the old band keep their interpretation. Only reads nothing
/// points at yet are claimed for the new one, so a swap cannot double-count a station.
pub async fn rebind_tag<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    new_tag: &TagId,
    athlete_id: &str,
    cmd: &OperatorCommand,
) -> Result<Vec<Interpreted>, OperatorError<S::Error>> {
    if state.athlete(athlete_id).is_none() {
        return Err(OperatorError::UnknownAthlete(athlete_id.to_string()));
    }
    // Changing which band carries someone's results is a correction, so CLAUDE.md 20 wants
    // a reason on the record.
    if cmd.stated_reason().is_none() {
        return Err(OperatorError::ReasonRequired);
    }
    let previous = state
        .bindings
        .tag_for_athlete(&state.session.id, athlete_id)
        .map(|t| t.to_string());
    state
        .bindings
        .rebind_athlete(&state.session.id, athlete_id, new_tag, cmd.at)
        .map_err(OperatorError::Binding)?;
    state.clear_pending_tag(new_tag);
    persist_bindings(state, store).await?;

    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "TAG_REBIND".to_string(),
            subject: athlete_id.to_string(),
            reason: cmd.stated_reason().map(str::to_string),
            before: previous,
            after: Some(new_tag.to_string()),
        })
        .await
        .map_err(OperatorError::Storage)?;

    claim_reads(state, store, new_tag, athlete_id).await
}

/// Interprets the stored reads of a tag that no interpretation points at yet, in
/// `detected_at` order (ADR 0001 D3).
///
/// Ordering is the whole rule: replaying entry-before-exit through the same folding the
/// live path uses is what makes a late binding indistinguishable from an early one. The
/// store only returns reads nothing has claimed, so calling this twice claims nothing twice.
async fn claim_reads<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    tag: &TagId,
    athlete_id: &str,
) -> Result<Vec<Interpreted>, OperatorError<S::Error>> {
    let reads = store
        .unclaimed_reads_for_tag(tag.as_str(), state.class_start)
        .await
        .map_err(OperatorError::Storage)?;
    if reads.is_empty() {
        return Ok(Vec::new());
    }

    let mut claimed = Vec::with_capacity(reads.len());
    for read in &reads {
        let event = attribute_read(
            state,
            store,
            athlete_id,
            true, // checked by the caller before the binding was made
            &read.device_id,
            &read.reader_id,
            Some(read.raw_event_id),
            read.detected_at,
        )
        .await
        .map_err(OperatorError::Storage)?;
        claimed.push(event);
    }
    store
        .save_session(&state.session, state.class_start)
        .await
        .map_err(OperatorError::Storage)?;
    Ok(claimed)
}

/// Writes the ledger back, closed rows included.
///
/// Every row rather than the one that changed: the port's write is an upsert that may only
/// stamp `unbound_at`, a class-sized ledger is tens of rows, and this way the stored ledger
/// cannot drift from the domain one whatever combination of bind, unbind and rebind ran.
async fn persist_bindings<S: HubStore>(
    state: &LiveSession,
    store: &S,
) -> Result<(), OperatorError<S::Error>> {
    for binding in state.bindings.history() {
        store.save_binding(binding).await.map_err(OperatorError::Storage)?;
    }
    Ok(())
}

/// Rebuilds the check-in queue after a restart: tags a reader has seen since the class
/// started that still belong to nobody (ADR 0001 D3).
///
/// Derived from the raw store rather than remembered, so a crash cannot lose the queue.
pub(crate) async fn pending_tags_since<S: HubStore>(
    store: &S,
    bindings: &domain::BindingLedger,
    since: Instant,
) -> Result<Vec<TagId>, S::Error> {
    let seen = store.raw_tags_since(since).await?;
    Ok(seen
        .iter()
        // An unusable tag id is not a check-in to-do: there is nothing to put on the list.
        .filter_map(|raw| TagId::parse(raw).ok())
        .filter(|tag| !bindings.active().any(|b| &b.tag_id == tag))
        .collect())
}
