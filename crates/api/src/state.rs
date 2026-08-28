//! The shared hub handle, and the three capabilities the ADR's surfaces are cut along.
//!
//! ADR 0001 divides the screens into one write surface, one narrow write surface and three
//! read-only ones. That division is expressed here as three types rather than as a
//! convention:
//!
//! ```text
//! ReadOnly<S>   reads               /coach  /live  /result/{id}
//! CheckIn<S>    reads + binding     /checkin
//! Operator<S>   reads + everything  /operator
//! ```
//!
//! Each router is built with exactly one of them as its axum state, so a handler declares
//! the capability it needs in its own signature (`State<ReadOnly<S>>`, and so on). The
//! capability types hold the store and the live session in **private** fields and expose no
//! accessor for either, so a read handler has no expression that reaches
//! `HubStore::void_interpreted` or `&mut LiveSession`. A write attempted from the read
//! module is a compile error, not a code review finding (ADR 0007).
//!
//! The lock lives here too, for the same reason: every use case that advances the session
//! must hold it across the store's awaits, and one place holding it is one place to check.

use application::{
    checkin_view, config, exceptions, finish, live, readers, results, session, CheckInView,
    DeviceHealth, HubStore, LiveSession, OperatorCommand, OperatorError, ReaderView,
    SessionResults, Snapshot, StoredException,
};
use domain::{
    Course, FinishPolicy, Instant, Interpreted, ReaderRegistration, Session, SessionConfig,
    TagId,
};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex, MutexGuard};

/// Where "now" comes from.
///
/// A port, not a call to the system clock: the development hub runs a fast virtual clock so
/// a twenty-minute class plays out in under two minutes, and an API that read the wall
/// clock itself would disagree with the events it is rendering. The composition root owns
/// the clock (CLAUDE.md 17).
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

/// The running hub: the live session, the store, the clock, and the snapshot fan-out.
///
/// Held by the composition root, which is the only thing that gets mutable access to the
/// session ([`Hub::lock`]). Handlers never see a `Hub`; they see one of the capability
/// wrappers below.
pub struct Hub<S> {
    session: Arc<Mutex<LiveSession>>,
    store: Arc<S>,
    clock: Arc<dyn Clock>,
    snapshots: broadcast::Sender<String>,
    /// How often the hub pushes a snapshot. Published to every screen so a client can tell
    /// "quiet" from "the socket died" without guessing a timeout (ADR 0001 D5).
    push_interval_ms: i64,
}

// Derived `Clone` would demand `S: Clone`, which no store is; every field is an `Arc` or a
// `Sender`, so cloning a `Hub` is cheap regardless.
impl<S> Clone for Hub<S> {
    fn clone(&self) -> Self {
        Self {
            session: Arc::clone(&self.session),
            store: Arc::clone(&self.store),
            clock: Arc::clone(&self.clock),
            snapshots: self.snapshots.clone(),
            push_interval_ms: self.push_interval_ms,
        }
    }
}

impl<S> Hub<S> {
    pub fn new(
        session: LiveSession,
        store: Arc<S>,
        clock: Arc<dyn Clock>,
        push_interval_ms: i64,
        channel_capacity: usize,
    ) -> Self {
        let (snapshots, _) = broadcast::channel(channel_capacity);
        Self {
            session: Arc::new(Mutex::new(session)),
            store,
            clock,
            snapshots,
            push_interval_ms,
        }
    }

    /// Full mutable access to the live session, for the composition root only: the tick
    /// loop that applies the finish policy, and the MQTT loop that ingests reads. No
    /// handler can call this, because no handler is ever handed a `Hub`.
    pub async fn lock(&self) -> MutexGuard<'_, LiveSession> {
        self.session.lock().await
    }

    pub fn store(&self) -> &Arc<S> {
        &self.store
    }

    pub fn now(&self) -> Instant {
        self.clock.now()
    }

    /// Broadcasts one already-serialised snapshot to every open socket.
    ///
    /// The error case is "nobody is listening", which is the normal state of a gym with no
    /// screen open, so it is deliberately not reported.
    pub fn publish(&self, payload: String) {
        let _ = self.snapshots.send(payload);
    }
}

/// Reads only. The state of `/coach`, `/live` and `/result/{id}` (ADR 0001).
///
/// Wraps a `Hub` in a private field and offers no way back out of it. Everything it returns
/// is either an owned projection from `application::live` or a stored read model -- never
/// the session, never the store.
pub struct ReadOnly<S>(Hub<S>);

impl<S> Clone for ReadOnly<S> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S> ReadOnly<S> {
    pub fn new(hub: Hub<S>) -> Self {
        Self(hub)
    }

    pub fn now(&self) -> Instant {
        self.0.now()
    }

    pub fn push_interval_ms(&self) -> i64 {
        self.0.push_interval_ms
    }

    /// How many sockets the hub is currently pushing to. Half of the mandatory liveness
    /// readout: a screen that believes it is connected while the hub counts nobody has a
    /// dead link (ADR 0001 D5).
    pub fn subscribers(&self) -> usize {
        self.0.snapshots.receiver_count()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.0.snapshots.subscribe()
    }

    pub async fn snapshot(&self) -> Snapshot {
        let now = self.now();
        let state = self.0.lock().await;
        live::snapshot(&state, now)
    }

    /// Age of the newest event, alone. Cheaper than a whole snapshot for the surfaces that
    /// need the freshness reading but not the roster.
    pub async fn last_event_age_ms(&self) -> Option<i64> {
        let now = self.now();
        let state = self.0.lock().await;
        live::last_event_age_ms(&state, now)
    }

    pub async fn readers(&self) -> Vec<ReaderView> {
        let now = self.now();
        let state = self.0.lock().await;
        live::reader_views(&state, now)
    }

    pub async fn checkin(&self) -> CheckInView {
        let state = self.0.lock().await;
        checkin_view(&state)
    }

    /// The session record and the configuration it is running under, copied out.
    pub async fn session(&self) -> (Session, SessionConfig, i64) {
        let now = self.now();
        let state = self.0.lock().await;
        (
            state.session.clone(),
            state.config.clone(),
            state.class_elapsed(now).millis(),
        )
    }

    /// Edge device health, for the operator's per-reader freshness (ADR 0001 D5).
    pub async fn devices(&self) -> Vec<DeviceHealth> {
        self.0.lock().await.devices().to_vec()
    }
}

impl<S: HubStore> ReadOnly<S> {
    /// Results for any stored session, running or long finished (CLAUDE.md 22).
    ///
    /// The one read that touches the store. It is a read: `application::results` rebuilds
    /// from the interpreted log and writes nothing.
    pub async fn results(&self, session_id: &str) -> Result<Option<SessionResults>, S::Error> {
        results::results(&*self.0.store, session_id).await
    }
}

/// The narrow write surface: bindings, and nothing else (ADR 0001, `/checkin`).
///
/// A check-in tablet is handed to whoever is on the door. It must not be able to arm a
/// session, close one, or void an event -- so its state type simply has no method that
/// could. Reads are reached through [`CheckIn::read`], which yields the read-only
/// capability rather than the hub.
pub struct CheckIn<S> {
    read: ReadOnly<S>,
    hub: Hub<S>,
}

impl<S> Clone for CheckIn<S> {
    fn clone(&self) -> Self {
        Self { read: self.read.clone(), hub: self.hub.clone() }
    }
}

impl<S> CheckIn<S> {
    pub fn new(hub: Hub<S>) -> Self {
        Self { read: ReadOnly::new(hub.clone()), hub }
    }

    pub fn read(&self) -> &ReadOnly<S> {
        &self.read
    }
}

impl<S: HubStore> CheckIn<S> {
    pub async fn bind(
        &self,
        tag: &TagId,
        athlete_id: &str,
        cmd: &OperatorCommand,
    ) -> Result<Vec<Interpreted>, OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        application::checkin::bind_tag(&mut state, &*self.hub.store, tag, athlete_id, cmd).await
    }

    pub async fn rebind(
        &self,
        tag: &TagId,
        athlete_id: &str,
        cmd: &OperatorCommand,
    ) -> Result<Vec<Interpreted>, OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        application::checkin::rebind_tag(&mut state, &*self.hub.store, tag, athlete_id, cmd).await
    }
}

/// The full write surface (ADR 0001, `/operator`).
///
/// One method per use case, each of them three lines: take the lock, call the use case,
/// hand back what it said. The rules are all below this layer -- what a transition is
/// legal from, when a reason is required, what a finish rule may do -- so nothing here can
/// change a decision, only which decision is asked for (CLAUDE.md 29).
pub struct Operator<S> {
    read: ReadOnly<S>,
    hub: Hub<S>,
}

impl<S> Clone for Operator<S> {
    fn clone(&self) -> Self {
        Self { read: self.read.clone(), hub: self.hub.clone() }
    }
}

impl<S> Operator<S> {
    pub fn new(hub: Hub<S>) -> Self {
        Self { read: ReadOnly::new(hub.clone()), hub }
    }

    pub fn read(&self) -> &ReadOnly<S> {
        &self.read
    }
}

impl<S: HubStore> Operator<S> {
    pub async fn arm(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        session::arm(&mut state, &*self.hub.store, cmd).await
    }

    pub async fn close(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        session::close(&mut state, &*self.hub.store, cmd).await
    }

    pub async fn reopen(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        session::reopen(&mut state, &*self.hub.store, cmd).await
    }

    pub async fn return_to_draft(
        &self,
        cmd: &OperatorCommand,
    ) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        session::return_to_draft(&mut state, &*self.hub.store, cmd).await
    }

    /// The coach ends the class by hand. Refused where no finish rule is configured, which
    /// is competition (CLAUDE.md 12, 28) -- the use case decides that, not this method.
    pub async fn end_class(
        &self,
        cmd: &OperatorCommand,
    ) -> Result<Vec<String>, OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        finish::end_class(&mut state, &*self.hub.store, cmd).await
    }

    pub async fn configure(
        &self,
        course: Option<Course>,
        finish_policy: FinishPolicy,
        cmd: &OperatorCommand,
    ) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        config::configure(&mut state, &*self.hub.store, course, finish_policy, cmd).await
    }

    pub async fn register_reader(
        &self,
        registration: &ReaderRegistration,
        cmd: &OperatorCommand,
    ) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        readers::register_reader(&mut state, &*self.hub.store, registration, cmd).await
    }

    pub async fn exceptions(&self) -> Result<Vec<StoredException>, OperatorError<S::Error>> {
        let state = self.hub.lock().await;
        exceptions::list(&state, &*self.hub.store).await
    }

    pub async fn void_exception(
        &self,
        interpreted_event_id: i64,
        cmd: &OperatorCommand,
    ) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        exceptions::void(&mut state, &*self.hub.store, interpreted_event_id, cmd).await
    }
}
