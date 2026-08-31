//! Workout templates: the reusable course a coach writes, and the exercises it is made of
//! (workout brief §3-§7; ADR 0008).
//!
//! A template is the *editable* artefact. It is never what a class runs off: instantiating
//! a class compiles the template into a flat [`crate::Course`], and that compiled course is
//! what gets snapshotted onto the session (ADR 0004). So a later edit to a template cannot
//! reach back into a class that already happened, and rounds, blocks and AMRAP never leak
//! into the timing path.

pub mod exercise;
pub mod template;

pub use exercise::{
    Exercise, ExerciseCategory, ExerciseLibrary, Target, TargetError, TargetType, Unit, Weight,
    WeightUnit,
};
pub use template::{
    BlockType, CompileError, TemplateCategory, TemplateSource, WorkoutBlock, WorkoutExercise,
    WorkoutTemplate,
};
