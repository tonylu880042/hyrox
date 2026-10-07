//! The workout template library, and building a class from one (workout brief §4, §8, §15;
//! ADR 0008).
//!
//! Two rules carry this module:
//!
//! * **A system template is read-only.** A coach who wants one changed duplicates it first
//!   (brief §4). Enforced here, once, rather than in every screen that might edit one.
//! * **A class never reads a template.** Creating a class *compiles* the template into a
//!   flat course and snapshots it onto the session (ADR 0004, 0008). Editing the template
//!   afterwards cannot reach a class that already ran, because nothing is looking.

use crate::live_session::LiveSession;
use crate::operator::OperatorCommand;
use crate::ports::{AuditEntry, AuditedResult, AuditedWrite, ClassAthlete, HubStore};
use crate::recover::RosterEntry;
use domain::{
    AthleteState, CompileError, ExerciseLibrary, FinishPolicy, Instant, Session, SessionConfig,
    SessionMode, WorkoutTemplate,
};

#[derive(Debug, thiserror::Error)]
pub enum TemplateError<E> {
    #[error("a system template cannot be edited or deleted; duplicate it first")]
    NotEditable,
    #[error("no template with id {0}")]
    UnknownTemplate(String),
    #[error("the template cannot be run as written: {0:?}")]
    Compile(CompileError),
    /// Replacing the hub's active class while it is timing somebody would orphan the class
    /// on the floor. Complete or cancel it first.
    #[error("a class is already in progress")]
    ClassInProgress,
    #[error("a session with id {0:?} already exists")]
    SessionIdConflict(String),
    #[error("a template with id {0:?} already exists")]
    TemplateIdConflict(String),
    /// Deleting a template destroys a plan a coach wrote. CLAUDE.md 20 wants a reason on
    /// the record for that, and whitespace is not a reason.
    #[error("this action changes recorded data, so it needs a reason")]
    ReasonRequired,
    #[error("storage: {0}")]
    Storage(E),
}

/// Every template the venue holds, system ones first, then the coaches' own by name.
///
/// Sorted here rather than in SQL so the fake and the real store cannot disagree about an
/// order the screens depend on.
pub async fn list_templates<S: HubStore>(store: &S) -> Result<Vec<WorkoutTemplate>, S::Error> {
    let mut templates = store.templates().await?;
    templates.sort_by(|a, b| {
        use domain::TemplateSource::System;
        (b.source == System, a.name.as_str()).cmp(&(a.source == System, b.name.as_str()))
    });
    Ok(templates)
}

/// Saves a coach's template, new or edited.
///
/// The version is bumped here, on the way through, so a saved edit is always distinguishable
/// from the version a running class was built from -- a caller cannot forget.
pub async fn save_template<S: HubStore>(
    store: &S,
    mut template: WorkoutTemplate,
    cmd: &OperatorCommand,
) -> Result<WorkoutTemplate, TemplateError<S::Error>> {
    if !template.is_editable() {
        return Err(TemplateError::NotEditable);
    }
    let existing = store
        .template(&template.id)
        .await
        .map_err(TemplateError::Storage)?;
    if let Some(previous) = &existing {
        if !previous.is_editable() {
            return Err(TemplateError::NotEditable);
        }
        template.version = previous.version;
        template.edited();
    }

    let action = if existing.is_some() {
        "TEMPLATE_UPDATE"
    } else {
        "TEMPLATE_CREATE"
    };
    let result = store
        .apply_audited(AuditedWrite::TemplateSave {
            template: template.clone(),
            audit: AuditEntry {
                at: cmd.at,
                operator: cmd.operator.clone(),
                action: action.to_string(),
                subject: template.id.clone(),
                reason: cmd.stated_reason().map(str::to_string),
                before: existing.as_ref().map(describe),
                after: Some(describe(&template)),
            },
        })
        .await
        .map_err(TemplateError::Storage)?;
    if matches!(result, AuditedResult::Conflict) {
        return Err(TemplateError::TemplateIdConflict(template.id));
    }
    Ok(template)
}

/// A coach's own copy of any template, system ones included (brief §4).
pub async fn duplicate_template<S: HubStore>(
    store: &S,
    source_id: &str,
    new_id: &str,
    new_name: &str,
    owner_id: Option<&str>,
    cmd: &OperatorCommand,
) -> Result<WorkoutTemplate, TemplateError<S::Error>> {
    let source = store
        .template(source_id)
        .await
        .map_err(TemplateError::Storage)?
        .ok_or_else(|| TemplateError::UnknownTemplate(source_id.to_string()))?;

    if store
        .template(new_id)
        .await
        .map_err(TemplateError::Storage)?
        .is_some()
    {
        return Err(TemplateError::TemplateIdConflict(new_id.to_string()));
    }

    let copy = source.duplicate(new_id, new_name, owner_id);
    let result = store
        .apply_audited(AuditedWrite::TemplateCreate {
            template: copy.clone(),
            audit: AuditEntry {
                at: cmd.at,
                operator: cmd.operator.clone(),
                action: "TEMPLATE_DUPLICATE".to_string(),
                subject: copy.id.clone(),
                reason: cmd.stated_reason().map(str::to_string),
                before: Some(describe(&source)),
                after: Some(describe(&copy)),
            },
        })
        .await
        .map_err(TemplateError::Storage)?;
    if matches!(result, AuditedResult::Conflict) {
        return Err(TemplateError::TemplateIdConflict(copy.id));
    }
    Ok(copy)
}

/// Deletes a coach's template. System templates are refused (brief §13).
pub async fn delete_template<S: HubStore>(
    store: &S,
    template_id: &str,
    cmd: &OperatorCommand,
) -> Result<(), TemplateError<S::Error>> {
    if cmd.stated_reason().is_none() {
        return Err(TemplateError::ReasonRequired);
    }
    let existing = store
        .template(template_id)
        .await
        .map_err(TemplateError::Storage)?
        .ok_or_else(|| TemplateError::UnknownTemplate(template_id.to_string()))?;
    if !existing.is_editable() {
        return Err(TemplateError::NotEditable);
    }

    let result = store
        .apply_audited(AuditedWrite::TemplateDelete {
            template_id: template_id.to_string(),
            audit: AuditEntry {
                at: cmd.at,
                operator: cmd.operator.clone(),
                action: "TEMPLATE_DELETE".to_string(),
                subject: template_id.to_string(),
                reason: cmd.stated_reason().map(str::to_string),
                before: Some(describe(&existing)),
                after: None,
            },
        })
        .await
        .map_err(TemplateError::Storage)?;
    if matches!(result, AuditedResult::Applied(false)) {
        return Err(TemplateError::UnknownTemplate(template_id.to_string()));
    }
    Ok(())
}

/// What a class is created with (brief §15).
pub struct NewClass {
    pub session_id: String,
    pub name: String,
    pub mode: SessionMode,
    pub coach_id: Option<String>,
    pub scheduled_at: Option<Instant>,
    pub finish_policy: FinishPolicy,
    pub roster: Vec<RosterEntry>,
    /// The class clock's origin, and the session row's `created_at`.
    pub created_at: Instant,
}

/// Template -> compiled course -> snapshot -> DRAFT class (brief §8; ADR 0008).
///
/// The class is left in DRAFT, not started: today's tweaks -- Wall Ball 50 -> 40 -- happen
/// next, through the ordinary `configure` use case, and they land on this session's own
/// snapshot. The template is not touched by any of it.
///
/// Refused while a class is live. Swapping the hub's active session out from under a class
/// that is timing people would abandon it mid-floor with no way back.
pub async fn create_class<S: HubStore>(
    state: &mut LiveSession,
    store: &S,
    template: &WorkoutTemplate,
    library: &ExerciseLibrary,
    new: NewClass,
    cmd: &OperatorCommand,
) -> Result<(), TemplateError<S::Error>> {
    if state.session.is_live() && state.session.status != domain::SessionStatus::Ready {
        return Err(TemplateError::ClassInProgress);
    }

    // A replacement is a handoff between two projections. Finish any raw/context write that
    // was in flight before choosing which session becomes current, otherwise the old session
    // can leave a pending claim behind while the new one is already active.
    crate::ingest::reconcile_pending(state, store)
        .await
        .map_err(TemplateError::Storage)?;

    // This check gives the normal API path a stable 409. The transaction below repeats the
    // protection with an INSERT-only statement, so two stores racing this read cannot
    // overwrite an existing class.
    if store
        .session(&new.session_id)
        .await
        .map_err(TemplateError::Storage)?
        .is_some()
    {
        return Err(TemplateError::SessionIdConflict(new.session_id));
    }

    let course = template.compile(library).map_err(TemplateError::Compile)?;
    let session = Session::new_draft(&new.session_id, &new.name, new.mode);
    let config = SessionConfig::new(&new.session_id)
        .with_course(course)
        .with_finish_policy(new.finish_policy);

    let mut athletes = Vec::with_capacity(new.roster.len());
    let mut bibs = Vec::with_capacity(new.roster.len());
    for (i, entry) in new.roster.iter().enumerate() {
        let bib = i as i64 + 1;
        athletes.push(AthleteState::ready(&entry.athlete_id, &entry.display_name));
        bibs.push((entry.athlete_id.clone(), bib));
    }

    let roster = new
        .roster
        .iter()
        .enumerate()
        .map(|(i, entry)| ClassAthlete {
            athlete_id: entry.athlete_id.clone(),
            display_name: entry.display_name.clone(),
            bib: i as i64 + 1,
            member_id: None,
        })
        .collect();
    let result = store
        .apply_audited(AuditedWrite::CreateClass {
            session: session.clone(),
            created_at: new.created_at,
            config: config.clone(),
            athletes: roster,
            audit: AuditEntry {
                at: cmd.at,
                operator: cmd.operator.clone(),
                action: "CLASS_CREATE".to_string(),
                subject: session.id.clone(),
                reason: cmd.stated_reason().map(str::to_string),
                before: None,
                after: Some(format!(
                    "template {:?} v{}, {} steps",
                    template.name,
                    template.version,
                    template.step_count()
                )),
            },
        })
        .await
        .map_err(TemplateError::Storage)?;
    if matches!(result, AuditedResult::Conflict) {
        return Err(TemplateError::SessionIdConflict(session.id));
    }

    let readers = std::mem::take(&mut state.readers);
    let bindings = std::mem::take(&mut state.bindings);
    *state = LiveSession::new(session, config, new.created_at)
        .with_athletes(athletes)
        .with_readers(readers)
        .with_bindings(bindings);
    for (athlete_id, bib) in bibs {
        state.note_bib(&athlete_id, bib);
    }
    Ok(())
}

/// A one-line summary for the audit trail. The whole template would be a JSON blob in a
/// column nobody reads; what a coach needs later is which plan, how big, whose (CLAUDE.md 20).
fn describe(template: &WorkoutTemplate) -> String {
    format!(
        "{:?} {:?} v{} ({} blocks, {} steps)",
        template.source,
        template.name,
        template.version,
        template.blocks.len(),
        template.step_count()
    )
}
