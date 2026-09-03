//! The system preset templates (workout brief §16).

use domain::{BlockType, ExerciseLibrary, TemplateSource, WorkoutTemplate};

fn presets() -> Vec<WorkoutTemplate> {
    WorkoutTemplate::presets()
}

#[test]
fn there_are_four_presets_and_all_of_them_are_system_templates() {
    let all = presets();
    assert_eq!(all.len(), 4);
    for t in &all {
        assert_eq!(
            t.source,
            TemplateSource::System,
            "{} must be read-only",
            t.name
        );
        assert!(!t.is_editable());
        assert!(
            t.blocks.iter().all(|b| !b.exercises.is_empty()),
            "{} has an empty block",
            t.name
        );
    }
}

#[test]
fn every_preset_id_is_distinct() {
    let all = presets();
    let mut ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
    ids.sort_unstable();
    let count = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), count);
}

/// The whole point of a preset: a coach picks it and starts. Every one must compile against
/// the shipped library, or the starter content is broken on first run.
#[test]
fn every_preset_compiles_into_a_runnable_course() {
    let lib = ExerciseLibrary::preset();
    for t in presets() {
        let course = t
            .compile(&lib)
            .unwrap_or_else(|e| panic!("{} does not compile: {e:?}", t.name));
        assert!(!course.is_empty(), "{} compiles to nothing", t.name);
        assert!(
            course.steps.iter().all(|s| s.target.is_some()),
            "{} has an untargeted step",
            t.name
        );
    }
}

#[test]
fn engine_800_is_the_brief_s_first_template() {
    let lib = ExerciseLibrary::preset();
    let t = presets()
        .into_iter()
        .find(|t| t.name == "HYROX Engine 800")
        .expect("Engine 800");
    assert_eq!(t.category, domain::TemplateCategory::Engine);
    assert_eq!(t.blocks[0].block_type, BlockType::Sequential);

    let course = t.compile(&lib).unwrap();
    assert_eq!(
        course.stations().collect::<Vec<_>>(),
        ["RUN", "SKIERG", "RUN", "ROWING", "RUN", "WALL BALLS"]
    );
    assert_eq!(
        course.step(1).unwrap().target,
        Some(domain::StationTarget::Distance { meters: 1_000 })
    );
    assert_eq!(
        course.step(5).unwrap().target,
        Some(domain::StationTarget::Repetitions { count: 50 })
    );
}

#[test]
fn engine_short_is_three_rounds_of_four() {
    let t = presets()
        .into_iter()
        .find(|t| t.name == "HYROX Engine Short")
        .expect("Engine Short");
    assert_eq!(t.blocks[0].block_type, BlockType::Rounds);
    assert_eq!(t.blocks[0].rounds, Some(3));
    assert_eq!(t.blocks[0].exercises.len(), 4);
    assert_eq!(t.step_count(), 12);
}

#[test]
fn power_is_three_rounds_of_five_functional_movements() {
    let lib = ExerciseLibrary::preset();
    let t = presets()
        .into_iter()
        .find(|t| t.name == "HYROX Power")
        .expect("Power");
    assert_eq!(t.category, domain::TemplateCategory::Power);
    assert_eq!(t.step_count(), 15);
    let course = t.compile(&lib).unwrap();
    assert_eq!(course.occurrences("SLED PUSH"), 3);
    assert_eq!(course.occurrences("SANDBAG LUNGES"), 3);
}

/// Eight runs interleaved with the eight stations -- the shape a HYROX race actually has.
#[test]
fn complete_short_alternates_runs_with_the_eight_stations() {
    let lib = ExerciseLibrary::preset();
    let t = presets()
        .into_iter()
        .find(|t| t.name == "HYROX Complete Short")
        .expect("Complete");
    let course = t.compile(&lib).unwrap();

    assert_eq!(course.len(), 16);
    assert_eq!(course.occurrences("RUN"), 8);
    for (i, station) in course.stations().enumerate() {
        if i % 2 == 0 {
            assert_eq!(station, "RUN", "step {i} should be a run");
        } else {
            assert_ne!(station, "RUN", "step {i} should be a station");
        }
    }
}

/// A preset a coach wants changed is duplicated, never edited (brief §4). Worth asserting on
/// the shipped content itself, because this is the property a screen relies on.
#[test]
fn a_preset_duplicates_into_something_editable() {
    let all = presets();
    let copy = all[0].duplicate("t-copy", "My Version", Some("coach-ana"));
    assert!(copy.is_editable());
    assert_eq!(copy.blocks, all[0].blocks);
}
