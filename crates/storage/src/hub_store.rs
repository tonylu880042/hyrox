//! `Store` as the application's persistence port (CLAUDE.md 3).
//!
//! The dependency runs this way round on purpose: the application declares what it needs
//! and this crate satisfies it. `application` knows nothing about SQLite, and swapping the
//! store means writing another adapter here, not touching a use case.

use crate::{RawEvent, Store, StoreError};
use application::{
    AuditEntry, HubStore, InterpretedWrite, RawCommit, RawRead, StoredException, StoredRawRead, SeenReader, VenueAsset,
};
use domain::{
    AthleteState, BindingLedger, Exercise, ExerciseLibrary, Instant, PhysicalStation,
    ReaderRegistration, ReaderRegistry, Session, SessionConfig, StationMap, TagBinding,
    WorkoutTemplate,
};
use contract::CommitOutcome;

impl HubStore for Store {
    type Error = StoreError;

    /// Returns `Ok` only once SQLite has committed, which is what the ACK is issued against
    /// (CLAUDE.md 15; ADR 0002).
    async fn commit_raw(&self, raw: &RawRead) -> Result<RawCommit, StoreError> {
        let (id, inserted) = self
            .save_raw(&RawEvent {
                device_id: raw.device_id.clone(),
                reader_id: raw.reader_id.clone(),
                boot_id: raw.boot_id,
                sequence: raw.sequence,
                tag_id: raw.tag_id.clone(),
                detected_at: raw.detected_at,
                received_at: raw.received_at,
            })
            .await?;
        Ok(RawCommit {
            raw_event_id: id,
            outcome: if inserted { CommitOutcome::Stored } else { CommitOutcome::AlreadyStored },
        })
    }

    async fn commit_interpreted(&self, w: InterpretedWrite<'_>) -> Result<i64, StoreError> {
        self.save_interpreted(w.session_id, w.athlete_id, w.raw_event_id, w.event)
            .await
    }

    async fn save_session(&self, session: &Session, created_at: Instant) -> Result<(), StoreError> {
        Store::save_session(self, session, created_at).await
    }

    async fn save_athlete(
        &self,
        session_id: &str,
        athlete_id: &str,
        display_name: &str,
        bib: i64,
        member_id: Option<&str>,
    ) -> Result<(), StoreError> {
        Store::save_athlete(self, session_id, athlete_id, display_name, bib, member_id).await
    }

    async fn active_session(&self) -> Result<Option<Session>, StoreError> {
        Store::active_session(self).await
    }

    async fn session(&self, session_id: &str) -> Result<Option<Session>, StoreError> {
        Store::session(self, session_id).await
    }

    async fn athlete_bibs(&self, session_id: &str) -> Result<Vec<(String, i64)>, StoreError> {
        Store::athlete_bibs(self, session_id).await
    }

    async fn save_athlete_finish(
        &self,
        session_id: &str,
        athlete_id: &str,
        finished_at: Option<Instant>,
    ) -> Result<(), StoreError> {
        Store::save_athlete_finish(self, session_id, athlete_id, finished_at).await
    }

    async fn rebuild_athletes(&self, session_id: &str) -> Result<Vec<AthleteState>, StoreError> {
        Store::rebuild_athletes(self, session_id).await
    }

    async fn session_created_at(&self, session_id: &str) -> Result<Option<Instant>, StoreError> {
        Store::session_created_at(self, session_id).await
    }

    async fn exception_count(&self, session_id: &str) -> Result<usize, StoreError> {
        Ok(Store::exception_count(self, session_id).await? as usize)
    }

    async fn exceptions(&self, session_id: &str) -> Result<Vec<StoredException>, StoreError> {
        Store::exceptions(self, session_id).await
    }

    async fn void_interpreted(
        &self,
        interpreted_event_id: i64,
        at: Instant,
        operator: &str,
        reason: &str,
    ) -> Result<bool, StoreError> {
        Store::void_interpreted(self, interpreted_event_id, at, operator, reason).await
    }

    async fn backup_to(&self, path: &std::path::Path) -> Result<(), StoreError> {
        Store::backup_to(self, path).await
    }

    async fn delete_reader(&self, device_id: &str, reader_id: &str) -> Result<(), StoreError> {
        Store::delete_reader(self, device_id, reader_id).await
    }

    async fn venue_settings(&self) -> Result<Vec<(String, String)>, StoreError> {
        Store::venue_settings(self).await
    }

    async fn save_venue_setting(
        &self,
        key: &str,
        value: &str,
        at: Instant,
        by: &str,
    ) -> Result<(), StoreError> {
        Store::save_venue_setting(self, key, value, at, by).await
    }

    async fn venue_asset(&self, key: &str) -> Result<Option<VenueAsset>, StoreError> {
        Store::venue_asset(self, key).await
    }

    async fn save_venue_asset(
        &self,
        key: &str,
        media_type: &str,
        bytes: &[u8],
        at: Instant,
        by: &str,
    ) -> Result<(), StoreError> {
        Store::save_venue_asset(self, key, media_type, bytes, at, by).await
    }

    async fn delete_venue_asset(&self, key: &str) -> Result<(), StoreError> {
        Store::delete_venue_asset(self, key).await
    }

    async fn record_audit(&self, entry: &AuditEntry) -> Result<(), StoreError> {
        self.save_audit(entry).await
    }

    async fn save_session_config(&self, config: &SessionConfig) -> Result<(), StoreError> {
        Store::save_session_config(self, config).await
    }

    async fn session_config(&self, session_id: &str) -> Result<Option<SessionConfig>, StoreError> {
        Store::session_config(self, session_id).await
    }

    async fn save_reader(&self, registration: &ReaderRegistration) -> Result<(), StoreError> {
        Store::save_reader(self, registration).await
    }

    async fn readers(&self) -> Result<ReaderRegistry, StoreError> {
        Store::readers(self).await
    }

    async fn save_binding(&self, binding: &TagBinding) -> Result<(), StoreError> {
        Store::save_binding(self, binding).await
    }

    async fn bindings(&self) -> Result<BindingLedger, StoreError> {
        Store::bindings(self).await
    }

    async fn reader_keys_seen(&self) -> Result<Vec<SeenReader>, StoreError> {
        Store::reader_keys_seen(self).await
    }

    async fn raw_tags_since(&self, since: Instant) -> Result<Vec<String>, StoreError> {
        Store::raw_tags_since(self, since).await
    }

    async fn unclaimed_reads_for_tag(
        &self,
        tag_id: &str,
        since: Instant,
    ) -> Result<Vec<StoredRawRead>, StoreError> {
        Store::unclaimed_reads_for_tag(self, tag_id, since).await
    }

    // --- the workout library (ADR 0008) --------------------------------------------------

    async fn save_template(&self, template: &WorkoutTemplate) -> Result<(), StoreError> {
        Store::save_template(self, template).await
    }

    async fn template(&self, template_id: &str) -> Result<Option<WorkoutTemplate>, StoreError> {
        Store::template(self, template_id).await
    }

    async fn templates(&self) -> Result<Vec<WorkoutTemplate>, StoreError> {
        Store::templates(self).await
    }

    async fn delete_template(&self, template_id: &str) -> Result<bool, StoreError> {
        Store::delete_template(self, template_id).await
    }

    async fn save_exercise(&self, exercise: &Exercise) -> Result<(), StoreError> {
        Store::save_exercise(self, exercise).await
    }

    async fn exercises(&self) -> Result<ExerciseLibrary, StoreError> {
        Store::exercises(self).await
    }

    async fn save_station(&self, station: &PhysicalStation) -> Result<(), StoreError> {
        Store::save_station(self, station).await
    }

    async fn stations(&self) -> Result<StationMap, StoreError> {
        Store::stations(self).await
    }
}
