//! In-memory fakes for the ports. No SQLite, no broker, no HTTP (CLAUDE.md 24).
//!
//! Each test binary compiles this module separately and uses a different part of it, so
//! unused helpers are expected here rather than a sign of dead code.
#![allow(dead_code)]

use application::{
    AuditEntry, AuditedResult, AuditedWrite, ClassAthlete, FinishOrigin, HubStore,
    InterpretedWrite, RawCommit, RawInterpretationContext, RawRead, SeenReader, StoredException,
    StoredRawRead,
};
use contract::CommitOutcome;
use domain::{
    AthleteState, BindingLedger, Exercise, ExerciseLibrary, Instant, Interpreted, MemberRef,
    PhysicalStation, ReaderRegistration, ReaderRegistry, Session, SessionConfig, StationMap,
    TagBinding, WorkoutTemplate,
};
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
    raw_contexts: Vec<(i64, RawInterpretationContext)>,
    /// The raw row each interpretation points at, which is what makes a claimed read
    /// unclaimable a second time.
    interpreted: Vec<(String, Option<i64>, Interpreted)>,
    /// Row ids of voided interpretations. Voided, never removed: the real table marks them
    /// too, because a correction trail that deleted its subject would prove nothing
    /// (CLAUDE.md 19, 20).
    voided: Vec<i64>,
    accepted: Vec<i64>,
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
    active_session_id: Option<String>,
}

#[derive(Default)]
pub struct FakeStore {
    inner: Mutex<Inner>,
    /// Simulates a store that cannot commit. The event is then not durable, so no ACK may
    /// exist for it (CLAUDE.md 15).
    pub fail_raw: bool,
    pub fail_interpreted: bool,
    pub fail_save_athlete_finish: bool,
    pub fail_save_session: bool,
    pub fail_record_audit: bool,
    pub fail_rebuild_athletes: bool,
    pub fail_exception_count: bool,
    pub fail_interpreted_event_count: bool,
}

impl FakeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn failing_raw() -> Self {
        Self {
            fail_raw: true,
            ..Self::default()
        }
    }

    pub fn failing_interpreted() -> Self {
        Self {
            fail_interpreted: true,
            ..Self::default()
        }
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
            inner.active_session_id = inner.sessions.last().map(|session| session.id.clone());
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

    pub fn with_interpreted_events(self, events: Vec<(String, Option<i64>, Interpreted)>) -> Self {
        self.inner.lock().unwrap().interpreted = events;
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
        self.inner
            .lock()
            .unwrap()
            .venue_settings
            .push((key.to_string(), value.to_string()));
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
        inner
            .configs
            .push(SessionConfig::new(session_id).with_finish_policy(policy));
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
        inner.calls.push(Call::Raw {
            tag_id: raw.tag_id.clone(),
            sequence: raw.sequence,
        });
        // Keyed the way `crates/storage` keys it: the round's idempotency key plus the tag,
        // because one round carries several tags (ADR 0014).
        let existing = inner.raw.iter().position(|r| {
            r.device_id == raw.device_id
                && r.boot_id == raw.boot_id
                && r.sequence == raw.sequence
                && r.tag_id == raw.tag_id
                && r.uptime_ms == raw.uptime_ms
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

    async fn commit_raw_with_context(
        &self,
        raw: &RawRead,
        context: &RawInterpretationContext,
    ) -> Result<RawCommit, FakeError> {
        if self.fail_raw {
            return Err(FakeError("raw commit failed"));
        }
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(Call::Raw {
            tag_id: raw.tag_id.clone(),
            sequence: raw.sequence,
        });
        let existing = inner.raw.iter().position(|r| {
            r.device_id == raw.device_id
                && r.boot_id == raw.boot_id
                && r.sequence == raw.sequence
                && r.tag_id == raw.tag_id
                && r.uptime_ms == raw.uptime_ms
        });
        let (id, inserted) = match existing {
            Some(i) => (i as i64 + 1, false),
            None => {
                inner.raw.push(raw.clone());
                (inner.raw.len() as i64, true)
            }
        };
        if !inner.raw_contexts.iter().any(|(raw_id, _)| *raw_id == id) {
            inner.raw_contexts.push((id, context.clone()));
        }
        Ok(RawCommit {
            raw_event_id: id,
            outcome: if inserted {
                CommitOutcome::Stored
            } else {
                CommitOutcome::AlreadyStored
            },
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

    async fn raw_event_has_interpretation(&self, raw_event_id: i64) -> Result<bool, FakeError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .interpreted
            .iter()
            .any(|(_, raw_id, _)| *raw_id == Some(raw_event_id)))
    }

    async fn save_session(&self, session: &Session, created_at: Instant) -> Result<(), FakeError> {
        if self.fail_save_session {
            return Err(FakeError("save_session failed"));
        }
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
        inner.active_session_id = if session.status.is_terminal() {
            None
        } else {
            Some(session.id.clone())
        };
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
        inner.calls.push(Call::Athlete {
            athlete_id: athlete_id.to_string(),
        });
        inner
            .athletes
            .push(AthleteState::ready(athlete_id, display_name));
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
        if self.fail_save_athlete_finish {
            return Err(FakeError("save_athlete_finish failed"));
        }
        let mut inner = self.inner.lock().unwrap();
        if let Some(a) = inner
            .athletes
            .iter_mut()
            .find(|a| a.athlete_id == athlete_id)
        {
            match finished_at {
                Some(at) => domain::finish(a, at),
                None => a.finished_at = None,
            }
        }
        Ok(())
    }

    async fn save_athlete_finishes(
        &self,
        _session_id: &str,
        finishes: &[(String, Instant)],
        _origin: FinishOrigin,
    ) -> Result<(), FakeError> {
        if self.fail_save_athlete_finish {
            return Err(FakeError("save_athlete_finish failed"));
        }
        let mut inner = self.inner.lock().unwrap();
        for (athlete_id, at) in finishes {
            if let Some(a) = inner
                .athletes
                .iter_mut()
                .find(|a| a.athlete_id == *athlete_id)
            {
                domain::finish(a, *at);
            }
        }
        Ok(())
    }

    async fn apply_audited(&self, write: AuditedWrite) -> Result<AuditedResult, FakeError> {
        if self.fail_record_audit {
            return Err(FakeError("record_audit failed"));
        }
        let mut inner = self.inner.lock().unwrap();
        let audit = match &write {
            AuditedWrite::Session { audit, .. }
            | AuditedWrite::SessionConfig { audit, .. }
            | AuditedWrite::Athlete { audit, .. }
            | AuditedWrite::Bindings { audit, .. }
            | AuditedWrite::ReaderSave { audit, .. }
            | AuditedWrite::ReaderDelete { audit, .. }
            | AuditedWrite::ExceptionAccept { audit, .. }
            | AuditedWrite::ExceptionVoid { audit, .. }
            | AuditedWrite::ExceptionReinterpret { audit, .. }
            | AuditedWrite::CreateClass { audit, .. }
            | AuditedWrite::TemplateSave { audit, .. }
            | AuditedWrite::TemplateCreate { audit, .. }
            | AuditedWrite::TemplateDelete { audit, .. }
            | AuditedWrite::VenueSetting { audit, .. }
            | AuditedWrite::VenueAssetSave { audit, .. }
            | AuditedWrite::VenueAssetDelete { audit, .. }
            | AuditedWrite::EndClass { audit, .. } => audit.clone(),
        };
        let result = match write {
            AuditedWrite::Session {
                session,
                created_at,
                ..
            } => {
                if self.fail_save_session {
                    return Err(FakeError("save_session failed"));
                }
                inner.calls.push(Call::Session {
                    status: format!("{:?}", session.status).to_uppercase(),
                    count: session.interpreted_event_count,
                });
                inner.created_at.get_or_insert(created_at);
                match inner.sessions.iter_mut().find(|s| s.id == session.id) {
                    Some(existing) => *existing = session,
                    None => inner.sessions.push(session),
                }
                inner.active_session_id = inner.sessions.last().map(|session| session.id.clone());
                AuditedResult::Applied(true)
            }
            AuditedWrite::CreateClass {
                session,
                config,
                athletes,
                ..
            } => {
                if inner.sessions.iter().any(|stored| stored.id == session.id) {
                    AuditedResult::Conflict
                } else {
                    inner.sessions.push(session.clone());
                    inner.configs.push(config);
                    for ClassAthlete {
                        athlete_id,
                        display_name,
                        bib,
                        member_id,
                    } in athletes
                    {
                        inner
                            .athletes
                            .push(AthleteState::ready(&athlete_id, &display_name));
                        inner.roster.push(SavedAthlete {
                            athlete_id,
                            display_name,
                            bib,
                            member_id,
                        });
                    }
                    inner.active_session_id = Some(session.id);
                    AuditedResult::Applied(true)
                }
            }
            AuditedWrite::SessionConfig { config, .. } => {
                match inner
                    .configs
                    .iter_mut()
                    .find(|c| c.session_id == config.session_id)
                {
                    Some(existing) => *existing = config,
                    None => inner.configs.push(config),
                }
                AuditedResult::Applied(true)
            }
            AuditedWrite::TemplateSave { template, .. } => {
                if inner
                    .templates
                    .iter()
                    .find(|stored| stored.id == template.id)
                    .is_some_and(|stored| !stored.is_editable())
                {
                    AuditedResult::Conflict
                } else {
                    match inner
                        .templates
                        .iter_mut()
                        .find(|stored| stored.id == template.id)
                    {
                        Some(existing) => *existing = template,
                        None => inner.templates.push(template),
                    }
                    AuditedResult::Applied(true)
                }
            }
            AuditedWrite::TemplateCreate { template, .. } => {
                if inner
                    .templates
                    .iter()
                    .any(|stored| stored.id == template.id)
                {
                    AuditedResult::Conflict
                } else {
                    inner.templates.push(template);
                    AuditedResult::Applied(true)
                }
            }
            AuditedWrite::TemplateDelete { template_id, .. } => {
                let before = inner.templates.len();
                inner
                    .templates
                    .retain(|template| template.id != template_id);
                AuditedResult::Applied(inner.templates.len() != before)
            }
            AuditedWrite::VenueSetting { key, value, .. } => {
                inner
                    .venue_settings
                    .retain(|(stored_key, _)| stored_key != &key);
                inner.venue_settings.push((key, value));
                AuditedResult::Applied(true)
            }
            AuditedWrite::VenueAssetSave {
                key,
                media_type,
                bytes,
                ..
            } => {
                inner
                    .venue_assets
                    .retain(|(stored_key, _)| stored_key != &key);
                inner
                    .venue_assets
                    .push((key, application::VenueAsset { media_type, bytes }));
                AuditedResult::Applied(true)
            }
            AuditedWrite::VenueAssetDelete { key, .. } => {
                inner
                    .venue_assets
                    .retain(|(stored_key, _)| stored_key != &key);
                AuditedResult::Applied(true)
            }
            AuditedWrite::Athlete {
                session_id: _,
                athlete_id,
                display_name,
                bib,
                member_id,
                ..
            } => {
                inner
                    .athletes
                    .push(AthleteState::ready(&athlete_id, &display_name));
                inner.roster.push(SavedAthlete {
                    athlete_id,
                    display_name,
                    bib,
                    member_id,
                });
                AuditedResult::Applied(true)
            }
            AuditedWrite::Bindings {
                bindings, claim, ..
            } => {
                for binding in bindings {
                    if let Some(existing) = inner.bindings.iter_mut().find(|b| {
                        b.session_id == binding.session_id
                            && b.tag_id == binding.tag_id
                            && b.bound_at == binding.bound_at
                    }) {
                        existing.unbound_at = binding.unbound_at;
                    } else {
                        inner.bindings.push(binding);
                    }
                }
                if let Some(claim) = claim {
                    let interpreted_ids: Vec<i64> = inner
                        .interpreted
                        .iter()
                        .filter_map(|(_, raw_id, _)| *raw_id)
                        .collect();
                    let candidate_ids: Vec<i64> = inner
                        .raw_contexts
                        .iter()
                        .filter_map(|(raw_id, context)| {
                            let raw = inner.raw.get(*raw_id as usize - 1)?;
                            (context.session_id == claim.session_id
                                && context.athlete_id.is_none()
                                && raw.tag_id.eq_ignore_ascii_case(&claim.tag_id)
                                && raw.detected_at >= claim.since
                                && raw.detected_at <= claim.until
                                && !interpreted_ids.contains(raw_id))
                            .then_some(*raw_id)
                        })
                        .collect();
                    for raw_id in candidate_ids {
                        if let Some((_, context)) =
                            inner.raw_contexts.iter_mut().find(|(id, _)| *id == raw_id)
                        {
                            context.athlete_id = Some(claim.athlete_id.clone());
                            context.bound_session_id = Some(claim.session_id.clone());
                            context.on_roster = true;
                        }
                    }
                }
                AuditedResult::Applied(true)
            }
            AuditedWrite::ReaderSave { registration, .. } => {
                match inner.readers.iter_mut().find(|r| r.key == registration.key) {
                    Some(existing) => *existing = registration,
                    None => inner.readers.push(registration),
                }
                AuditedResult::Applied(true)
            }
            AuditedWrite::ReaderDelete {
                device_id,
                reader_id,
                ..
            } => {
                let before = inner.readers.len();
                inner.readers.retain(|r| {
                    r.key.device_id.as_str() != device_id || r.key.reader_id.as_str() != reader_id
                });
                if inner.readers.len() != before {
                    inner
                        .deleted_readers
                        .push((device_id.clone(), reader_id.clone()));
                }
                AuditedResult::Applied(inner.readers.len() != before)
            }
            AuditedWrite::ExceptionAccept {
                interpreted_event_id,
                ..
            } => {
                if !matches!(
                    inner
                        .interpreted
                        .get(interpreted_event_id as usize - 1)
                        .map(|(_, _, event)| event),
                    Some(Interpreted::Exception { .. })
                ) || inner.voided.contains(&interpreted_event_id)
                {
                    AuditedResult::Applied(false)
                } else {
                    inner.accepted.push(interpreted_event_id);
                    AuditedResult::Applied(true)
                }
            }
            AuditedWrite::ExceptionVoid {
                interpreted_event_id,
                ..
            } => {
                if inner
                    .interpreted
                    .get(interpreted_event_id as usize - 1)
                    .is_none()
                    || inner.voided.contains(&interpreted_event_id)
                {
                    AuditedResult::Applied(false)
                } else {
                    inner.voided.push(interpreted_event_id);
                    AuditedResult::Applied(true)
                }
            }
            AuditedWrite::ExceptionReinterpret {
                interpreted_event_id,
                replacement,
                ..
            } => {
                if !matches!(
                    inner
                        .interpreted
                        .get(interpreted_event_id as usize - 1)
                        .map(|(_, _, event)| event),
                    Some(Interpreted::Exception { .. })
                ) || inner.voided.contains(&interpreted_event_id)
                {
                    AuditedResult::Applied(false)
                } else {
                    inner.voided.push(interpreted_event_id);
                    inner.interpreted.push((
                        replacement.athlete_id,
                        replacement.raw_event_id,
                        replacement.event,
                    ));
                    AuditedResult::Interpreted(inner.interpreted.len() as i64)
                }
            }
            AuditedWrite::EndClass {
                session, finishes, ..
            } => {
                if self.fail_save_session || self.fail_save_athlete_finish {
                    return Err(FakeError("end_class persistence failed"));
                }
                for (athlete_id, at) in finishes {
                    if let Some(athlete) = inner
                        .athletes
                        .iter_mut()
                        .find(|athlete| athlete.athlete_id == athlete_id)
                    {
                        domain::finish(athlete, at);
                    }
                }
                match inner.sessions.iter_mut().find(|s| s.id == session.id) {
                    Some(existing) => *existing = session,
                    None => inner.sessions.push(session),
                }
                inner.active_session_id = inner
                    .sessions
                    .last()
                    .filter(|session| !session.status.is_terminal())
                    .map(|session| session.id.clone());
                AuditedResult::Applied(true)
            }
        };
        inner.calls.push(Call::Audit {
            action: audit.action.clone(),
        });
        inner.audits.push(audit);
        Ok(result)
    }

    async fn active_session(&self) -> Result<Option<Session>, FakeError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .active_session_id
            .as_ref()
            .and_then(|id| inner.sessions.iter().find(|session| &session.id == id))
            .cloned())
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
        Ok(inner
            .roster
            .iter()
            .map(|a| (a.athlete_id.clone(), a.bib))
            .collect())
    }

    async fn rebuild_athletes(&self, _session_id: &str) -> Result<Vec<AthleteState>, FakeError> {
        if self.fail_rebuild_athletes {
            return Err(FakeError("rebuild_athletes failed"));
        }
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
                let event = match event {
                    Interpreted::Entered { .. }
                        if state.station_state == domain::StationState::Inside =>
                    {
                        continue
                    }
                    Interpreted::Entered { station, at, .. } => Interpreted::Entered {
                        station: station.clone(),
                        at: *at,
                        transition: state.last_exit_at.map(|previous| at.since(previous)),
                        started_timing: state.status == domain::AthleteStatus::Ready,
                    },
                    other => other.clone(),
                };
                domain::apply(state, &event);
            }
        }
        Ok(out)
    }

    async fn session_created_at(&self, _session_id: &str) -> Result<Option<Instant>, FakeError> {
        Ok(self.inner.lock().unwrap().created_at)
    }

    async fn exception_count(&self, _session_id: &str) -> Result<usize, FakeError> {
        if self.fail_exception_count {
            return Err(FakeError("exception_count failed"));
        }
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .interpreted
            .iter()
            .enumerate()
            .filter(|(i, (_, _, e))| {
                matches!(e, Interpreted::Exception { .. })
                    && !inner.voided.contains(&(*i as i64 + 1))
                    && !inner.accepted.contains(&(*i as i64 + 1))
            })
            .count())
    }

    async fn interpreted_event_count(&self, _session_id: &str) -> Result<u64, FakeError> {
        if self.fail_interpreted_event_count {
            return Err(FakeError("interpreted_event_count failed"));
        }
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .interpreted
            .iter()
            .enumerate()
            .filter(|(i, (_, _, event))| {
                !matches!(event, Interpreted::Exception { .. })
                    && !inner.voided.contains(&(*i as i64 + 1))
            })
            .count() as u64)
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

    async fn acknowledge_interpreted(
        &self,
        interpreted_event_id: i64,
        _at: Instant,
        _operator: &str,
        _reason: Option<&str>,
    ) -> Result<bool, FakeError> {
        let mut inner = self.inner.lock().unwrap();
        let open = interpreted_event_id >= 1
            && interpreted_event_id as usize <= inner.interpreted.len()
            && !inner.voided.contains(&interpreted_event_id);
        if open {
            inner.accepted.push(interpreted_event_id);
        }
        Ok(open)
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
        inner
            .venue_settings
            .push((key.to_string(), value.to_string()));
        Ok(())
    }

    async fn venue_asset(&self, key: &str) -> Result<Option<application::VenueAsset>, FakeError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .venue_assets
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, a)| a.clone()))
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
            application::VenueAsset {
                media_type: media_type.to_string(),
                bytes: bytes.to_vec(),
            },
        ));
        Ok(())
    }

    async fn delete_venue_asset(&self, key: &str) -> Result<(), FakeError> {
        self.inner
            .lock()
            .unwrap()
            .venue_assets
            .retain(|(k, _)| k != key);
        Ok(())
    }

    async fn record_audit(&self, entry: &AuditEntry) -> Result<(), FakeError> {
        if self.fail_record_audit {
            return Err(FakeError("record_audit failed"));
        }
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(Call::Audit {
            action: entry.action.clone(),
        });
        inner.audits.push(entry.clone());
        Ok(())
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

    /// Append or close, never re-attribute: the same contract the real table enforces.
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
                presence: r.presence,
                context: inner
                    .raw_contexts
                    .iter()
                    .find(|(raw_id, _)| *raw_id == i as i64 + 1)
                    .map(|(_, c)| c.clone()),
            })
            .collect();
        out.sort_by_key(|r| (r.detected_at, r.raw_event_id));
        Ok(out)
    }

    async fn pending_raw_reads(&self, session_id: &str) -> Result<Vec<StoredRawRead>, FakeError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .raw_contexts
            .iter()
            .filter(|(raw_id, context)| {
                context.session_id == session_id
                    && !inner
                        .interpreted
                        .iter()
                        .any(|(_, raw, _)| *raw == Some(*raw_id))
            })
            .filter_map(|(raw_id, context)| {
                inner
                    .raw
                    .get(*raw_id as usize - 1)
                    .map(|raw| StoredRawRead {
                        raw_event_id: *raw_id,
                        device_id: raw.device_id.clone(),
                        reader_id: raw.reader_id.clone(),
                        detected_at: raw.detected_at,
                        presence: raw.presence,
                        context: Some(context.clone()),
                    })
            })
            .collect())
    }

    async fn raw_interpretation_context(
        &self,
        raw_event_id: i64,
    ) -> Result<Option<RawInterpretationContext>, FakeError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .raw_contexts
            .iter()
            .find(|(id, _)| *id == raw_event_id)
            .map(|(_, context)| context.clone()))
    }

    async fn interpretations(
        &self,
        _session_id: &str,
    ) -> Result<Vec<application::StoredInterpretation>, FakeError> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .interpreted
            .iter()
            .enumerate()
            .filter(|(index, _)| !inner.voided.contains(&(*index as i64 + 1)))
            .map(
                |(index, (athlete_id, raw_event_id, event))| application::StoredInterpretation {
                    interpreted_event_id: index as i64 + 1,
                    athlete_id: athlete_id.clone(),
                    raw_event_id: *raw_event_id,
                    event: event.clone(),
                },
            )
            .collect())
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
        Ok(ExerciseLibrary::new(
            self.inner.lock().unwrap().exercises.clone(),
        ))
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
        Interpreted::Started { .. } => "STARTED".into(),
        Interpreted::Crossed { .. } => "CROSSED".into(),
        Interpreted::VisitWithdrawn { .. } => "VISIT_WITHDRAWN".into(),
        Interpreted::ExitWithdrawn { .. } => "EXIT_WITHDRAWN".into(),
        Interpreted::Passed { .. } => "PASSED".into(),
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
