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
    checkin_view, config, exceptions, finish, health, live, readers, results, session, stages,
    templates, CheckInView, DeviceHealth, Entrant, Health, HubStore, LiveSession, NewClass,
    OperatorCommand, OperatorError, ReaderView, SessionResults, Snapshot, StageView,
    StoredException, TemplateError,
};
use domain::{
    Course, ExerciseLibrary, Expectation, FinishPolicy, Instant, Interpreted, ReaderRegistration,
    Session, SessionConfig, StationMap, TagId, WorkoutTemplate,
};

/// One athlete's progress through the class, as `/api/stages` reports it.
pub struct AthleteProgress {
    pub athlete_id: String,
    pub name: String,
    /// How the station they are standing in compares with the plan (workout brief §11).
    /// `None` between stations. A label, never a judgement -- nothing acts on it.
    pub expectation: Option<Expectation>,
    pub stages: Vec<StageView>,
}
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

/// What the settings screen can ask of the machine itself (M6).
///
/// A port, because switching a machine off is the most platform-specific thing this system
/// does (CLAUDE.md 2): the appliance's implementation shells out to systemd, a developer's
/// build refuses, and neither belongs in a handler. The *policy* -- refuse while a class is
/// live -- is decided here, above the port, so it holds whatever the machine underneath is.
pub trait Power: Send + Sync + 'static {
    /// Carry out the action. Implementations must return **after scheduling** it rather
    /// than after doing it: a handler that powers the machine off before it answers leaves
    /// the operator staring at a failed request, unsure whether it worked.
    fn request(&self, action: PowerAction) -> Result<(), String>;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PowerAction {
    Poweroff,
    Reboot,
    /// Restart the hub's own service. Safe mid-class, unlike the other two: the session is
    /// rebuilt by replaying the log, and events published meanwhile are delivered by the
    /// broker's queue afterwards (CLAUDE.md 15, 21).
    RestartService,
}

impl PowerAction {
    /// Does this action end the venue's evening? Only the two that stop the machine do; a
    /// service restart is how a wedged screen is fixed without walking to the box.
    pub fn stops_the_machine(self) -> bool {
        matches!(self, Self::Poweroff | Self::Reboot)
    }

    pub fn audit_action(self) -> &'static str {
        match self {
            Self::Poweroff => "POWER_POWEROFF",
            Self::Reboot => "POWER_REBOOT",
            Self::RestartService => "POWER_RESTART_SERVICE",
        }
    }
}

/// The default: a build nobody wired a machine into. Refusing is the honest answer -- a
/// no-op would let a settings screen report success while nothing happened.
pub struct NoPower;

impl Power for NoPower {
    fn request(&self, _action: PowerAction) -> Result<(), String> {
        Err("this hub was not given power control".to_string())
    }
}

/// Loading a venue's worth of demo data, and driving it (M6 follow-up).
///
/// A port for the same reason [`Power`] is one: what it does is entirely
/// composition-root business -- a fixture course, a fixture roster, and an emulated
/// collector publishing over the real broker -- and none of that belongs in a handler.
///
/// **Off unless a machine says otherwise.** A customer's hub answers `available() == false`
/// and the settings screen shows nothing, because a button that fills a gym's database with
/// invented athletes is not a button a gym should be able to press by accident.
pub trait Demo: Send + Sync + 'static {
    /// Whether this build and this machine offer demo data at all.
    fn available(&self) -> bool;

    /// Put a full venue in place -- course, roster, readers, bands -- and start producing
    /// reads through the ordinary ingestion path, so what is being tested is the real
    /// pipeline rather than a shortcut into the read model.
    fn load(&self) -> Result<(), String>;

    /// Stop producing reads and stand the demo class down. What was already recorded stays:
    /// `raw_events` is immutable (CLAUDE.md 19), and a test that erased its own evidence
    /// would be a strange kind of test.
    fn clear(&self) -> Result<(), String>;
}

/// The default: no demo data on this machine.
pub struct NoDemo;

impl Demo for NoDemo {
    fn available(&self) -> bool {
        false
    }
    fn load(&self) -> Result<(), String> {
        Err("this hub does not carry demo data".to_string())
    }
    fn clear(&self) -> Result<(), String> {
        Err("this hub does not carry demo data".to_string())
    }
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
    /// The running binary's version, supplied by the composition root (ADR 0009). This
    /// crate's own version would be a different number the moment the workspace stops
    /// moving in lockstep, and the question being answered is "what is on that machine".
    version: &'static str,
    /// Where backups are written (ADR 0012). Configuration, never a request field: a
    /// client choosing where the hub writes files is a path traversal with extra steps.
    backup_dir: Arc<std::path::Path>,
    /// How the settings screen reaches the machine (M6). Refuses by default.
    power: Arc<dyn Power>,
    /// Demo data, where a machine is set up to offer it. Refuses by default.
    demo: Arc<dyn Demo>,
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
            version: self.version,
            backup_dir: Arc::clone(&self.backup_dir),
            power: Arc::clone(&self.power),
            demo: Arc::clone(&self.demo),
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
        version: &'static str,
        backup_dir: impl AsRef<std::path::Path>,
    ) -> Self {
        let (snapshots, _) = broadcast::channel(channel_capacity);
        Self {
            session: Arc::new(Mutex::new(session)),
            store,
            clock,
            snapshots,
            push_interval_ms,
            version,
            backup_dir: Arc::from(backup_dir.as_ref()),
            power: Arc::new(NoPower),
            demo: Arc::new(NoDemo),
        }
    }

    /// Hands the hub a way to switch the machine off. The composition root's job: only it
    /// knows whether this process is on an appliance or on somebody's laptop.
    pub fn with_power(mut self, power: Arc<dyn Power>) -> Self {
        self.power = power;
        self
    }

    /// Hands the hub a venue's worth of demo data. Only a machine meant for testing gets
    /// one; everything else keeps [`NoDemo`], which answers "not available".
    pub fn with_demo(mut self, demo: Arc<dyn Demo>) -> Self {
        self.demo = demo;
        self
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

    pub fn demo(&self) -> &Arc<dyn Demo> {
        &self.demo
    }

    pub fn backup_dir(&self) -> &std::path::Path {
        &self.backup_dir
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

    /// Whether the appliance may be stopped right now (ADR 0009 §6). Read by the nightly
    /// maintenance window before it updates and powers the machine off.
    pub async fn health(&self) -> Health {
        let now = self.now();
        let state = self.0.lock().await;
        health::health_with_version(&state, now, self.0.version)
    }
}

impl<S: HubStore> ReadOnly<S> {
    /// The current session's results, ranked where the finish rule allows it (ADR 0010).
    ///
    /// The leaderboard screen reads this rather than `/api/result/{id}`, so it does not have
    /// to ask which session is on before it can show anything.
    pub async fn leaderboard(&self) -> SessionResults {
        results::live_results(&*self.0.lock().await)
    }

    /// Results for any stored session, running or long finished (CLAUDE.md 22).
    ///
    /// The one read that touches the store. It is a read: `application::results` rebuilds
    /// from the interpreted log and writes nothing.
    /// Whether this machine offers demo data (M6 follow-up).
    pub fn demo_available(&self) -> bool {
        self.0.demo.available()
    }

    /// One of the venue's images (M6 follow-up).
    pub async fn venue_asset(&self, key: &str) -> Result<Option<application::VenueAsset>, S::Error>
    where
        S: HubStore,
    {
        application::venue_asset(&*self.0.store, key).await
    }

    /// The venue's own numbers, defaults filled in (M6 follow-up).
    pub async fn venue_settings(&self) -> Result<application::VenueSettings, S::Error>
    where
        S: HubStore,
    {
        application::venue_settings(&*self.0.store).await
    }

    pub async fn results(&self, session_id: &str) -> Result<Option<SessionResults>, S::Error> {
        results::results(&*self.0.store, session_id).await
    }

    /// The exercise library the builder screen offers (workout brief §3).
    pub async fn exercises(&self) -> Result<ExerciseLibrary, S::Error> {
        self.0.store.exercises().await
    }

    pub async fn stations(&self) -> Result<StationMap, S::Error> {
        self.0.store.stations().await
    }

    /// Every template, system ones first (workout brief §13).
    pub async fn templates(&self) -> Result<Vec<WorkoutTemplate>, S::Error> {
        templates::list_templates(&*self.0.store).await
    }

    pub async fn template(&self, id: &str) -> Result<Option<WorkoutTemplate>, S::Error> {
        self.0.store.template(id).await
    }

    /// Every athlete's stage list, derived from the session's snapshot course and their
    /// replayed runs (workout brief §10). Deliberately not part of the pushed snapshot: a
    /// full stage list per athlete would multiply the size of every WebSocket frame the big
    /// screen receives, and only the coach screen reads it.
    pub async fn stages(&self) -> Vec<AthleteProgress> {
        let now = self.now();
        let state = self.0.lock().await;
        let Some(course) = state.config.course.as_ref() else {
            return Vec::new();
        };
        state
            .athletes
            .iter()
            .map(|a| AthleteProgress {
                athlete_id: a.athlete_id.clone(),
                name: a.display_name.clone(),
                expectation: stages::current_expectation(course, a),
                stages: stages::stages(course, a, now),
            })
            .collect()
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
        Self {
            read: self.read.clone(),
            hub: self.hub.clone(),
        }
    }
}

impl<S> CheckIn<S> {
    pub fn new(hub: Hub<S>) -> Self {
        Self {
            read: ReadOnly::new(hub.clone()),
            hub,
        }
    }

    pub fn read(&self) -> &ReadOnly<S> {
        &self.read
    }
}

impl<S: HubStore> CheckIn<S> {
    /// Puts somebody on the roster -- a member, or a walk-in with nothing but a name
    /// (ADR 0010). This is what a door tablet does at a competition, and it is still
    /// nowhere near the session's clock.
    pub async fn enter(
        &self,
        entrant: Entrant,
        cmd: &OperatorCommand,
    ) -> Result<String, OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        application::enter(&mut state, &*self.hub.store, entrant, cmd).await
    }

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
        Self {
            read: self.read.clone(),
            hub: self.hub.clone(),
        }
    }
}

impl<S> Operator<S> {
    pub fn new(hub: Hub<S>) -> Self {
        Self {
            read: ReadOnly::new(hub.clone()),
            hub,
        }
    }

    pub fn read(&self) -> &ReadOnly<S> {
        &self.read
    }
}

impl<S: HubStore> Operator<S> {
    pub async fn mark_ready(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        session::mark_ready(&mut state, &*self.hub.store, cmd).await
    }

    pub async fn start(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        session::start(&mut state, &*self.hub.store, cmd).await
    }

    pub async fn pause(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        session::pause(&mut state, &*self.hub.store, cmd).await
    }

    pub async fn resume(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        session::resume(&mut state, &*self.hub.store, cmd).await
    }

    pub async fn complete(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        session::complete(&mut state, &*self.hub.store, cmd).await
    }

    pub async fn cancel(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        session::cancel(&mut state, &*self.hub.store, cmd).await
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

    // --- the workout library (ADR 0008) --------------------------------------------------

    pub async fn save_template(
        &self,
        template: WorkoutTemplate,
        cmd: &OperatorCommand,
    ) -> Result<WorkoutTemplate, TemplateError<S::Error>> {
        templates::save_template(&*self.hub.store, template, cmd).await
    }

    pub async fn duplicate_template(
        &self,
        source_id: &str,
        new_id: &str,
        new_name: &str,
        owner_id: Option<&str>,
        cmd: &OperatorCommand,
    ) -> Result<WorkoutTemplate, TemplateError<S::Error>> {
        templates::duplicate_template(&*self.hub.store, source_id, new_id, new_name, owner_id, cmd)
            .await
    }

    pub async fn delete_template(
        &self,
        template_id: &str,
        cmd: &OperatorCommand,
    ) -> Result<(), TemplateError<S::Error>> {
        templates::delete_template(&*self.hub.store, template_id, cmd).await
    }

    /// Template -> compiled course -> snapshot -> DRAFT class (workout brief §15).
    ///
    /// The library is read from the store rather than taken from the caller: a class must be
    /// compiled against the exercises this hub actually holds, not against whatever a
    /// request happened to describe.
    pub async fn create_class(
        &self,
        template_id: &str,
        new: NewClass,
        cmd: &OperatorCommand,
    ) -> Result<(), TemplateError<S::Error>> {
        let template = self
            .hub
            .store
            .template(template_id)
            .await
            .map_err(TemplateError::Storage)?
            .ok_or_else(|| TemplateError::UnknownTemplate(template_id.to_string()))?;
        let library = self
            .hub
            .store
            .exercises()
            .await
            .map_err(TemplateError::Storage)?;
        let mut state = self.hub.lock().await;
        templates::create_class(&mut state, &*self.hub.store, &template, &library, new, cmd).await
    }

    pub async fn register_reader(
        &self,
        registration: &ReaderRegistration,
        cmd: &OperatorCommand,
    ) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        readers::register_reader(&mut state, &*self.hub.store, registration, cmd).await
    }

    /// The exception inbox, which lives in the store rather than in the session.
    ///
    /// The session id is taken and the guard **dropped** before the query. Holding the lock
    /// across disk I/O would put a growing table between a reader's tap and its ACK, and the
    /// settings screen asks for this every five seconds. `backup()` has the same shape for
    /// the same reason. The id cannot go stale under us: a session id never changes, and a
    /// query for a session that has just been replaced returns that session's exceptions,
    /// which is what was asked for.
    /// Clears one exception without erasing it (ADR 0001 D4).
    pub async fn accept_exception(
        &self,
        interpreted_event_id: i64,
        cmd: &OperatorCommand,
    ) -> Result<(), OperatorError<S::Error>> {
        let mut state = self.hub.lock().await;
        exceptions::accept(&mut state, &*self.hub.store, interpreted_event_id, cmd).await
    }

    pub async fn exceptions(&self) -> Result<Vec<StoredException>, OperatorError<S::Error>> {
        let session_id = {
            let state = self.hub.lock().await;
            state.session.id.clone()
        };
        exceptions::list(&session_id, &*self.hub.store).await
    }

    /// Takes a backup and returns where it went (ADR 0012).
    ///
    /// The file is named for the moment it was taken, in epoch milliseconds: sortable by
    /// name, and the same number the audit row carries, so a restore months later can be
    /// matched to the operator who asked for it. The directory comes from configuration.
    pub async fn backup(
        &self,
        cmd: &OperatorCommand,
        request_reason: Option<String>,
    ) -> Result<std::path::PathBuf, OperatorError<S::Error>>
    where
        S: HubStore,
    {
        let now = cmd.at;
        // Creating the directory is the adapter's job, not this layer's: filesystem
        // concerns live where the file does.
        let path = self.hub.backup_dir().join(format!("hyrox-{}.db", now.0));
        self.hub
            .store
            .backup_to(&path)
            .await
            .map_err(OperatorError::Storage)?;
        // A copy of every event in the venue's history just left the database. Who asked
        // for it belongs in the trail, even though nothing recorded was changed
        // (CLAUDE.md 20).
        self.hub
            .store
            .record_audit(&application::AuditEntry {
                at: now,
                operator: cmd.operator.clone(),
                action: "DATABASE_BACKUP".to_string(),
                subject: path.display().to_string(),
                // A backup needs no reason; if the caller gave one it is kept.
                reason: request_reason.clone(),
                before: None,
                after: None,
            })
            .await
            .map_err(OperatorError::Storage)?;
        Ok(path)
    }

    /// Carry out a power action, with the one guard that makes it safe to put on a screen
    /// with no login: **a class on the floor wins** (M6).
    ///
    /// It asks the same question the nightly window asks -- `safe_to_stop` -- so the two
    /// cannot drift apart. Restarting the hub's own service is exempt, because that is
    /// recoverable by design and is how a wedged screen gets fixed.
    pub async fn power(
        &self,
        action: PowerAction,
        cmd: &OperatorCommand,
        reason: Option<String>,
    ) -> Result<(), OperatorError<S::Error>>
    where
        S: HubStore,
    {
        if action.stops_the_machine() {
            let state = self.hub.lock().await;
            if state.session.is_live() {
                return Err(OperatorError::ClassInProgress);
            }
        }
        self.hub
            .store
            .record_audit(&application::AuditEntry {
                at: cmd.at,
                operator: cmd.operator.clone(),
                action: action.audit_action().to_string(),
                subject: String::new(),
                reason,
                before: None,
                after: None,
            })
            .await
            .map_err(OperatorError::Storage)?;
        // Audited first: if the machine goes down a moment later, the record of who asked
        // is already durable. The other order loses the row exactly when it matters.
        self.hub
            .power
            .request(action)
            .map_err(OperatorError::PowerUnavailable)
    }

    /// Load a venue's worth of demo data (M6 follow-up), with the guard that makes it safe
    /// to offer at all: a class on the floor refuses.
    pub async fn load_demo(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>>
    where
        S: HubStore,
    {
        if !self.hub.demo.available() {
            return Err(OperatorError::DemoUnavailable);
        }
        {
            let state = self.hub.lock().await;
            if state.session.is_live() {
                return Err(OperatorError::ClassInProgress);
            }
        }
        self.audit_demo(cmd, "DEMO_LOAD").await?;
        self.hub.demo.load().map_err(OperatorError::DemoFailed)
    }

    /// Stop it. No class guard: stopping invented reads cannot hurt a real class, and the
    /// moment somebody wants the off switch is the moment something has gone wrong.
    pub async fn clear_demo(&self, cmd: &OperatorCommand) -> Result<(), OperatorError<S::Error>>
    where
        S: HubStore,
    {
        if !self.hub.demo.available() {
            return Err(OperatorError::DemoUnavailable);
        }
        self.audit_demo(cmd, "DEMO_CLEAR").await?;
        self.hub.demo.clear().map_err(OperatorError::DemoFailed)
    }

    async fn audit_demo(
        &self,
        cmd: &OperatorCommand,
        action: &str,
    ) -> Result<(), OperatorError<S::Error>>
    where
        S: HubStore,
    {
        self.hub
            .store
            .record_audit(&application::AuditEntry {
                at: cmd.at,
                operator: cmd.operator.clone(),
                action: action.to_string(),
                subject: String::new(),
                reason: None,
                before: None,
                after: None,
            })
            .await
            .map_err(OperatorError::Storage)
    }

    /// Take one reader off the map (ADR 0007 §7, amended).
    pub async fn unregister_reader(
        &self,
        key: &domain::ReaderKey,
        cmd: &OperatorCommand,
    ) -> Result<(), OperatorError<S::Error>>
    where
        S: HubStore,
    {
        let mut state = self.hub.lock().await;
        readers::unregister_reader(&mut state, &*self.hub.store, key, cmd).await
    }

    /// Antennas the hub has heard from and cannot resolve (M6).
    pub async fn unregistered_readers(
        &self,
    ) -> Result<Vec<application::SeenReader>, OperatorError<S::Error>>
    where
        S: HubStore,
    {
        let readers = {
            let state = self.hub.lock().await;
            state.readers.clone()
        };
        application::unregistered_readers(&readers, &*self.hub.store)
            .await
            .map_err(OperatorError::Storage)
    }

    /// Store one venue setting (M6 follow-up).
    /// Store the venue's logo, after the application layer has decided it is an image we
    /// will serve back (M6 follow-up).
    pub async fn save_logo(
        &self,
        bytes: Vec<u8>,
        cmd: &OperatorCommand,
    ) -> Result<application::VenueAsset, application::AssetError<S::Error>>
    where
        S: HubStore,
    {
        application::save_venue_asset(&*self.hub.store, application::VENUE_LOGO, bytes, cmd).await
    }

    pub async fn remove_logo(
        &self,
        cmd: &OperatorCommand,
    ) -> Result<(), application::AssetError<S::Error>>
    where
        S: HubStore,
    {
        application::delete_venue_asset(&*self.hub.store, application::VENUE_LOGO, cmd).await
    }

    pub async fn save_setting(
        &self,
        key: &str,
        value: &str,
        cmd: &OperatorCommand,
    ) -> Result<(), application::SettingError<S::Error>>
    where
        S: HubStore,
    {
        application::save_venue_setting(&*self.hub.store, key, value, cmd).await
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
