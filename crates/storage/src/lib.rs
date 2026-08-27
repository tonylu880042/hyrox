//! SQLite persistence for the Central Hub (CLAUDE.md 19, 21).
//!
//! Raw events are append-only and never modified. Athlete state is not stored: it is rebuilt
//! by replaying the non-voided interpreted events, so a restart cannot disagree with the log.

use domain::{
    AthleteState, ExceptionReason, Instant, Interpreted, Session, SessionMode, SessionStatus,
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
    }
}
fn parse_reason(s: &str) -> Result<ExceptionReason, StoreError> {
    match s {
        "SESSION_NOT_ARMED" => Ok(ExceptionReason::SessionNotArmed),
        "IMPOSSIBLE_TRANSITION" => Ok(ExceptionReason::ImpossibleTransition),
        "ALREADY_FINISHED" => Ok(ExceptionReason::AlreadyFinished),
        "UNKNOWN_READER" => Ok(ExceptionReason::UnknownReader),
        other => Err(StoreError::Corrupt(format!("exception reason {other}"))),
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
