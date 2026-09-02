-- One inventory round, several tags (ADR 0014).
--
-- UHF anti-collision reports every tag in the field at once, so an edge event now carries a
-- list. The unit of *delivery* becomes the round -- one message, one `sequence`, one ACK --
-- while the unit of *record* stays the tag: one row per tag, so `idx_raw_by_tag`, the
-- retroactive claim and the check-in queue keep working unchanged.
--
-- That means widening the uniqueness from (device, boot, sequence) to
-- (device, boot, sequence, tag). SQLite cannot alter a table constraint, so `raw_events` is
-- rebuilt. `id` values are copied verbatim: `interpreted_events.raw_event_id` names them,
-- and so do the audit records of voided events (CLAUDE.md 19, 20).
--
-- Same recipe as 0004, and for the same reason: sqlx runs this inside a transaction, where
-- `PRAGMA foreign_keys=OFF` is silently ignored, so `DROP TABLE raw_events` would be refused
-- while `interpreted_events` points at it. Instead the old table is renamed aside -- which
-- rewrites the child to point at `raw_events_old` -- the child is rebuilt against the new
-- table, and only then is the old one dropped, by which point nothing references it.

ALTER TABLE raw_events RENAME TO raw_events_old;

CREATE TABLE raw_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id   TEXT NOT NULL,
    reader_id   TEXT NOT NULL,
    boot_id     INTEGER NOT NULL,
    sequence    INTEGER NOT NULL,
    tag_id      TEXT NOT NULL,
    detected_at INTEGER NOT NULL,  -- official timing (CLAUDE.md 11, 17)
    received_at INTEGER NOT NULL,  -- diagnostics only
    -- The idempotency key is still device + boot + sequence (CLAUDE.md 16); the tag says
    -- which read *within* that round this row is.
    UNIQUE (device_id, boot_id, sequence, tag_id)
);

INSERT INTO raw_events
    (id, device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at)
SELECT id, device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at
FROM raw_events_old;

CREATE INDEX IF NOT EXISTS idx_raw_by_tag ON raw_events (tag_id, detected_at, id);

-- The one child. Rebuilt only to point its `raw_event_id` back at the new `raw_events`;
-- every column and every id is carried over unchanged.
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

-- Nothing references the old table any more.
DROP TABLE raw_events_old;
