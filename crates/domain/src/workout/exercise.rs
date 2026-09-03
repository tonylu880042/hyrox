//! The exercise library, the unit system and target validation (workout brief §3, §7, §18).
//!
//! An exercise is *what the athlete does*; a station is *where they do it* (see
//! [`crate::station`]). The two are kept apart so a venue can hold three rowers without
//! three exercises.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExerciseCategory {
    Run,
    Erg,
    Functional,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TargetType {
    Distance,
    Reps,
    Time,
    Calories,
}

/// The units a coach may write a target in. Each belongs to exactly one target type, which
/// is what makes `RUN + REPS` unrepresentable rather than merely discouraged (brief §18).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Unit {
    Meter,
    Kilometer,
    Reps,
    Second,
    Minute,
    Calorie,
}

impl Unit {
    pub fn target_type(self) -> TargetType {
        match self {
            Unit::Meter | Unit::Kilometer => TargetType::Distance,
            Unit::Reps => TargetType::Reps,
            Unit::Second | Unit::Minute => TargetType::Time,
            Unit::Calorie => TargetType::Calories,
        }
    }

    /// How many canonical units one of this unit is worth. Canonical is metres, reps,
    /// seconds and calories (brief §7) -- everything the hub computes with.
    pub fn canonical_factor(self) -> u32 {
        match self {
            Unit::Kilometer => 1_000,
            Unit::Minute => 60,
            _ => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Unit::Meter => "M",
            Unit::Kilometer => "KM",
            Unit::Reps => "REPS",
            Unit::Second => "S",
            Unit::Minute => "MIN",
            Unit::Calorie => "CAL",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WeightUnit {
    Kilogram,
    Pound,
}

/// A prescribed load. Held in grams so a 2.5 kg wall ball and a 20 lb sandbag are both
/// exact, with the unit the coach wrote kept beside it for display (brief §7).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Weight {
    pub grams: u32,
    pub unit: WeightUnit,
}

impl Weight {
    pub fn new(value: u32, unit: WeightUnit) -> Self {
        let grams = match unit {
            WeightUnit::Kilogram => value * 1_000,
            // 453.59237 g, truncated. A gym plate is not weighed to the milligram.
            WeightUnit::Pound => value * 45_359 / 100,
        };
        Self { grams, unit }
    }

    pub fn from_grams(grams: u32, unit: WeightUnit) -> Self {
        Self { grams, unit }
    }

    /// "4 KG", "2.5 KG", "20 LB". One decimal at most: a coach writes 2.5, never 2.53.
    pub fn label(self) -> String {
        let (per_unit, suffix) = match self.unit {
            WeightUnit::Kilogram => (1_000u32, "KG"),
            WeightUnit::Pound => (45_359u32 / 100, "LB"),
        };
        let whole = self.grams / per_unit;
        let tenths = (self.grams % per_unit) * 10 / per_unit;
        if tenths == 0 {
            format!("{whole} {suffix}")
        } else {
            format!("{whole}.{tenths} {suffix}")
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TargetError {
    /// `target_value > 0` (brief §18). A zero-metre run is not a plan.
    NotPositive,
    /// e.g. `RUN + REPS`, or `WALL_BALL + DISTANCE` (brief §18).
    UnsupportedTargetType {
        code: String,
        target_type: TargetType,
    },
}

/// What the athlete is meant to do at one step, as a value and the unit it was written in.
///
/// Never a string: `"800m"` cannot be compared, converted or summed (brief §7, §28).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Target {
    pub target_type: TargetType,
    pub value: u32,
    pub unit: Unit,
}

impl Target {
    /// The only constructor, and it takes the exercise: a target that the exercise does not
    /// support cannot be built at all, so no later stage has to re-check it (brief §18).
    pub fn new(exercise: &Exercise, value: u32, unit: Unit) -> Result<Self, TargetError> {
        if value == 0 {
            return Err(TargetError::NotPositive);
        }
        let target_type = unit.target_type();
        if !exercise.accepts(target_type) {
            return Err(TargetError::UnsupportedTargetType {
                code: exercise.code.clone(),
                target_type,
            });
        }
        Ok(Self {
            target_type,
            value,
            unit,
        })
    }

    /// Metres, reps, seconds or calories (brief §7).
    pub fn canonical(self) -> u32 {
        self.value * self.unit.canonical_factor()
    }

    pub fn label(self) -> String {
        format!("{} {}", self.value, self.unit.label())
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Exercise {
    /// The stable identifier a template refers to, e.g. `WALL_BALL`.
    pub code: String,
    /// What a coach reads, e.g. "Wall Ball".
    pub display_name: String,
    pub category: ExerciseCategory,
    /// The station name a course step and a reader registration both carry.
    ///
    /// Deliberately not the code. The venue's readers are registered against "WALL BALLS"
    /// and the live screen picks its pictogram from a slug of the same string; an exercise
    /// vocabulary arriving later must not silently repoint either (ADR 0008).
    pub station_key: String,
    pub default_target_type: TargetType,
    pub supported_target_types: Vec<TargetType>,
    pub enabled: bool,
}

impl Exercise {
    pub fn accepts(&self, target_type: TargetType) -> bool {
        self.supported_target_types.contains(&target_type)
    }
}

/// The exercises this hub knows about. A Vec rather than a map: there are nine of them,
/// lookup is not hot, and the order is what a coach's picker shows.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExerciseLibrary {
    exercises: Vec<Exercise>,
}

impl ExerciseLibrary {
    pub fn new(exercises: Vec<Exercise>) -> Self {
        Self { exercises }
    }

    /// Case-insensitive: a template written by hand should not fail on `wall_ball`.
    pub fn get(&self, code: &str) -> Option<&Exercise> {
        self.exercises
            .iter()
            .find(|e| e.code.eq_ignore_ascii_case(code))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Exercise> {
        self.exercises.iter()
    }

    pub fn len(&self) -> usize {
        self.exercises.len()
    }

    pub fn is_empty(&self) -> bool {
        self.exercises.is_empty()
    }

    /// The HYROX movements, seeded on first run (brief §3).
    ///
    /// Station keys are the spellings the venue and the live screen already use, which is
    /// why they are not simply the codes with the underscores taken out.
    pub fn preset() -> Self {
        use ExerciseCategory::*;
        use TargetType::*;
        let e = |code: &str, name: &str, station: &str, cat, default, supported: &[TargetType]| {
            Exercise {
                code: code.to_string(),
                display_name: name.to_string(),
                category: cat,
                station_key: station.to_string(),
                default_target_type: default,
                supported_target_types: supported.to_vec(),
                enabled: true,
            }
        };
        Self::new(vec![
            e("RUN", "Run", "RUN", Run, Distance, &[Distance, Time]),
            e(
                "SKIERG",
                "SkiErg",
                "SKIERG",
                Erg,
                Distance,
                &[Distance, Time, Calories],
            ),
            e(
                "ROWERG",
                "RowErg",
                "ROWING",
                Erg,
                Distance,
                &[Distance, Time, Calories],
            ),
            e(
                "SLED_PUSH",
                "Sled Push",
                "SLED PUSH",
                Functional,
                Distance,
                &[Distance, Time],
            ),
            e(
                "SLED_PULL",
                "Sled Pull",
                "SLED PULL",
                Functional,
                Distance,
                &[Distance, Time],
            ),
            e(
                "BURPEE_BROAD_JUMP",
                "Burpee Broad Jump",
                "BURPEE BROAD JUMP",
                Functional,
                Distance,
                &[Distance, Reps, Time],
            ),
            e(
                "FARMERS_CARRY",
                "Farmers Carry",
                "FARMERS CARRY",
                Functional,
                Distance,
                &[Distance, Time],
            ),
            e(
                "SANDBAG_LUNGE",
                "Sandbag Lunge",
                "SANDBAG LUNGES",
                Functional,
                Distance,
                &[Distance, Reps, Time],
            ),
            e(
                "WALL_BALL",
                "Wall Ball",
                "WALL BALLS",
                Functional,
                Reps,
                &[Reps, Time],
            ),
        ])
    }
}
