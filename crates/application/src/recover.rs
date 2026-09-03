//! Startup: resume the interrupted session, or begin a new one (CLAUDE.md 21).
//!
//! Nothing critical lives only in memory. An ARMED session found in the store is resumed by
//! replaying its interpreted events, which is why a restart cannot disagree with the log --
//! the log is the only source.
//!
//! Configuration is recovered the same way. A resumed session comes back with the course and
//! the finish policy it was **armed with**, never with a default and never with whatever the
//! caller happened to pass in: a class that started under a one-hour limit must not finish
//! under a different rule because the process restarted (ADR 0004).
//!
//! The same applies to the reader map and the binding ledger. Both are loaded from the store
//! rather than rebuilt by the caller, so a read resolves after a restart exactly as it did
//! before one, and a band still belongs to the athlete it was handed to.

use crate::checkin::pending_tags_since;
use crate::live_session::LiveSession;
use crate::ports::HubStore;
use domain::{AthleteState, Instant, Session, SessionConfig};

/// What to start if there is nothing to resume.
pub struct SessionPlan {
    /// A DRAFT session; `start_now` decides whether it is armed and started as well.
    pub session: Session,
    pub config: SessionConfig,
    pub roster: Vec<RosterEntry>,
    pub class_start: Instant,
    /// Whether to arm and start it immediately.
    ///
    /// False is what a hub boots with: an empty DRAFT class, nothing timing, waiting for a
    /// coach to build it. It used to be unconditionally true, which was invisible while the
    /// startup plan carried a fixture course and twelve invented athletes -- and wrong the
    /// moment it did not, because a RUNNING class cannot be configured (ADR 0001 D2), so a
    /// venue could not put a course on the very class its hub had started for it.
    pub start_now: bool,
}

pub struct RosterEntry {
    pub athlete_id: String,
    pub display_name: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Recovery {
    /// An ARMED session was found and rebuilt from its events and its stored configuration.
    Resumed,
    /// An ARMED session was found, but it has no stored configuration -- a session armed by
    /// a build older than ADR 0004. The plan's configuration was used, which means the
    /// course and the finish rule may not be the ones the class started under.
    ///
    /// Reported rather than hidden: the caller has to be able to say so, because this is
    /// precisely the silent substitution the stored configuration exists to prevent.
    ResumedWithoutStoredConfig,
    Started,
}

/// Resumes a live session if the store has one, otherwise starts and persists the plan.
///
/// Live means READY, RUNNING or PAUSED (ADR 0008). A COMPLETED or CANCELLED session is
/// finished business, and a DRAFT one is still being written. A class that was PAUSED when
/// the hub died comes back paused, with its accumulated pause intact -- resuming it as
/// RUNNING would hand back time the class never ran.
pub async fn resume_or_start<S: HubStore>(
    store: &S,
    plan: SessionPlan,
) -> Result<(LiveSession, Recovery), S::Error> {
    if let Some(existing) = store.active_session().await? {
        if existing.is_live() {
            // The stored creation time is the class clock's origin; falling back to the
            // plan's would restart the class clock at zero on every reboot.
            let class_start = store
                .session_created_at(&existing.id)
                .await?
                .unwrap_or(plan.class_start);
            let stored = store.session_config(&existing.id).await?;
            let recovery = match stored {
                Some(_) => Recovery::Resumed,
                None => Recovery::ResumedWithoutStoredConfig,
            };
            let config = stored.unwrap_or(plan.config);
            let athletes = store.rebuild_athletes(&existing.id).await?;
            let exceptions = store.exception_count(&existing.id).await?;
            // Bibs are stored, not replayed (they are not events), so they have to be read
            // back explicitly. Without this the door hands out a number somebody is already
            // wearing and the store refuses the write -- an entrant turned away at a race
            // they paid to enter, by a restart nobody saw (CLAUDE.md 21).
            let bibs = store.athlete_bibs(&existing.id).await?;
            let mut state = LiveSession::new(existing, config, class_start).with_athletes(athletes);
            for (athlete_id, bib) in bibs {
                state.note_bib(&athlete_id, bib);
            }
            state.exception_count = exceptions;
            load_venue(store, &mut state).await?;
            return Ok((state, recovery));
        }
    }

    let mut session = plan.session;
    if plan.start_now {
        session
            .mark_ready()
            .and_then(|()| session.start())
            .expect("a fresh draft always starts; terminal states go through the reopen use case");
    }
    store.save_session(&session, plan.class_start).await?;
    // After the session row, not before: the configuration belongs to a session that exists.
    store.save_session_config(&plan.config).await?;

    let mut athletes = Vec::with_capacity(plan.roster.len());
    let mut bibs = Vec::with_capacity(plan.roster.len());
    for (i, entry) in plan.roster.iter().enumerate() {
        store
            .save_athlete(
                &session.id,
                &entry.athlete_id,
                &entry.display_name,
                i as i64 + 1,
                None,
            )
            .await?;
        athletes.push(AthleteState::ready(&entry.athlete_id, &entry.display_name));
        bibs.push((entry.athlete_id.clone(), i as i64 + 1));
    }

    let mut state =
        LiveSession::new(session, plan.config, plan.class_start).with_athletes(athletes);
    for (athlete_id, bib) in bibs {
        state.note_bib(&athlete_id, bib);
    }
    // A new class in a venue that already has readers and bands should see them.
    load_venue(store, &mut state).await?;
    Ok((state, Recovery::Started))
}

/// Loads the reader map, the binding ledger and the check-in queue into a session.
///
/// The queue is derived, not stored: a tag that a reader has seen since the class started
/// and that still belongs to nobody is still waiting to be claimed (ADR 0001 D3). Deriving
/// it means a crash cannot lose it, and cannot resurrect a tag someone bound in the meantime.
async fn load_venue<S: HubStore>(store: &S, state: &mut LiveSession) -> Result<(), S::Error> {
    state.readers = store.readers().await?;
    state.bindings = store.bindings().await?;
    for tag in pending_tags_since(store, &state.bindings, state.class_start).await? {
        state.note_pending_tag(tag);
    }
    Ok(())
}
