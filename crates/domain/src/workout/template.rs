//! Workout templates, blocks and the compile step (workout brief §4-§6; ADR 0008).
//!
//! Ordering is the position in the `Vec`, not a stored `position` column. A vector cannot
//! hold two items at position 3 and cannot skip position 2, so the invariant the brief asks
//! for is structural rather than something a later write could violate.

use super::exercise::{ExerciseLibrary, Target, TargetType, Weight};
use crate::course::{Course, CourseStep, StationTarget};
use crate::time::Duration;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TemplateCategory {
    Foundational,
    Engine,
    Power,
    Complete,
    RaceSimulation,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TemplateSource {
    /// Shipped with the hub. Read-only: editing one means duplicating it first (brief §4).
    System,
    Coach,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BlockType {
    Sequential,
    Rounds,
    Amrap,
    Interval,
    ZoneRotation,
}

impl BlockType {
    /// Whether a block of this type has a fixed, knowable list of steps. AMRAP and zone
    /// rotation do not: how many rounds an athlete gets is the *result*, not the plan, so
    /// there is no honest flat course to compile them into (CLAUDE.md 28).
    pub fn is_runnable(self) -> bool {
        matches!(self, BlockType::Sequential | BlockType::Rounds | BlockType::Interval)
    }
}

/// One prescribed movement inside a block.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WorkoutExercise {
    pub exercise_code: String,
    pub target: Target,
    pub weight: Option<Weight>,
    pub time_limit: Option<Duration>,
    pub notes: Option<String>,
}

impl WorkoutExercise {
    pub fn new(exercise_code: impl Into<String>, target: Target) -> Self {
        Self {
            exercise_code: exercise_code.into(),
            target,
            weight: None,
            time_limit: None,
            notes: None,
        }
    }

    pub fn with_weight(mut self, weight: Weight) -> Self {
        self.weight = Some(weight);
        self
    }

    pub fn with_time_limit(mut self, limit: Duration) -> Self {
        self.time_limit = Some(limit);
        self
    }

    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WorkoutBlock {
    pub name: String,
    pub block_type: BlockType,
    /// Required by ROUNDS and INTERVAL, meaningless for the others.
    pub rounds: Option<u32>,
    /// The AMRAP window, or an interval's work period.
    pub duration: Option<Duration>,
    pub rest: Option<Duration>,
    pub exercises: Vec<WorkoutExercise>,
}

impl WorkoutBlock {
    pub fn new(name: impl Into<String>, block_type: BlockType) -> Self {
        Self {
            name: name.into(),
            block_type,
            rounds: None,
            duration: None,
            rest: None,
            exercises: Vec::new(),
        }
    }

    pub fn sequential(name: impl Into<String>) -> Self {
        Self::new(name, BlockType::Sequential)
    }

    pub fn rounds(name: impl Into<String>, rounds: u32) -> Self {
        Self::new(name, BlockType::Rounds).with_rounds(rounds)
    }

    pub fn with_rounds(mut self, rounds: u32) -> Self {
        self.rounds = Some(rounds);
        self
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    pub fn with_rest(mut self, rest: Duration) -> Self {
        self.rest = Some(rest);
        self
    }

    pub fn with_exercises(mut self, exercises: Vec<WorkoutExercise>) -> Self {
        self.exercises = exercises;
        self
    }

    /// How many times this block's exercises are walked. One unless rounds say otherwise.
    fn repeats(&self) -> Result<u32, CompileError> {
        match self.block_type {
            BlockType::Sequential => Ok(1),
            BlockType::Rounds | BlockType::Interval => match self.rounds {
                // Zero rounds is the same mistake as no rounds: it prescribes no work.
                Some(n) if n >= 1 => Ok(n),
                _ => Err(CompileError::RoundsMissing { block: self.name.clone() }),
            },
            other => Err(CompileError::BlockTypeNotRunnable {
                block: self.name.clone(),
                block_type: other,
            }),
        }
    }

    /// Steps this block contributes to a class. Zero if it cannot be compiled.
    pub fn step_count(&self) -> usize {
        self.repeats().map(|n| n as usize * self.exercises.len()).unwrap_or(0)
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CompileError {
    /// A template with no work in it is not a class -- and a course-completion finish rule
    /// would read an empty course as instantly complete.
    Empty,
    UnknownExercise { code: String },
    RoundsMissing { block: String },
    BlockTypeNotRunnable { block: String, block_type: BlockType },
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WorkoutTemplate {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub category: TemplateCategory,
    pub source: TemplateSource,
    /// Which coach owns it. `None` for a system template, and for a template that belongs
    /// to the venue rather than to a person.
    pub owner_id: Option<String>,
    /// Bumped on every edit. A class records the version it was built from, so "which plan
    /// did Friday's class actually run?" stays answerable after the template moves on.
    pub version: u32,
    pub difficulty: Option<String>,
    pub estimated_duration_minutes: Option<u32>,
    pub enabled: bool,
    pub blocks: Vec<WorkoutBlock>,
}

impl WorkoutTemplate {
    pub fn new(id: impl Into<String>, name: impl Into<String>, category: TemplateCategory) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            category,
            source: TemplateSource::Coach,
            owner_id: None,
            version: 1,
            difficulty: None,
            estimated_duration_minutes: None,
            enabled: true,
            blocks: Vec::new(),
        }
    }

    pub fn system(
        id: impl Into<String>,
        name: impl Into<String>,
        category: TemplateCategory,
    ) -> Self {
        Self { source: TemplateSource::System, ..Self::new(id, name, category) }
    }

    pub fn with_block(mut self, block: WorkoutBlock) -> Self {
        self.blocks.push(block);
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_owner(mut self, owner_id: impl Into<String>) -> Self {
        self.owner_id = Some(owner_id.into());
        self
    }

    pub fn with_estimated_duration(mut self, minutes: u32) -> Self {
        self.estimated_duration_minutes = Some(minutes);
        self
    }

    pub fn with_difficulty(mut self, difficulty: impl Into<String>) -> Self {
        self.difficulty = Some(difficulty.into());
        self
    }

    /// System templates are read-only (brief §4). A coach who wants one changed duplicates
    /// it, which is what [`WorkoutTemplate::duplicate`] is for.
    pub fn is_editable(&self) -> bool {
        self.source == TemplateSource::Coach
    }

    /// A coach's own copy: same work, new identity, its own version history.
    pub fn duplicate(
        &self,
        new_id: impl Into<String>,
        new_name: impl Into<String>,
        owner_id: Option<&str>,
    ) -> Self {
        Self {
            id: new_id.into(),
            name: new_name.into(),
            source: TemplateSource::Coach,
            owner_id: owner_id.map(str::to_string),
            version: 1,
            blocks: self.blocks.clone(),
            description: self.description.clone(),
            category: self.category,
            difficulty: self.difficulty.clone(),
            estimated_duration_minutes: self.estimated_duration_minutes,
            enabled: true,
        }
    }

    /// Records that the work changed. Callers bump the version through this rather than by
    /// hand, so an edit that forgot to is a missing call rather than a silent stale number.
    pub fn edited(&mut self) {
        self.version += 1;
    }

    /// Steps a class built from this template will actually walk.
    pub fn step_count(&self) -> usize {
        self.blocks.iter().map(WorkoutBlock::step_count).sum()
    }

    /// Flatten the template into the course a class runs on (ADR 0008).
    ///
    /// This is the seam of the whole feature. Everything upstream -- blocks, rounds,
    /// exercise codes, units -- stops here; everything downstream (the snapshot, the live
    /// screen, the finish rule, event replay) sees the same flat `Course` it always has.
    ///
    /// Compiled steps carry `station_key`, not the exercise code, because that is the string
    /// readers are registered against and the live screen slugs into a pictogram.
    pub fn compile(&self, library: &ExerciseLibrary) -> Result<Course, CompileError> {
        let mut steps = Vec::with_capacity(self.step_count());
        for block in &self.blocks {
            let repeats = block.repeats()?;
            for _ in 0..repeats {
                for exercise in &block.exercises {
                    let known = library.get(&exercise.exercise_code).ok_or_else(|| {
                        CompileError::UnknownExercise { code: exercise.exercise_code.clone() }
                    })?;
                    steps.push(CourseStep {
                        station: known.station_key.clone(),
                        target: Some(station_target(exercise.target)),
                    });
                }
            }
        }
        if steps.is_empty() {
            return Err(CompileError::Empty);
        }
        Ok(Course::new(self.name.clone(), steps))
    }

    /// Every exercise code the template names that the library does not hold. Reported as a
    /// list rather than the first failure, so a builder screen can mark them all at once.
    pub fn unknown_exercises(&self, library: &ExerciseLibrary) -> Vec<String> {
        let mut missing: Vec<String> = Vec::new();
        for block in &self.blocks {
            for e in &block.exercises {
                if library.get(&e.exercise_code).is_none() && !missing.contains(&e.exercise_code) {
                    missing.push(e.exercise_code.clone());
                }
            }
        }
        missing
    }

    pub fn exercise(&self, block: usize, index: usize) -> Option<&WorkoutExercise> {
        self.blocks.get(block)?.exercises.get(index)
    }
}

/// Targets are canonical by the time they reach a course: metres, reps, seconds, calories
/// (brief §7). The display unit stays on the template, where a coach edits it.
fn station_target(target: Target) -> StationTarget {
    let value = target.canonical();
    match target.target_type {
        TargetType::Distance => StationTarget::Distance { meters: value },
        TargetType::Reps => StationTarget::Repetitions { count: value },
        TargetType::Time => StationTarget::Duration { duration: Duration(value as i64 * 1_000) },
        TargetType::Calories => StationTarget::Calories { count: value },
    }
}

/// The starter templates a hub ships with (workout brief §16).
///
/// Demo content, not official HYROX programming: the brief says so explicitly, and nothing
/// here should be read as a prescription. They exist so a coach opening the builder for the
/// first time has something to duplicate rather than an empty screen.
///
/// All four are SYSTEM, so none of them can be edited in place. Their ids are stable and
/// prefixed `sys-`, because seeding runs on every start and must not create a second copy.
impl WorkoutTemplate {
    pub fn presets() -> Vec<WorkoutTemplate> {
        use super::exercise::Unit::{Meter, Reps};
        vec![
            Self::system("sys-engine-800", "HYROX Engine 800", TemplateCategory::Engine)
                .with_description("Cardio / endurance. Runs between the two ergs.")
                .with_difficulty("INTERMEDIATE")
                .with_estimated_duration(50)
                .with_block(WorkoutBlock::sequential("Main").with_exercises(preset_exercises(&[
                    ("RUN", 800, Meter),
                    ("SKIERG", 1_000, Meter),
                    ("RUN", 800, Meter),
                    ("ROWERG", 1_000, Meter),
                    ("RUN", 800, Meter),
                    ("WALL_BALL", 50, Reps),
                ]))),
            Self::system("sys-engine-short", "HYROX Engine Short", TemplateCategory::Engine)
                .with_description("Three shorter rounds. A first class for a new member.")
                .with_difficulty("BEGINNER")
                .with_estimated_duration(40)
                .with_block(WorkoutBlock::rounds("Main", 3).with_exercises(preset_exercises(&[
                    ("RUN", 400, Meter),
                    ("SKIERG", 500, Meter),
                    ("ROWERG", 500, Meter),
                    ("WALL_BALL", 20, Reps),
                ]))),
            Self::system("sys-power", "HYROX Power", TemplateCategory::Power)
                .with_description("Strength endurance. The carries and the sleds.")
                .with_difficulty("INTERMEDIATE")
                .with_estimated_duration(50)
                .with_block(WorkoutBlock::rounds("Main", 3).with_exercises(preset_exercises(&[
                    ("SLED_PUSH", 25, Meter),
                    ("SLED_PULL", 25, Meter),
                    ("FARMERS_CARRY", 100, Meter),
                    ("SANDBAG_LUNGE", 50, Meter),
                    ("WALL_BALL", 25, Reps),
                ]))),
            Self::system("sys-complete-short", "HYROX Complete Short", TemplateCategory::Complete)
                .with_description("All eight stations, a run between each. Race shaped.")
                .with_difficulty("ADVANCED")
                .with_estimated_duration(60)
                .with_block(WorkoutBlock::sequential("Main").with_exercises(preset_exercises(&[
                    ("RUN", 800, Meter),
                    ("SKIERG", 1_000, Meter),
                    ("RUN", 800, Meter),
                    ("SLED_PUSH", 50, Meter),
                    ("RUN", 800, Meter),
                    ("SLED_PULL", 50, Meter),
                    ("RUN", 800, Meter),
                    ("BURPEE_BROAD_JUMP", 40, Meter),
                    ("RUN", 800, Meter),
                    ("ROWERG", 1_000, Meter),
                    ("RUN", 800, Meter),
                    ("FARMERS_CARRY", 100, Meter),
                    ("RUN", 800, Meter),
                    ("SANDBAG_LUNGE", 50, Meter),
                    ("RUN", 800, Meter),
                    ("WALL_BALL", 50, Reps),
                ]))),
        ]
    }
}

/// Builds the preset exercises against the shipped library.
///
/// `expect` rather than a `Result`: these are compile-time-known constants of this crate,
/// and a preset naming an exercise the library does not hold, or a unit it does not accept,
/// is a bug in this file -- caught by `presets.rs` on every test run, never by a coach.
fn preset_exercises(spec: &[(&str, u32, super::exercise::Unit)]) -> Vec<WorkoutExercise> {
    let library = ExerciseLibrary::preset();
    spec.iter()
        .map(|(code, value, unit)| {
            let exercise = library
                .get(code)
                .unwrap_or_else(|| panic!("preset names {code}, which is not in the library"));
            let target = Target::new(exercise, *value, *unit)
                .unwrap_or_else(|e| panic!("preset target for {code} is invalid: {e:?}"));
            WorkoutExercise::new(*code, target)
        })
        .collect()
}
