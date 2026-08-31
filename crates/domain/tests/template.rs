//! Workout templates, blocks, and compiling a template into a runnable course
//! (workout brief §4-§6, §16; ADR 0008).

use domain::{
    BlockType, CompileError, Course, ExerciseLibrary, StationTarget, Target, TargetType,
    TemplateCategory, TemplateSource, Unit, Weight, WeightUnit, WorkoutBlock, WorkoutExercise,
    WorkoutTemplate,
};

fn lib() -> ExerciseLibrary {
    ExerciseLibrary::preset()
}

fn ex(code: &str, value: u32, unit: Unit) -> WorkoutExercise {
    let lib = lib();
    let exercise = lib.get(code).expect("a known exercise");
    WorkoutExercise::new(code, Target::new(exercise, value, unit).expect("a legal target"))
}

fn engine_800() -> WorkoutTemplate {
    WorkoutTemplate::new("t1", "HYROX Engine 800", TemplateCategory::Engine).with_block(
        WorkoutBlock::sequential("Main").with_exercises(vec![
            ex("RUN", 800, Unit::Meter),
            ex("SKIERG", 1_000, Unit::Meter),
            ex("RUN", 800, Unit::Meter),
            ex("ROWERG", 1_000, Unit::Meter),
            ex("RUN", 800, Unit::Meter),
            ex("WALL_BALL", 50, Unit::Reps),
        ]),
    )
}

// --- shape -------------------------------------------------------------------------------

#[test]
fn a_new_template_belongs_to_the_coach_who_wrote_it() {
    let t = WorkoutTemplate::new("t1", "Friday Engine", TemplateCategory::Custom);
    assert_eq!(t.source, TemplateSource::Coach);
    assert_eq!(t.version, 1);
    assert!(t.enabled);
}

#[test]
fn a_template_holds_ordered_blocks_of_ordered_exercises() {
    let t = engine_800();
    assert_eq!(t.blocks.len(), 1);
    assert_eq!(t.blocks[0].exercises.len(), 6);
    assert_eq!(t.blocks[0].exercises[1].exercise_code, "SKIERG");
}

#[test]
fn an_exercise_may_carry_a_load_a_time_limit_and_a_note() {
    let e = ex("WALL_BALL", 50, Unit::Reps)
        .with_weight(Weight::new(4, WeightUnit::Kilogram))
        .with_time_limit(domain::Duration(180_000))
        .with_notes("chest to wall");
    assert_eq!(e.weight.expect("a load").grams, 4_000);
    assert_eq!(e.time_limit.expect("a limit").millis(), 180_000);
    assert_eq!(e.notes.as_deref(), Some("chest to wall"));
}

// --- system vs coach templates (brief §4) --------------------------------------------------

#[test]
fn a_system_template_is_not_editable() {
    let t = WorkoutTemplate::system("sys1", "HYROX Engine", TemplateCategory::Engine);
    assert_eq!(t.source, TemplateSource::System);
    assert!(!t.is_editable());
}

#[test]
fn a_coach_template_is_editable() {
    assert!(engine_800().is_editable());
}

#[test]
fn duplicating_a_system_template_produces_an_editable_coach_template() {
    let system = WorkoutTemplate::system("sys1", "HYROX Engine 800", TemplateCategory::Engine)
        .with_block(WorkoutBlock::sequential("Main").with_exercises(vec![ex("RUN", 800, Unit::Meter)]));

    let copy = system.duplicate("t9", "Friday Engine Class", Some("coach-ana"));

    assert_eq!(copy.id, "t9");
    assert_eq!(copy.name, "Friday Engine Class");
    assert_eq!(copy.source, TemplateSource::Coach);
    assert_eq!(copy.owner_id.as_deref(), Some("coach-ana"));
    assert_eq!(copy.version, 1, "a copy starts its own history");
    assert!(copy.is_editable());
    assert_eq!(copy.blocks, system.blocks, "the work itself is carried over verbatim");
}

/// Editing a template is what makes a later class differ from an earlier one, so the
/// version has to move. Sessions record the version they were built from (ADR 0008).
#[test]
fn editing_a_template_advances_its_version() {
    let mut t = engine_800();
    assert_eq!(t.version, 1);
    t.edited();
    assert_eq!(t.version, 2);
}

// --- compiling to a course (ADR 0008) ------------------------------------------------------

#[test]
fn a_sequential_block_compiles_to_its_exercises_in_order() {
    let course: Course = engine_800().compile(&lib()).expect("a compilable template");

    let stations: Vec<&str> = course.stations().collect();
    assert_eq!(
        stations,
        ["RUN", "SKIERG", "RUN", "ROWING", "RUN", "WALL BALLS"],
        "compiled steps carry station keys, which is what readers are registered against"
    );
    assert_eq!(course.name, "HYROX Engine 800");
}

#[test]
fn a_compiled_step_carries_the_target_the_live_screen_shows() {
    let course = engine_800().compile(&lib()).unwrap();
    assert_eq!(course.step(0).unwrap().target, Some(StationTarget::Distance { meters: 800 }));
    assert_eq!(course.step(5).unwrap().target, Some(StationTarget::Repetitions { count: 50 }));
}

#[test]
fn a_time_target_compiles_to_a_duration_in_milliseconds() {
    let t = WorkoutTemplate::new("t1", "Intervals", TemplateCategory::Engine).with_block(
        WorkoutBlock::sequential("Main").with_exercises(vec![ex("RUN", 3, Unit::Minute)]),
    );
    let course = t.compile(&lib()).unwrap();
    assert_eq!(
        course.step(0).unwrap().target,
        Some(StationTarget::Duration { duration: domain::Duration(180_000) })
    );
}

#[test]
fn a_calorie_target_compiles_to_a_calorie_step() {
    let t = WorkoutTemplate::new("t1", "Cals", TemplateCategory::Engine).with_block(
        WorkoutBlock::sequential("Main").with_exercises(vec![ex("ROWERG", 40, Unit::Calorie)]),
    );
    let course = t.compile(&lib()).unwrap();
    assert_eq!(course.step(0).unwrap().target, Some(StationTarget::Calories { count: 40 }));
}

/// The whole reason blocks exist. Three rounds of four exercises is twelve steps on the
/// floor, and the timing engine only ever sees the twelve (ADR 0008).
#[test]
fn a_rounds_block_is_expanded_into_repeated_steps() {
    let t = WorkoutTemplate::new("t2", "HYROX Engine Short", TemplateCategory::Engine).with_block(
        WorkoutBlock::rounds("Main", 3).with_exercises(vec![
            ex("RUN", 400, Unit::Meter),
            ex("SKIERG", 500, Unit::Meter),
        ]),
    );

    let course = t.compile(&lib()).unwrap();

    assert_eq!(course.len(), 6);
    assert_eq!(
        course.stations().collect::<Vec<_>>(),
        ["RUN", "SKIERG", "RUN", "SKIERG", "RUN", "SKIERG"]
    );
    assert_eq!(course.occurrences("RUN"), 3);
}

#[test]
fn blocks_compile_one_after_another() {
    let t = WorkoutTemplate::new("t3", "Two Parts", TemplateCategory::Custom)
        .with_block(WorkoutBlock::sequential("Warm-up").with_exercises(vec![ex("RUN", 400, Unit::Meter)]))
        .with_block(WorkoutBlock::rounds("Main", 2).with_exercises(vec![ex("WALL_BALL", 20, Unit::Reps)]));

    let course = t.compile(&lib()).unwrap();

    assert_eq!(
        course.stations().collect::<Vec<_>>(),
        ["RUN", "WALL BALLS", "WALL BALLS"]
    );
}

#[test]
fn an_interval_block_with_rounds_expands_like_rounds() {
    let t = WorkoutTemplate::new("t4", "Intervals", TemplateCategory::Engine).with_block(
        WorkoutBlock::new("Main", BlockType::Interval)
            .with_rounds(4)
            .with_exercises(vec![ex("RUN", 200, Unit::Meter)]),
    );
    assert_eq!(t.compile(&lib()).unwrap().len(), 4);
}

/// An AMRAP has no fixed number of steps, so there is no honest flat course to compile it
/// into. Refused by name rather than guessed at (CLAUDE.md 28).
#[test]
fn an_amrap_block_cannot_be_compiled_yet() {
    let t = WorkoutTemplate::new("t5", "AMRAP 12", TemplateCategory::Custom).with_block(
        WorkoutBlock::new("Main", BlockType::Amrap)
            .with_duration(domain::Duration(720_000))
            .with_exercises(vec![ex("WALL_BALL", 10, Unit::Reps)]),
    );

    assert_eq!(
        t.compile(&lib()),
        Err(CompileError::BlockTypeNotRunnable { block: "Main".into(), block_type: BlockType::Amrap })
    );
}

#[test]
fn a_rounds_block_without_a_round_count_is_refused() {
    let t = WorkoutTemplate::new("t6", "Broken", TemplateCategory::Custom).with_block(
        WorkoutBlock::new("Main", BlockType::Rounds)
            .with_exercises(vec![ex("RUN", 400, Unit::Meter)]),
    );

    assert_eq!(
        t.compile(&lib()),
        Err(CompileError::RoundsMissing { block: "Main".into() })
    );
}

#[test]
fn zero_rounds_is_refused() {
    let t = WorkoutTemplate::new("t7", "Broken", TemplateCategory::Custom).with_block(
        WorkoutBlock::rounds("Main", 0).with_exercises(vec![ex("RUN", 400, Unit::Meter)]),
    );
    assert_eq!(t.compile(&lib()), Err(CompileError::RoundsMissing { block: "Main".into() }));
}

#[test]
fn an_exercise_the_library_does_not_know_is_refused_by_name() {
    let t = WorkoutTemplate::new("t8", "Typo", TemplateCategory::Custom).with_block(
        WorkoutBlock::sequential("Main").with_exercises(vec![WorkoutExercise::new(
            "ROW_ERG",
            Target { target_type: TargetType::Distance, value: 500, unit: Unit::Meter },
        )]),
    );

    assert_eq!(
        t.compile(&lib()),
        Err(CompileError::UnknownExercise { code: "ROW_ERG".into() })
    );
}

/// A template with no work in it is not a class. Compiling it would produce a course the
/// finish rule reads as instantly complete.
#[test]
fn an_empty_template_is_refused() {
    let t = WorkoutTemplate::new("t9", "Nothing", TemplateCategory::Custom);
    assert_eq!(t.compile(&lib()), Err(CompileError::Empty));

    let empty_block = WorkoutTemplate::new("t10", "Nothing", TemplateCategory::Custom)
        .with_block(WorkoutBlock::sequential("Main"));
    assert_eq!(empty_block.compile(&lib()), Err(CompileError::Empty));
}

// --- estimated work ------------------------------------------------------------------------

#[test]
fn a_template_reports_how_many_steps_a_class_will_actually_walk() {
    let t = WorkoutTemplate::new("t11", "Short", TemplateCategory::Engine).with_block(
        WorkoutBlock::rounds("Main", 3)
            .with_exercises(vec![ex("RUN", 400, Unit::Meter), ex("SKIERG", 500, Unit::Meter)]),
    );
    assert_eq!(t.step_count(), 6);
}
