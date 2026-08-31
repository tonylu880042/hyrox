-- Workout templates, the class-session lifecycle, and exercise/station capability
-- (workout brief §3-§12; ADR 0008).
--
-- Additive except for one deliberate rewrite: `sessions.status` gains three states and
-- renames two. SQLite cannot alter a CHECK constraint, so the table is rebuilt and the
-- existing rows are mapped ARMED -> RUNNING, CLOSED -> COMPLETED. No row is dropped, no
-- other table is touched, and `raw_events` / `interpreted_events` are not touched at all.
--
-- The store opens with `foreign_keys=ON` and `sqlx` runs every migration inside a
-- transaction, where `PRAGMA foreign_keys=OFF` is silently ignored. So the rebuild cannot
-- use the usual create-copy-drop: `DROP TABLE sessions` performs an implicit delete whose
-- foreign-key check is refused while `session_athletes`, `session_configs` or
-- `interpreted_events` hold rows -- verified against a real pre-0004 database.
--
-- Instead the old table is renamed aside, which rewrites those three children to point at
-- `sessions_old`, and each child is then rebuilt pointing at the new `sessions`. Every
-- child is a leaf -- nothing references them -- so each rebuild is unblocked, and by the
-- time `sessions_old` is dropped nothing references it either. All of it inside the one
-- transaction sqlx provides, so a crash leaves the old schema, never half of the new one.

-- 1. Exercise library ------------------------------------------------------------------
--
-- `station_key` is the slug the live screen already uses to pick a pictogram, and the
-- string a reader is registered against. Keeping it beside the code is what lets an
-- exercise vocabulary (`WALL_BALL`) arrive without breaking screens and reader maps that
-- were built on the display names (`WALL BALLS`).
CREATE TABLE IF NOT EXISTS exercises (
    code                   TEXT PRIMARY KEY,
    display_name           TEXT NOT NULL,
    category               TEXT NOT NULL
        CHECK (category IN ('RUN', 'ERG', 'FUNCTIONAL', 'OTHER')),
    station_key            TEXT NOT NULL,
    default_target_type    TEXT NOT NULL,
    -- JSON array. A list on one row rather than a child table: it is read whole, written
    -- whole, and nothing joins against it.
    supported_target_types TEXT NOT NULL,
    enabled                INTEGER NOT NULL DEFAULT 1,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL
);

-- 2. Templates -------------------------------------------------------------------------
--
-- One document per template rather than template/block/exercise tables. The blocks are a
-- nested, ordered structure the hub reads whole and writes whole, exactly like the course
-- snapshot in `session_configs` (migration 0003's reasoning, unchanged): normalising it
-- would buy joins nobody performs, and ordering columns nobody can violate anyway.
CREATE TABLE IF NOT EXISTS workout_templates (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    description   TEXT,
    category      TEXT NOT NULL CHECK (category IN
                      ('FOUNDATIONAL', 'ENGINE', 'POWER', 'COMPLETE', 'RACE_SIMULATION', 'CUSTOM')),
    -- SYSTEM templates are read-only; editing one means duplicating it first (brief §4).
    source        TEXT NOT NULL CHECK (source IN ('SYSTEM', 'COACH')),
    owner_id      TEXT,
    version       INTEGER NOT NULL DEFAULT 1,
    difficulty    TEXT,
    estimated_duration_minutes INTEGER,
    enabled       INTEGER NOT NULL DEFAULT 1,
    blocks_json   TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_templates_listing ON workout_templates (source, enabled, name);

-- 3. Physical stations -------------------------------------------------------------------
--
-- An exercise is not a station (brief §12): ROWERG is the work, ROW_01 is the machine. A
-- station declares which exercise it can serve, so a venue can hold three rowers without
-- three exercises. Readers stay keyed on (device_id, reader_id) as before; this table adds
-- the capability question the expectation check asks.
CREATE TABLE IF NOT EXISTS stations (
    id            TEXT PRIMARY KEY,
    exercise_code TEXT NOT NULL REFERENCES exercises(code),
    display_name  TEXT NOT NULL,
    zone          TEXT
);

CREATE INDEX IF NOT EXISTS idx_stations_by_exercise ON stations (exercise_code);

-- 4. The class-session lifecycle -------------------------------------------------------

ALTER TABLE sessions RENAME TO sessions_old;

CREATE TABLE sessions (
    id                      TEXT PRIMARY KEY,
    name                    TEXT NOT NULL,
    mode                    TEXT NOT NULL CHECK (mode IN ('COMPETITION', 'TRAINING')),
    status                  TEXT NOT NULL CHECK (status IN
                                ('DRAFT', 'READY', 'RUNNING', 'PAUSED', 'COMPLETED', 'CANCELLED')),
    interpreted_event_count INTEGER NOT NULL DEFAULT 0,
    created_at              INTEGER NOT NULL,
    -- Class-clock pause accounting (ADR 0008). A hub that restarts mid-pause must come back
    -- paused: without these two columns the pause would be handed back as class time.
    paused_total_ms         INTEGER NOT NULL DEFAULT 0,
    paused_since            INTEGER,
    -- Which template this class was built from, and at which version. Provenance only: the
    -- class runs off its own snapshot in `session_configs`, never off the template, so a
    -- later edit to the template cannot reach back into a class that already happened.
    template_id             TEXT REFERENCES workout_templates(id),
    template_version        INTEGER,
    coach_id                TEXT,
    scheduled_at            INTEGER
);

INSERT INTO sessions
    (id, name, mode, status, interpreted_event_count, created_at)
SELECT id, name, mode,
    CASE status WHEN 'ARMED' THEN 'RUNNING' WHEN 'CLOSED' THEN 'COMPLETED' ELSE status END,
    interpreted_event_count, created_at
FROM sessions_old;

-- 5. Repoint the children -----------------------------------------------------------------
--
-- The rename in step 4 rewrote each of these to say `REFERENCES "sessions_old"(id)`. They
-- are rebuilt verbatim apart from that clause: same columns, same constraints, same rows.

CREATE TABLE session_athletes_new (
    session_id   TEXT NOT NULL REFERENCES sessions(id),
    athlete_id   TEXT NOT NULL,
    display_name TEXT NOT NULL,
    bib          INTEGER NOT NULL,
    PRIMARY KEY (session_id, athlete_id)
);
INSERT INTO session_athletes_new SELECT * FROM session_athletes;
DROP TABLE session_athletes;
ALTER TABLE session_athletes_new RENAME TO session_athletes;

CREATE TABLE session_configs_new (
    session_id  TEXT PRIMARY KEY REFERENCES sessions(id),
    config_json TEXT NOT NULL
);
INSERT INTO session_configs_new SELECT * FROM session_configs;
DROP TABLE session_configs;
ALTER TABLE session_configs_new RENAME TO session_configs;

-- Immutable in spirit but not in schema: the row ids must survive, because voided-event
-- audit records and `raw_event_id` links both name them. `INSERT ... SELECT` with the id
-- column listed explicitly preserves every one.
CREATE TABLE interpreted_events_new (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       TEXT NOT NULL REFERENCES sessions(id),
    athlete_id       TEXT NOT NULL,
    raw_event_id     INTEGER REFERENCES raw_events(id),
    kind             TEXT NOT NULL CHECK (kind IN ('ENTERED', 'EXITED', 'EXCEPTION')),
    station          TEXT,
    detected_at      INTEGER NOT NULL,
    transition_ms    INTEGER,
    started_timing   INTEGER NOT NULL DEFAULT 0,
    exception_reason TEXT,
    voided_at        INTEGER,
    voided_by        TEXT,
    void_reason      TEXT
);
INSERT INTO interpreted_events_new
    (id, session_id, athlete_id, raw_event_id, kind, station, detected_at, transition_ms,
     started_timing, exception_reason, voided_at, voided_by, void_reason)
SELECT id, session_id, athlete_id, raw_event_id, kind, station, detected_at, transition_ms,
       started_timing, exception_reason, voided_at, voided_by, void_reason
FROM interpreted_events;
DROP TABLE interpreted_events;
ALTER TABLE interpreted_events_new RENAME TO interpreted_events;

CREATE INDEX IF NOT EXISTS idx_interpreted_replay
    ON interpreted_events (session_id, detected_at, id);

-- 6. Nothing references the old table any more -----------------------------------------------
DROP TABLE sessions_old;
