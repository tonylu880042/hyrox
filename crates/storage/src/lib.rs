//! SQLite persistence for the Central Hub (CLAUDE.md 19, 21).
//!
//! Raw events are append-only and never modified. Athlete state is not stored: it is rebuilt
//! by replaying the non-voided interpreted events, so a restart cannot disagree with the log.

mod hub_store;
mod workout;

use application::{
    AuditEntry, FinishOrigin, RawInterpretationContext, SeenReader, StoredException, StoredRawRead,
    VenueAsset,
};
use domain::{
    AthleteState, BindingLedger, Duration, ExceptionReason, Instant, Interpreted, ReaderBinding,
    ReaderKey, ReaderMode, ReaderRegistration, ReaderRegistry, Session, SessionConfig, SessionMode,
    SessionStatus, TagBinding, TagId,
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
    /// A stored JSON document -- a session snapshot, or a template's blocks -- did not
    /// parse. Distinct from `Corrupt` so the message carries serde's own diagnosis.
    #[error("stored document: {0}")]
    Json(#[from] serde_json::Error),
    #[error("backup: {0}")]
    Io(#[from] std::io::Error),
    /// The file itself is damaged. Separate from every other error because the answer is
    /// different: not "retry", but "stop, and restore last night's backup" (ADR 0012).
    #[error("the database is damaged: {}", .0.join("; "))]
    Damaged(Vec<String>),
}

/// Whether an error from SQLite means "this file is damaged" rather than "that write did
/// not go through". The two need different answers, and only one of them is worth waking
/// somebody up for (ADR 0012).
fn is_corruption(error: &sqlx::Error) -> bool {
    let Some(db) = error.as_database_error() else {
        return false;
    };
    matches!(db.code().as_deref(), Some("11") | Some("26"))
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
    pub(crate) pool: SqlitePool,
}

impl Store {
    /// Applies one domain mutation and its audit row in the same SQLite transaction. The
    /// regular single-purpose methods remain for non-audited ingestion/derived writes; operator
    /// mutations use this boundary so a failed audit cannot leave memory and durable state
    /// disagreeing.
    pub async fn apply_audited(
        &self,
        write: application::AuditedWrite,
    ) -> Result<application::AuditedResult, StoreError> {
        let mut tx = self.pool.begin().await?;
        let result = match write {
            application::AuditedWrite::Session {
                session,
                created_at,
                audit,
            } => {
                save_session_tx(&mut tx, &session, created_at).await?;
                set_active_session_tx(&mut tx, &session, created_at).await?;
                audit_tx(&mut tx, &audit).await?;
                application::AuditedResult::Applied(true)
            }
            application::AuditedWrite::CreateClass {
                session,
                created_at,
                config,
                athletes,
                audit,
            } => {
                if !insert_session_tx(&mut tx, &session, created_at).await? {
                    application::AuditedResult::Conflict
                } else {
                    insert_session_config_tx(&mut tx, &config).await?;
                    for athlete in athletes {
                        sqlx::query(
                            "INSERT INTO session_athletes
                            (session_id, athlete_id, display_name, bib, member_id)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        )
                        .bind(&session.id)
                        .bind(athlete.athlete_id)
                        .bind(athlete.display_name)
                        .bind(athlete.bib)
                        .bind(athlete.member_id)
                        .execute(&mut *tx)
                        .await?;
                    }
                    set_active_session_tx(&mut tx, &session, created_at).await?;
                    audit_tx(&mut tx, &audit).await?;
                    application::AuditedResult::Applied(true)
                }
            }
            application::AuditedWrite::SessionConfig { config, audit } => {
                let json = serde_json::to_string(&config)
                    .map_err(|e| StoreError::Corrupt(format!("session config: {e}")))?;
                sqlx::query(
                    "INSERT INTO session_configs (session_id, config_json) VALUES (?1, ?2)
                     ON CONFLICT(session_id) DO UPDATE SET config_json = excluded.config_json",
                )
                .bind(&config.session_id)
                .bind(json)
                .execute(&mut *tx)
                .await?;
                audit_tx(&mut tx, &audit).await?;
                application::AuditedResult::Applied(true)
            }
            application::AuditedWrite::TemplateSave { template, audit } => {
                if workout::save_template_tx(&mut tx, &template, false).await? {
                    audit_tx(&mut tx, &audit).await?;
                    application::AuditedResult::Applied(true)
                } else {
                    application::AuditedResult::Conflict
                }
            }
            application::AuditedWrite::TemplateCreate { template, audit } => {
                if workout::save_template_tx(&mut tx, &template, true).await? {
                    audit_tx(&mut tx, &audit).await?;
                    application::AuditedResult::Applied(true)
                } else {
                    application::AuditedResult::Conflict
                }
            }
            application::AuditedWrite::TemplateDelete { template_id, audit } => {
                let changed = workout::delete_template_tx(&mut tx, &template_id).await?;
                if changed {
                    audit_tx(&mut tx, &audit).await?;
                }
                application::AuditedResult::Applied(changed)
            }
            application::AuditedWrite::VenueSetting {
                key,
                value,
                at,
                by,
                audit,
            } => {
                sqlx::query(
                    "INSERT INTO venue_settings (key, value, updated_at, updated_by)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(key) DO UPDATE SET
                        value = excluded.value,
                        updated_at = excluded.updated_at,
                        updated_by = excluded.updated_by",
                )
                .bind(key)
                .bind(value)
                .bind(at.0)
                .bind(by)
                .execute(&mut *tx)
                .await?;
                audit_tx(&mut tx, &audit).await?;
                application::AuditedResult::Applied(true)
            }
            application::AuditedWrite::VenueAssetSave {
                key,
                media_type,
                bytes,
                at,
                by,
                audit,
            } => {
                sqlx::query(
                    "INSERT INTO venue_assets (key, media_type, bytes, updated_at, updated_by)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(key) DO UPDATE SET
                        media_type = excluded.media_type,
                        bytes = excluded.bytes,
                        updated_at = excluded.updated_at,
                        updated_by = excluded.updated_by",
                )
                .bind(key)
                .bind(media_type)
                .bind(bytes)
                .bind(at.0)
                .bind(by)
                .execute(&mut *tx)
                .await?;
                audit_tx(&mut tx, &audit).await?;
                application::AuditedResult::Applied(true)
            }
            application::AuditedWrite::VenueAssetDelete { key, audit } => {
                sqlx::query("DELETE FROM venue_assets WHERE key = ?1")
                    .bind(key)
                    .execute(&mut *tx)
                    .await?;
                audit_tx(&mut tx, &audit).await?;
                application::AuditedResult::Applied(true)
            }
            application::AuditedWrite::Athlete {
                session_id,
                athlete_id,
                display_name,
                bib,
                member_id,
                audit,
            } => {
                sqlx::query(
                    "INSERT INTO session_athletes
                        (session_id, athlete_id, display_name, bib, member_id)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(session_id, athlete_id) DO UPDATE SET
                        display_name = excluded.display_name,
                        bib = excluded.bib,
                        member_id = excluded.member_id",
                )
                .bind(session_id)
                .bind(athlete_id)
                .bind(display_name)
                .bind(bib)
                .bind(member_id)
                .execute(&mut *tx)
                .await?;
                audit_tx(&mut tx, &audit).await?;
                application::AuditedResult::Applied(true)
            }
            application::AuditedWrite::Bindings {
                bindings,
                claim,
                audit,
            } => {
                for binding in bindings {
                    sqlx::query(
                        "INSERT INTO tag_bindings
                            (session_id, tag_id, athlete_id, bound_at, unbound_at)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(session_id, tag_id, bound_at) DO UPDATE SET
                            unbound_at = excluded.unbound_at",
                    )
                    .bind(binding.session_id)
                    .bind(binding.tag_id.to_string())
                    .bind(binding.athlete_id)
                    .bind(binding.bound_at.0)
                    .bind(binding.unbound_at.map(|at| at.0))
                    .execute(&mut *tx)
                    .await?;
                }
                if let Some(claim) = claim {
                    // Persist the claim intent with the binding.  The interpretation rows are
                    // intentionally written later, one by one, so a failed row remains visible
                    // to recovery instead of losing the handoff at the check-in boundary.
                    sqlx::query(
                        "UPDATE raw_interpretation_context
                            SET athlete_id = ?1, bound_session_id = ?2, on_roster = 1
                          WHERE session_id = ?2
                            AND athlete_id IS NULL
                            AND raw_event_id IN (
                                SELECT r.id FROM raw_events r
                                 WHERE r.tag_id = ?3 COLLATE NOCASE
                                   AND r.detected_at >= ?4
                                   AND r.detected_at <= ?5
                                   AND NOT EXISTS (
                                       SELECT 1 FROM interpreted_events i
                                        WHERE i.raw_event_id = r.id
                                   )
                            )",
                    )
                    .bind(claim.athlete_id)
                    .bind(claim.session_id)
                    .bind(claim.tag_id)
                    .bind(claim.since.0)
                    .bind(claim.until.0)
                    .execute(&mut *tx)
                    .await?;
                }
                audit_tx(&mut tx, &audit).await?;
                application::AuditedResult::Applied(true)
            }
            application::AuditedWrite::ReaderSave {
                registration,
                audit,
            } => {
                sqlx::query(
                    "INSERT INTO readers (device_id, reader_id, station, zone, mode)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(device_id, reader_id) DO UPDATE SET
                        station = excluded.station,
                        zone = excluded.zone,
                        mode = excluded.mode",
                )
                .bind(registration.key.device_id.as_str())
                .bind(registration.key.reader_id.as_str())
                .bind(&registration.station)
                .bind(registration.zone.as_deref())
                .bind(reader_mode_str(registration.mode))
                .execute(&mut *tx)
                .await?;
                audit_tx(&mut tx, &audit).await?;
                application::AuditedResult::Applied(true)
            }
            application::AuditedWrite::ReaderDelete {
                device_id,
                reader_id,
                audit,
            } => {
                let changed =
                    sqlx::query("DELETE FROM readers WHERE device_id = ?1 AND reader_id = ?2")
                        .bind(device_id)
                        .bind(reader_id)
                        .execute(&mut *tx)
                        .await?
                        .rows_affected()
                        > 0;
                if changed {
                    audit_tx(&mut tx, &audit).await?;
                }
                application::AuditedResult::Applied(changed)
            }
            application::AuditedWrite::ExceptionAccept {
                session_id,
                interpreted_event_id,
                at,
                operator,
                reason,
                audit,
            } => {
                let changed = sqlx::query(
                    "UPDATE interpreted_events
                        SET acknowledged_at = ?2, acknowledged_by = ?3,
                            acknowledge_reason = ?4
                      WHERE id = ?1 AND session_id = ?5
                        AND kind = 'EXCEPTION' AND voided_at IS NULL",
                )
                .bind(interpreted_event_id)
                .bind(at.0)
                .bind(operator)
                .bind(reason)
                .bind(session_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                    > 0;
                if changed {
                    audit_tx(&mut tx, &audit).await?;
                }
                application::AuditedResult::Applied(changed)
            }
            application::AuditedWrite::ExceptionVoid {
                session_id,
                interpreted_event_id,
                at,
                operator,
                reason,
                audit,
            } => {
                let changed = sqlx::query(
                    "UPDATE interpreted_events
                        SET voided_at = ?2, voided_by = ?3, void_reason = ?4
                      WHERE id = ?1 AND session_id = ?5 AND voided_at IS NULL",
                )
                .bind(interpreted_event_id)
                .bind(at.0)
                .bind(operator)
                .bind(reason)
                .bind(session_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                    > 0;
                if changed {
                    audit_tx(&mut tx, &audit).await?;
                }
                application::AuditedResult::Applied(changed)
            }
            application::AuditedWrite::ExceptionReinterpret {
                session_id,
                interpreted_event_id,
                at,
                operator,
                reason,
                replacement,
                audit,
            } => {
                let changed = sqlx::query(
                    "UPDATE interpreted_events
                        SET voided_at = ?2, voided_by = ?3, void_reason = ?4
                      WHERE id = ?1 AND session_id = ?5
                        AND kind = 'EXCEPTION' AND voided_at IS NULL",
                )
                .bind(interpreted_event_id)
                .bind(at.0)
                .bind(&operator)
                .bind(&reason)
                .bind(session_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
                    > 0;
                if !changed {
                    application::AuditedResult::Applied(false)
                } else {
                    let id = insert_interpreted_tx(&mut tx, &replacement).await?;
                    audit_tx(&mut tx, &audit).await?;
                    application::AuditedResult::Interpreted(id)
                }
            }
            application::AuditedWrite::EndClass {
                session,
                created_at,
                finishes,
                audit,
            } => {
                for (athlete_id, at) in finishes {
                    sqlx::query(
                        "UPDATE session_athletes
                            SET finished_at = ?3, finish_origin = 'MANUAL'
                          WHERE session_id = ?1 AND athlete_id = ?2",
                    )
                    .bind(&session.id)
                    .bind(athlete_id)
                    .bind(at.0)
                    .execute(&mut *tx)
                    .await?;
                }
                save_session_tx(&mut tx, &session, created_at).await?;
                set_active_session_tx(&mut tx, &session, created_at).await?;
                audit_tx(&mut tx, &audit).await?;
                application::AuditedResult::Applied(true)
            }
        };
        tx.commit().await?;
        Ok(result)
    }

    /// Opens (creating if needed) the database at `path` and applies migrations.
    /// WAL keeps readers unblocked while ingestion writes (CLAUDE.md 19).
    pub async fn open(path: &str) -> Result<Self, StoreError> {
        let opts = SqliteConnectOptions::from_str(path)
            .map_err(StoreError::Db)?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            // FULL, not NORMAL: the hub ships as an appliance that is switched off at the
            // wall (ADR 0009). In WAL mode NORMAL only fsyncs at a checkpoint, so a commit
            // survives a process crash but not a pulled plug -- and the ACK we send on the
            // strength of that commit tells the ESP32 to delete its only other copy of the
            // event (ADR 0002; CLAUDE.md 15, 31). One fsync per commit, against roughly two
            // rows per athlete per station.
            .synchronous(sqlx::sqlite::SqliteSynchronous::Full)
            .foreign_keys(true);
        // Damage can surface here rather than in the check below: opening sets the journal
        // mode, and that reads pages. Either way the answer is the same one.
        let pool = SqlitePool::connect_with(opts).await.map_err(|e| {
            if is_corruption(&e) {
                StoreError::Damaged(vec![e.to_string()])
            } else {
                e.into()
            }
        })?;
        let store = Self { pool };
        // Before the migrations, not after: a migration against a damaged file writes into
        // the damage. And before anything is served, because acknowledging a read out of a
        // broken database tells the ESP32 to delete its only other copy of it
        // (ADR 0002, 0012; CLAUDE.md 15, 31).
        store.quick_check().await?;
        sqlx::migrate!("../../migrations").run(&store.pool).await?;
        Ok(store)
    }

    /// Reads every page and reports what is wrong with them (ADR 0012).
    ///
    /// `quick_check` rather than `integrity_check`: it finds the damage that matters -- a
    /// page that will not parse -- without the index cross-check, which on a venue's
    /// database is seconds rather than milliseconds. This runs on every start, so it has to
    /// be cheap enough that nobody is tempted to switch it off.
    pub async fn quick_check(&self) -> Result<(), StoreError> {
        let rows: Vec<String> = match sqlx::query_scalar("PRAGMA quick_check")
            .fetch_all(&self.pool)
            .await
        {
            Ok(rows) => rows,
            // Badly damaged files fail the check itself rather than reporting rows:
            // SQLITE_CORRUPT (11) or SQLITE_NOTADB (26). That is still a damage report,
            // and callers must not have to tell the two shapes apart.
            Err(e) if is_corruption(&e) => return Err(StoreError::Damaged(vec![e.to_string()])),
            Err(e) => return Err(e.into()),
        };
        // SQLite answers with the single row "ok", or one row per problem.
        if rows.len() == 1 && rows[0].eq_ignore_ascii_case("ok") {
            return Ok(());
        }
        Err(StoreError::Damaged(rows))
    }

    /// Copies the database to `path` while the hub keeps running (ADR 0012).
    ///
    /// `VACUUM INTO` is SQLite's supported online backup: it reads a consistent snapshot
    /// through a normal read transaction, so committed data sitting in the `-wal` is
    /// included. Copying the main file with `cp` is the classic way to produce a backup
    /// that is missing exactly the transactions you wanted, or is outright corrupt.
    ///
    /// It refuses an existing target, and we keep that: a backup that silently replaces
    /// last night's is one backup rather than a history.
    pub async fn backup_to(&self, path: &std::path::Path) -> Result<(), StoreError> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        // Single-quoted SQL literal, so a path containing one has to be doubled. Not a
        // bound parameter: VACUUM INTO does not take one.
        let target = path.display().to_string().replace('\'', "''");
        sqlx::query(&format!("VACUUM INTO '{target}'"))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The connection pool, for asserting the settings this store was opened with.
    /// Read-only by convention: every write goes through a method on `Store`.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn open_in_memory() -> Result<Self, StoreError> {
        Self::open("sqlite::memory:").await
    }

    pub async fn save_session(&self, s: &Session, created_at: Instant) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        save_session_tx(&mut tx, s, created_at).await?;
        set_active_session_tx(&mut tx, s, created_at).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn save_athlete(
        &self,
        session_id: &str,
        athlete_id: &str,
        display_name: &str,
        bib: i64,
        member_id: Option<&str>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO session_athletes (session_id, athlete_id, display_name, bib, member_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id, athlete_id) DO UPDATE SET
                display_name = excluded.display_name,
                bib = excluded.bib,
                member_id = excluded.member_id",
        )
        .bind(session_id)
        .bind(athlete_id)
        .bind(display_name)
        .bind(bib)
        .bind(member_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// `(athlete_id, bib)` for a session's roster, in bib order.
    pub async fn athlete_bibs(&self, session_id: &str) -> Result<Vec<(String, i64)>, StoreError> {
        let rows = sqlx::query(
            "SELECT athlete_id, bib FROM session_athletes WHERE session_id = ?1 ORDER BY bib",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("athlete_id"), r.get("bib")))
            .collect())
    }

    /// Stores one tag's read, returning its id. A redelivery of the same
    /// `device_id + boot_id + sequence` **and tag** returns the existing id instead of
    /// inserting again: duplicate delivery is allowed, duplicate processing is not
    /// (CLAUDE.md 16).
    ///
    /// The tag is part of the lookup because a UHF inventory round carries several of them
    /// under one sequence (ADR 0014). The idempotency key the edge and the ACK speak is
    /// still `device_id + boot_id + sequence`; this row is one read inside that round.
    pub async fn save_raw(&self, e: &RawEvent) -> Result<(i64, bool), StoreError> {
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM raw_events
             WHERE device_id = ?1 AND boot_id = ?2 AND sequence = ?3 AND tag_id = ?4",
        )
        .bind(&e.device_id)
        .bind(e.boot_id)
        .bind(e.sequence)
        .bind(&e.tag_id)
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

    /// Commits a raw row and the interpretation context that was observed for it in one
    /// transaction. The context is insert-once: a redelivery must not overwrite the original
    /// reader/binding decision with a newer venue configuration.
    pub async fn save_raw_with_context(
        &self,
        e: &RawEvent,
        context: &RawInterpretationContext,
    ) -> Result<(i64, bool), StoreError> {
        let mut tx = self.pool.begin().await?;
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM raw_events
             WHERE device_id = ?1 AND boot_id = ?2 AND sequence = ?3 AND tag_id = ?4",
        )
        .bind(&e.device_id)
        .bind(e.boot_id)
        .bind(e.sequence)
        .bind(&e.tag_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (id, inserted) = if let Some(id) = existing {
            (id, false)
        } else {
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
            .fetch_one(&mut *tx)
            .await?;
            (id, true)
        };

        if inserted {
            let planned_json = context
                .planned
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?;
            sqlx::query(
                "INSERT INTO raw_interpretation_context
                    (raw_event_id, session_id, athlete_id, bound_session_id, on_roster,
                     reader_station, reader_mode, planned_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(raw_event_id) DO NOTHING",
            )
            .bind(id)
            .bind(&context.session_id)
            .bind(context.athlete_id.as_deref())
            .bind(context.bound_session_id.as_deref())
            .bind(context.on_roster as i64)
            .bind(context.reader.as_ref().map(|r| r.station.as_str()))
            .bind(context.reader.as_ref().map(|r| reader_mode_str(r.mode)))
            .bind(planned_json)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok((id, inserted))
    }

    pub async fn save_interpreted(
        &self,
        session_id: &str,
        athlete_id: &str,
        raw_event_id: Option<i64>,
        event: &Interpreted,
    ) -> Result<i64, StoreError> {
        let (kind, station, detected_at, transition_ms, started, reason) = match event {
            Interpreted::Entered {
                station,
                at,
                transition,
                started_timing,
            } => (
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
            Interpreted::Exception { reason, at } => (
                "EXCEPTION",
                None,
                at.0,
                None,
                false,
                Some(reason_str(reason)),
            ),
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

    /// Checks whether a raw event has already produced an interpreted event row.
    pub async fn raw_event_has_interpretation(
        &self,
        raw_event_id: i64,
    ) -> Result<bool, StoreError> {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM interpreted_events WHERE raw_event_id = ?1 LIMIT 1")
                .bind(raw_event_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(exists.is_some())
    }

    /// The session to resume after a restart, selected by the durable current-session
    /// pointer. Unlike a status heuristic this keeps a newly-created DRAFT current even
    /// when an older READY session remains in the history (F07).
    pub async fn active_session(&self) -> Result<Option<Session>, StoreError> {
        let row = sqlx::query(
            "SELECT id, name, mode, status, interpreted_event_count,
                    paused_total_ms, paused_since
             FROM sessions
             JOIN active_session ON active_session.session_id = sessions.id
             WHERE active_session.singleton = 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(session_from_row).transpose()
    }

    /// One session by id, active or not. `/result/{id}` has to be able to name a session
    /// the hub stopped running hours ago (CLAUDE.md 22).
    pub async fn session(&self, id: &str) -> Result<Option<Session>, StoreError> {
        let row = sqlx::query(
            "SELECT id, name, mode, status, interpreted_event_count,
                    paused_total_ms, paused_since
             FROM sessions WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(session_from_row).transpose()
    }

    /// The session's live exceptions, oldest first (ADR 0001 D4). Ordered by `detected_at`
    /// like the replay, so the inbox reads in the order the venue produced it.
    /// Marks one exception as looked at and left alone (migration 0011).
    ///
    /// Returns false when there is no such row, so the caller can answer 404 rather than
    /// report a success that changed nothing. Accepting twice is harmless -- the second
    /// write restamps who cleared it, which is the more useful of the two records.
    pub async fn acknowledge_interpreted(
        &self,
        interpreted_event_id: i64,
        at: Instant,
        operator: &str,
        reason: Option<&str>,
    ) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "UPDATE interpreted_events
                SET acknowledged_at = ?2, acknowledged_by = ?3, acknowledge_reason = ?4
              WHERE id = ?1 AND kind = 'EXCEPTION' AND voided_at IS NULL",
        )
        .bind(interpreted_event_id)
        .bind(at.0)
        .bind(operator)
        .bind(reason)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn exceptions(&self, session_id: &str) -> Result<Vec<StoredException>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, athlete_id, raw_event_id, detected_at, exception_reason
             FROM interpreted_events
             WHERE session_id = ?1 AND kind = 'EXCEPTION' AND voided_at IS NULL
               AND acknowledged_at IS NULL
             ORDER BY detected_at, id",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(StoredException {
                    interpreted_event_id: r.get("id"),
                    athlete_id: r.get("athlete_id"),
                    reason: parse_reason(
                        r.get::<Option<String>, _>("exception_reason")
                            .unwrap_or_default()
                            .as_str(),
                    )?,
                    at: Instant(r.get::<i64, _>("detected_at")),
                    raw_event_id: r.get("raw_event_id"),
                })
            })
            .collect()
    }

    /// Rebuilds every athlete in the session by replaying the non-voided interpreted events
    /// in `detected_at` order (CLAUDE.md 21). Voided rows are excluded, which is how an
    /// operator correction reaches the derived values (CLAUDE.md 20).
    /// Records a finish the finish rule decided (migration 0010).
    ///
    /// The column is nullable and `None` writes NULL: not finished by a rule, which is both
    /// "still running" and "finished by completing the course", since that one replays.
    pub async fn save_athlete_finish(
        &self,
        session_id: &str,
        athlete_id: &str,
        finished_at: Option<Instant>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE session_athletes SET finished_at = ?3, finish_origin = 'MANUAL'
             WHERE session_id = ?1 AND athlete_id = ?2",
        )
        .bind(session_id)
        .bind(athlete_id)
        .bind(finished_at.map(|t| t.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn save_athlete_finishes(
        &self,
        session_id: &str,
        finishes: &[(String, Instant)],
        origin: FinishOrigin,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for (athlete_id, at) in finishes {
            sqlx::query(
                "UPDATE session_athletes SET finished_at = ?3, finish_origin = ?4
                 WHERE session_id = ?1 AND athlete_id = ?2",
            )
            .bind(session_id)
            .bind(athlete_id)
            .bind(at.0)
            .bind(finish_origin_name(origin))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn rebuild_athletes(
        &self,
        session_id: &str,
    ) -> Result<Vec<AthleteState>, StoreError> {
        let config = self.session_config(session_id).await?;
        let roster = sqlx::query(
            "SELECT athlete_id, display_name, finished_at, finish_origin FROM session_athletes
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

        // Keep a second replay that includes voided rows only for class-completion legacy
        // detection. A pre-0013 roster value may have been written by the old
        // CourseComplete implementation; if its timestamp matches the final exit even after
        // that exit was voided, it is policy-derived and must not resurrect on rebuild.
        let historical_rows = sqlx::query(
            "SELECT athlete_id, kind, station, detected_at, transition_ms,
                    started_timing, exception_reason
             FROM interpreted_events
             WHERE session_id = ?1
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
            if let Some(event) = replay_event(state, row_to_interpreted(r)?) {
                domain::apply(state, &event);
            }
        }

        let mut historical: Vec<AthleteState> = roster
            .iter()
            .map(|r| {
                AthleteState::ready(
                    r.get::<String, _>("athlete_id"),
                    r.get::<String, _>("display_name"),
                )
            })
            .collect();
        for r in &historical_rows {
            let aid: String = r.get("athlete_id");
            let Some(state) = historical.iter_mut().find(|a| a.athlete_id == aid) else {
                continue;
            };
            if let Some(event) = replay_event(state, row_to_interpreted(r)?) {
                domain::apply(state, &event);
            }
        }

        // After the replay, not before: a finish the clock decided comes at the end of the
        // class, so applying it first would let a later read reopen a closed result. Course
        // completion is different: its answer is already in the interpreted exit log, so a
        // policy-origin finish must not survive a void/replay. Manual end-class finishes do.
        for r in &roster {
            let Some(ms) = r.get::<Option<i64>, _>("finished_at") else {
                continue;
            };
            let aid: String = r.get("athlete_id");
            let origin = r.get::<Option<String>, _>("finish_origin");
            if config
                .as_ref()
                .is_some_and(|c| c.finish_policy == domain::FinishPolicy::CourseComplete)
                && !stored_finish_is_manual(
                    origin.as_deref(),
                    Instant(ms),
                    historical.iter().find(|a| a.athlete_id == aid),
                    config.as_ref().and_then(|c| c.course.as_ref()),
                )
            {
                continue;
            }
            if let Some(state) = out.iter_mut().find(|a| a.athlete_id == aid) {
                domain::finish(state, Instant(ms));
            }
        }
        Ok(out)
    }

    /// Marks one interpretation voided, and reports whether a row matched. Voiding is an
    /// UPDATE and never a DELETE: the corrected event has to stay readable afterwards
    /// (CLAUDE.md 19, 20).
    pub async fn void_interpreted(
        &self,
        id: i64,
        at: Instant,
        operator: &str,
        reason: &str,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query(
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
        Ok(result.rows_affected() > 0)
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
            // The badge counts outstanding work, so it counts what the inbox lists:
            // accepted exceptions are still in the log and no longer on anyone's list.
            "SELECT COUNT(*) FROM interpreted_events
             WHERE session_id = ?1 AND kind = 'EXCEPTION' AND voided_at IS NULL
               AND acknowledged_at IS NULL",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn interpreted_event_count(&self, session_id: &str) -> Result<u64, StoreError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM interpreted_events
             WHERE session_id = ?1 AND voided_at IS NULL AND kind <> 'EXCEPTION'",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count as u64)
    }

    pub async fn raw_event_count(&self) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM raw_events")
            .fetch_one(&self.pool)
            .await?)
    }
}

/// Recompute derived timing fields from the replay prefix instead of trusting values that were
/// decided before a historical correction. The event kind, station and timestamp remain the
/// durable interpretation; `started_timing` and `transition` are properties of their position
/// in the corrected timeline.
fn replay_event(state: &AthleteState, event: Interpreted) -> Option<Interpreted> {
    match event {
        Interpreted::Entered { .. } if state.station_state == domain::StationState::Inside => {
            // A historical correction can make a later persisted ENTERED row contradict the
            // prefix that now precedes it.  It was already recorded in the log, so leave it
            // auditable, but do not manufacture a second open station run in the projection.
            None
        }
        Interpreted::Entered { station, at, .. } => Some(Interpreted::Entered {
            station,
            at,
            transition: state.last_exit_at.map(|previous| at.since(previous)),
            started_timing: state.status == domain::AthleteStatus::Ready,
        }),
        other => Some(other),
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
            transition: r
                .get::<Option<i64>, _>("transition_ms")
                .map(domain::Duration),
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

async fn save_session_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    s: &Session,
    created_at: Instant,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sessions
            (id, name, mode, status, interpreted_event_count, created_at,
             paused_total_ms, paused_since)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            status = excluded.status,
            interpreted_event_count = excluded.interpreted_event_count,
            paused_total_ms = excluded.paused_total_ms,
            paused_since = excluded.paused_since",
    )
    .bind(&s.id)
    .bind(&s.name)
    .bind(mode_str(s.mode))
    .bind(status_str(s.status))
    .bind(s.interpreted_event_count as i64)
    .bind(created_at.0)
    .bind(s.paused_total.millis())
    .bind(s.paused_since.map(|i| i.0))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_session_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    s: &Session,
    created_at: Instant,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO sessions
            (id, name, mode, status, interpreted_event_count, created_at,
             paused_total_ms, paused_since)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO NOTHING",
    )
    .bind(&s.id)
    .bind(&s.name)
    .bind(mode_str(s.mode))
    .bind(status_str(s.status))
    .bind(s.interpreted_event_count as i64)
    .bind(created_at.0)
    .bind(s.paused_total.millis())
    .bind(s.paused_since.map(|i| i.0))
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() > 0)
}

async fn insert_session_config_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    config: &SessionConfig,
) -> Result<(), StoreError> {
    let json = serde_json::to_string(config)?;
    sqlx::query("INSERT INTO session_configs (session_id, config_json) VALUES (?1, ?2)")
        .bind(&config.session_id)
        .bind(json)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn set_active_session_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session: &Session,
    updated_at: Instant,
) -> Result<(), sqlx::Error> {
    if session.status.is_terminal() {
        sqlx::query("DELETE FROM active_session WHERE singleton = 1 AND session_id = ?1")
            .bind(&session.id)
            .execute(&mut **tx)
            .await?;
    } else {
        sqlx::query(
            "INSERT INTO active_session (singleton, session_id, updated_at)
             VALUES (1, ?1, ?2)
             ON CONFLICT(singleton) DO UPDATE SET
                session_id = excluded.session_id,
                updated_at = excluded.updated_at",
        )
        .bind(&session.id)
        .bind(updated_at.0)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn audit_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    entry: &AuditEntry,
) -> Result<(), sqlx::Error> {
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
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_interpreted_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    write: &application::AuditedInterpretedWrite,
) -> Result<i64, sqlx::Error> {
    let (kind, station, detected_at, transition_ms, started, reason) = match &write.event {
        Interpreted::Entered {
            station,
            at,
            transition,
            started_timing,
        } => (
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
        Interpreted::Exception { reason, at } => (
            "EXCEPTION",
            None,
            at.0,
            None,
            false,
            Some(reason_str(reason)),
        ),
    };
    sqlx::query_scalar(
        "INSERT INTO interpreted_events
            (session_id, athlete_id, raw_event_id, kind, station, detected_at,
             transition_ms, started_timing, exception_reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id",
    )
    .bind(&write.session_id)
    .bind(&write.athlete_id)
    .bind(write.raw_event_id)
    .bind(kind)
    .bind(station)
    .bind(detected_at)
    .bind(transition_ms)
    .bind(started as i64)
    .bind(reason)
    .fetch_one(&mut **tx)
    .await
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
/// One place both session reads build a `Session`, so a column added to one is not
/// forgotten by the other.
fn session_from_row(r: sqlx::sqlite::SqliteRow) -> Result<Session, StoreError> {
    Ok(Session {
        id: r.get("id"),
        name: r.get("name"),
        mode: parse_mode(r.get::<String, _>("mode").as_str())?,
        status: parse_status(r.get::<String, _>("status").as_str())?,
        interpreted_event_count: r.get::<i64, _>("interpreted_event_count") as u64,
        paused_total: Duration(r.get::<i64, _>("paused_total_ms")),
        paused_since: r.get::<Option<i64>, _>("paused_since").map(Instant),
    })
}

fn status_str(s: SessionStatus) -> &'static str {
    match s {
        SessionStatus::Draft => "DRAFT",
        SessionStatus::Ready => "READY",
        SessionStatus::Running => "RUNNING",
        SessionStatus::Paused => "PAUSED",
        SessionStatus::Completed => "COMPLETED",
        SessionStatus::Cancelled => "CANCELLED",
    }
}
fn parse_status(s: &str) -> Result<SessionStatus, StoreError> {
    match s {
        "DRAFT" => Ok(SessionStatus::Draft),
        "READY" => Ok(SessionStatus::Ready),
        "RUNNING" => Ok(SessionStatus::Running),
        "PAUSED" => Ok(SessionStatus::Paused),
        "COMPLETED" => Ok(SessionStatus::Completed),
        "CANCELLED" => Ok(SessionStatus::Cancelled),
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

    /// One of the venue's images, or `None` if nobody uploaded it (M6 follow-up).
    pub async fn venue_asset(&self, key: &str) -> Result<Option<VenueAsset>, StoreError> {
        let row = sqlx::query("SELECT media_type, bytes FROM venue_assets WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| VenueAsset {
            media_type: r.get("media_type"),
            bytes: r.get("bytes"),
        }))
    }

    /// Stores one, replacing whatever was there. A venue has one logo, not a gallery.
    pub async fn save_venue_asset(
        &self,
        key: &str,
        media_type: &str,
        bytes: &[u8],
        at: Instant,
        by: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO venue_assets (key, media_type, bytes, updated_at, updated_by)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(key) DO UPDATE SET
                media_type = excluded.media_type,
                bytes = excluded.bytes,
                updated_at = excluded.updated_at,
                updated_by = excluded.updated_by",
        )
        .bind(key)
        .bind(media_type)
        .bind(bytes)
        .bind(at.0)
        .bind(by)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_venue_asset(&self, key: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM venue_assets WHERE key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Forgets one reader's mapping (ADR 0007 §7, amended). The reads it produced stay in
    /// `raw_events`, which nothing here touches.
    pub async fn delete_reader(&self, device_id: &str, reader_id: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM readers WHERE device_id = ?1 AND reader_id = ?2")
            .bind(device_id)
            .bind(reader_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Every venue setting that has been chosen (M6 follow-up).
    pub async fn venue_settings(&self) -> Result<Vec<(String, String)>, StoreError> {
        let rows = sqlx::query("SELECT key, value FROM venue_settings ORDER BY key")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| (r.get("key"), r.get("value")))
            .collect())
    }

    /// Stores one, replacing any previous value. Keyed upsert, so setting the same number
    /// twice is one row rather than a history -- the history is the audit log.
    pub async fn save_venue_setting(
        &self,
        key: &str,
        value: &str,
        at: Instant,
        by: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO venue_settings (key, value, updated_at, updated_by)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at,
                updated_by = excluded.updated_by",
        )
        .bind(key)
        .bind(value)
        .bind(at.0)
        .bind(by)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Every reader the hub has ever heard from (M6 settings screen).
    ///
    /// Over `raw_events`, which holds the reads it could not attribute as well as the ones
    /// it could -- that is the whole point: an antenna nobody has configured yet is
    /// invisible everywhere except here.
    pub async fn reader_keys_seen(&self) -> Result<Vec<SeenReader>, StoreError> {
        let rows = sqlx::query(
            "SELECT device_id, reader_id, MAX(detected_at) AS last_seen, COUNT(*) AS reads
             FROM raw_events
             GROUP BY device_id, reader_id
             ORDER BY last_seen DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| SeenReader {
                device_id: r.get("device_id"),
                reader_id: r.get("reader_id"),
                last_seen: Instant(r.get::<i64, _>("last_seen")),
                reads: r.get("reads"),
            })
            .collect())
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
            "SELECT r.id, r.device_id, r.reader_id, r.detected_at,
                    c.session_id AS context_session_id, c.athlete_id AS context_athlete_id,
                    c.bound_session_id AS context_bound_session_id,
                    c.on_roster AS context_on_roster,
                    c.reader_station AS context_reader_station,
                    c.reader_mode AS context_reader_mode,
                    c.planned_json AS context_planned_json
             FROM raw_events r
             LEFT JOIN raw_interpretation_context c ON c.raw_event_id = r.id
             WHERE r.tag_id = ?1 COLLATE NOCASE AND r.detected_at >= ?2
               AND NOT EXISTS (
                   SELECT 1 FROM interpreted_events i WHERE i.raw_event_id = r.id)
             ORDER BY r.detected_at, r.id",
        )
        .bind(tag_id)
        .bind(since.0)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(StoredRawRead {
                    raw_event_id: r.get("id"),
                    device_id: r.get("device_id"),
                    reader_id: r.get("reader_id"),
                    detected_at: Instant(r.get::<i64, _>("detected_at")),
                    context: context_from_row(r)?,
                })
            })
            .collect()
    }

    /// Raw rows whose context contains a precomputed interpretation but whose interpreted row
    /// is still absent. The context table is the durable handoff between raw commit and replay.
    pub async fn pending_raw_reads(
        &self,
        session_id: &str,
    ) -> Result<Vec<StoredRawRead>, StoreError> {
        let rows = sqlx::query(
            "SELECT r.id, r.device_id, r.reader_id, r.detected_at,
                    c.session_id AS context_session_id, c.athlete_id AS context_athlete_id,
                    c.bound_session_id AS context_bound_session_id,
                    c.on_roster AS context_on_roster,
                    c.reader_station AS context_reader_station,
                    c.reader_mode AS context_reader_mode,
                    c.planned_json AS context_planned_json
             FROM raw_events r
             JOIN raw_interpretation_context c ON c.raw_event_id = r.id
             WHERE c.session_id = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM interpreted_events i WHERE i.raw_event_id = r.id)
             ORDER BY r.detected_at, r.id",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(StoredRawRead {
                    raw_event_id: r.get("id"),
                    device_id: r.get("device_id"),
                    reader_id: r.get("reader_id"),
                    detected_at: Instant(r.get::<i64, _>("detected_at")),
                    context: context_from_row(r)?,
                })
            })
            .collect()
    }

    pub async fn raw_interpretation_context(
        &self,
        raw_event_id: i64,
    ) -> Result<Option<RawInterpretationContext>, StoreError> {
        let row = sqlx::query(
            "SELECT session_id AS context_session_id,
                    athlete_id AS context_athlete_id,
                    bound_session_id AS context_bound_session_id,
                    on_roster AS context_on_roster,
                    reader_station AS context_reader_station,
                    reader_mode AS context_reader_mode,
                    planned_json AS context_planned_json
             FROM raw_interpretation_context
             WHERE raw_event_id = ?1",
        )
        .bind(raw_event_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.as_ref().map(context_from_row).transpose()?.flatten())
    }

    /// Returns the non-voided interpretation log in the same order used by replay.  The id is
    /// retained for same-millisecond ordering and for historical corrections that replace one
    /// existing row.
    pub async fn interpretations(
        &self,
        session_id: &str,
    ) -> Result<Vec<application::StoredInterpretation>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, athlete_id, raw_event_id, kind, station, detected_at,
                    transition_ms, started_timing, exception_reason
             FROM interpreted_events
             WHERE session_id = ?1 AND voided_at IS NULL
             ORDER BY detected_at, id",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(application::StoredInterpretation {
                    interpreted_event_id: row.get("id"),
                    athlete_id: row.get("athlete_id"),
                    raw_event_id: row.get("raw_event_id"),
                    event: row_to_interpreted(row)?,
                })
            })
            .collect()
    }
}

fn context_from_row(
    r: &sqlx::sqlite::SqliteRow,
) -> Result<Option<RawInterpretationContext>, StoreError> {
    let Some(session_id) = r.get::<Option<String>, _>("context_session_id") else {
        return Ok(None);
    };
    let reader = match (
        r.get::<Option<String>, _>("context_reader_station"),
        r.get::<Option<String>, _>("context_reader_mode"),
    ) {
        (Some(station), Some(mode)) => Some(ReaderBinding {
            station,
            mode: parse_reader_mode(&mode)?,
        }),
        (None, None) => None,
        _ => {
            return Err(StoreError::Corrupt(
                "raw context has incomplete reader".into(),
            ))
        }
    };
    let planned = r
        .get::<Option<String>, _>("context_planned_json")
        .map(|json| serde_json::from_str(&json))
        .transpose()?;
    Ok(Some(RawInterpretationContext {
        session_id,
        athlete_id: r.get("context_athlete_id"),
        bound_session_id: r.get("context_bound_session_id"),
        on_roster: r.get::<i64, _>("context_on_roster") != 0,
        reader,
        planned,
    }))
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

fn finish_origin_name(origin: FinishOrigin) -> &'static str {
    match origin {
        FinishOrigin::Policy => "POLICY",
        FinishOrigin::Manual => "MANUAL",
    }
}

/// Legacy rows predate `finish_origin`. Preserve a coach's manual stop, but recognise the
/// old CourseComplete implementation's stored value when it exactly equals the final exit
/// that the event log already derives. Future rows always carry an explicit origin.
fn stored_finish_is_manual(
    origin: Option<&str>,
    finished_at: Instant,
    state: Option<&AthleteState>,
    course: Option<&domain::Course>,
) -> bool {
    match origin {
        Some("MANUAL") => true,
        Some("POLICY") => false,
        Some(_) => true,
        None => !matches!(
            state.map(|state| {
                domain::FinishPolicy::CourseComplete.evaluate(
                    state,
                    domain::ClassClock::started_at(Instant(0)),
                    Instant(0),
                    course,
                )
            }),
            Some(domain::FinishDecision::Finished { at }) if at == finished_at
        ),
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
