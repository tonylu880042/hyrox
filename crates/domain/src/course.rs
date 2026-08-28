//! Course definition (CLAUDE.md 9.2), also used as a competition template (CLAUDE.md 9.1).
//!
//! A course is a plan the operator wrote down, not a rule the engine enforces. Training
//! records whatever actually happens and must not warn on a different order (CLAUDE.md 9.2),
//! so nothing here offers an accept/reject verb -- comparison against the plan belongs to
//! the competition path in the interpretation layer.

use crate::time::Duration;
use serde::{Deserialize, Serialize};

/// What the athlete is meant to do at one step. Optional per step (CLAUDE.md 9.2).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StationTarget {
    Distance { meters: u32 },
    Repetitions { count: u32 },
    /// The hub only learns entry/exit times from RFID, so a duration target is a label to
    /// display, not something it can verify (ADR 0001, 2026-08-27 addendum).
    // A struct variant, not a newtype: an internally tagged enum cannot serialise a
    // newtype variant wrapping a scalar.
    Duration { duration: Duration },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseStep {
    pub station: String,
    pub target: Option<StationTarget>,
}

impl CourseStep {
    pub fn new(station: impl Into<String>) -> Self {
        Self { station: station.into(), target: None }
    }

    pub fn with_target(mut self, target: StationTarget) -> Self {
        self.target = Some(target);
        self
    }
}

/// An ordered list of steps. A station repeats simply by appearing more than once, and
/// each occurrence keeps its own target -- the two runs of an interval are different legs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Course {
    pub name: String,
    pub steps: Vec<CourseStep>,
}

impl Course {
    pub fn new(name: impl Into<String>, steps: Vec<CourseStep>) -> Self {
        Self { name: name.into(), steps }
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// None past the end rather than a panic: a course may be shorter than the events.
    pub fn step(&self, index: usize) -> Option<&CourseStep> {
        self.steps.get(index)
    }

    /// How many times the plan visits a station, for a UI that shows "run 2 of 4".
    pub fn occurrences(&self, station: &str) -> usize {
        self.steps.iter().filter(|s| s.station == station).count()
    }

    pub fn stations(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().map(|s| s.station.as_str())
    }
}
