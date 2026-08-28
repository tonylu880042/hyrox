//! In-memory fakes for the ports. No SQLite, no broker, no HTTP (CLAUDE.md 24).
//!
//! Each test binary compiles this module separately and uses a different part of it, so
//! unused helpers are expected here rather than a sign of dead code.
#![allow(dead_code)]

use application::{AuditEntry, HubStore, InterpretedWrite, RawCommit, RawRead};
use domain::{AthleteState, Instant, Interpreted, MemberRef, Session};
use mqtt::CommitOutcome;
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

#[derive(Debug, PartialEq, Eq)]
pub struct FakeError(pub &'static str);

#[derive(Default)]
struct Inner {
    calls: Vec<Call>,
    raw: Vec<(String, i64, i64)>, // device_id, boot_id, sequence -> index + 1 is the row id
    interpreted: Vec<(String, Interpreted)>,
    audits: Vec<AuditEntry>,
    sessions: Vec<Session>,
    athletes: Vec<AthleteState>,
    created_at: Option<Instant>,
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
    pub fn with_session(self, session: Session, created_at: Instant) -> Self {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.sessions.push(session);
            inner.created_at = Some(created_at);
        }
        self
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
        self.inner.lock().unwrap().interpreted.clone()
    }

    pub fn audits(&self) -> Vec<AuditEntry> {
        self.inner.lock().unwrap().audits.clone()
    }

    pub fn saved_sessions(&self) -> Vec<Session> {
        self.inner.lock().unwrap().sessions.clone()
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
        let key = (raw.device_id.clone(), raw.boot_id, raw.sequence);
        if let Some(i) = inner.raw.iter().position(|k| k == &key) {
            return Ok(RawCommit {
                raw_event_id: i as i64 + 1,
                outcome: CommitOutcome::AlreadyStored,
            });
        }
        inner.raw.push(key);
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
            .push((w.athlete_id.to_string(), w.event.clone()));
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
        _bib: i64,
    ) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(Call::Athlete { athlete_id: athlete_id.to_string() });
        inner.athletes.push(AthleteState::ready(athlete_id, display_name));
        Ok(())
    }

    async fn active_session(&self) -> Result<Option<Session>, FakeError> {
        Ok(self.inner.lock().unwrap().sessions.first().cloned())
    }

    async fn rebuild_athletes(&self, _session_id: &str) -> Result<Vec<AthleteState>, FakeError> {
        Ok(self.inner.lock().unwrap().athletes.clone())
    }

    async fn session_created_at(&self, _session_id: &str) -> Result<Option<Instant>, FakeError> {
        Ok(self.inner.lock().unwrap().created_at)
    }

    async fn exception_count(&self, _session_id: &str) -> Result<usize, FakeError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .interpreted
            .iter()
            .filter(|(_, e)| matches!(e, Interpreted::Exception { .. }))
            .count())
    }

    async fn record_audit(&self, entry: &AuditEntry) -> Result<(), FakeError> {
        let mut inner = self.inner.lock().unwrap();
        inner.calls.push(Call::Audit { action: entry.action.clone() });
        inner.audits.push(entry.clone());
        Ok(())
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
