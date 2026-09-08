//! Reader configuration: which `(device_id, reader_id)` means which station (CLAUDE.md 8).
//!
//! The ESP32 publishes hardware identity and nothing else, so this mapping is the entire
//! difference between a read the hub can attribute and an `UNKNOWN_READER` exception. It is
//! venue configuration, not session data: the readers on the wall outlive any one class,
//! and they are stored so a restart resolves reads exactly as the run before it did
//! (CLAUDE.md 21; ADR 0004).

use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, AuditedResult, AuditedWrite, HubStore, SeenReader};
use domain::{ReaderKey, ReaderRegistration};

/// Registers or reconfigures one reader, in the store first and then in memory.
///
/// Audited only when the mapping actually changes. Re-registering an unchanged reader is
/// what every startup does, and a trail full of "SKIERG ENTRY is still SKIERG ENTRY" would
/// bury the one line that matters -- the evening someone repointed a reader mid-class
/// (CLAUDE.md 20).
pub async fn register_reader<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    registration: &ReaderRegistration,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    crate::ingest::reconcile_pending(state, store)
        .await
        .map_err(OperatorError::Storage)?;
    let previous = state.readers.resolve(&registration.key).ok().cloned();
    if previous.as_ref() == Some(registration) {
        return Ok(());
    }

    store
        .apply_audited(AuditedWrite::ReaderSave {
            registration: registration.clone(),
            audit: AuditEntry {
                at: cmd.at,
                operator: cmd.operator.clone(),
                action: "READER_REGISTER".to_string(),
                subject: format!(
                    "{}/{}",
                    registration.key.device_id, registration.key.reader_id
                ),
                reason: cmd.stated_reason().map(str::to_string),
                before: previous.map(|p| describe(&p)),
                after: Some(describe(registration)),
            },
        })
        .await
        .map_err(OperatorError::Storage)?;
    state.readers.register(registration.clone());
    Ok(())
}

fn describe(r: &ReaderRegistration) -> String {
    match &r.zone {
        Some(zone) => format!("{} {:?} in {}", r.station, r.mode, zone),
        None => format!("{} {:?}", r.station, r.mode),
    }
}

/// The readers the hub has heard from and cannot resolve, most recently tapped first.
///
/// The order is the order somebody walks a floor in: tap an antenna, assign the reader that
/// just jumped to the top, tap the next one. Registering one removes it from here, because
/// "unregistered" is derived from the registry rather than from a list somebody maintains.
pub async fn unregistered_readers<S: HubStore>(
    readers: &domain::ReaderRegistry,
    store: &S,
) -> Result<Vec<SeenReader>, S::Error> {
    let mut seen = store.reader_keys_seen().await?;
    seen.retain(|r| match ReaderKey::parse(&r.device_id, &r.reader_id) {
        Ok(key) => readers.resolve(&key).is_err(),
        // A key the domain will not even parse can never be registered, so it stays on the
        // list: somebody has to see it to know their firmware is sending something odd.
        Err(_) => true,
    });
    seen.sort_by(|a, b| b.last_seen.0.cmp(&a.last_seen.0));
    Ok(seen)
}

/// Takes a reader off the map (ADR 0007 §7, amended 2026-09-02).
///
/// Needs a reason, like every other action that changes what the hub will record
/// (CLAUDE.md 20): from here on, reads from this antenna are `UNKNOWN_READER` exceptions
/// rather than progress. Nothing already recorded moves -- `raw_events` keeps the device
/// and reader that produced every read, and an interpretation names the station, not the
/// reader -- so this is a decision about the future only.
///
/// The audit row carries what the reader *used to mean*, because that is the thing nobody
/// can reconstruct afterwards from the map itself.
pub async fn unregister_reader<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    key: &ReaderKey,
    cmd: &OperatorCommand,
) -> Result<(), OperatorError<S::Error>> {
    crate::ingest::reconcile_pending(state, store)
        .await
        .map_err(OperatorError::Storage)?;
    let Some(reason) = cmd.stated_reason() else {
        return Err(OperatorError::ReasonRequired);
    };
    let Some(existing) = state.readers.resolve(key).ok().cloned() else {
        return Err(OperatorError::UnknownReader {
            device_id: key.device_id.to_string(),
            reader_id: key.reader_id.to_string(),
        });
    };

    let applied = store
        .apply_audited(AuditedWrite::ReaderDelete {
            device_id: key.device_id.to_string(),
            reader_id: key.reader_id.to_string(),
            audit: AuditEntry {
                at: cmd.at,
                operator: cmd.operator.clone(),
                action: "READER_REMOVE".to_string(),
                subject: format!("{} {}", key.device_id, key.reader_id),
                reason: Some(reason.to_string()),
                before: Some(format!("{} {}", existing.station, mode_name(existing.mode))),
                after: None,
            },
        })
        .await
        .map_err(OperatorError::Storage)?;
    if !matches!(applied, AuditedResult::Applied(true)) {
        return Err(OperatorError::UnknownReader {
            device_id: key.device_id.to_string(),
            reader_id: key.reader_id.to_string(),
        });
    }
    state.readers.remove(key);
    Ok(())
}

/// The mode as the audit trail spells it. Not `Debug`: a record somebody reads a year later
/// should not depend on a derive.
fn mode_name(mode: domain::ReaderMode) -> &'static str {
    match mode {
        domain::ReaderMode::Entry => "ENTRY",
        domain::ReaderMode::Exit => "EXIT",
        domain::ReaderMode::Toggle => "TOGGLE",
        domain::ReaderMode::Checkpoint => "CHECKPOINT",
        domain::ReaderMode::Passage => "PASSAGE",
    }
}
