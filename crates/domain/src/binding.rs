//! RFID tag ↔ athlete binding (CLAUDE.md 7.2; ADR 0001 D3).
//!
//! The ledger is append-only. A binding is ended by stamping `unbound_at`, never by
//! rewriting who held the tag, because the audit trail for a manual correction has to be
//! able to answer "who was wearing this band at 10:15" after the fact (CLAUDE.md 20).

use crate::time::Instant;
use serde::Serialize;
use std::fmt;

/// The EPC as the reader reported it. Normalised to upper case only: the real tag format
/// depends on hardware not yet validated in the venue (CLAUDE.md 28), so imposing a
/// length or an alphabet here would reject live reads for no defensible reason.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize)]
#[serde(transparent)]
pub struct TagId(String);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TagIdError {
    Empty,
}

impl TagId {
    pub fn parse(raw: &str) -> Result<Self, TagIdError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TagIdError::Empty);
        }
        Ok(Self(trimmed.to_ascii_uppercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TagId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One binding window. `unbound_at == None` means it is the one in force.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TagBinding {
    pub session_id: String,
    pub tag_id: TagId,
    pub athlete_id: String,
    pub bound_at: Instant,
    pub unbound_at: Option<Instant>,
}

impl TagBinding {
    pub fn is_active(&self) -> bool {
        self.unbound_at.is_none()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BindingError {
    /// The band is already on someone's wrist -- possibly in another session.
    TagAlreadyBound { session_id: String, athlete_id: String },
    /// One athlete, one active tag per session (ADR D3).
    AthleteAlreadyBound { tag_id: TagId },
    NotBound,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BindingLedger {
    entries: Vec<TagBinding>,
}

impl BindingLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild a ledger from stored rows (CLAUDE.md 21). Closed bindings are kept, not
    /// filtered: the history is the audit trail, and a restart that dropped it would make
    /// "who was wearing this band at 10:15" unanswerable after the fact (CLAUDE.md 20).
    ///
    /// Order must be `bound_at` ascending, which is the order the invariants were checked
    /// in when the rows were first written.
    pub fn restore(entries: Vec<TagBinding>) -> Self {
        Self { entries }
    }

    /// Bind a tag to an athlete for a session.
    ///
    /// Idempotent for an identical, already-active pair: the check-in tablet double-taps,
    /// and a second record would fork the history for no operational gain.
    pub fn bind(
        &mut self,
        session_id: &str,
        tag_id: &TagId,
        athlete_id: &str,
        at: Instant,
    ) -> Result<(), BindingError> {
        // Tag uniqueness is checked across all sessions: one physical band cannot be on
        // two wrists at once, whichever class each belongs to.
        if let Some(held) = self.active_for_tag(tag_id) {
            if held.session_id == session_id && held.athlete_id == athlete_id {
                return Ok(());
            }
            return Err(BindingError::TagAlreadyBound {
                session_id: held.session_id.clone(),
                athlete_id: held.athlete_id.clone(),
            });
        }
        // Athlete uniqueness is per session, because the same person may appear in the
        // roster of two sessions with two different bands.
        if let Some(existing) = self.active_for_athlete(session_id, athlete_id) {
            return Err(BindingError::AthleteAlreadyBound { tag_id: existing.tag_id.clone() });
        }

        self.entries.push(TagBinding {
            session_id: session_id.to_string(),
            tag_id: tag_id.clone(),
            athlete_id: athlete_id.to_string(),
            bound_at: at,
            unbound_at: None,
        });
        Ok(())
    }

    /// Close the active binding for a tag in this session. The record stays.
    pub fn unbind(
        &mut self,
        session_id: &str,
        tag_id: &TagId,
        at: Instant,
    ) -> Result<(), BindingError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|b| b.is_active() && b.session_id == session_id && &b.tag_id == tag_id)
            .ok_or(BindingError::NotBound)?;
        entry.unbound_at = Some(at);
        Ok(())
    }

    /// Swap an athlete onto a different band: close the old binding, open a new one, two
    /// auditable records (ADR D3). Validated fully before anything is written, so a
    /// rejected swap can never leave the athlete with no tag at all.
    pub fn rebind_athlete(
        &mut self,
        session_id: &str,
        athlete_id: &str,
        new_tag: &TagId,
        at: Instant,
    ) -> Result<(), BindingError> {
        if let Some(held) = self.active_for_tag(new_tag) {
            if held.session_id == session_id && held.athlete_id == athlete_id {
                return Ok(());
            }
            return Err(BindingError::TagAlreadyBound {
                session_id: held.session_id.clone(),
                athlete_id: held.athlete_id.clone(),
            });
        }

        let old_tag = self.active_for_athlete(session_id, athlete_id).map(|b| b.tag_id.clone());
        if let Some(old_tag) = old_tag {
            self.unbind(session_id, &old_tag, at)?;
        }
        self.bind(session_id, new_tag, athlete_id, at)
    }

    pub fn athlete_for_tag(&self, session_id: &str, tag_id: &TagId) -> Option<&str> {
        self.entries
            .iter()
            .find(|b| b.is_active() && b.session_id == session_id && &b.tag_id == tag_id)
            .map(|b| b.athlete_id.as_str())
    }

    pub fn tag_for_athlete(&self, session_id: &str, athlete_id: &str) -> Option<&TagId> {
        self.active_for_athlete(session_id, athlete_id).map(|b| &b.tag_id)
    }

    pub fn active(&self) -> impl Iterator<Item = &TagBinding> {
        self.entries.iter().filter(|b| b.is_active())
    }

    /// Every binding ever made, closed ones included. This is the audit trail.
    pub fn history(&self) -> &[TagBinding] {
        &self.entries
    }

    fn active_for_tag(&self, tag_id: &TagId) -> Option<&TagBinding> {
        self.entries.iter().find(|b| b.is_active() && &b.tag_id == tag_id)
    }

    fn active_for_athlete(&self, session_id: &str, athlete_id: &str) -> Option<&TagBinding> {
        self.entries
            .iter()
            .find(|b| b.is_active() && b.session_id == session_id && b.athlete_id == athlete_id)
    }
}
