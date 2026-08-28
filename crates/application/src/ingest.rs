//! The ingestion use case (CLAUDE.md 8, 11, 15, 16, 19).
//!
//! One raw read walks the whole pipeline here:
//!
//! ```text
//! raw edge event
//!   -> commit raw            (durable, idempotent; earns the ACK -- ADR 0002)
//!   -> resolve the reader    through ReaderRegistry   -> UNKNOWN_READER exception
//!   -> resolve the tag       through BindingLedger    -> pending binding, or a roster exception
//!   -> domain::decide + apply
//!   -> commit the interpretation
//! ```
//!
//! No branch drops the event (CLAUDE.md 31 principle 1). A read the hub cannot make sense
//! of still ends up in the raw store, and either in the operator's exception inbox or on
//! the check-in list (ADR 0001 D3, D4).

use crate::live_session::LiveSession;
use crate::ports::{HubStore, InterpretedWrite, RawRead};
use domain::{ExceptionReason, Instant, Interpreted, ReaderKey, TagId};
use mqtt::{Ack, AckStatus, CommitOutcome, EventStore, ReceivedEvent, WireError};
use std::sync::atomic::{AtomicI64, Ordering};

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

/// A committed read and the ACK it earned.
#[derive(Debug)]
pub struct Ingested {
    /// Proof the read is durable. Publishing it releases the edge's copy (CLAUDE.md 15).
    pub ack: Ack,
    pub outcome: IngestOutcome,
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
    let sink = RawSink { store, raw_event_id: AtomicI64::new(0) };
    // Routed through `mqtt::ingest` on purpose: it is the only place an `Ack` can be minted,
    // and only on the line after a successful commit (ADR 0002). Doing the commit here and
    // building an ACK beside it would reopen exactly the hole that ADR closed.
    let ack = mqtt::ingest(&sink, received).await.map_err(|e| match e {
        mqtt::IngestError::Storage(e) => IngestError::Storage(e),
        mqtt::IngestError::Malformed(w) => IngestError::Malformed(w),
    })?;
    if ack.payload().status == AckStatus::Duplicate {
        return Ok(Ingested { ack, outcome: IngestOutcome::Duplicate });
    }
    let raw_event_id = sink.raw_event_id.load(Ordering::Relaxed);

    let edge = received.event();
    // Official timing, always (CLAUDE.md 11, 17).
    let at = Instant(edge.detected_at);

    // Reader resolution (CLAUDE.md 8). A pair that parses but is not registered, and a pair
    // that does not even parse, are the same thing to an operator: hardware the hub has no
    // mapping for.
    let reader = ReaderKey::parse(edge.device_id.as_str(), edge.reader_id.as_str())
        .ok()
        .and_then(|key| state.readers.resolve(&key).ok().map(|r| r.binding()));

    let Ok(tag) = TagId::parse(&edge.tag_id) else {
        return Ok(Ingested { ack, outcome: IngestOutcome::Unattributable });
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
        return Ok(Ingested { ack, outcome: IngestOutcome::PendingBinding { tag_id: tag } });
    };

    let on_roster = bound_session == state.session.id && state.athlete(&athlete_id).is_some();
    let event = match (on_roster, reader) {
        (false, _) => Interpreted::Exception { reason: ExceptionReason::AthleteNotInSession, at },
        // The reader is unknown but the athlete is not: attribute the exception to them, so
        // it shows up against the person the operator has to talk to.
        (true, None) => Interpreted::Exception { reason: ExceptionReason::UnknownReader, at },
        (true, Some(binding)) => {
            // Cloned because `decide` reads the session while the athlete is borrowed
            // mutably. A Session is an id, a name and two small fields.
            let session = state.session.clone();
            let athlete = state.athlete_mut(&athlete_id).expect("checked by on_roster");
            domain::interpret(athlete, &binding, at, &session)
        }
    };

    match event {
        // Exceptions are recorded but are not progress: the count gates ARMED -> DRAFT
        // (ADR 0001 D2).
        Interpreted::Exception { .. } => state.exception_count += 1,
        _ => state.session.interpreted_event_count += 1,
    }

    let write = InterpretedWrite {
        session_id: &state.session.id,
        athlete_id: &athlete_id,
        raw_event_id: Some(raw_event_id),
        event: &event,
    };
    if let Err(source) = store.commit_interpreted(write).await {
        return Err(IngestError::Interpretation { ack, source });
    }
    if let Err(source) = store.save_session(&state.session, state.class_start).await {
        return Err(IngestError::Interpretation { ack, source });
    }

    Ok(Ingested { ack, outcome: IngestOutcome::Interpreted { athlete_id, event } })
}

/// Adapts the hub's store to `mqtt::EventStore` so the ACK keeps its type-level guarantee,
/// while still surfacing the row id the interpretation has to be linked to.
///
/// The id is stashed in an atomic rather than returned because `EventStore::commit` is the
/// contract ADR 0002 froze, and widening it would put ACK minting back in every adapter's
/// hands. Written and read within one task, either side of a single await.
struct RawSink<'a, S: HubStore> {
    store: &'a S,
    raw_event_id: AtomicI64,
}

impl<S: HubStore> EventStore for RawSink<'_, S> {
    type Error = S::Error;

    async fn commit(&self, event: &ReceivedEvent) -> Result<CommitOutcome, S::Error> {
        let edge = event.event();
        let committed = self
            .store
            .commit_raw(&RawRead {
                device_id: edge.device_id.as_str().to_string(),
                reader_id: edge.reader_id.as_str().to_string(),
                boot_id: edge.boot_id,
                sequence: edge.sequence,
                tag_id: edge.tag_id.clone(),
                detected_at: Instant(edge.detected_at),
                received_at: Instant(event.received_at()),
            })
            .await?;
        self.raw_event_id.store(committed.raw_event_id, Ordering::Relaxed);
        Ok(committed.outcome)
    }
}
