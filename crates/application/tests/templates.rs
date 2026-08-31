//! Template library use cases, and building a class from one (workout brief §4, §8, §15,
//! §20, §27; ADR 0008).

mod support;

use application::{
    configure, create_class, delete_template, duplicate_template, list_templates, save_template,
    LiveSession, NewClass, OperatorCommand, TemplateError,
};
use domain::{
    ExerciseLibrary, FinishPolicy, Instant, Session, SessionConfig, SessionMode, StationTarget,
    Target, TemplateCategory, TemplateSource, Unit, WorkoutBlock, WorkoutExercise,
    WorkoutTemplate,
};
use support::FakeStore;

const NOW: Instant = Instant(2_000_000);

fn lib() -> ExerciseLibrary {
    ExerciseLibrary::preset()
}

fn tablet() -> OperatorCommand {
    OperatorCommand::new("COACH TABLET", NOW)
}

fn ex(code: &str, value: u32, unit: Unit) -> WorkoutExercise {
    let lib = lib();
    WorkoutExercise::new(code, Target::new(lib.get(code).unwrap(), value, unit).unwrap())
}

fn engine_800(id: &str) -> WorkoutTemplate {
    WorkoutTemplate::system(id, "HYROX Engine 800", TemplateCategory::Engine).with_block(
        WorkoutBlock::sequential("Main").with_exercises(vec![
            ex("RUN", 800, Unit::Meter),
            ex("SKIERG", 1_000, Unit::Meter),
            ex("WALL_BALL", 50, Unit::Reps),
        ]),
    )
}

fn draft_session() -> LiveSession {
    LiveSession::new(
        Session::new_draft("s0", "placeholder", SessionMode::Training),
        SessionConfig::new("s0"),
        Instant(0),
    )
}

// --- the library (brief §4, §13) -----------------------------------------------------------

#[tokio::test]
async fn a_coach_template_is_saved_and_audited() {
    let store = FakeStore::new();
    let t = WorkoutTemplate::new("t1", "Friday Engine", TemplateCategory::Custom)
        .with_block(WorkoutBlock::sequential("Main").with_exercises(vec![ex("RUN", 600, Unit::Meter)]));

    let saved = save_template(&store, t, &tablet()).await.expect("saved");

    assert_eq!(saved.version, 1, "a brand new template starts at version 1");
    assert_eq!(store.templates_held().len(), 1);
    let audit = store.audits().pop().expect("an audit record");
    assert_eq!(audit.action, "TEMPLATE_CREATE");
    assert_eq!(audit.operator, "COACH TABLET");
}

#[tokio::test]
async fn saving_an_existing_template_advances_its_version() {
    let store = FakeStore::new();
    let t = WorkoutTemplate::new("t1", "Friday Engine", TemplateCategory::Custom)
        .with_block(WorkoutBlock::sequential("Main").with_exercises(vec![ex("RUN", 800, Unit::Meter)]));
    save_template(&store, t.clone(), &tablet()).await.unwrap();

    let edited = WorkoutTemplate::new("t1", "Friday Engine", TemplateCategory::Custom)
        .with_block(WorkoutBlock::sequential("Main").with_exercises(vec![ex("RUN", 600, Unit::Meter)]));
    let saved = save_template(&store, edited, &tablet()).await.expect("saved");

    assert_eq!(saved.version, 2);
    assert_eq!(store.audits().pop().unwrap().action, "TEMPLATE_UPDATE");
}

#[tokio::test]
async fn a_system_template_cannot_be_saved_over() {
    let store = FakeStore::new();
    let err = save_template(&store, engine_800("sys1"), &tablet())
        .await
        .expect_err("system templates are read-only");
    assert!(matches!(err, TemplateError::NotEditable));
    assert!(store.templates_held().is_empty());
}

/// The dangerous case: a coach template that shares an id with a stored system one. The
/// stored row decides, not the payload -- otherwise the read-only rule is bypassed by
/// sending `"source": "COACH"`.
#[tokio::test]
async fn a_coach_payload_cannot_overwrite_a_stored_system_template() {
    let store = FakeStore::new();
    store.seed_template(engine_800("sys1"));

    let impostor = WorkoutTemplate::new("sys1", "Hijacked", TemplateCategory::Custom)
        .with_block(WorkoutBlock::sequential("Main").with_exercises(vec![ex("RUN", 1, Unit::Meter)]));
    let err = save_template(&store, impostor, &tablet()).await.expect_err("refused");

    assert!(matches!(err, TemplateError::NotEditable));
    assert_eq!(store.templates_held()[0].name, "HYROX Engine 800");
}

#[tokio::test]
async fn a_system_template_cannot_be_deleted() {
    let store = FakeStore::new();
    store.seed_template(engine_800("sys1"));

    let err = delete_template(&store, "sys1", &tablet().with_reason("不要了")).await.expect_err("refused");

    assert!(matches!(err, TemplateError::NotEditable));
    assert_eq!(store.templates_held().len(), 1);
}

#[tokio::test]
async fn a_coach_template_is_deleted_and_audited() {
    let store = FakeStore::new();
    let t = WorkoutTemplate::new("t1", "Friday Engine", TemplateCategory::Custom)
        .with_block(WorkoutBlock::sequential("Main").with_exercises(vec![ex("RUN", 600, Unit::Meter)]));
    save_template(&store, t, &tablet()).await.unwrap();

    delete_template(&store, "t1", &tablet().with_reason("重複")).await.expect("deleted");

    assert!(store.templates_held().is_empty());
    assert_eq!(store.audits().pop().unwrap().action, "TEMPLATE_DELETE");
}

#[tokio::test]
async fn deleting_a_template_that_does_not_exist_says_so() {
    let store = FakeStore::new();
    let err = delete_template(&store, "nope", &tablet().with_reason("清理")).await.expect_err("unknown");
    assert!(matches!(err, TemplateError::UnknownTemplate(id) if id == "nope"));
}

#[tokio::test]
async fn templates_list_with_the_system_ones_first() {
    let store = FakeStore::new();
    store.seed_template(engine_800("sys1"));
    store.seed_template(WorkoutTemplate::new("t2", "Alpha", TemplateCategory::Custom));
    store.seed_template(WorkoutTemplate::system("sys2", "Aardvark", TemplateCategory::Power));

    let names: Vec<String> = list_templates(&store).await.unwrap().into_iter().map(|t| t.name).collect();

    assert_eq!(names, ["Aardvark", "HYROX Engine 800", "Alpha"]);
}

// --- scenario A (brief §27) -----------------------------------------------------------------

#[tokio::test]
async fn scenario_a_duplicate_a_system_template_then_edit_the_copy() {
    let store = FakeStore::new();
    store.seed_template(engine_800("sys1"));

    let copy = duplicate_template(&store, "sys1", "t9", "Friday Engine Class", Some("coach-ana"), &tablet())
        .await
        .expect("duplicated");

    assert_eq!(copy.source, TemplateSource::Coach);
    assert!(copy.is_editable());

    // Run 800 -> 600, and a farmers carry added.
    let mut edited = copy.clone();
    edited.blocks[0].exercises[0] = ex("RUN", 600, Unit::Meter);
    edited.blocks[0].exercises.push(ex("FARMERS_CARRY", 100, Unit::Meter));
    let edited = save_template(&store, edited, &tablet()).await.expect("saved");

    assert_eq!(edited.version, 2);
    assert_eq!(edited.blocks[0].exercises.len(), 4);

    // The system template it came from is untouched.
    let system = store.templates_held().into_iter().find(|t| t.id == "sys1").unwrap();
    assert_eq!(system.blocks[0].exercises[0].target.value, 800);
    assert_eq!(system.blocks[0].exercises.len(), 3);
}

// --- scenario B (brief §27) -----------------------------------------------------------------

fn new_class() -> NewClass {
    NewClass {
        session_id: "class-1".into(),
        name: "FRI 19:00 Engine".into(),
        mode: SessionMode::Training,
        coach_id: Some("coach-ana".into()),
        scheduled_at: None,
        finish_policy: FinishPolicy::CoachDecides,
        roster: vec![],
        created_at: Instant(500_000),
    }
}

#[tokio::test]
async fn a_class_is_created_from_a_compiled_template_and_left_in_draft() {
    let store = FakeStore::new();
    let mut state = draft_session();

    create_class(&mut state, &store, &engine_800("sys1"), &lib(), new_class(), &tablet())
        .await
        .expect("created");

    assert_eq!(state.session.id, "class-1");
    assert_eq!(state.session.status, domain::SessionStatus::Draft);
    assert!(state.session.accepts_config_edits(), "today's tweaks come next");
    let course = state.config.course.as_ref().expect("a compiled course");
    assert_eq!(course.stations().collect::<Vec<_>>(), ["RUN", "SKIERG", "WALL BALLS"]);
    assert_eq!(store.audits().pop().unwrap().action, "CLASS_CREATE");
}

/// The heart of the brief (§8): today's change lands on this class only.
#[tokio::test]
async fn scenario_b_a_session_tweak_does_not_touch_the_template() {
    let store = FakeStore::new();
    store.seed_template(engine_800("sys1"));
    let mut state = draft_session();
    let template = store.templates_held().into_iter().next().unwrap();

    create_class(&mut state, &store, &template, &lib(), new_class(), &tablet())
        .await
        .expect("created");

    // Wall Ball 50 -> 40, for tonight only.
    let mut course = state.config.course.clone().unwrap();
    course.steps[2].target = Some(StationTarget::Repetitions { count: 40 });
    configure(&mut state, &store, Some(course), FinishPolicy::CoachDecides, &tablet())
        .await
        .expect("reconfigured");

    assert_eq!(
        state.config.course.as_ref().unwrap().step(2).unwrap().target,
        Some(StationTarget::Repetitions { count: 40 })
    );
    // The template still says 50.
    let stored = store.templates_held().into_iter().find(|t| t.id == "sys1").unwrap();
    assert_eq!(stored.blocks[0].exercises[2].target.value, 50);
}

/// The other half of §8: editing the template afterwards must not reach back into a class
/// that already ran off its own snapshot (ADR 0004).
#[tokio::test]
async fn editing_the_template_afterwards_does_not_change_a_class_already_created() {
    let store = FakeStore::new();
    let source = engine_800("sys1");
    store.seed_template(source.clone());
    let mut state = draft_session();
    create_class(&mut state, &store, &source, &lib(), new_class(), &tablet()).await.unwrap();

    let mut copy = source.duplicate("t9", "Copy", None);
    copy.blocks[0].exercises[0] = ex("RUN", 400, Unit::Meter);
    save_template(&store, copy, &tablet()).await.unwrap();

    assert_eq!(
        state.config.course.as_ref().unwrap().step(0).unwrap().target,
        Some(StationTarget::Distance { meters: 800 }),
        "the class runs off its snapshot, not off any template"
    );
}

#[tokio::test]
async fn a_class_cannot_be_created_while_one_is_running() {
    let store = FakeStore::new();
    let mut state = draft_session();
    state.session.mark_ready().unwrap();
    state.session.start().unwrap();

    let err = create_class(&mut state, &store, &engine_800("sys1"), &lib(), new_class(), &tablet())
        .await
        .expect_err("a class is on the floor");

    assert!(matches!(err, TemplateError::ClassInProgress));
    assert_eq!(state.session.id, "s0", "the running class is left alone");
}

#[tokio::test]
async fn a_template_that_cannot_be_compiled_does_not_create_a_class() {
    let store = FakeStore::new();
    let mut state = draft_session();
    let empty = WorkoutTemplate::new("t1", "Nothing", TemplateCategory::Custom);

    let err = create_class(&mut state, &store, &empty, &lib(), new_class(), &tablet())
        .await
        .expect_err("nothing to run");

    assert!(matches!(err, TemplateError::Compile(domain::CompileError::Empty)));
    assert!(store.saved_sessions().is_empty(), "a refused class writes no session row");
}

#[tokio::test]
async fn a_created_class_keeps_the_venue_readers_and_bands() {
    let store = FakeStore::new();
    let mut state = draft_session();
    let mut readers = domain::ReaderRegistry::new();
    readers.register(domain::ReaderRegistration::new(
        domain::ReaderKey::parse("esp32-a4cf128b3d91", "rfid-01").unwrap(),
        "SKIERG",
        domain::ReaderMode::Toggle,
    ));
    state = state.with_readers(readers);

    create_class(&mut state, &store, &engine_800("sys1"), &lib(), new_class(), &tablet())
        .await
        .expect("created");

    assert_eq!(state.readers.len(), 1, "the readers on the wall outlive the class");
}

/// Deleting a template destroys a plan a coach wrote, so CLAUDE.md 20 wants a reason.
#[tokio::test]
async fn deleting_a_template_without_a_reason_is_refused() {
    let store = FakeStore::new();
    let t = WorkoutTemplate::new("t1", "Friday Engine", TemplateCategory::Custom)
        .with_block(WorkoutBlock::sequential("Main").with_exercises(vec![ex("RUN", 600, Unit::Meter)]));
    save_template(&store, t, &tablet()).await.unwrap();

    let err = delete_template(&store, "t1", &tablet()).await.expect_err("reason required");

    assert!(matches!(err, TemplateError::ReasonRequired));
    assert_eq!(store.templates_held().len(), 1, "a refused delete removes nothing");
}
