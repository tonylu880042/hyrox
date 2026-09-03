//! An in-memory hub for the HTTP tests: no SQLite, no broker, no sockets (CLAUDE.md 24).
//!
//! The store fake here is deliberately smaller than `crates/application`'s. That one exists
//! to observe the *order* of a use case's writes; these tests only need the API to be able
//! to run a use case end to end and report what it said, so this one remembers what it was
//! given and answers queries from it.
//!
//! Each test binary compiles this module separately and uses a different part of it, so
//! unused helpers are expected here rather than a sign of dead code.
#![allow(dead_code)]

use api::{Clock, Hub};
use application::{
    AuditEntry, HubStore, InterpretedWrite, LiveSession, RawCommit, RawRead, StoredException,
    StoredRawRead, SeenReader,
};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use contract::CommitOutcome;
use domain::{
    AthleteState, BindingLedger, Exercise, ExerciseLibrary, Instant, Interpreted,
    PhysicalStation, ReaderRegistration, ReaderRegistry, Session, SessionConfig, SessionMode,
    StationMap, TagBinding, WorkoutTemplate,
};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

pub const START: Instant = Instant(1_000_000);
/// Far enough after `START` that a class clock has visibly run.
pub const NOW: Instant = Instant(1_600_000);

#[derive(Debug)]
pub struct FakeError(pub &'static str);

impl std::fmt::Display for FakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// A clock the test sets by hand. The API must never read the wall clock itself, and this
/// is how that is checked: every timestamp in a response comes from here.
pub struct FixedClock(pub Instant);

impl Clock for FixedClock {
    fn now(&self) -> Instant {
        self.0
    }
}

#[derive(Default)]
struct Inner {
    raw: Vec<RawRead>,
    interpreted: Vec<(String, Option<i64>, Interpreted)>,
    voided: Vec<i64>,
    audits: Vec<AuditEntry>,
    seen_readers: Vec<SeenReader>,
    venue_settings: Vec<(String, String)>,
    deleted_readers: Vec<(String, String)>,
    venue_assets: Vec<(String, application::VenueAsset)>,
    power_actions: Vec<String>,
    backups: Vec<std::path::PathBuf>,
    sessions: Vec<Session>,
    configs: Vec<SessionConfig>,
    readers: Vec<ReaderRegistration>,
    bindings: Vec<TagBinding>,
    athletes: Vec<AthleteState>,
    roster: Vec<SavedAthlete>,
    templates: Vec<WorkoutTemplate>,
    exercises: Vec<Exercise>,
    stations: Vec<PhysicalStation>,
}

/// One roster row exactly as the store was asked to write it, so a test can assert on the
/// bib and the member reference (ADR 0010), not only on who is on the roster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedAthlete {
    pub athlete_id: String,
    pub display_name: String,
    pub bib: i64,
    pub member_id: Option<String>,
}

#[derive(Default)]
pub struct FakeStore {
    inner: Mutex<Inner>,
    /// A read that will not return until the test says so, for asserting that a slow store
    /// cannot stall the ingestion path. `Some` means `exceptions()` parks.
    parked: Mutex<Option<Arc<tokio::sync::Notify>>>,
}

impl FakeStore {
    /// Makes the next `exceptions()` hang until the returned gate is notified.
    pub fn park_exceptions(&self) -> Arc<tokio::sync::Notify> {
        let gate = Arc::new(tokio::sync::Notify::new());
        *self.parked.lock().unwrap() = Some(Arc::clone(&gate));
        gate
    }

    /// Puts a system template in the store without going through a use case, so a test can
    /// start from a library that already exists.
    pub fn seed_system_template(&self, id: &str, name: &str) {
        use domain::{Target, TargetType, Unit, WorkoutBlock, WorkoutExercise};
        let template = WorkoutTemplate::system(id, name, domain::TemplateCategory::Engine)
            .with_block(WorkoutBlock::sequential("Main").with_exercises(vec![
                WorkoutExercise::new(
                    "RUN",
                    Target { target_type: TargetType::Distance, value: 800, unit: Unit::Meter },
                ),
            ]));
        self.inner.lock().unwrap().templates.push(template);
    }

    /// A store as a hub has it after first-run seeding: the exercise library is present,
    /// because nothing can compile a template without it.
    pub fn new() -> Self {
        let store = Self::default();
        store.inner.lock().unwrap().exercises =
            ExerciseLibrary::preset().iter().cloned().collect();
        store
    }

    pub fn with_athletes(self, athletes: Vec<AthleteState>) -> Self {
        self.inner.lock().unwrap().athletes = athletes;
        self
    }

    /// Seeds one interpreted event, as if a reader had produced it. Returns its row id --
    /// what an operator names when voiding (CLAUDE.md 19, 20).
    pub fn seed_interpreted(&self, athlete_id: &str, event: Interpreted) -> i64 {
        let mut inner = self.inner.lock().unwrap();
        inner
            .interpreted
            .push((athlete_id.to_string(), None, event));
        inner.interpreted.len() as i64
    }

    /// Where a backup was asked to be written, in the order it was asked for.
    pub fn backups(&self) -> Vec<std::path::PathBuf> {
        self.inner.lock().unwrap().backups.clone()
    }

    /// What the settings screen asked of the machine, in order (M6).
    pub fn power_actions(&self) -> Vec<String> {
        self.inner.lock().unwrap().power_actions.clone()
    }

    /// Readers the map was asked to forget, in order.
    pub fn deleted_readers(&self) -> Vec<(String, String)> {
        self.inner.lock().unwrap().deleted_readers.clone()
    }

    pub fn audits(&self) -> Vec<AuditEntry> {
        self.inner.lock().unwrap().audits.clone()
    }

    pub fn saved_sessions(&self) -> Vec<Session> {
        self.inner.lock().unwrap().sessions.clone()
    }

    pub fn saved_configs(&self) -> Vec<SessionConfig> {
        self.inner.lock().unwrap().configs.clone()
    }

    pub fn saved_readers(&self) -> Vec<ReaderRegistration> {
        self.inner.lock().unwrap().readers.clone()
    }

    pub fn saved_bindings(&self) -> Vec<TagBinding> {
        self.inner.lock().unwrap().bindings.clone()
    }
}

impl FakeStore {
    pub fn saved_athletes(&self) -> Vec<SavedAthlete> {
        self.inner.lock().unwrap().roster.clone()
    }
}

impl HubStore for FakeStore {
    type Error = FakeError;

    async fn commit_raw(&self, raw: &RawRead) -> Result<RawCommit, FakeError> {
        let mut inner = self.inner.lock().unwrap();
        inner.raw.push(raw.clone());
        Ok(RawCommit {
            raw_event_id: inner.raw.len() as i64,
            outcome: CommitOutcome::Stored,
        })
    }

    async fn commit_interpreted(&self, w: InterpretedWrite<'_>) -> Result<i64, FakeError> {
        let mut inner = self.inner.lock().unwrap();
        inner
            .interpreted
            .push((w.athlete_id.to_string(), w.raw_event_id, w.event.clone()));
        Ok(inner.interpreted.len() as i64)
    }

    async fn save_session(&self, session: &Session, _created_at: Instant) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        match inner.sessions.iter_mut().find(|s| s.id == session.id) {
            Some(existing) => *existing = session.clone(),
            None => inner.sessions.push(session.clone()),
        }
        Ok(())
    }

    async fn save_athlete(
        &self,
        _session_id: &str,
        athlete_id: &str,
        display_name: &str,
        bib: i64,
        member_id: Option<&str>,
    ) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        inner.athletes.push(AthleteState::ready(athlete_id, display_name));
        inner.roster.push(SavedAthlete {
            athlete_id: athlete_id.to_string(),
            display_name: display_name.to_string(),
            bib,
            member_id: member_id.map(str::to_string),
        });
        Ok(())
    }

    async fn save_athlete_finish(
        &self,
        _session_id: &str,
        athlete_id: &str,
        finished_at: Option<domain::Instant>,
    ) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(a) = inner.athletes.iter_mut().find(|a| a.athlete_id == athlete_id) {
            match finished_at {
                Some(at) => domain::finish(a, at),
                None => a.finished_at = None,
            }
        }
        Ok(())
    }

    async fn active_session(&self) -> Result<Option<Session>, FakeError> {
        Ok(self.inner.lock().unwrap().sessions.first().cloned())
    }

    async fn session(&self, session_id: &str) -> Result<Option<Session>, FakeError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .cloned())
    }

    async fn save_session_config(&self, config: &SessionConfig) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        match inner
            .configs
            .iter_mut()
            .find(|c| c.session_id == config.session_id)
        {
            Some(existing) => *existing = config.clone(),
            None => inner.configs.push(config.clone()),
        }
        Ok(())
    }

    async fn session_config(&self, session_id: &str) -> Result<Option<SessionConfig>, FakeError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .configs
            .iter()
            .find(|c| c.session_id == session_id)
            .cloned())
    }

    async fn save_reader(&self, registration: &ReaderRegistration) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        match inner.readers.iter_mut().find(|r| r.key == registration.key) {
            Some(existing) => *existing = registration.clone(),
            None => inner.readers.push(registration.clone()),
        }
        Ok(())
    }

    async fn readers(&self) -> Result<ReaderRegistry, FakeError> {
        let mut registry = ReaderRegistry::new();
        for r in &self.inner.lock().unwrap().readers {
            registry.register(r.clone());
        }
        Ok(registry)
    }

    async fn save_binding(&self, binding: &TagBinding) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        let key = |b: &TagBinding| (b.session_id.clone(), b.tag_id.clone(), b.bound_at);
        match inner.bindings.iter_mut().find(|b| key(b) == key(binding)) {
            Some(existing) => existing.unbound_at = binding.unbound_at,
            None => inner.bindings.push(binding.clone()),
        }
        Ok(())
    }

    async fn bindings(&self) -> Result<BindingLedger, FakeError> {
        let mut entries = self.inner.lock().unwrap().bindings.clone();
        entries.sort_by_key(|b| b.bound_at);
        Ok(BindingLedger::restore(entries))
    }

    async fn reader_keys_seen(&self) -> Result<Vec<SeenReader>, FakeError> {
        Ok(self.inner.lock().unwrap().seen_readers.clone())
    }

    async fn raw_tags_since(&self, since: Instant) -> Result<Vec<String>, FakeError> {
        let mut seen: Vec<String> = Vec::new();
        for r in &self.inner.lock().unwrap().raw {
            if r.detected_at >= since && !seen.contains(&r.tag_id) {
                seen.push(r.tag_id.clone());
            }
        }
        Ok(seen)
    }

    async fn unclaimed_reads_for_tag(
        &self,
        tag_id: &str,
        since: Instant,
    ) -> Result<Vec<StoredRawRead>, FakeError> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<StoredRawRead> = inner
            .raw
            .iter()
            .enumerate()
            .filter(|(i, r)| {
                r.tag_id.eq_ignore_ascii_case(tag_id)
                    && r.detected_at >= since
                    && !inner
                        .interpreted
                        .iter()
                        .any(|(_, raw_id, _)| *raw_id == Some(*i as i64 + 1))
            })
            .map(|(i, r)| StoredRawRead {
                raw_event_id: i as i64 + 1,
                device_id: r.device_id.clone(),
                reader_id: r.reader_id.clone(),
                detected_at: r.detected_at,
            })
            .collect();
        out.sort_by_key(|r| (r.detected_at, r.raw_event_id));
        Ok(out)
    }

    async fn athlete_bibs(&self, _session_id: &str) -> Result<Vec<(String, i64)>, FakeError> {
        let inner = self.inner.lock().unwrap();
        if inner.roster.is_empty() {
            // A fixture that never went through `enter`: roster order is the bib, exactly
            // as it was before the door could assign one.
            return Ok(inner
                .athletes
                .iter()
                .enumerate()
                .map(|(i, a)| (a.athlete_id.clone(), i as i64 + 1))
                .collect());
        }
        Ok(inner.roster.iter().map(|a| (a.athlete_id.clone(), a.bib)).collect())
    }

    async fn rebuild_athletes(&self, _session_id: &str) -> Result<Vec<AthleteState>, FakeError> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<AthleteState> = inner
            .athletes
            .iter()
            .map(|a| AthleteState::ready(&a.athlete_id, &a.display_name))
            .collect();
        for (i, (athlete_id, _, event)) in inner.interpreted.iter().enumerate() {
            if inner.voided.contains(&(i as i64 + 1)) {
                continue;
            }
            if let Some(state) = out.iter_mut().find(|a| &a.athlete_id == athlete_id) {
                domain::apply(state, event);
            }
        }
        Ok(out)
    }

    async fn session_created_at(&self, _session_id: &str) -> Result<Option<Instant>, FakeError> {
        Ok(Some(START))
    }

    async fn exception_count(&self, _session_id: &str) -> Result<usize, FakeError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .interpreted
            .iter()
            .enumerate()
            .filter(|(i, (_, _, e))| {
                matches!(e, Interpreted::Exception { .. })
                    && !inner.voided.contains(&(*i as i64 + 1))
            })
            .count())
    }

    async fn exceptions(&self, _session_id: &str) -> Result<Vec<StoredException>, FakeError> {
        // A disk that is busy, or a table that has grown a season deep.
        let parked = self.parked.lock().unwrap().clone();
        if let Some(gate) = parked {
            gate.notified().await;
        }
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .interpreted
            .iter()
            .enumerate()
            .filter(|(i, _)| !inner.voided.contains(&(*i as i64 + 1)))
            .filter_map(|(i, (athlete_id, raw_event_id, event))| match event {
                Interpreted::Exception { reason, at } => Some(StoredException {
                    interpreted_event_id: i as i64 + 1,
                    athlete_id: athlete_id.clone(),
                    reason: reason.clone(),
                    at: *at,
                    raw_event_id: *raw_event_id,
                }),
                _ => None,
            })
            .collect())
    }

    async fn void_interpreted(
        &self,
        interpreted_event_id: i64,
        _at: Instant,
        _operator: &str,
        _reason: &str,
    ) -> Result<bool, FakeError> {
        let mut inner = self.inner.lock().unwrap();
        let exists =
            interpreted_event_id >= 1 && interpreted_event_id as usize <= inner.interpreted.len();
        if exists {
            inner.voided.push(interpreted_event_id);
        }
        Ok(exists)
    }

    /// The fake writes no file: what the tests care about is that a backup was asked for,
    /// and by which surface.
    async fn backup_to(&self, path: &std::path::Path) -> Result<(), FakeError> {
        self.inner.lock().unwrap().backups.push(path.to_path_buf());
        Ok(())
    }

    async fn delete_reader(&self, device_id: &str, reader_id: &str) -> Result<(), FakeError> {
        self.inner
            .lock()
            .unwrap()
            .deleted_readers
            .push((device_id.to_string(), reader_id.to_string()));
        Ok(())
    }

    async fn venue_settings(&self) -> Result<Vec<(String, String)>, FakeError> {
        Ok(self.inner.lock().unwrap().venue_settings.clone())
    }

    async fn save_venue_setting(
        &self,
        key: &str,
        value: &str,
        _at: Instant,
        _by: &str,
    ) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        inner.venue_settings.retain(|(k, _)| k != key);
        inner.venue_settings.push((key.to_string(), value.to_string()));
        Ok(())
    }

    async fn venue_asset(
        &self,
        key: &str,
    ) -> Result<Option<application::VenueAsset>, FakeError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.venue_assets.iter().find(|(k, _)| k == key).map(|(_, a)| a.clone()))
    }

    async fn save_venue_asset(
        &self,
        key: &str,
        media_type: &str,
        bytes: &[u8],
        _at: Instant,
        _by: &str,
    ) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        inner.venue_assets.retain(|(k, _)| k != key);
        inner.venue_assets.push((
            key.to_string(),
            application::VenueAsset { media_type: media_type.to_string(), bytes: bytes.to_vec() },
        ));
        Ok(())
    }

    async fn delete_venue_asset(&self, key: &str) -> Result<(), FakeError> {
        self.inner.lock().unwrap().venue_assets.retain(|(k, _)| k != key);
        Ok(())
    }

    async fn record_audit(&self, entry: &AuditEntry) -> Result<(), FakeError> {
        self.inner.lock().unwrap().audits.push(entry.clone());
        Ok(())
    }
    // --- the workout library (ADR 0008) --------------------------------------------------

    async fn save_template(&self, template: &WorkoutTemplate) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        match inner.templates.iter_mut().find(|t| t.id == template.id) {
            Some(existing) => *existing = template.clone(),
            None => inner.templates.push(template.clone()),
        }
        Ok(())
    }

    async fn template(&self, template_id: &str) -> Result<Option<WorkoutTemplate>, FakeError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .templates
            .iter()
            .find(|t| t.id == template_id)
            .cloned())
    }

    async fn templates(&self) -> Result<Vec<WorkoutTemplate>, FakeError> {
        Ok(self.inner.lock().unwrap().templates.clone())
    }

    async fn delete_template(&self, template_id: &str) -> Result<bool, FakeError> {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.templates.len();
        inner.templates.retain(|t| t.id != template_id);
        Ok(inner.templates.len() < before)
    }

    async fn save_exercise(&self, exercise: &Exercise) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        match inner.exercises.iter_mut().find(|e| e.code == exercise.code) {
            Some(existing) => *existing = exercise.clone(),
            None => inner.exercises.push(exercise.clone()),
        }
        Ok(())
    }

    async fn exercises(&self) -> Result<ExerciseLibrary, FakeError> {
        Ok(ExerciseLibrary::new(self.inner.lock().unwrap().exercises.clone()))
    }

    async fn save_station(&self, station: &PhysicalStation) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        match inner.stations.iter_mut().find(|s| s.id == station.id) {
            Some(existing) => *existing = station.clone(),
            None => inner.stations.push(station.clone()),
        }
        Ok(())
    }

    async fn stations(&self) -> Result<StationMap, FakeError> {
        Ok(StationMap::new(self.inner.lock().unwrap().stations.clone()))
    }
}

/// A DRAFT training session with two athletes on the roster.
pub fn draft_session() -> LiveSession {
    LiveSession::new(
        Session::new_draft("s1", "THU 19:00 HYROX CLASS", SessionMode::Training),
        SessionConfig::new("s1"),
        START,
    )
    .with_athletes(vec![
        AthleteState::ready("a1", "CHEN YU-TING"),
        AthleteState::ready("a2", "LIN WEI"),
    ])
}

/// A class that is running: DRAFT -> READY -> RUNNING (ADR 0008).
pub fn running_session() -> LiveSession {
    let mut state = draft_session();
    state.session.mark_ready().expect("ready");
    state.session.start().expect("start");
    state
}

/// The fake is also the power control. One object recording everything keeps the fixtures
/// unchanged and the assertions in one place; a real deployment has two separate things.
impl api::Power for FakeStore {
    fn request(&self, action: api::PowerAction) -> Result<(), String> {
        let name = match action {
            api::PowerAction::Poweroff => "POWEROFF",
            api::PowerAction::Reboot => "REBOOT",
            api::PowerAction::RestartService => "RESTART_SERVICE",
        };
        self.inner.lock().unwrap().power_actions.push(name.to_string());
        Ok(())
    }
}

/// A hub that offers demo data, and a handle to see what was asked of it.
pub struct FakeDemo {
    calls: std::sync::Mutex<Vec<&'static str>>,
}

impl FakeDemo {
    pub fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}

impl api::Demo for FakeDemo {
    fn available(&self) -> bool {
        true
    }
    fn load(&self) -> Result<(), String> {
        self.calls.lock().unwrap().push("load");
        Ok(())
    }
    fn clear(&self) -> Result<(), String> {
        self.calls.lock().unwrap().push("clear");
        Ok(())
    }
}

/// A hub set up the way a test machine is: demo data available.
pub fn demo_hub(state: LiveSession) -> (Router, Arc<FakeDemo>) {
    let store = Arc::new(FakeStore::new());
    let demo = Arc::new(FakeDemo { calls: std::sync::Mutex::new(Vec::new()) });
    let hub = Hub::new(
        state,
        Arc::clone(&store),
        Arc::new(FixedClock(NOW)),
        250,
        16,
        "test",
        std::env::temp_dir().join("hyrox-test-backups"),
    )
    .with_power(Arc::clone(&store) as Arc<dyn api::Power>)
    .with_demo(Arc::clone(&demo) as Arc<dyn api::Demo>);
    (api::router(hub), demo)
}

/// A router over the given session and store, on a clock the test controls.
pub fn hub(state: LiveSession, store: Arc<FakeStore>) -> (Router, Arc<FakeStore>) {
    let hub = Hub::new(
        state,
        Arc::clone(&store),
        Arc::new(FixedClock(NOW)),
        250,
        16,
        "test",
        std::env::temp_dir().join("hyrox-test-backups"),
    )
    .with_power(Arc::clone(&store) as Arc<dyn api::Power>);
    (api::router(hub), store)
}

/// The common case: a running class and a freshly seeded store.
pub fn running() -> (Router, Arc<FakeStore>) {
    let store = Arc::new(FakeStore::new());
    hub(running_session(), store)
}

/// A class that has been run and closed, which is when the machine may be switched off.
pub fn completed_session() -> LiveSession {
    let mut state = running_session();
    state.session.complete().expect("complete");
    state
}

pub fn completed() -> (Router, Arc<FakeStore>) {
    hub(completed_session(), Arc::new(FakeStore::new()))
}

pub fn draft() -> (Router, Arc<FakeStore>) {
    let store = Arc::new(FakeStore::new());
    hub(draft_session(), store)
}

/// One request, driven straight into the router. No listener, no port, no network.
pub async fn call(router: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("the router always answers");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a readable body");
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

/// The same, when the answer is not JSON: status, content type, body as text.
/// The entry QR is an SVG, and "the hub draws it itself" is the property under test.
pub async fn raw(router: &Router, request: Request<Body>) -> (StatusCode, String, String) {
    let response = router.clone().oneshot(request).await.expect("the router always answers");
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a readable body");
    (status, content_type, String::from_utf8_lossy(&bytes).into_owned())
}

/// The same again, without turning the body into text. `from_utf8_lossy` replaces every
/// byte that is not valid UTF-8, so a PNG comes back a different length than it went in --
/// which is exactly what an image assertion needs to compare.
pub async fn raw_bytes(router: &Router, request: Request<Body>) -> (StatusCode, String, Vec<u8>) {
    let response = router.clone().oneshot(request).await.expect("the router always answers");
    let status = response.status();
    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a readable body");
    (status, content_type, bytes.to_vec())
}

pub fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("a valid request")
}

/// A write carrying an operator device name, which is the audit identity (ADR 0001 D1).
pub fn post(path: &str, device: &str, body: serde_json::Value) -> Request<Body> {
    write("POST", path, Some(device), body)
}

pub fn put(path: &str, device: &str, body: serde_json::Value) -> Request<Body> {
    write("PUT", path, Some(device), body)
}

pub fn del(path: &str, device: &str, body: serde_json::Value) -> Request<Body> {
    write("DELETE", path, Some(device), body)
}

/// A write with no identity at all. Must be refused, never defaulted (ADR 0001 D1).
pub fn anonymous(method: &str, path: &str, body: serde_json::Value) -> Request<Body> {
    write(method, path, None, body)
}

/// A binary upload: the venue's logo arrives as raw bytes, not as JSON.
pub fn upload(path: &str, device: &str, content_type: &str, bytes: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", content_type)
        .header(api::OPERATOR_HEADER, device)
        .body(Body::from(bytes))
        .expect("a valid request")
}

pub fn upload_anonymous(path: &str, content_type: &str, bytes: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", content_type)
        .body(Body::from(bytes))
        .expect("a valid request")
}

fn write(
    method: &str,
    path: &str,
    device: Option<&str>,
    body: serde_json::Value,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(device) = device {
        builder = builder.header(api::OPERATOR_HEADER, device);
    }
    builder
        .body(Body::from(body.to_string()))
        .expect("a valid request")
}
