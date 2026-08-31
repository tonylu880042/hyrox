# ADR 0008 — Workout templates compile into the existing course

**Status:** accepted, 2026-08-29
**Supersedes nothing. Amends:** ADR 0001 D2 (session states), ADR 0004 (recovered configuration).

## Context

Group classes are not always a full HYROX race. A coach builds a plan — six stations, or three
rounds of four — reuses it week to week, and adjusts it on the night. The hub had no such
object: a `Course` existed only attached to one session, with no library, no blocks, no
rounds, and station names as bare strings with nothing saying which targets are legal.

What it *did* already have is the part that matters most, and that nothing here may break:

* athlete state derived by replaying non-voided interpreted events (CLAUDE.md 21);
* the session's plan snapshotted at arm time and recovered from the store, never from the
  caller (ADR 0004);
* raw events immutable, corrections at the interpreted layer only (CLAUDE.md 19, 20).

## Decision

### 1. A template is the editable artefact; a class runs on a compiled course

```text
WorkoutTemplate ──compile──> Course ──snapshot──> ClassSession ──replay──> AthleteState
  (blocks, rounds,            (flat,              (session_configs)        (runs, splits)
   exercise codes, units)      station keys)
```

`WorkoutTemplate::compile` expands rounds and resolves exercise codes to station keys,
producing exactly the `Course` the engine already consumes. Everything upstream of the
compile step — blocks, rounds, AMRAP, units, weights — stops there. The live screen, the
finish policy, replay and recovery are untouched, and a later edit to a template cannot
reach a class that already ran, because nothing downstream is looking at templates at all.

**Alternative rejected:** teaching `Course` about blocks and rounds. It would have pushed
repetition semantics into the finish rule (`CourseComplete` counts steps), into
`live::course_view`, and into every screen — for a structure that only matters while a coach
is editing.

### 2. Exercise and physical station stay separate; the station *key* is the join

`ROWERG` is the work; `ROW_01` is the machine. A course step and a reader registration both
carry the exercise's `station_key` — the string the venue already uses.

The station key is deliberately **not** the exercise code. The venue's readers are registered
against `"ROWING"` and the live screen slugs the same string into a pictogram
(`design/live/icons/rowing.png`). Making `ROWERG` the course station would have silently
unmapped every reader and blanked every icon.

### 3. Stages are derived, not stored

The brief asks for `AthleteSession` / `AthleteStage` rows. Storing them would create a second
source of truth that voiding an interpretation would not update, contradicting CLAUDE.md 21.
`application::stages` projects the same shape from the snapshot course and the replayed runs,
so a correction changes the stage list for free.

### 4. Expectation is recorded, never enforced

`domain::expectation` labels a read EXPECTED / OUT_OF_ORDER / UNEXPECTED / UNKNOWN. It
gates nothing. Training must not warn on a different order (CLAUDE.md 9.2) and the
competition exception rules are still undecided (CLAUDE.md 28), so no athlete is
disqualified by it. `UNKNOWN` exists so "there is no plan" cannot be read as "wrong".

### 5. The session lifecycle grows to six states

`DRAFT → READY → RUNNING ⇄ PAUSED → COMPLETED`, plus `CANCELLED` from any live state.
RUNNING is what earlier builds spelled ARMED; COMPLETED is what they spelled CLOSED.
Migration 0004 rewrites the stored values.

* **READY** is where a class built from a template is tweaked for tonight. Configuration is
  editable in DRAFT and READY, and locked from RUNNING onwards — ADR 0001 D2's rule, with
  the editable window given its own state instead of being implied.
* **PAUSED** changes what official time means, so it is not cosmetic. Paused wall time is
  excluded from the class clock via `domain::ClassClock`, which the session persists
  (`paused_total_ms`, `paused_since`) so a hub restarting mid-pause comes back paused. A
  `ClassDuration` finish rule resolves to `class_start + limit + paused_total` — still an
  exact instant, still not the moment a tick noticed (CLAUDE.md 11, 17).
* **COMPLETED → RUNNING** is refused by `start` and allowed by `reopen`, which the
  application layer will not run without a stated reason. ADR 0001 D2's mis-tap case is
  kept; it is a correction, not an ordinary transition.
* **CANCELLED** is terminal and is never reopened. Cancelling says the class did not happen;
  the honest repair is a new class.

### 6. Reads and writes stay split by path, not by resource

ADR 0007 §5 makes the read/write split structural. So the library is **read** on the
read-only surface (`/api/exercises`, `/api/workout-templates`) and **written** under
`/api/operator/**`. The brief's `/api/workout-templates` verbs would have put mutating
routes on a path space that a sweep test asserts is verb-free.

## Portability

Nothing added is platform specific. The domain gains no IO; the storage adapter is the same
SQLite; the builder is one static page with no build step. macOS and Linux are unaffected.

## Consequences

* Migration 0004 rebuilds `sessions` **and** its three child tables to widen a CHECK
  constraint. `sqlx` runs migrations inside a transaction where `PRAGMA foreign_keys=OFF` is
  ignored, so create-copy-drop is refused on any database holding events. The rename-first
  form is used instead, and `crates/storage/tests/migration_0004.rs` reconstructs a pre-0004
  database and asserts the whole thing — two earlier drafts passed on a fresh database and
  failed on a real one.
* AMRAP and ZONE_ROTATION blocks are storable but not compilable: how many rounds an athlete
  gets is the result, not the plan. `CompileError::BlockTypeNotRunnable` says so by name
  rather than guessing (CLAUDE.md 28).
* `StationTarget` gains a `Calories` variant, because ergs are commonly prescribed in
  calories. Like every other target it is a label the hub displays, not a number it verifies.
* The retired vocabulary is gone: `ARMED` and `CLOSED` no longer parse, and the migration's
  CHECK refuses them.
