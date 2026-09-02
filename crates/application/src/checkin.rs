//! Check-in: putting a member on the roster and a band on their wrist (ADR 0001 D3).
//!
//! `/checkin` is the narrow write surface -- it may bind tags and nothing else, so a
//! check-in tablet handed to a helper can never touch timing.

use crate::ingest::attribute_read;
use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, HubStore};
use domain::{AthleteState, EntryCode, Instant, Interpreted, MemberRef, TagId};

/// Somebody to put on the roster (ADR 0010).
///
/// A competition takes entries from people the gym has never seen, so a member reference is
/// **provenance, not a precondition**. An athlete is identified by `athlete_id`; whether
/// 健身管 knows them is recorded and never checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entrant {
    /// `None` for a walk-in. When present it is also the athlete id, so a member keeps one
    /// identity across every class they ever enter.
    pub member_id: Option<String>,
    pub display_name: String,
    /// `None` takes the next free number. Competition bibs are printed in advance, so the
    /// door has to be able to name one.
    pub bib: Option<i64>,
}

impl Entrant {
    pub fn walk_in(display_name: impl Into<String>) -> Self {
        Self { member_id: None, display_name: display_name.into(), bib: None }
    }

    /// Membership status is deliberately not read. Confirmed with the user 2026-08-27: if
    /// 健身管 returns the member they may be timed, and a gate here would stop somebody's
    /// clock over a billing detail (CLAUDE.md 31).
    pub fn member(member: &MemberRef) -> Self {
        Self {
            member_id: Some(member.member_id.clone()),
            display_name: member.display_name.clone(),
            bib: None,
        }
    }

    pub fn with_bib(mut self, bib: i64) -> Self {
        self.bib = Some(bib);
        self
    }
}

/// Puts one person on this session's roster and returns their athlete id.
///
/// Idempotent for a member: a door tablet's double tap is one roster line, and the same id
/// comes back so the helper is not told a different number the second time.
pub async fn enter<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    entrant: Entrant,
    cmd: &OperatorCommand,
) -> Result<String, OperatorError<S::Error>> {
    let name = entrant.display_name.trim();
    if name.is_empty() {
        return Err(OperatorError::NameRequired);
    }

    if let Some(member_id) = &entrant.member_id {
        if state.athlete(member_id).is_some() {
            return Ok(member_id.clone());
        }
    }

    let bib = match entrant.bib {
        Some(asked) => {
            if state.bibs().any(|b| b == asked) {
                return Err(OperatorError::BibTaken(asked));
            }
            asked
        }
        // The next free one, stepping over anything already handed out at the door.
        None => (1..).find(|n| !state.bibs().any(|b| b == *n)).expect("a free bib exists"),
    };

    // A member keeps one identity across every class. A walk-in is issued an entry code:
    // six characters that are their athlete id, the number on the QR they carry, and the
    // number they type afterwards to find their result (ADR 0011). One value for all three,
    // so nothing can drift out of step.
    let athlete_id = match entrant.member_id.clone() {
        Some(member_id) => member_id,
        None => free_entry_code(state).to_string(),
    };

    store
        .save_athlete(&state.session.id, &athlete_id, name, bib, entrant.member_id.as_deref())
        .await
        .map_err(OperatorError::Storage)?;
    state.athletes.push(AthleteState::ready(&athlete_id, name));
    state.note_bib(&athlete_id, bib);

    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "ATHLETE_ENTER".to_string(),
            subject: athlete_id.clone(),
            reason: cmd.stated_reason().map(str::to_string),
            before: None,
            after: Some(match &entrant.member_id {
                Some(m) => format!("{name:?} bib {bib}, member {m}"),
                None => format!("{name:?} bib {bib}, walk-in"),
            }),
        })
        .await
        .map_err(OperatorError::Storage)?;
    Ok(athlete_id)
}

/// A code nobody on this roster is already using.
///
/// The randomness is `RandomState`, which the standard library seeds from the OS. No crate
/// is needed for it, and the domain stays pure: it only knows how to turn a number into six
/// characters (CLAUDE.md 29).
///
/// Six characters out of a 32-character alphabet is a billion codes; the retry is here for
/// correctness rather than because a collision is expected, and it gives up rather than
/// looping forever on a session that somehow holds them all.
fn free_entry_code(state: &LiveSession) -> EntryCode {
    use std::hash::{BuildHasher, Hasher, RandomState};
    for _ in 0..64 {
        let code = EntryCode::encode(RandomState::new().build_hasher().finish());
        if state.athlete(code.as_str()).is_none() {
            return code;
        }
    }
    // Every attempt collided, which means the roster is impossibly large or the OS entropy
    // is stuck. Falling back to the bib keeps the door open instead of refusing an entrant.
    EntryCode::encode(state.athletes.len() as u64)
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
