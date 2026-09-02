//! The venue's own images (M6 follow-up). Today: a logo for the screens.
//!
//! Two rules carry this module, and both are about serving a file somebody uploaded back to
//! a browser on the same origin:
//!
//! * **PNG and JPEG only.** Not SVG. An SVG is a document that may contain script, and this
//!   hub would serve it from its own origin to every screen in the venue -- including the
//!   operator surface. A gym's logo is not worth that.
//! * **Checked by content, not by file name.** `logo.png` is a claim; the magic bytes at
//!   the front of the file are the fact.

use crate::operator::OperatorCommand;
use crate::ports::{AuditEntry, HubStore};

pub const VENUE_LOGO: &str = "venue.logo";

/// Big enough for a detailed mark at projector size, small enough that it cannot bloat the
/// database or the nightly backup. A logo past this is a photograph by mistake.
pub const MAX_ASSET_BYTES: usize = 512 * 1024;

pub const PNG: &str = "image/png";
pub const JPEG: &str = "image/jpeg";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VenueAsset {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum AssetError<E> {
    #[error("{0} bytes is larger than the {MAX_ASSET_BYTES} byte limit")]
    TooLarge(usize),
    /// Not a picture this hub will serve. The message names what was recognised so an
    /// operator who exported the wrong thing can tell.
    #[error("{0}")]
    Unsupported(String),
    #[error("store write failed")]
    Storage(E),
}

/// What kind of image this actually is, read from its first bytes.
///
/// A browser decides how to treat a file by what it is told it is, so the type served has
/// to come from the content rather than from a name anybody can write.
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];
    if bytes.starts_with(PNG_MAGIC) {
        Some(PNG)
    } else if bytes.starts_with(JPEG_MAGIC) {
        Some(JPEG)
    } else {
        None
    }
}

pub async fn venue_asset<S: HubStore>(
    store: &S,
    key: &str,
) -> Result<Option<VenueAsset>, S::Error> {
    store.venue_asset(key).await
}

/// Stores one image after checking it is one, and audits who put it there.
pub async fn save_venue_asset<S: HubStore>(
    store: &S,
    key: &str,
    bytes: Vec<u8>,
    cmd: &OperatorCommand,
) -> Result<VenueAsset, AssetError<S::Error>> {
    if bytes.len() > MAX_ASSET_BYTES {
        return Err(AssetError::TooLarge(bytes.len()));
    }
    let media_type = sniff(&bytes).ok_or_else(|| {
        AssetError::Unsupported(if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") {
            // Worth naming: SVG is the format somebody will reach for first, and refusing
            // it silently looks like a bug rather than a decision.
            "SVG is not accepted: it can carry script, and the hub would serve it from its \
             own origin. Export the logo as PNG."
                .to_string()
        } else {
            "not a PNG or JPEG image".to_string()
        })
    })?;

    store
        .save_venue_asset(key, media_type, &bytes, cmd.at, &cmd.operator)
        .await
        .map_err(AssetError::Storage)?;
    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "VENUE_ASSET".to_string(),
            subject: key.to_string(),
            reason: None,
            before: None,
            after: Some(format!("{media_type}, {} bytes", bytes.len())),
        })
        .await
        .map_err(AssetError::Storage)?;
    Ok(VenueAsset { media_type: media_type.to_string(), bytes })
}

/// Removes it. The screens fall back to showing no logo, which is what they did before one
/// was uploaded -- nothing breaks, the corner is simply empty again.
pub async fn delete_venue_asset<S: HubStore>(
    store: &S,
    key: &str,
    cmd: &OperatorCommand,
) -> Result<(), AssetError<S::Error>> {
    store.delete_venue_asset(key).await.map_err(AssetError::Storage)?;
    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: "VENUE_ASSET".to_string(),
            subject: key.to_string(),
            reason: None,
            before: None,
            after: Some("removed".to_string()),
        })
        .await
        .map_err(AssetError::Storage)
}
