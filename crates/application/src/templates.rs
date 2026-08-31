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
use crate::ports::{AuditEntry, HubStore};
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
    let existing = store.template(&template.id).await.map_err(TemplateError::Storage)?;
    if let Some(previous) = &existing {
        if !previous.is_editable() {
            return Err(TemplateError::NotEditable);
        }
        template.version = previous.version;
        template.edited();
    }

    store.save_template(&template).await.map_err(TemplateError::Storage)?;
    audit(
        store,
        cmd,
        if existing.is_some() { "TEMPLATE_UPDATE" } else { "TEMPLATE_CREATE" },
        &template.id,
        existing.as_ref().map(describe),
        Some(describe(&template)),
    )
    .await?;
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

    let copy = source.duplicate(new_id, new_name, owner_id);
    store.save_template(&copy).await.map_err(TemplateError::Storage)?;
    audit(store, cmd, "TEMPLATE_DUPLICATE", &copy.id, Some(describe(&source)), Some(describe(&copy)))
        .await?;
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

    store.delete_template(template_id).await.map_err(TemplateError::Storage)?;
    audit(store, cmd, "TEMPLATE_DELETE", template_id, Some(describe(&existing)), None).await
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

    let course = template.compile(library).map_err(TemplateError::Compile)?;
    let session = Session::new_draft(&new.session_id, &new.name, new.mode);
    let config = SessionConfig::new(&new.session_id)
        .with_course(course)
        .with_finish_policy(new.finish_policy);

    store.save_session(&session, new.created_at).await.map_err(TemplateError::Storage)?;
    // After the session row, not before: the configuration belongs to a session that exists.
    store.save_session_config(&config).await.map_err(TemplateError::Storage)?;

    let mut athletes = Vec::with_capacity(new.roster.len());
    let mut bibs = Vec::with_capacity(new.roster.len());
    for (i, entry) in new.roster.iter().enumerate() {
        store
            .save_athlete(&session.id, &entry.athlete_id, &entry.display_name, i as i64 + 1, None)
            .await
            .map_err(TemplateError::Storage)?;
        athletes.push(AthleteState::ready(&entry.athlete_id, &entry.display_name));
        bibs.push((entry.athlete_id.clone(), i as i64 + 1));
    }

    audit(
        store,
        cmd,
        "CLASS_CREATE",
        &session.id,
        None,
        Some(format!(
            "template {:?} v{}, {} steps",
            template.name,
            template.version,
            template.step_count()
        )),
    )
    .await?;

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

async fn audit<S: HubStore>(
    store: &S,
    cmd: &OperatorCommand,
    action: &str,
    subject: &str,
    before: Option<String>,
    after: Option<String>,
) -> Result<(), TemplateError<S::Error>> {
    store
        .record_audit(&AuditEntry {
            at: cmd.at,
            operator: cmd.operator.clone(),
            action: action.to_string(),
            subject: subject.to_string(),
            reason: cmd.stated_reason().map(str::to_string),
            before,
            after,
        })
        .await
        .map_err(TemplateError::Storage)
}
