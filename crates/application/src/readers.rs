//! Reader configuration: which `(device_id, reader_id)` means which station (CLAUDE.md 8).
//!
//! The ESP32 publishes hardware identity and nothing else, so this mapping is the entire
//! difference between a read the hub can attribute and an `UNKNOWN_READER` exception. It is
//! venue configuration, not session data: the readers on the wall outlive any one class,
//! and they are stored so a restart resolves reads exactly as the run before it did
//! (CLAUDE.md 21; ADR 0004).

use crate::live_session::LiveSession;
use crate::operator::{OperatorCommand, OperatorError};
use crate::ports::{AuditEntry, HubStore};
use domain::ReaderRegistration;

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
    let previous = state.readers.resolve(&registration.key).ok().cloned();
    if previous.as_ref() == Some(registration) {
        return Ok(());
    }

    store
        .save_reader(registration)
        .await
        .map_err(OperatorError::Storage)?;
    state.readers.register(registration.clone());

    store
        .record_audit(&AuditEntry {
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
        })
        .await
        .map_err(OperatorError::Storage)?;
    Ok(())
}

fn describe(r: &ReaderRegistration) -> String {
    match &r.zone {
        Some(zone) => format!("{} {:?} in {}", r.station, r.mode, zone),
        None => format!("{} {:?}", r.station, r.mode),
    }
}
