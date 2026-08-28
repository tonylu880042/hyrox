//! Startup: resume the interrupted session, or begin a new one (CLAUDE.md 21).
//!
//! Nothing critical lives only in memory. An ARMED session found in the store is resumed by
//! replaying its interpreted events, which is why a restart cannot disagree with the log --
//! the log is the only source.
//!
//! The reader registry and the binding ledger are *not* recovered here: Phase 1 has no
//! tables for them, so the caller supplies them at startup. That gap is real and is called
//! out in the crate docs.

use crate::live_session::LiveSession;
use crate::ports::HubStore;
use domain::{AthleteState, Instant, Session, SessionConfig};

/// What to start if there is nothing to resume.
pub struct SessionPlan {
    /// A DRAFT session; it is armed as part of starting.
    pub session: Session,
    pub config: SessionConfig,
    pub roster: Vec<RosterEntry>,
    pub class_start: Instant,
}

pub struct RosterEntry {
    pub athlete_id: String,
    pub display_name: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Recovery {
    /// An ARMED session was found and rebuilt from its events.
    Resumed,
    Started,
}

/// Resumes an ARMED session if the store has one, otherwise arms and persists the plan.
///
/// Only an ARMED session is resumed: a CLOSED one is finished business, and a DRAFT one was
/// never accepting events (ADR 0001 D2).
pub async fn resume_or_start<S: HubStore>(
    store: &S,
    plan: SessionPlan,
) -> Result<(LiveSession, Recovery), S::Error> {
    if let Some(existing) = store.active_session().await? {
        if existing.accepts_events() {
            // The stored creation time is the class clock's origin; falling back to the
            // plan's would restart the class clock at zero on every reboot.
            let class_start = store
                .session_created_at(&existing.id)
                .await?
                .unwrap_or(plan.class_start);
            let athletes = store.rebuild_athletes(&existing.id).await?;
            let exceptions = store.exception_count(&existing.id).await?;
            let mut state = LiveSession::new(existing, plan.config, class_start)
                .with_athletes(athletes);
            state.exception_count = exceptions;
            return Ok((state, Recovery::Resumed));
        }
    }

    let mut session = plan.session;
    session
        .arm()
        .expect("a fresh draft always arms; CLOSED is handled by the reopen use case");
    store.save_session(&session, plan.class_start).await?;

    let mut athletes = Vec::with_capacity(plan.roster.len());
    for (i, entry) in plan.roster.iter().enumerate() {
        store
            .save_athlete(&session.id, &entry.athlete_id, &entry.display_name, i as i64 + 1)
            .await?;
        athletes.push(AthleteState::ready(&entry.athlete_id, &entry.display_name));
    }

    let state = LiveSession::new(session, plan.config, plan.class_start).with_athletes(athletes);
    Ok((state, Recovery::Started))
}
