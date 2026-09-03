//! Venue settings: the few numbers a site adjusts for itself (M6 follow-up).
//!
//! Not session configuration. A `SessionConfig` describes one class -- its course, its
//! finish rule -- and is snapshotted so a later edit cannot rewrite what happened
//! (ADR 0008). These outlive every class, like the reader map: they describe the room.
//!
//! Deliberately a short, closed list rather than free-form key/value. An unknown key is a
//! typo, and a typo that stores successfully is a setting somebody will swear they changed.

use crate::operator::OperatorCommand;
use crate::ports::{AuditEntry, HubStore};

/// How long the live screen holds a page before rotating, when the venue has not said.
/// Ten seconds: long enough to find your own name, short enough that the next page is not
/// a wait.
pub const DEFAULT_LIVE_PAGE_MS: i64 = 10_000;

/// Sanity, not product policy. Under three seconds is unreadable; over two minutes is
/// broken in a way nobody notices until a class has run its whole length.
pub const LIVE_PAGE_MS_RANGE: std::ops::RangeInclusive<i64> = 3_000..=120_000;

pub const LIVE_PAGE_MS: &str = "live.page_ms";
pub const LIVE_PAGE_SIZE: &str = "live.page_size";

/// How many athletes fit on one page of the live screen, and the grid that holds them.
///
/// A closed list, not a number: 2560x1440 minus the header and footer is a fixed area, and
/// only a few splits of it produce cards that are square-ish and readable across a room.
/// Seven people would leave a ragged row and cards nobody chose the proportions of.
///
/// The tuple is `(page size, columns, rows)`; the screen reads the geometry from here so
/// the set is defined once rather than in both the picker and the projector.
pub const LIVE_PAGE_LAYOUTS: [(i64, i64, i64); 4] = [
    // Card sizes on the projector, for the record: 840x615, 620x405, 495x300, 410x240.
    (6, 3, 2),  // a small class in a big room
    (12, 4, 3), // the default the screen was designed around
    (20, 5, 4),
    (30, 6, 5), // as dense as stays legible at ten metres
];

pub const DEFAULT_LIVE_PAGE_SIZE: i64 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VenueSettings {
    pub live_page_ms: i64,
    pub live_page_size: i64,
}

impl Default for VenueSettings {
    fn default() -> Self {
        Self {
            live_page_ms: DEFAULT_LIVE_PAGE_MS,
            live_page_size: DEFAULT_LIVE_PAGE_SIZE,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingError<E> {
    /// A key this build does not define. Refused rather than stored: a stored typo is a
    /// setting somebody will swear they changed.
    #[error("no venue setting named {0:?}")]
    Unknown(String),
    #[error("{key}: {message}")]
    Invalid { key: String, message: String },
    #[error("store write failed")]
    Storage(E),
}

/// What this venue has chosen, with the shipped defaults filling every gap.
///
/// A stored value that will not parse, or that is out of range, falls back to the default
/// rather than failing: a row edited by hand into something absurd must not take the
/// projector down.
pub async fn venue_settings<S: HubStore>(store: &S) -> Result<VenueSettings, S::Error> {
    let mut settings = VenueSettings::default();
    for (key, value) in store.venue_settings().await? {
        if key == LIVE_PAGE_MS {
            if let Ok(ms) = value.parse::<i64>() {
                if LIVE_PAGE_MS_RANGE.contains(&ms) {
                    settings.live_page_ms = ms;
                }
            }
        } else if key == LIVE_PAGE_SIZE {
            if let Ok(size) = value.parse::<i64>() {
                if LIVE_PAGE_LAYOUTS.iter().any(|(s, _, _)| *s == size) {
                    settings.live_page_size = size;
                }
            }
        }
    }
    Ok(settings)
}

/// Stores one setting, after checking that it is one we define and that the value is usable.
pub async fn save_venue_setting<S: HubStore>(
    store: &S,
    key: &str,
    value: &str,
    cmd: &OperatorCommand,
) -> Result<(), SettingError<S::Error>> {
    match key {
        LIVE_PAGE_MS => {
            let ms: i64 = value.trim().parse().map_err(|_| SettingError::Invalid {
                key: key.to_string(),
                message: format!("{value:?} is not a number of milliseconds"),
            })?;
            if !LIVE_PAGE_MS_RANGE.contains(&ms) {
                return Err(SettingError::Invalid {
                    key: key.to_string(),
                    message: format!(
                        "{ms} is outside {}..={}",
                        LIVE_PAGE_MS_RANGE.start(),
                        LIVE_PAGE_MS_RANGE.end()
                    ),
                });
            }
        }
        LIVE_PAGE_SIZE => {
            let size: i64 = value.trim().parse().map_err(|_| SettingError::Invalid {
                key: key.to_string(),
                message: format!("{value:?} is not a page size"),
            })?;
            // An offered layout or nothing. This is the difference between a setting and a
            // free-for-all: every value on the list has had its card proportions chosen.
            if !LIVE_PAGE_LAYOUTS.iter().any(|(s, _, _)| *s == size) {
                return Err(SettingError::Invalid {
                    key: key.to_string(),
                    message: format!(
                        "{size} is not one of the offered layouts ({})",
                        LIVE_PAGE_LAYOUTS
                            .iter()
                            .map(|(s, _, _)| s.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                });
            }
        }
        other => return Err(SettingError::Unknown(other.to_string())),
    }

    store
        .save_venue_setting(key, value.trim(), cmd.at, &cmd.operator)
        .await
        .map_err(SettingError::Storage)?;
    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "VENUE_SETTING".to_string(),
            subject: key.to_string(),
            reason: None,
            before: None,
            after: Some(value.trim().to_string()),
        })
        .await
        .map_err(SettingError::Storage)
}
