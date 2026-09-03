//! Physical stations, and whether an athlete is where the plan expects (brief §11, §12, §20).
//!
//! Two things the venue keeps apart:
//!
//! * an **exercise** is the work -- `ROWERG`;
//! * a **physical station** is the machine -- `ROW_01`, `ROW_02`, `ROW_03`.
//!
//! A course step carries the exercise's `station_key` (`"ROWING"`), which is also what a
//! reader is registered against, so a venue with three rowers needs three registrations and
//! one exercise. This module is what maps a machine back to the work it can serve.
//!
//! Expectation is **recorded, never enforced**. Training records what actually happens and
//! must not warn on a different order (CLAUDE.md 9.2), and the competition exception rules
//! are still an open product question (CLAUDE.md 28) -- so nothing here disqualifies
//! anybody. It is a label a screen may show and a future rule engine may consume.

use crate::athlete::AthleteState;
use crate::course::Course;
use crate::workout::ExerciseLibrary;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PhysicalStation {
    /// The venue's own name for the machine, e.g. `ROW_01`.
    pub id: String,
    /// The exercise it can serve, e.g. `ROWERG`.
    pub exercise_code: String,
    pub display_name: String,
    pub zone: Option<String>,
}

impl PhysicalStation {
    pub fn new(
        id: impl Into<String>,
        exercise_code: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            exercise_code: exercise_code.into(),
            display_name: display_name.into(),
            zone: None,
        }
    }

    pub fn with_zone(mut self, zone: impl Into<String>) -> Self {
        self.zone = Some(zone.into());
        self
    }
}

/// The venue's equipment. Like the reader registry it is a Vec: a gym has tens of machines,
/// lookup is not hot, and insertion order is what an operator screen shows.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StationMap {
    stations: Vec<PhysicalStation>,
}

impl StationMap {
    pub fn new(stations: Vec<PhysicalStation>) -> Self {
        Self { stations }
    }

    pub fn get(&self, station_id: &str) -> Option<&PhysicalStation> {
        self.stations
            .iter()
            .find(|s| s.id.eq_ignore_ascii_case(station_id))
    }

    /// Every machine that can serve one exercise, in venue order.
    pub fn serving<'a>(
        &'a self,
        exercise_code: &'a str,
    ) -> impl Iterator<Item = &'a PhysicalStation> {
        self.stations
            .iter()
            .filter(move |s| s.exercise_code.eq_ignore_ascii_case(exercise_code))
    }

    pub fn can_serve(&self, station_id: &str, exercise_code: &str) -> bool {
        self.get(station_id)
            .is_some_and(|s| s.exercise_code.eq_ignore_ascii_case(exercise_code))
    }

    /// The course station key a machine satisfies -- `ROW_02` -> `"ROWING"`. `None` when
    /// the machine is unknown, or serves an exercise the library does not hold.
    pub fn station_key(&self, station_id: &str, library: &ExerciseLibrary) -> Option<String> {
        let station = self.get(station_id)?;
        Some(library.get(&station.exercise_code)?.station_key.clone())
    }

    pub fn iter(&self) -> impl Iterator<Item = &PhysicalStation> {
        self.stations.iter()
    }

    pub fn len(&self) -> usize {
        self.stations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stations.is_empty()
    }

    /// Register or replace one machine, keyed on its id.
    pub fn register(&mut self, station: PhysicalStation) -> Option<PhysicalStation> {
        match self
            .stations
            .iter_mut()
            .find(|s| s.id.eq_ignore_ascii_case(&station.id))
        {
            Some(existing) => Some(std::mem::replace(existing, station)),
            None => {
                self.stations.push(station);
                None
            }
        }
    }
}

/// How a read compares with the plan (brief §11).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Expectation {
    /// The station the plan calls for next.
    Expected,
    /// In the plan, but not at this point in it.
    OutOfOrder,
    /// Not in the plan at all.
    Unexpected,
    /// There is no plan to compare against -- a drop-in class with no course. Distinct from
    /// `Unexpected` on purpose: "no answer" is not "wrong" (CLAUDE.md 12, 28).
    Unknown,
}

/// Compare where an athlete has arrived with where the plan expects them.
///
/// Pure and derived: it reads the athlete's completed runs, never a stored stage row, so
/// voiding an interpretation changes the answer exactly as it changes everything else
/// derived from the log (CLAUDE.md 20, 21).
pub fn expectation(course: &Course, state: &AthleteState, station: &str) -> Expectation {
    if course.is_empty() {
        return Expectation::Unknown;
    }
    let completed = state.runs.iter().filter(|r| r.exited_at.is_some()).count();
    match course.step(completed) {
        Some(step) if step.station == station => Expectation::Expected,
        _ if course.stations().any(|s| s == station) => Expectation::OutOfOrder,
        _ => Expectation::Unexpected,
    }
}
