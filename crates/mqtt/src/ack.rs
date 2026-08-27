//! The application-level ACK protocol (CLAUDE.md 15).
//!
//! MQTT QoS 1 only proves a broker took the packet; it says nothing about the hub having
//! kept it. The edge must therefore hold every event until the hub says it is durable:
//!
//! ```text
//! receive → SQLite COMMIT → ACK → edge releases the event
//! ```
//!
//! The rule "do not ACK before persistent storage commit succeeds" is enforced by shape,
//! not by discipline. [`Ack`] has no public constructor; the only way to obtain one is
//! [`Commit::into_ack`], and a [`Commit`] is only minted inside [`ingest`] on the line
//! after [`EventStore::commit`] returned `Ok`. Code that wants to ACK early has nothing to
//! ACK with.

use crate::{DeviceId, EdgeEvent, EventId, ReceivedEvent, WireError};
use serde::{Deserialize, Serialize};

/// What the store did with an event it has now durably committed.
///
/// `AlreadyStored` is a success: duplicate delivery is allowed (CLAUDE.md 16), and it must
/// still be acknowledged or the edge would resend it for ever.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Stored,
    AlreadyStored,
}

/// The port the hub implements over its persistent store (CLAUDE.md 3, 15).
///
/// The contract is one sentence: **return `Ok` only after the durable commit succeeded.**
/// A store that buffers in memory and commits later must not return `Ok` yet.
///
/// The implementation must be idempotent on `device_id + boot_id + sequence`
/// (CLAUDE.md 16) — a redelivery reports `AlreadyStored` rather than writing a second row.
///
/// Deliberately not wired to `crates/storage`: this crate is the contract, the store is an
/// adapter for it. `async fn` in trait means the trait is not `dyn`-compatible; call it
/// through a generic, as [`ingest`] does.
#[allow(async_fn_in_trait)]
pub trait EventStore {
    type Error;

    async fn commit(&self, event: &ReceivedEvent) -> Result<CommitOutcome, Self::Error>;
}

/// Proof that one event is durable.
///
/// Only [`ingest`] can mint one, and only from a successful [`EventStore::commit`].
#[derive(Debug)]
pub struct Commit {
    key: EventId,
    outcome: CommitOutcome,
}

impl Commit {
    pub(crate) fn new(key: EventId, outcome: CommitOutcome) -> Self {
        Self { key, outcome }
    }

    pub fn key(&self) -> &EventId {
        &self.key
    }

    pub fn outcome(&self) -> CommitOutcome {
        self.outcome
    }

    /// Turns durability into permission for the edge to forget the event.
    pub fn into_ack(self) -> Ack {
        Ack(AckPayload {
            device_id: self.key.device_id().clone(),
            boot_id: self.key.boot_id(),
            sequence: self.key.sequence(),
            status: match self.outcome {
                CommitOutcome::Stored => AckStatus::Stored,
                CommitOutcome::AlreadyStored => AckStatus::Duplicate,
            },
        })
    }
}

/// An acknowledgement the hub is entitled to publish.
///
/// Unconstructible except from a [`Commit`], which is the whole point: the publish helpers
/// take an `Ack`, so an un-earned acknowledgement cannot reach the wire.
#[derive(Debug)]
pub struct Ack(AckPayload);

impl Ack {
    pub fn payload(&self) -> &AckPayload {
        &self.0
    }

    pub fn into_payload(self) -> AckPayload {
        self.0
    }

    pub fn encode(&self) -> Vec<u8> {
        self.0.encode()
    }
}

/// The ACK as it travels on the wire, and as the edge parses it.
///
/// Separate from [`Ack`] on purpose: anyone may *read* an acknowledgement, only a commit
/// may *make* one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckPayload {
    pub device_id: DeviceId,
    pub boot_id: i64,
    pub sequence: i64,
    pub status: AckStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AckStatus {
    /// Newly committed.
    Stored,
    /// Already present under the same idempotency key; release it anyway.
    Duplicate,
}

impl AckPayload {
    pub fn decode(payload: &[u8]) -> Result<Self, WireError> {
        serde_json::from_slice(payload).map_err(|e| WireError::Malformed(e.to_string()))
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("AckPayload is always serialisable")
    }
}

impl From<&AckPayload> for EventId {
    fn from(p: &AckPayload) -> EventId {
        EventId::new(p.device_id.clone(), p.boot_id, p.sequence)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError<E> {
    #[error("rejected before storage: {0}")]
    Malformed(#[from] WireError),
    /// The event is *not* durable. No ACK exists, so the edge keeps it and will resend.
    #[error("persistent storage commit failed")]
    Storage(E),
}

/// Commit one received event, then acknowledge it — in that order, always.
pub async fn ingest<S: EventStore>(
    store: &S,
    event: &ReceivedEvent,
) -> Result<Ack, IngestError<S::Error>> {
    let outcome = store.commit(event).await.map_err(IngestError::Storage)?;
    Ok(Commit::new(event.id(), outcome).into_ack())
}

/// Decode an arriving MQTT payload and ingest it.
///
/// A malformed payload never reaches the store: it is not an event, so there is nothing to
/// make durable and nothing to acknowledge.
pub async fn ingest_payload<S: EventStore>(
    store: &S,
    payload: &[u8],
    received_at: i64,
) -> Result<Ack, IngestError<S::Error>> {
    let event = EdgeEvent::decode(payload)?;
    ingest(store, &ReceivedEvent::new(event, received_at)).await
}
