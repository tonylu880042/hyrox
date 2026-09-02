//! In-memory fakes for the ports. No SQLite, no broker, no HTTP (CLAUDE.md 24).
//!
//! Each test binary compiles this module separately and uses a different part of it, so
//! unused helpers are expected here rather than a sign of dead code.
#![allow(dead_code)]

use application::{
    AuditEntry, HubStore, InterpretedWrite, RawCommit, RawRead, StoredException, StoredRawRead, SeenReader,
};
use domain::{
    AthleteState, BindingLedger, Exercise, ExerciseLibrary, Instant, Interpreted, MemberRef,
    PhysicalStation, ReaderRegistration, ReaderRegistry, Session, SessionConfig, StationMap,
    TagBinding, WorkoutTemplate,
};
use contract::CommitOutcome;
use std::sync::Mutex;

/// What the store was asked to do, in order. The ingestion contract is as much about
/// ordering as about content -- raw before interpreted, ACK after the commit -- so the
/// order has to be observable (ADR 0002).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Raw { tag_id: String, sequence: i64 },
    Interpreted { athlete_id: String, kind: String },
    Session { status: String, count: u64 },
    Athlete { athlete_id: String },
    Audit { action: String },
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

#[derive(Debug, PartialEq, Eq)]
pub struct FakeError(pub &'static str);

#[derive(Default)]
struct Inner {
    calls: Vec<Call>,
    /// Whole reads, not just their keys: retroactive claim reads them back out (ADR 0001 D3),
    /// so the fake has to be able to answer that query. Index + 1 is the row id.
    raw: Vec<RawRead>,
    /// The raw row each interpretation points at, which is what makes a claimed read
    /// unclaimable a second time.
    interpreted: Vec<(String, Option<i64>, Interpreted)>,
    /// Row ids of voided interpretations. Voided, never removed: the real table marks them
    /// too, because a correction trail that deleted its subject would prove nothing
    /// (CLAUDE.md 19, 20).
    voided: Vec<i64>,
    audits: Vec<AuditEntry>,
    seen_readers: Vec<SeenReader>,
    venue_settings: Vec<(String, String)>,
    deleted_readers: Vec<(String, String)>,
    venue_assets: Vec<(String, application::VenueAsset)>,
    backups: Vec<std::path::PathBuf>,
    sessions: Vec<Session>,
    configs: Vec<SessionConfig>,
    readers: Vec<ReaderRegistration>,
    bindings: Vec<TagBinding>,
    athletes: Vec<AthleteState>,
    roster: Vec<SavedAthlete>,
    created_at: Option<Instant>,
    templates: Vec<WorkoutTemplate>,
    exercises: Vec<Exercise>,
    stations: Vec<PhysicalStation>,
}

#[derive(Default)]
pub struct FakeStore {
    inner: Mutex<Inner>,
    /// Simulates a store that cannot commit. The event is then not durable, so no ACK may
    /// exist for it (CLAUDE.md 15).
    pub fail_raw: bool,
    pub fail_interpreted: bool,
}

impl FakeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn failing_raw() -> Self {
        Self { fail_raw: true, ..Self::default() }
    }

    pub fn failing_interpreted() -> Self {
        Self { fail_interpreted: true, ..Self::default() }
    }

    /// Seed a session as if a previous run had left it behind (CLAUDE.md 21).
    ///
    /// Its configuration is seeded too, because arming a session stores both: a session row
    /// with no configuration means a database written before ADR 0004, which
    /// [`Self::with_unconfigured_session`] is for.
    pub fn with_session(self, session: Session, created_at: Instant) -> Self {
        let config = SessionConfig::new(&session.id);
        self.with_session_config(session, created_at, Some(config))
    }

    /// A session left behind by a build that did not store configuration.
    pub fn with_unconfigured_session(self, session: Session, created_at: Instant) -> Self {
        self.with_session_config(session, created_at, None)
    }

    pub fn with_session_config(
        self,
        session: Session,
        created_at: Instant,
        config: Option<SessionConfig>,
    ) -> Self {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.sessions.push(session);
            inner.configs.extend(config);
            inner.created_at = Some(created_at);
        }
        self
    }

    pub fn saved_bindings(&self) -> Vec<TagBinding> {
        self.inner.lock().unwrap().bindings.clone()
    }

    pub fn saved_readers(&self) -> Vec<ReaderRegistration> {
        self.inner.lock().unwrap().readers.clone()
    }

    pub fn saved_configs(&self) -> Vec<SessionConfig> {
        self.inner.lock().unwrap().configs.clone()
    }

    pub fn with_rebuilt_athletes(self, athletes: Vec<AthleteState>) -> Self {
        self.inner.lock().unwrap().athletes = athletes;
        self
    }

    pub fn calls(&self) -> Vec<Call> {
        self.inner.lock().unwrap().calls.clone()
    }

    pub fn raw_count(&self) -> usize {
        self.inner.lock().unwrap().raw.len()
    }

    pub fn interpreted(&self) -> Vec<(String, Interpreted)> {
        self.inner
            .lock()
            .unwrap()
            .interpreted
            .iter()
            .map(|(a, _, e)| (a.clone(), e.clone()))
            .collect()
    }

    /// Just the events, for comparing one run against another (ADR 0001 D3 equivalence).
    pub fn interpreted_events(&self) -> Vec<Interpreted> {
        self.inner
            .lock()
            .unwrap()
            .interpreted
            .iter()
            .map(|(_, _, e)| e.clone())
            .collect()
    }

    /// Readers the hub has heard from, whether configured or not (M6 settings screen).
    pub fn with_reader_keys_seen(self, seen: Vec<(String, String, Instant, i64)>) -> Self {
        self.inner.lock().unwrap().seen_readers = seen
            .into_iter()
            .map(|(device_id, reader_id, last_seen, reads)| SeenReader {
                device_id,
                reader_id,
                last_seen,
                reads,
            })
            .collect();
        self
    }

    /// A venue setting already stored, including one that makes no sense.
    pub fn with_venue_setting(self, key: &str, value: &str) -> Self {
        self.inner.lock().unwrap().venue_settings.push((key.to_string(), value.to_string()));
        self
    }

    /// Readers the map was asked to forget, in order.
    pub fn deleted_readers(&self) -> Vec<(String, String)> {
        self.inner.lock().unwrap().deleted_readers.clone()
    }

    pub fn audits(&self) -> Vec<AuditEntry> {
        self.inner.lock().unwrap().audits.clone()
    }

    pub fn templates_held(&self) -> Vec<WorkoutTemplate> {
        self.inner.lock().unwrap().templates.clone()
    }

    /// Puts a template in the store without going through a use case, so a test can start
    /// from a library that already exists.
    pub fn seed_template(&self, template: WorkoutTemplate) {
        self.inner.lock().unwrap().templates.push(template);
    }

    pub fn saved_sessions(&self) -> Vec<Session> {
        self.inner.lock().unwrap().sessions.clone()
    }
}

impl FakeStore {
    /// A stored, finished session, for the ranking tests. Athletes are given a finish time
    /// directly rather than driven through the engine: what is under test is how results
    /// *order* finishers, not how somebody comes to be finished.
    pub fn seed_finished_session(
        &self,
        session_id: &str,
        policy: domain::FinishPolicy,
        finishes: &[(&str, Option<i64>)],
    ) {
        let mut inner = self.inner.lock().unwrap();
        let mut session =
            Session::new_draft(session_id, "OPEN 2026", domain::SessionMode::Competition);
        session.mark_ready().unwrap();
        session.start().unwrap();
        inner.sessions.push(session);
        inner.configs.push(SessionConfig::new(session_id).with_finish_policy(policy));
        inner.athletes = finishes
            .iter()
            .map(|(name, at)| {
                let mut a = AthleteState::ready(*name, *name);
                if let Some(at) = at {
                    a.status = domain::AthleteStatus::Active;
                    a.started_at = Some(Instant(0));
                    domain::finish(&mut a, Instant(*at));
                }
                a
            })
            .collect();
    }

    pub fn saved_athletes(&self) -> Vec<SavedAthlete> {
        self.inner.lock().unwrap().roster.clone()
    }
}

impl HubStore for FakeStore {
    type Error = FakeError;

    async fn commit_raw(&self, raw: &RawRead) -> Result<RawCommit, FakeError> {
        if self.fail_raw {
            return Err(FakeError("raw commit failed"));
        }
        let mut inner = self.inner.lock().unwrap();
        inner
            .calls
            .push(Call::Raw { tag_id: raw.tag_id.clone(), sequence: raw.sequence });
        // Keyed the way `crates/storage` keys it: the round's idempotency key plus the tag,
        // because one round carries several tags (ADR 0014).
        let existing = inner.raw.iter().position(|r| {
            r.device_id == raw.device_id
                && r.boot_id == raw.boot_id
                && r.sequence == raw.sequence
                && r.tag_id == raw.tag_id
        });
        if let Some(i) = existing {
            return Ok(RawCommit {
                raw_event_id: i as i64 + 1,
                outcome: CommitOutcome::AlreadyStored,
            });
        }
        inner.raw.push(raw.clone());
        Ok(RawCommit {
            raw_event_id: inner.raw.len() as i64,
            outcome: CommitOutcome::Stored,
        })
    }

    async fn commit_interpreted(&self, w: InterpretedWrite<'_>) -> Result<i64, FakeError> {
        if self.fail_interpreted {
            return Err(FakeError("interpreted commit failed"));
        }
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(Call::Interpreted {
            athlete_id: w.athlete_id.to_string(),
            kind: kind_of(w.event),
        });
        inner
            .interpreted
            .push((w.athlete_id.to_string(), w.raw_event_id, w.event.clone()));
        Ok(inner.interpreted.len() as i64)
    }

    async fn save_session(&self, session: &Session, created_at: Instant) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(Call::Session {
            status: format!("{:?}", session.status).to_uppercase(),
            count: session.interpreted_event_count,
        });
        inner.created_at.get_or_insert(created_at);
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
        inner.calls.push(Call::Athlete { athlete_id: athlete_id.to_string() });
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

    /// Replays the non-voided interpretations over the seeded roster, like the real store.
    /// Seeded athletes with no interpretations come back untouched, which is what the
    /// recovery tests rely on.
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
        if inner.interpreted.is_empty() {
            return Ok(inner.athletes.clone());
        }
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
        Ok(self.inner.lock().unwrap().created_at)
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

    /// Row ids are index + 1, exactly as `commit_interpreted` hands them out, and voided
    /// rows are filtered here for the same reason the real table filters them.
    async fn exceptions(&self, _session_id: &str) -> Result<Vec<StoredException>, FakeError> {
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
        let exists = interpreted_event_id >= 1
            && interpreted_event_id as usize <= inner.interpreted.len();
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
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(Call::Audit { action: entry.action.clone() });
        inner.audits.push(entry.clone());
        Ok(())
    }

    async fn save_session_config(&self, config: &SessionConfig) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        match inner.configs.iter_mut().find(|c| c.session_id == config.session_id) {
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

    /// Append or close, never re-attribute: the same contract the real table enforces.
    async fn save_binding(&self, binding: &TagBinding) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        let key = |b: &TagBinding| {
            (b.session_id.clone(), b.tag_id.clone(), b.bound_at)
        };
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

fn kind_of(e: &Interpreted) -> String {
    match e {
        Interpreted::Entered { .. } => "ENTERED".into(),
        Interpreted::Exited { .. } => "EXITED".into(),
        Interpreted::Exception { reason, .. } => format!("EXCEPTION/{reason:?}"),
    }
}

/// A 健身管 stand-in that answers from a fixed table, so the "membership never gates
/// timing" rule can be exercised without knowing the real contract.
pub struct FakeDirectory(pub Vec<MemberRef>);

impl application::MemberDirectory for FakeDirectory {
    type Error = FakeError;

    async fn lookup(&self, member_id: &str) -> Result<Option<MemberRef>, FakeError> {
        Ok(self.0.iter().find(|m| m.member_id == member_id).cloned())
    }
}

/// Puts a device's journal report into a live session, for the maintenance-window tests
/// (ADR 0009). Goes through the real use case, so a report the hub could not actually
/// receive is not something these tests can assert about.
pub fn note_backlog(
    state: &mut application::LiveSession,
    device_id: &domain::DeviceId,
    pending_events: u64,
    at: Instant,
) {
    application::note_device_status(
        state,
        application::DeviceReport {
            device_id: device_id.clone(),
            boot_id: 1,
            pending_events,
            journal_capacity: 10_000,
            warning: None,
        },
        at,
    );
}
