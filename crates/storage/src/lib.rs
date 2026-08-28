//! SQLite persistence for the Central Hub (CLAUDE.md 19, 21).
//!
//! Raw events are append-only and never modified. Athlete state is not stored: it is rebuilt
//! by replaying the non-voided interpreted events, so a restart cannot disagree with the log.

mod hub_store;

use application::{AuditEntry, StoredRawRead};
use domain::{
    AthleteState, BindingLedger, ExceptionReason, Instant, Interpreted, ReaderKey, ReaderMode,
    ReaderRegistration, ReaderRegistry, Session, SessionConfig, SessionMode, SessionStatus,
    TagBinding, TagId,
};
use sqlx::{sqlite::SqliteConnectOptions, Row, SqlitePool};
use std::str::FromStr;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("stored row is not valid: {0}")]
    Corrupt(String),
}

/// A raw reader event exactly as it arrived from the edge (CLAUDE.md 16).
#[derive(Clone, Debug)]
pub struct RawEvent {
    pub device_id: String,
    pub reader_id: String,
    pub boot_id: i64,
    pub sequence: i64,
    pub tag_id: String,
    pub detected_at: Instant,
    pub received_at: Instant,
}

pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Opens (creating if needed) the database at `path` and applies migrations.
    /// WAL keeps readers unblocked while ingestion writes (CLAUDE.md 19).
    pub async fn open(path: &str) -> Result<Self, StoreError> {
        let opts = SqliteConnectOptions::from_str(path)
            .map_err(StoreError::Db)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .foreign_keys(true);
        let pool = SqlitePool::connect_with(opts).await?;
        sqlx::migrate!("../../migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn open_in_memory() -> Result<Self, StoreError> {
        Self::open("sqlite::memory:").await
    }

    pub async fn save_session(&self, s: &Session, created_at: Instant) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sessions (id, name, mode, status, interpreted_event_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                status = excluded.status,
                interpreted_event_count = excluded.interpreted_event_count",
        )
        .bind(&s.id)
        .bind(&s.name)
        .bind(mode_str(s.mode))
        .bind(status_str(s.status))
        .bind(s.interpreted_event_count as i64)
        .bind(created_at.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn save_athlete(
        &self,
        session_id: &str,
        athlete_id: &str,
        display_name: &str,
        bib: i64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO session_athletes (session_id, athlete_id, display_name, bib)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id, athlete_id) DO UPDATE SET
                display_name = excluded.display_name, bib = excluded.bib",
        )
        .bind(session_id)
        .bind(athlete_id)
        .bind(display_name)
        .bind(bib)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stores a raw event, returning its id. A redelivery of the same
    /// `device_id + boot_id + sequence` returns the existing id instead of inserting again:
    /// duplicate delivery is allowed, duplicate processing is not (CLAUDE.md 16).
    pub async fn save_raw(&self, e: &RawEvent) -> Result<(i64, bool), StoreError> {
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM raw_events WHERE device_id = ?1 AND boot_id = ?2 AND sequence = ?3",
        )
        .bind(&e.device_id)
        .bind(e.boot_id)
        .bind(e.sequence)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(id) = existing {
            return Ok((id, false));
        }
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO raw_events
                (device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) RETURNING id",
        )
        .bind(&e.device_id)
        .bind(&e.reader_id)
        .bind(e.boot_id)
        .bind(e.sequence)
        .bind(&e.tag_id)
        .bind(e.detected_at.0)
        .bind(e.received_at.0)
        .fetch_one(&self.pool)
        .await?;
        Ok((id, true))
    }

    pub async fn save_interpreted(
        &self,
        session_id: &str,
        athlete_id: &str,
        raw_event_id: Option<i64>,
        event: &Interpreted,
    ) -> Result<i64, StoreError> {
        let (kind, station, detected_at, transition_ms, started, reason) = match event {
            Interpreted::Entered { station, at, transition, started_timing } => (
                "ENTERED",
                Some(station.clone()),
                at.0,
                transition.map(|d| d.millis()),
                *started_timing,
                None,
            ),
            Interpreted::Exited { station, at } => {
                ("EXITED", Some(station.clone()), at.0, None, false, None)
            }
            Interpreted::Exception { reason, at } => {
                ("EXCEPTION", None, at.0, None, false, Some(reason_str(reason)))
            }
        };
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO interpreted_events
                (session_id, athlete_id, raw_event_id, kind, station, detected_at,
                 transition_ms, started_timing, exception_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id",
        )
        .bind(session_id)
        .bind(athlete_id)
        .bind(raw_event_id)
        .bind(kind)
        .bind(station)
        .bind(detected_at)
        .bind(transition_ms)
        .bind(started as i64)
        .bind(reason)
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// The session to resume after a restart: the ARMED one, else the most recent (CLAUDE.md 21).
    pub async fn active_session(&self) -> Result<Option<Session>, StoreError> {
        let row = sqlx::query(
            "SELECT id, name, mode, status, interpreted_event_count FROM sessions
             ORDER BY (status = 'ARMED') DESC, created_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            Ok(Session {
                id: r.get("id"),
                name: r.get("name"),
                mode: parse_mode(r.get::<String, _>("mode").as_str())?,
                status: parse_status(r.get::<String, _>("status").as_str())?,
                interpreted_event_count: r.get::<i64, _>("interpreted_event_count") as u64,
            })
        })
        .transpose()
    }

    /// Rebuilds every athlete in the session by replaying the non-voided interpreted events
    /// in `detected_at` order (CLAUDE.md 21). Voided rows are excluded, which is how an
    /// operator correction reaches the derived values (CLAUDE.md 20).
    pub async fn rebuild_athletes(
        &self,
        session_id: &str,
    ) -> Result<Vec<AthleteState>, StoreError> {
        let roster = sqlx::query(
            "SELECT athlete_id, display_name FROM session_athletes
             WHERE session_id = ?1 ORDER BY bib",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        let rows = sqlx::query(
            "SELECT athlete_id, kind, station, detected_at, transition_ms,
                    started_timing, exception_reason
             FROM interpreted_events
             WHERE session_id = ?1 AND voided_at IS NULL
             ORDER BY detected_at, id",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        let mut out: Vec<AthleteState> = roster
            .iter()
            .map(|r| {
                AthleteState::ready(
                    r.get::<String, _>("athlete_id"),
                    r.get::<String, _>("display_name"),
                )
            })
            .collect();

        for r in &rows {
            let aid: String = r.get("athlete_id");
            let Some(state) = out.iter_mut().find(|a| a.athlete_id == aid) else {
                continue; // event for someone no longer on the roster
            };
            domain::apply(state, &row_to_interpreted(r)?);
        }
        Ok(out)
    }

    pub async fn void_interpreted(
        &self,
        id: i64,
        at: Instant,
        operator: &str,
        reason: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE interpreted_events
             SET voided_at = ?2, voided_by = ?3, void_reason = ?4
             WHERE id = ?1",
        )
        .bind(id)
        .bind(at.0)
        .bind(operator)
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Appends an audit record (CLAUDE.md 20). Append-only, like the raw events: a
    /// correction trail that could itself be corrected would prove nothing.
    pub async fn save_audit(&self, entry: &AuditEntry) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO audit_log
                (at, operator, action, subject, reason, before_state, after_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(entry.at.0)
        .bind(&entry.operator)
        .bind(&entry.action)
        .bind(&entry.subject)
        .bind(entry.reason.as_deref())
        .bind(entry.before.as_deref())
        .bind(entry.after.as_deref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Live exceptions in a session. Voided rows are excluded, so clearing one in the
    /// inbox clears the badge too (CLAUDE.md 20; ADR 0001 D4).
    pub async fn exception_count(&self, session_id: &str) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM interpreted_events
             WHERE session_id = ?1 AND kind = 'EXCEPTION' AND voided_at IS NULL",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn raw_event_count(&self) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM raw_events")
            .fetch_one(&self.pool)
            .await?)
    }
}

fn row_to_interpreted(r: &sqlx::sqlite::SqliteRow) -> Result<Interpreted, StoreError> {
    let kind: String = r.get("kind");
    let at = Instant(r.get::<i64, _>("detected_at"));
    match kind.as_str() {
        "ENTERED" => Ok(Interpreted::Entered {
            station: r
                .get::<Option<String>, _>("station")
                .ok_or_else(|| StoreError::Corrupt("ENTERED without station".into()))?,
            at,
            transition: r.get::<Option<i64>, _>("transition_ms").map(domain::Duration),
            started_timing: r.get::<i64, _>("started_timing") != 0,
        }),
        "EXITED" => Ok(Interpreted::Exited {
            station: r
                .get::<Option<String>, _>("station")
                .ok_or_else(|| StoreError::Corrupt("EXITED without station".into()))?,
            at,
        }),
        "EXCEPTION" => Ok(Interpreted::Exception {
            reason: parse_reason(
                r.get::<Option<String>, _>("exception_reason")
                    .unwrap_or_default()
                    .as_str(),
            )?,
            at,
        }),
        other => Err(StoreError::Corrupt(format!("unknown kind {other}"))),
    }
}

fn mode_str(m: SessionMode) -> &'static str {
    match m {
        SessionMode::Competition => "COMPETITION",
        SessionMode::Training => "TRAINING",
    }
}
fn parse_mode(s: &str) -> Result<SessionMode, StoreError> {
    match s {
        "COMPETITION" => Ok(SessionMode::Competition),
        "TRAINING" => Ok(SessionMode::Training),
        other => Err(StoreError::Corrupt(format!("mode {other}"))),
    }
}
fn status_str(s: SessionStatus) -> &'static str {
    match s {
        SessionStatus::Draft => "DRAFT",
        SessionStatus::Armed => "ARMED",
        SessionStatus::Closed => "CLOSED",
    }
}
fn parse_status(s: &str) -> Result<SessionStatus, StoreError> {
    match s {
        "DRAFT" => Ok(SessionStatus::Draft),
        "ARMED" => Ok(SessionStatus::Armed),
        "CLOSED" => Ok(SessionStatus::Closed),
        other => Err(StoreError::Corrupt(format!("status {other}"))),
    }
}
fn reason_str(r: &ExceptionReason) -> &'static str {
    match r {
        ExceptionReason::SessionNotArmed => "SESSION_NOT_ARMED",
        ExceptionReason::ImpossibleTransition => "IMPOSSIBLE_TRANSITION",
        ExceptionReason::AlreadyFinished => "ALREADY_FINISHED",
        ExceptionReason::UnknownReader => "UNKNOWN_READER",
        ExceptionReason::AthleteNotInSession => "ATHLETE_NOT_IN_SESSION",
    }
}
fn parse_reason(s: &str) -> Result<ExceptionReason, StoreError> {
    match s {
        "SESSION_NOT_ARMED" => Ok(ExceptionReason::SessionNotArmed),
        "IMPOSSIBLE_TRANSITION" => Ok(ExceptionReason::ImpossibleTransition),
        "ALREADY_FINISHED" => Ok(ExceptionReason::AlreadyFinished),
        "UNKNOWN_READER" => Ok(ExceptionReason::UnknownReader),
        "ATHLETE_NOT_IN_SESSION" => Ok(ExceptionReason::AthleteNotInSession),
        other => Err(StoreError::Corrupt(format!("exception reason {other}"))),
    }
}

/// Configuration, reader map and binding ledger (ADR 0004).
///
/// These three are what a restart used to lose. Athlete state has always been rebuilt from
/// the interpreted log; without these, a resumed session was rebuilt against whatever
/// configuration the caller supplied, which could differ from the one it was armed under.
impl Store {
    /// Stores the course and the policies as one JSON document.
    ///
    /// A column rather than a set of tables: the course is nested, ordered and repeatable,
    /// the hub reads and writes it whole, and nothing queries inside it. The trade is that
    /// SQL cannot ask "which sessions used SKIERG"; when something needs that, it can be
    /// indexed alongside without changing this column.
    pub async fn save_session_config(&self, config: &SessionConfig) -> Result<(), StoreError> {
        let json = serde_json::to_string(config)
            .map_err(|e| StoreError::Corrupt(format!("session config: {e}")))?;
        sqlx::query(
            "INSERT INTO session_configs (session_id, config_json) VALUES (?1, ?2)
             ON CONFLICT(session_id) DO UPDATE SET config_json = excluded.config_json",
        )
        .bind(&config.session_id)
        .bind(json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn session_config(&self, id: &str) -> Result<Option<SessionConfig>, StoreError> {
        let json: Option<String> =
            sqlx::query_scalar("SELECT config_json FROM session_configs WHERE session_id = ?1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        json.map(|j| {
            serde_json::from_str(&j)
                .map_err(|e| StoreError::Corrupt(format!("session config for {id}: {e}")))
        })
        .transpose()
    }

    pub async fn save_reader(&self, r: &ReaderRegistration) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO readers (device_id, reader_id, station, zone, mode)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(device_id, reader_id) DO UPDATE SET
                station = excluded.station, zone = excluded.zone, mode = excluded.mode",
        )
        .bind(r.key.device_id.as_str())
        .bind(r.key.reader_id.as_str())
        .bind(&r.station)
        .bind(r.zone.as_deref())
        .bind(reader_mode_str(r.mode))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The venue's reader map. Insertion order is the operator screen's order, so rows come
    /// back sorted rather than in whatever order SQLite happens to hold them.
    pub async fn readers(&self) -> Result<ReaderRegistry, StoreError> {
        let rows = sqlx::query(
            "SELECT device_id, reader_id, station, zone, mode FROM readers
             ORDER BY device_id, reader_id",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut registry = ReaderRegistry::new();
        for r in &rows {
            let device: String = r.get("device_id");
            let reader: String = r.get("reader_id");
            let key = ReaderKey::parse(&device, &reader)
                .map_err(|e| StoreError::Corrupt(format!("reader {device}/{reader}: {e:?}")))?;
            let mut registration = ReaderRegistration::new(
                key,
                r.get::<String, _>("station"),
                parse_reader_mode(r.get::<String, _>("mode").as_str())?,
            );
            if let Some(zone) = r.get::<Option<String>, _>("zone") {
                registration = registration.with_zone(zone);
            }
            registry.register(registration);
        }
        Ok(registry)
    }

    /// Appends a binding, or stamps `unbound_at` on one already stored.
    ///
    /// `athlete_id` is deliberately not in the update list: a stored row may be closed, never
    /// re-attributed, which is what keeps the ledger usable as an audit trail (CLAUDE.md 20).
    pub async fn save_binding(&self, b: &TagBinding) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO tag_bindings (session_id, tag_id, athlete_id, bound_at, unbound_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id, tag_id, bound_at) DO UPDATE SET
                unbound_at = excluded.unbound_at",
        )
        .bind(&b.session_id)
        .bind(b.tag_id.as_str())
        .bind(&b.athlete_id)
        .bind(b.bound_at.0)
        .bind(b.unbound_at.map(|t| t.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn bindings(&self) -> Result<BindingLedger, StoreError> {
        let rows = sqlx::query(
            "SELECT session_id, tag_id, athlete_id, bound_at, unbound_at FROM tag_bindings
             ORDER BY bound_at, rowid",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut entries = Vec::with_capacity(rows.len());
        for r in &rows {
            let raw: String = r.get("tag_id");
            entries.push(TagBinding {
                session_id: r.get("session_id"),
                tag_id: TagId::parse(&raw)
                    .map_err(|e| StoreError::Corrupt(format!("tag {raw}: {e:?}")))?,
                athlete_id: r.get("athlete_id"),
                bound_at: Instant(r.get::<i64, _>("bound_at")),
                unbound_at: r.get::<Option<i64>, _>("unbound_at").map(Instant),
            });
        }
        Ok(BindingLedger::restore(entries))
    }

    /// Distinct tags any reader has seen since `since`, first sighting first. The check-in
    /// queue is derived from this (ADR 0001 D3) rather than held in memory.
    pub async fn raw_tags_since(&self, since: Instant) -> Result<Vec<String>, StoreError> {
        let rows = sqlx::query(
            "SELECT tag_id FROM raw_events WHERE detected_at >= ?1
             GROUP BY tag_id ORDER BY MIN(detected_at), MIN(id)",
        )
        .bind(since.0)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| r.get::<String, _>("tag_id")).collect())
    }

    /// Reads of one tag that no interpreted event points at, oldest first (ADR 0001 D3).
    ///
    /// The `NOT EXISTS` is the idempotency: once a read has an interpretation -- including a
    /// voided one, which an operator removed on purpose -- claiming will not produce a
    /// second. Matching is case-insensitive because the raw row keeps the wire spelling
    /// while `TagId` upper-cases.
    pub async fn unclaimed_reads_for_tag(
        &self,
        tag_id: &str,
        since: Instant,
    ) -> Result<Vec<StoredRawRead>, StoreError> {
        let rows = sqlx::query(
            "SELECT r.id, r.device_id, r.reader_id, r.detected_at FROM raw_events r
             WHERE r.tag_id = ?1 COLLATE NOCASE AND r.detected_at >= ?2
               AND NOT EXISTS (
                   SELECT 1 FROM interpreted_events i WHERE i.raw_event_id = r.id)
             ORDER BY r.detected_at, r.id",
        )
        .bind(tag_id)
        .bind(since.0)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| StoredRawRead {
                raw_event_id: r.get("id"),
                device_id: r.get("device_id"),
                reader_id: r.get("reader_id"),
                detected_at: Instant(r.get::<i64, _>("detected_at")),
            })
            .collect())
    }
}

fn reader_mode_str(m: ReaderMode) -> &'static str {
    match m {
        ReaderMode::Entry => "ENTRY",
        ReaderMode::Exit => "EXIT",
        ReaderMode::Toggle => "TOGGLE",
        ReaderMode::Checkpoint => "CHECKPOINT",
        ReaderMode::Passage => "PASSAGE",
    }
}
fn parse_reader_mode(s: &str) -> Result<ReaderMode, StoreError> {
    match s {
        "ENTRY" => Ok(ReaderMode::Entry),
        "EXIT" => Ok(ReaderMode::Exit),
        "TOGGLE" => Ok(ReaderMode::Toggle),
        "CHECKPOINT" => Ok(ReaderMode::Checkpoint),
        "PASSAGE" => Ok(ReaderMode::Passage),
        other => Err(StoreError::Corrupt(format!("reader mode {other}"))),
    }
}

impl Store {
    pub async fn session_created_at(&self, id: &str) -> Result<Option<Instant>, StoreError> {
        let v: Option<i64> = sqlx::query_scalar("SELECT created_at FROM sessions WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(v.map(Instant))
    }

    /// Latest `detected_at` in the session, so a resumed run can pick up where it stopped
    /// rather than rewinding the clock (CLAUDE.md 21).
    pub async fn max_detected_at(&self, session_id: &str) -> Result<Option<Instant>, StoreError> {
        let v: Option<i64> = sqlx::query_scalar(
            "SELECT MAX(detected_at) FROM interpreted_events WHERE session_id = ?1",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(v.map(Instant))
    }
}
