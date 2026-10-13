-- ADR 0018: a raw read is the key, the tag and `uptime_ms` -- not the key and the tag alone.
--
-- A device whose flash was wiped counts `boot_id` from 1 again, so its new reads arrive
-- under keys the hub has already stored. With uniqueness on (device, boot, sequence, tag)
-- those reads were indistinguishable from redeliveries: acknowledged as duplicates, never
-- stored, and no error anywhere (CLAUDE.md 31 principle 1). `uptime_ms` is on the wire for
-- exactly this (firmware protocol v1.1 section 3) and was being dropped on arrival.
--
--   raw_events.uptime_ms   as sent. NULL on rows that predate this migration: nobody
--                          recorded it, and a made-up value would be a fabricated read.
--   UNIQUE                 + uptime_ms
--   idx_raw_by_tag         restored (see the end of this file)
--
-- Widening a table constraint means a rebuild, by the recipe of 0007: rename the old table
-- aside -- which repoints its children at `raw_events_old` -- rebuild the children against
-- the new one, and only then drop the old. Every row and every id is carried over unchanged
-- (CLAUDE.md 19).

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
    presence    TEXT NOT NULL DEFAULT 'ARRIVED'
        CHECK (presence IN ('ARRIVED', 'DEPARTED')),
    uptime_ms   INTEGER,           -- NULL = stored before ADR 0018
    -- The key the edge and the ACK speak is still device + boot + sequence (CLAUDE.md 16).
    -- The tag says which read within that round; `uptime_ms` says the round is the same
    -- one and not a wiped device reusing the number. NULLs never collide in SQLite, so a
    -- legacy row is matched by the store's lookup, not by this constraint.
    UNIQUE (device_id, boot_id, sequence, tag_id, uptime_ms)
);

INSERT INTO raw_events
    (id, device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at, presence)
SELECT id, device_id, reader_id, boot_id, sequence, tag_id, detected_at, received_at, presence
FROM raw_events_old;

-- The two children, rebuilt only to point `raw_event_id` back at the new `raw_events`.

CREATE TABLE interpreted_events_new (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id         TEXT NOT NULL REFERENCES sessions(id),
    athlete_id         TEXT NOT NULL,
    raw_event_id       INTEGER REFERENCES raw_events(id),
    kind               TEXT NOT NULL
        CHECK (kind IN ('ENTERED', 'EXITED', 'STARTED', 'PASSED', 'CROSSED',
                        'VISIT_WITHDRAWN', 'EXIT_WITHDRAWN', 'EXCEPTION')),
    station            TEXT,
    detected_at        INTEGER NOT NULL,
    transition_ms      INTEGER,
    started_timing     INTEGER NOT NULL DEFAULT 0,
    exception_reason   TEXT,
    voided_at          INTEGER,
    voided_by          TEXT,
    void_reason        TEXT,
    acknowledged_at    INTEGER,
    acknowledged_by    TEXT,
    acknowledge_reason TEXT
);
INSERT INTO interpreted_events_new
    (id, session_id, athlete_id, raw_event_id, kind, station, detected_at, transition_ms,
     started_timing, exception_reason, voided_at, voided_by, void_reason,
     acknowledged_at, acknowledged_by, acknowledge_reason)
SELECT id, session_id, athlete_id, raw_event_id, kind, station, detected_at, transition_ms,
       started_timing, exception_reason, voided_at, voided_by, void_reason,
       acknowledged_at, acknowledged_by, acknowledge_reason
FROM interpreted_events;
DROP TABLE interpreted_events;
ALTER TABLE interpreted_events_new RENAME TO interpreted_events;

CREATE INDEX idx_interpreted_replay
    ON interpreted_events (session_id, detected_at, id);
CREATE INDEX idx_interpreted_by_raw_event
    ON interpreted_events (raw_event_id);

CREATE TABLE raw_interpretation_context_new (
    raw_event_id        INTEGER PRIMARY KEY REFERENCES raw_events(id),
    session_id          TEXT NOT NULL,
    athlete_id          TEXT,
    bound_session_id    TEXT,
    on_roster           INTEGER NOT NULL,
    reader_station      TEXT,
    reader_mode         TEXT,
    planned_json        TEXT,
    reader_min_visit_ms INTEGER
);
INSERT INTO raw_interpretation_context_new
    (raw_event_id, session_id, athlete_id, bound_session_id, on_roster, reader_station,
     reader_mode, planned_json, reader_min_visit_ms)
SELECT raw_event_id, session_id, athlete_id, bound_session_id, on_roster, reader_station,
       reader_mode, planned_json, reader_min_visit_ms
FROM raw_interpretation_context;
DROP TABLE raw_interpretation_context;
ALTER TABLE raw_interpretation_context_new RENAME TO raw_interpretation_context;

CREATE INDEX idx_raw_context_pending
    ON raw_interpretation_context (session_id, raw_event_id);

-- Nothing references the old table any more.
DROP TABLE raw_events_old;

-- Created last, and without IF NOT EXISTS, on purpose. 0007 wrote
-- `CREATE INDEX IF NOT EXISTS idx_raw_by_tag` while the old table still held an index of
-- that name, so the statement did nothing and the index then went away with the old table:
-- every database since has looked tags up by scanning. Here the name is free again.
CREATE INDEX idx_raw_by_tag ON raw_events (tag_id, detected_at, id);
