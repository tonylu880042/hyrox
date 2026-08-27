-- Central Hub schema, Phase 1 (CLAUDE.md 19).
-- Raw RFID events are immutable; interpretation and corrections live in separate tables.

CREATE TABLE IF NOT EXISTS sessions (
    id                      TEXT PRIMARY KEY,
    name                    TEXT NOT NULL,
    mode                    TEXT NOT NULL CHECK (mode IN ('COMPETITION', 'TRAINING')),
    status                  TEXT NOT NULL CHECK (status IN ('DRAFT', 'ARMED', 'CLOSED')),
    interpreted_event_count INTEGER NOT NULL DEFAULT 0,
    created_at              INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS session_athletes (
    session_id   TEXT NOT NULL REFERENCES sessions(id),
    athlete_id   TEXT NOT NULL,
    display_name TEXT NOT NULL,
    bib          INTEGER NOT NULL,
    PRIMARY KEY (session_id, athlete_id)
);

-- Immutable (CLAUDE.md 19). Never updated, never deleted, not even by a correction.
-- The unique triple is the idempotency key from the edge protocol (CLAUDE.md 16).
CREATE TABLE IF NOT EXISTS raw_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id   TEXT NOT NULL,
    reader_id   TEXT NOT NULL,
    boot_id     INTEGER NOT NULL,
    sequence    INTEGER NOT NULL,
    tag_id      TEXT NOT NULL,
    detected_at INTEGER NOT NULL,  -- official timing (CLAUDE.md 11, 17)
    received_at INTEGER NOT NULL,  -- diagnostics only
    UNIQUE (device_id, boot_id, sequence)
);

-- The hub's interpretation. Operators may add rows here and void existing ones; replaying
-- the non-voided rows in detected_at order rebuilds athlete state (CLAUDE.md 20, 21).
CREATE TABLE IF NOT EXISTS interpreted_events (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       TEXT NOT NULL REFERENCES sessions(id),
    athlete_id       TEXT NOT NULL,
    raw_event_id     INTEGER REFERENCES raw_events(id),  -- NULL for operator-added events
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

CREATE INDEX IF NOT EXISTS idx_interpreted_replay
    ON interpreted_events (session_id, detected_at, id);
