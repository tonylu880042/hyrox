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
use crate::ports::{HubStore, InterpretedWrite, RawInterpretationContext, RawRead, StoredRawRead};
use contract::{Ack, AckStatus, CommitOutcome, EdgeEvent, EventStore, ReceivedEvent, WireError};
use domain::{ExceptionReason, Instant, Interpreted, ReaderBinding, ReaderKey, TagId};
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
    Interpreted {
        athlete_id: String,
        event: Interpreted,
    },
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
    // Finish any earlier raw/interpretation handoff before deciding a later read. This keeps
    // an EXIT from being planned against the pre-ENTRY state after a transient write failure.
    let reconciled_pending = reconcile_pending(state, store)
        .await
        .map_err(IngestError::Storage)?;
    let contexts = plan_contexts(state, received.event());
    let sink = RawSink {
        store,
        raw_event_ids: Mutex::new(Vec::new()),
        raw_outcomes: Mutex::new(Vec::new()),
        contexts: Mutex::new(contexts.clone()),
    };
    // Routed through `contract::ingest` on purpose: it is the only place an `Ack` can be
    // minted, and only on the line after a successful commit (ADR 0002). Doing the commit here
    // and building an ACK beside it would reopen exactly the hole that ADR closed.
    let ack = contract::ingest(&sink, received)
        .await
        .map_err(|e| match e {
            contract::IngestError::Storage(e) => IngestError::Storage(e),
            contract::IngestError::Malformed(w) => IngestError::Malformed(w),
        })?;
    let raw_event_ids = sink
        .raw_event_ids
        .into_inner()
        .expect("the sink is not poisoned");
    let raw_outcomes = sink
        .raw_outcomes
        .into_inner()
        .expect("the sink is not poisoned");

    if ack.payload().status == AckStatus::Duplicate {
        let mut all_interpreted = true;
        for &id in &raw_event_ids {
            if !store
                .raw_event_has_interpretation(id)
                .await
                .map_err(IngestError::Storage)?
            {
                all_interpreted = false;
                break;
            }
        }
        if all_interpreted {
            if !reconciled_pending.is_empty() {
                // This redelivery was the trigger that completed a durable handoff. Report the
                // recovered interpretation once to the caller; a later redelivery, with no
                // recovery work left, takes the ordinary Duplicate path below.
                let mut recovered = Vec::new();
                for &id in &raw_event_ids {
                    if !reconciled_pending.contains(&id) {
                        continue;
                    }
                    let Some(context) = store
                        .raw_interpretation_context(id)
                        .await
                        .map_err(IngestError::Storage)?
                    else {
                        continue;
                    };
                    if let (Some(athlete_id), Some(event)) = (context.athlete_id, context.planned) {
                        recovered.push(IngestOutcome::Interpreted { athlete_id, event });
                    }
                }
                if !recovered.is_empty() {
                    return Ok(Ingested {
                        ack,
                        outcomes: recovered,
                    });
                }
            }
            return Ok(Ingested {
                ack,
                outcomes: vec![IngestOutcome::Duplicate],
            });
        }
    }

    // A redelivery must use the context stored with the original raw row. Re-planning it from
    // today's reader/binding map would turn an ACKed pending row into a different event after
    // a reconfiguration or rebind. A missing context is a legacy row, so it remains explicit
    // and unattributable rather than being guessed into history.
    let edge = received.event();
    // Official timing, always (CLAUDE.md 11, 17). Every tag in a round was in the field at
    // the same instant, so they share it.
    // Each tag is interpreted on its own: they are different people, and one of them being
    // unbound says nothing about the next. `raw_event_ids` is in the order the tags were
    // committed, which is the order they arrived in.
    let mut outcomes = Vec::with_capacity(edge.tag_id.len());
    for (((raw_tag, raw_event_id), fresh_context), outcome) in edge
        .tag_id
        .iter()
        .zip(raw_event_ids)
        .zip(contexts)
        .zip(raw_outcomes)
    {
        if store
            .raw_event_has_interpretation(raw_event_id)
            .await
            .map_err(IngestError::Storage)?
        {
            outcomes.push(IngestOutcome::Duplicate);
            continue;
        }
        let context = match outcome {
            CommitOutcome::Stored => fresh_context,
            CommitOutcome::AlreadyStored => {
                let Some(context) = store
                    .raw_interpretation_context(raw_event_id)
                    .await
                    .map_err(IngestError::Storage)?
                else {
                    outcomes.push(IngestOutcome::Unattributable);
                    continue;
                };
                context
            }
        };
        let Ok(tag) = TagId::parse(raw_tag) else {
            outcomes.push(IngestOutcome::Unattributable);
            continue;
        };

        let Some(athlete_id) = context.athlete_id.clone() else {
            state.note_pending_tag(tag.clone());
            outcomes.push(IngestOutcome::PendingBinding { tag_id: tag });
            continue;
        };

        // The event was decided before raw commit and stored with it. Reusing this exact
        // decision is what makes a retry safe after a reader reconfiguration or a rebind.
        let Some(event) = context.planned.clone() else {
            // A context with an owner but no plan is a legacy row. Keep the raw read visible
            // and require an explicit operator correction instead of guessing a historical
            // station from today's map.
            outcomes.push(IngestOutcome::Unattributable);
            continue;
        };
        let session_id = state.session.id.clone();
        match commit_and_apply(
            state,
            store,
            &session_id,
            &athlete_id,
            Some(raw_event_id),
            &event,
        )
        .await
        {
            Ok(()) => {}
            Err(source) => return Err(IngestError::Interpretation { ack, source }),
        }
        outcomes.push(IngestOutcome::Interpreted { athlete_id, event });
    }

    if let Err(source) = store.save_session(&state.session, state.class_start).await {
        return Err(IngestError::Interpretation { ack, source });
    }
    state.session_save_dirty = false;

    Ok(Ingested { ack, outcomes })
}

/// The context of a read ready to be attributed to an athlete.
pub(crate) struct ReadContext<'a> {
    pub athlete_id: &'a str,
    pub on_roster: bool,
    pub device_id: &'a str,
    pub reader_id: &'a str,
    pub raw_event_id: Option<i64>,
    pub at: Instant,
    pub presence: domain::Presence,
    /// A persisted reader snapshot used by retroactive claims. Live reads leave this `None`
    /// and resolve the current venue map once while deciding the event.
    /// `None` means resolve the current venue map (a live read or a legacy row). `Some(None)`
    /// means the original context explicitly saw an unknown reader and must not be re-resolved
    /// after a later reader registration.
    pub reader: Option<Option<ReaderBinding>>,
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
    ctx: ReadContext<'_>,
) -> Result<Interpreted, S::Error> {
    // Reader resolution (CLAUDE.md 8). A pair that parses but is not registered, and a pair
    // that does not even parse, are the same thing to an operator: hardware the hub has no
    // mapping for.
    let reader = ctx.reader.clone().unwrap_or_else(|| {
        ReaderKey::parse(ctx.device_id, ctx.reader_id)
            .ok()
            .and_then(|key| state.readers.resolve(&key).ok().map(|r| r.binding()))
    });

    let event = match (ctx.on_roster, reader) {
        (false, _) => Interpreted::Exception {
            reason: ExceptionReason::AthleteNotInSession,
            at: ctx.at,
        },
        // The reader is unknown but the athlete is not: attribute the exception to them, so
        // it shows up against the person the operator has to talk to.
        (true, None) => Interpreted::Exception {
            reason: ExceptionReason::UnknownReader,
            at: ctx.at,
        },
        (true, Some(binding)) => {
            let athlete = state.athlete(ctx.athlete_id).expect("checked by on_roster");
            // A live event older than the last event already folded for this athlete cannot be
            // safely inserted into the current state without re-deciding history. Preserve it
            // as an explicit exception; the raw row remains evidence and replay will make the
            // same decision from the stored context.
            if athlete.last_event_at.is_some_and(|last| ctx.at < last) {
                Interpreted::Exception {
                    reason: domain::ExceptionReason::ImpossibleTransition,
                    at: ctx.at,
                }
            } else {
                domain::decide_read(athlete, &binding, ctx.presence, ctx.at, &state.session)
            }
        }
    };

    let session_id = state.session.id.clone();
    commit_and_apply(
        state,
        store,
        &session_id,
        ctx.athlete_id,
        ctx.raw_event_id,
        &event,
    )
    .await?;
    Ok(event)
}

/// Commits an already-decided event and folds it into the live aggregate. Deciding before the
/// write is important for recovery; applying only after the write keeps memory and the log in
/// sync when a store fails.
async fn commit_and_apply<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    session_id: &str,
    athlete_id: &str,
    raw_event_id: Option<i64>,
    event: &Interpreted,
) -> Result<(), S::Error> {
    store
        .commit_interpreted(InterpretedWrite {
            session_id,
            athlete_id,
            raw_event_id,
            event,
        })
        .await?;

    match event {
        // Exceptions are recorded but are not progress: the count gates ARMED -> DRAFT
        // (ADR 0001 D2).
        Interpreted::Exception { .. } => state.exception_count += 1,
        _ => state.session.interpreted_event_count += 1,
    }
    if let Some(athlete) = state.athlete_mut(athlete_id) {
        domain::apply(athlete, event);
    }
    state.session_save_dirty = true;
    Ok(())
}

/// Rebuilds derived athlete state and the exception badge after a durable correction. The dirty
/// bit is deliberately cleared only after every read succeeds, so a transient failure can be
/// retried by the next tick or operator boundary without trying to void/reinterpret twice.
pub(crate) async fn rebuild_projection<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
) -> Result<(), S::Error> {
    // Read the complete projection before changing any live field. A correction is already
    // durable when this runs, so a later read failure must leave the old projection marked
    // dirty and retryable rather than exposing a half-rebuilt aggregate.
    let mut athletes = store.rebuild_athletes(&state.session.id).await?;
    let exception_count = store.exception_count(&state.session.id).await?;
    let interpreted_event_count = store.interpreted_event_count(&state.session.id).await?;
    crate::finish::apply_course_completion(
        &mut athletes,
        state.config.finish_policy,
        state.config.course.as_ref(),
    );

    state.athletes = athletes;
    state.exception_count = exception_count;
    if state.session.interpreted_event_count != interpreted_event_count {
        state.session.interpreted_event_count = interpreted_event_count;
        state.session_save_dirty = true;
    }
    state.projection_dirty = false;
    Ok(())
}

/// Computes the immutable context for every tag before the raw rows are written. A temporary
/// athlete vector lets two tags in one UHF round for the same athlete be decided in arrival
/// order without mutating the live state before any interpretation is durable.
fn plan_contexts(state: &LiveSession, edge: &EdgeEvent) -> Vec<RawInterpretationContext> {
    let at = Instant(edge.detected_at);
    let mut planning_athletes = state.athletes.clone();
    edge.tag_id
        .iter()
        .map(|raw_tag| {
            let reader = ReaderKey::parse(edge.device_id.as_str(), edge.reader_id.as_str())
                .ok()
                .and_then(|key| state.readers.resolve(&key).ok().map(|r| r.binding()));
            let Ok(tag) = TagId::parse(raw_tag) else {
                return RawInterpretationContext {
                    session_id: state.session.id.clone(),
                    athlete_id: None,
                    bound_session_id: None,
                    on_roster: false,
                    reader,
                    planned: None,
                };
            };
            // Tag resolution across every session is timestamp-aware. A read before a rebind
            // belongs to the old owner; a read before the first bind stays pending for check-in.
            let holder = state
                .bindings
                .binding_at(&tag, at)
                .map(|b| (b.session_id.clone(), b.athlete_id.clone()));
            let (athlete_id, bound_session_id, on_roster, planned) = match holder.as_ref() {
                None => (None, None, false, None),
                Some((bound_session_id, athlete_id)) => {
                    let on_roster = bound_session_id == &state.session.id
                        && planning_athletes
                            .iter()
                            .any(|a| a.athlete_id == *athlete_id);
                    let planned = if !on_roster {
                        Some(Interpreted::Exception {
                            reason: ExceptionReason::AthleteNotInSession,
                            at,
                        })
                    } else {
                        let athlete = planning_athletes
                            .iter()
                            .find(|a| a.athlete_id == *athlete_id)
                            .expect("on_roster was checked above");
                        let event = match reader.as_ref() {
                            Some(_) if athlete.last_event_at.is_some_and(|last| at < last) => {
                                Interpreted::Exception {
                                    reason: ExceptionReason::ImpossibleTransition,
                                    at,
                                }
                            }
                            Some(reader) => domain::decide_read(
                                athlete,
                                reader,
                                edge.presence,
                                at,
                                &state.session,
                            ),
                            None => Interpreted::Exception {
                                reason: ExceptionReason::UnknownReader,
                                at,
                            },
                        };
                        if let Some(athlete) = planning_athletes
                            .iter_mut()
                            .find(|a| a.athlete_id == *athlete_id)
                        {
                            domain::apply(athlete, &event);
                        }
                        Some(event)
                    };
                    (
                        Some(athlete_id.clone()),
                        Some(bound_session_id.clone()),
                        on_roster,
                        planned,
                    )
                }
            };
            RawInterpretationContext {
                session_id: state.session.id.clone(),
                athlete_id,
                bound_session_id,
                on_roster,
                reader,
                planned,
            }
        })
        .collect()
}

/// Completes raw rows whose interpretation write was interrupted after the ACK-worthy commit.
/// Only precomputed plans are replayed; unbound rows remain available for a later check-in.
pub async fn reconcile_pending<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
) -> Result<Vec<i64>, S::Error> {
    if state.projection_dirty {
        rebuild_projection(state, store).await?;
    }
    let pending = store.pending_raw_reads(&state.session.id).await?;
    let mut repaired = Vec::new();
    for StoredRawRead {
        raw_event_id,
        device_id,
        reader_id,
        detected_at,
        presence,
        context,
    } in pending
    {
        let Some(context) = context else { continue };
        let Some(athlete_id) = context.athlete_id else {
            continue;
        };
        if let Some(event) = context.planned.clone() {
            commit_and_apply(
                state,
                store,
                &context.session_id,
                &athlete_id,
                Some(raw_event_id),
                &event,
            )
            .await?;
        } else if context.bound_session_id.as_deref() == Some(state.session.id.as_str())
            && context.on_roster
        {
            // A binding transaction can leave a durable claim intent whose plan is still None.
            // Decide it now against the recovered projection, then commit the same way as a
            // live/retroactive read. Unowned legacy rows remain deliberately untouched.
            attribute_read(
                state,
                store,
                ReadContext {
                    athlete_id: &athlete_id,
                    on_roster: true,
                    device_id: &device_id,
                    reader_id: &reader_id,
                    raw_event_id: Some(raw_event_id),
                    at: detected_at,
                    presence,
                    reader: Some(context.reader.clone()),
                },
            )
            .await?;
        } else {
            continue;
        }
        repaired.push(raw_event_id);
    }
    if !repaired.is_empty() || state.session_save_dirty {
        store
            .save_session(&state.session, state.class_start)
            .await?;
        state.session_save_dirty = false;
    }
    Ok(repaired)
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
    raw_outcomes: Mutex<Vec<CommitOutcome>>,
    contexts: Mutex<Vec<RawInterpretationContext>>,
}

impl<S: HubStore> EventStore for RawSink<'_, S> {
    type Error = S::Error;

    /// Commits every tag in the round. Returning `Ok` -- and so earning the ACK -- means all
    /// of them are durable; a failure part way through leaves the earlier rows stored and no
    /// ACK, and the resend finds them by their key (CLAUDE.md 16).
    async fn commit(&self, event: &ReceivedEvent) -> Result<CommitOutcome, S::Error> {
        let edge = event.event();
        let contexts = self
            .contexts
            .lock()
            .expect("the sink is not poisoned")
            .clone();
        let mut ids = Vec::with_capacity(edge.tag_id.len());
        // `AlreadyStored` only when the *whole* round was already here. A partially stored
        // round is a resend of one the hub never finished, so it still has work to do.
        let mut every_tag_already_stored = true;
        for (tag, context) in edge.tag_id.iter().zip(contexts.iter()) {
            let committed = self
                .store
                .commit_raw_with_context(
                    &RawRead {
                        device_id: edge.device_id.as_str().to_string(),
                        reader_id: edge.reader_id.as_str().to_string(),
                        boot_id: edge.boot_id,
                        sequence: edge.sequence,
                        tag_id: tag.clone(),
                        detected_at: Instant(edge.detected_at),
                        received_at: Instant(event.received_at()),
                        presence: edge.presence,
                    },
                    context,
                )
                .await?;
            if committed.outcome == CommitOutcome::Stored {
                every_tag_already_stored = false;
            }
            ids.push(committed.raw_event_id);
            self.raw_outcomes
                .lock()
                .expect("the sink is not poisoned")
                .push(committed.outcome);
        }
        *self.raw_event_ids.lock().expect("the sink is not poisoned") = ids;
        Ok(if every_tag_already_stored {
            CommitOutcome::AlreadyStored
        } else {
            CommitOutcome::Stored
        })
    }
}
