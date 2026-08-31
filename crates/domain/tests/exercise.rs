//! The exercise library and the unit rules (workout brief §3, §7, §18).

use domain::{
    ExerciseCategory, ExerciseLibrary, Target, TargetError, TargetType, Unit, Weight, WeightUnit,
};

fn library() -> ExerciseLibrary {
    ExerciseLibrary::preset()
}

#[test]
fn the_preset_library_holds_the_nine_hyrox_movements() {
    let lib = library();
    for code in [
        "RUN",
        "SKIERG",
        "ROWERG",
        "SLED_PUSH",
        "SLED_PULL",
        "BURPEE_BROAD_JUMP",
        "FARMERS_CARRY",
        "SANDBAG_LUNGE",
        "WALL_BALL",
    ] {
        assert!(lib.get(code).is_some(), "{code} is missing from the library");
    }
    assert_eq!(lib.len(), 9);
}

#[test]
fn a_code_is_matched_regardless_of_case() {
    assert_eq!(library().get("wall_ball").map(|e| e.code.as_str()), Some("WALL_BALL"));
}

/// The station key is the string a course step and a reader registration both carry. It is
/// deliberately NOT the code: the venue's readers and the live screen's pictograms were
/// built on the display spelling, and an exercise vocabulary arriving later must not
/// silently repoint either of them (ADR 0008).
#[test]
fn an_exercise_carries_the_station_name_the_venue_already_uses() {
    let lib = library();
    assert_eq!(lib.get("WALL_BALL").unwrap().station_key, "WALL BALLS");
    assert_eq!(lib.get("ROWERG").unwrap().station_key, "ROWING");
    assert_eq!(lib.get("SKIERG").unwrap().station_key, "SKIERG");
}

#[test]
fn exercises_are_categorised() {
    let lib = library();
    assert_eq!(lib.get("RUN").unwrap().category, ExerciseCategory::Run);
    assert_eq!(lib.get("ROWERG").unwrap().category, ExerciseCategory::Erg);
    assert_eq!(lib.get("WALL_BALL").unwrap().category, ExerciseCategory::Functional);
}

// --- target types ----------------------------------------------------------------------

#[test]
fn an_exercise_accepts_only_the_target_types_it_declares() {
    let lib = library();
    let run = lib.get("RUN").unwrap();
    assert!(run.accepts(TargetType::Distance));
    assert!(run.accepts(TargetType::Time));
    assert!(!run.accepts(TargetType::Reps), "a run is not counted in reps");

    let wall_ball = lib.get("WALL_BALL").unwrap();
    assert!(wall_ball.accepts(TargetType::Reps));
    assert!(!wall_ball.accepts(TargetType::Distance), "wall balls are not measured in metres");
}

#[test]
fn every_exercise_supports_its_own_default_target_type() {
    for e in library().iter() {
        assert!(
            e.accepts(e.default_target_type),
            "{} defaults to a target type it does not support",
            e.code
        );
    }
}

// --- units -------------------------------------------------------------------------------

#[test]
fn a_unit_belongs_to_exactly_one_target_type() {
    assert_eq!(Unit::Meter.target_type(), TargetType::Distance);
    assert_eq!(Unit::Kilometer.target_type(), TargetType::Distance);
    assert_eq!(Unit::Reps.target_type(), TargetType::Reps);
    assert_eq!(Unit::Second.target_type(), TargetType::Time);
    assert_eq!(Unit::Minute.target_type(), TargetType::Time);
    assert_eq!(Unit::Calorie.target_type(), TargetType::Calories);
}

/// Canonical units are metres, reps, seconds and calories (brief §7). Everything the hub
/// computes with is canonical; the display unit is a presentation choice kept beside it.
#[test]
fn values_convert_to_canonical_units() {
    let lib = library();
    let run = lib.get("RUN").unwrap();
    assert_eq!(Target::new(run, 800, Unit::Meter).unwrap().canonical(), 800);
    assert_eq!(Target::new(run, 5, Unit::Kilometer).unwrap().canonical(), 5_000);
    assert_eq!(Target::new(run, 3, Unit::Minute).unwrap().canonical(), 180);
    let wb = lib.get("WALL_BALL").unwrap();
    assert_eq!(Target::new(wb, 50, Unit::Reps).unwrap().canonical(), 50);
}

#[test]
fn a_target_remembers_the_unit_it_was_written_in() {
    let lib = library();
    let t = Target::new(lib.get("RUN").unwrap(), 1, Unit::Kilometer).unwrap();
    assert_eq!(t.value, 1);
    assert_eq!(t.unit, Unit::Kilometer);
    assert_eq!(t.canonical(), 1_000);
}

// --- validation (brief §18) ---------------------------------------------------------------

#[test]
fn a_target_of_zero_is_rejected() {
    let lib = library();
    assert_eq!(
        Target::new(lib.get("RUN").unwrap(), 0, Unit::Meter),
        Err(TargetError::NotPositive)
    );
}

#[test]
fn a_unit_that_does_not_match_the_exercise_is_rejected() {
    let lib = library();
    assert_eq!(
        Target::new(lib.get("RUN").unwrap(), 50, Unit::Reps),
        Err(TargetError::UnsupportedTargetType {
            code: "RUN".into(),
            target_type: TargetType::Reps
        })
    );
    assert_eq!(
        Target::new(lib.get("WALL_BALL").unwrap(), 800, Unit::Meter),
        Err(TargetError::UnsupportedTargetType {
            code: "WALL_BALL".into(),
            target_type: TargetType::Distance
        })
    );
}

// --- weight --------------------------------------------------------------------------------

#[test]
fn weight_is_stored_in_grams_so_half_kilos_survive() {
    let w = Weight::new(4, WeightUnit::Kilogram);
    assert_eq!(w.grams, 4_000);
    let half = Weight::from_grams(2_500, WeightUnit::Kilogram);
    assert_eq!(half.label(), "2.5 KG");
}

#[test]
fn zero_weight_is_legal_because_an_unloaded_sled_is_a_real_prescription() {
    assert_eq!(Weight::new(0, WeightUnit::Kilogram).grams, 0);
}

#[test]
fn pounds_convert_to_grams_and_display_as_pounds() {
    let w = Weight::new(20, WeightUnit::Pound);
    assert_eq!(w.grams, 9_071);
    assert_eq!(w.label(), "20 LB");
}
