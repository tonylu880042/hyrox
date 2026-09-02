//! The ingestion use case (CLAUDE.md 8, 11, 15, 16, 19).
//!
//! One inventory round walks the whole pipeline here. UHF anti-collision means a round may
//! carry several tags (ADR 0014): the round is the unit of delivery -- one message, one
//! idempotency key, one ACK -- and each tag in it is a read of its own from there on.
//!
//! ```text
//! raw edge event (1..n tags)
//!   -> commit raw            (durable, idempotent; earns the ACK -- ADR 0002)
//!   -> resolve the reader    through ReaderRegistry   -> UNKNOWN_READER exception
//!   -> resolve the tag       through BindingLedger    -> pending binding, or a roster exception
//!   -> domain::decide + apply
//!   -> commit the interpretation
//! ```
//!
//! Every tag in the round is committed before the ACK is minted, so a round is released
//! all-or-nothing: a failure part way through leaves the earlier tags stored, no ACK, and a
//! resend that finds them already there (CLAUDE.md 15, 16).
//!
//! No branch drops the event (CLAUDE.md 31 principle 1). A read the hub cannot make sense
//! of still ends up in the raw store, and either in the operator's exception inbox or on
//! the check-in list (ADR 0001 D3, D4).

use crate::live_session::LiveSession;
use crate::ports::{HubStore, InterpretedWrite, RawRead};
use contract::{Ack, AckStatus, CommitOutcome, EventStore, ReceivedEvent, WireError};
use domain::{ExceptionReason, Instant, Interpreted, ReaderKey, TagId};
use std::sync::Mutex;

/// What one committed read turned out to mean.
#[derive(Clone, Debug)]
pub enum IngestOutcome {
    /// The same `device_id + boot_id + sequence` was already stored, so it was already
    /// interpreted. Acknowledged again -- duplicate delivery is allowed, duplicate business
    /// processing is not (CLAUDE.md 16).
    Duplicate,
    /// Attributed to an athlete. `event` may be an `Exception`: those are interpretations
    /// too, and they belong to someone (ADR 0001 D4).
    Interpreted { athlete_id: String, event: Interpreted },
    /// The band belongs to nobody yet. Not an error: it goes to `/checkin`, and the raw
    /// read is kept so it can be claimed retroactively once the tag is bound (ADR 0001 D3).
    PendingBinding { tag_id: TagId },
    /// The tag id itself is unusable, so the read cannot even be listed for check-in. It is
    /// still durable; nothing else can be said about it without inventing a tag.
    Unattributable,
}

/// A committed round and the ACK it earned.
#[derive(Debug)]
pub struct Ingested {
    /// Proof the whole round is durable. Publishing it releases the edge's copy
    /// (CLAUDE.md 15).
    pub ack: Ack,
    /// One outcome per tag in the round, in the order the reader reported them. A
    /// redelivery yields a single [`IngestOutcome::Duplicate`]: the round was interpreted
    /// the first time, and doing it again is exactly what CLAUDE.md 16 forbids.
    pub outcomes: Vec<IngestOutcome>,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError<E> {
    #[error("rejected before storage: {0}")]
    Malformed(WireError),
    /// The read is **not** durable. No ACK exists, so the edge keeps it and resends
    /// (CLAUDE.md 15; ADR 0002).
    #[error("raw event was not committed")]
    Storage(E),
    /// The raw read IS durable and the ACK is earned, but a write after it failed. The ACK
    /// is handed back so the edge is still released: the read itself is safe in the raw
    /// store, which is the guarantee that matters (CLAUDE.md 31 principle 1). The
    /// interpretation is missing and must be re-derived or added by an operator.
    #[error("raw event committed, but the interpretation was not stored")]
    Interpretation { ack: Ack, source: E },
}

/// Commits a raw read, then interprets it. Never returns without the read being either
/// durable or reported as lost.
pub async fn ingest_read<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    received: &ReceivedEvent,
) -> Result<Ingested, IngestError<S::Error>> {
    let sink = RawSink { store, raw_event_ids: Mutex::new(Vec::new()) };
    // Routed through `contract::ingest` on purpose: it is the only place an `Ack` can be
    // minted, and only on the line after a successful commit (ADR 0002). Doing the commit here
    // and building an ACK beside it would reopen exactly the hole that ADR closed.
    let ack = contract::ingest(&sink, received).await.map_err(|e| match e {
        contract::IngestError::Storage(e) => IngestError::Storage(e),
        contract::IngestError::Malformed(w) => IngestError::Malformed(w),
    })?;
    if ack.payload().status == AckStatus::Duplicate {
        return Ok(Ingested { ack, outcomes: vec![IngestOutcome::Duplicate] });
    }
    let raw_event_ids = sink.raw_event_ids.into_inner().expect("the sink is not poisoned");

    let edge = received.event();
    // Official timing, always (CLAUDE.md 11, 17). Every tag in a round was in the field at
    // the same instant, so they share it.
    let at = Instant(edge.detected_at);

    // Each tag is interpreted on its own: they are different people, and one of them being
    // unbound says nothing about the next. `raw_event_ids` is in the order the tags were
    // committed, which is the order they arrived in.
    let mut outcomes = Vec::with_capacity(edge.tag_id.len());
    for (raw_tag, raw_event_id) in edge.tag_id.iter().zip(raw_event_ids) {
        let Ok(tag) = TagId::parse(raw_tag) else {
            outcomes.push(IngestOutcome::Unattributable);
            continue;
        };

        // Tag resolution across every session, not just this one: one band is on one wrist
        // (ADR 0001 D3), so a tag held elsewhere is "someone else's", not "unbound".
        let holder = state
            .bindings
            .active()
            .find(|b| b.tag_id == tag)
            .map(|b| (b.session_id.clone(), b.athlete_id.clone()));
        let Some((bound_session, athlete_id)) = holder else {
            state.note_pending_tag(tag.clone());
            outcomes.push(IngestOutcome::PendingBinding { tag_id: tag });
            continue;
        };

        let on_roster = bound_session == state.session.id && state.athlete(&athlete_id).is_some();
        let event = match attribute_read(
            state,
            store,
            &athlete_id,
            on_roster,
            edge.device_id.as_str(),
            edge.reader_id.as_str(),
            Some(raw_event_id),
            at,
        )
        .await
        {
            Ok(event) => event,
            Err(source) => return Err(IngestError::Interpretation { ack, source }),
        };
        outcomes.push(IngestOutcome::Interpreted { athlete_id, event });
    }

    if let Err(source) = store.save_session(&state.session, state.class_start).await {
        return Err(IngestError::Interpretation { ack, source });
    }

    Ok(Ingested { ack, outcomes })
}

/// Interprets one read for a known athlete and folds it in -- but only after the store has
/// taken it.
///
/// The write comes between `domain::decide` and `domain::apply` deliberately. Doing both
/// halves first (what `domain::interpret` does) advanced the in-memory athlete before the
/// event log knew about it, so a failed write left memory claiming a station the log had
/// never recorded, and the two disagreed until the next restart. Deciding is pure, so
/// nothing is lost by deciding early; applying is what must wait (CLAUDE.md 21, 29).
///
/// Shared with the retroactive claim in [`crate::checkin`] so a read claimed after the fact
/// goes through exactly the same resolution and the same folding as one interpreted live
/// (ADR 0001 D3).
pub(crate) async fn attribute_read<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    athlete_id: &str,
    on_roster: bool,
    device_id: &str,
    reader_id: &str,
    raw_event_id: Option<i64>,
    at: Instant,
) -> Result<Interpreted, S::Error> {
    // Reader resolution (CLAUDE.md 8). A pair that parses but is not registered, and a pair
    // that does not even parse, are the same thing to an operator: hardware the hub has no
    // mapping for.
    let reader = ReaderKey::parse(device_id, reader_id)
        .ok()
        .and_then(|key| state.readers.resolve(&key).ok().map(|r| r.binding()));

    let event = match (on_roster, reader) {
        (false, _) => Interpreted::Exception { reason: ExceptionReason::AthleteNotInSession, at },
        // The reader is unknown but the athlete is not: attribute the exception to them, so
        // it shows up against the person the operator has to talk to.
        (true, None) => Interpreted::Exception { reason: ExceptionReason::UnknownReader, at },
        (true, Some(binding)) => {
            let athlete = state.athlete(athlete_id).expect("checked by on_roster");
            domain::decide(athlete, &binding, at, &state.session)
        }
    };

    store
        .commit_interpreted(InterpretedWrite {
            session_id: &state.session.id,
            athlete_id,
            raw_event_id,
            event: &event,
        })
        .await?;

    match event {
        // Exceptions are recorded but are not progress: the count gates ARMED -> DRAFT
        // (ADR 0001 D2).
        Interpreted::Exception { .. } => state.exception_count += 1,
        _ => state.session.interpreted_event_count += 1,
    }
    if let Some(athlete) = state.athlete_mut(athlete_id) {
        domain::apply(athlete, &event);
    }
    Ok(event)
}

/// Adapts the hub's store to `contract::EventStore` so the ACK keeps its type-level guarantee,
/// while still surfacing the row ids the interpretations have to be linked to.
///
/// The ids are stashed here rather than returned because `EventStore::commit` is the
/// contract ADR 0002 froze, and widening it would put ACK minting back in every adapter's
/// hands. Written and read within one task, either side of a single await.
struct RawSink<'a, S: HubStore> {
    store: &'a S,
    raw_event_ids: Mutex<Vec<i64>>,
}

impl<S: HubStore> EventStore for RawSink<'_, S> {
    type Error = S::Error;

    /// Commits every tag in the round. Returning `Ok` -- and so earning the ACK -- means all
    /// of them are durable; a failure part way through leaves the earlier rows stored and no
    /// ACK, and the resend finds them by their key (CLAUDE.md 16).
    async fn commit(&self, event: &ReceivedEvent) -> Result<CommitOutcome, S::Error> {
        let edge = event.event();
        let mut ids = Vec::with_capacity(edge.tag_id.len());
        // `AlreadyStored` only when the *whole* round was already here. A partially stored
        // round is a resend of one the hub never finished, so it still has work to do.
        let mut every_tag_already_stored = true;
        for tag in &edge.tag_id {
            let committed = self
                .store
                .commit_raw(&RawRead {
                    device_id: edge.device_id.as_str().to_string(),
                    reader_id: edge.reader_id.as_str().to_string(),
                    boot_id: edge.boot_id,
                    sequence: edge.sequence,
                    tag_id: tag.clone(),
                    detected_at: Instant(edge.detected_at),
                    received_at: Instant(event.received_at()),
                })
                .await?;
            if committed.outcome == CommitOutcome::Stored {
                every_tag_already_stored = false;
            }
            ids.push(committed.raw_event_id);
        }
        *self.raw_event_ids.lock().expect("the sink is not poisoned") = ids;
        Ok(if every_tag_already_stored {
            CommitOutcome::AlreadyStored
        } else {
            CommitOutcome::Stored
        })
    }
}
