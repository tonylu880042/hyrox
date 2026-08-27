//! Member reference (CLAUDE.md 7.1).
//!
//! The complete profile lives in 健身管. The hub keeps only what it needs to put a name
//! on a live screen and to know whether the source system considers the membership good;
//! duplicating the member database here would create a second source of truth to reconcile.

use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MembershipStatus {
    Active,
    Suspended,
    Expired,
    /// 健身管 reported something this hub does not recognise. The exact API contract is
    /// an open issue (CLAUDE.md 28), so an unmapped value must stay visibly unknown rather
    /// than collapsing into Active and granting more than the source system did.
    Unknown,
}

/// Reported by 健身管; the hub does not infer it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Gender {
    Female,
    Male,
    Other,
}

/// Fetched from 健身管 (the hub calls them, keyed by a member id obtained from a QR code).
/// Every field beyond the identity pair is optional: the source system may not hold it,
/// and a missing value must never block timing (CLAUDE.md 31).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MemberRef {
    pub member_id: String,
    pub display_name: String,
    /// Informational only. Confirmed with the user 2026-08-27: the hub does NOT gate
    /// timing on membership validity. If 健身管 returns the member, they may be timed.
    pub status: MembershipStatus,
    pub gender: Option<Gender>,
    /// Age as 健身管 reported it, not a birth date, so it is a snapshot that goes stale.
    /// Age-group divisions computed months later must re-fetch rather than trust this.
    pub age: Option<u8>,
    /// Where the portrait lives. Kept as a reference, not bytes: caching it for offline
    /// use is an infrastructure concern (CLAUDE.md 31), not a domain one.
    pub photo_url: Option<String>,
    pub height_cm: Option<u16>,
    /// Whole kilograms. The hub only displays this; it never computes on it.
    pub weight_kg: Option<u16>,
}

impl MemberRef {
    pub fn new(
        member_id: impl Into<String>,
        display_name: impl Into<String>,
        status: MembershipStatus,
    ) -> Self {
        Self {
            member_id: member_id.into(),
            display_name: display_name.into(),
            status,
            gender: None,
            age: None,
            photo_url: None,
            height_cm: None,
            weight_kg: None,
        }
    }

}
