-- ADR 0016: the start/finish mat. Two CHECK constraints widen, and SQLite cannot alter a
-- CHECK, so both tables are rebuilt with every row and every id carried over unchanged
-- (the same move as migration 0007). Nothing references either table by foreign key.
--
--   interpreted_events.kind  + STARTED  the mat started an athlete's clock
--                            + PASSED   the route ran over the mat; recorded, not an exception
--   readers.mode             + START_FINISH

CREATE TABLE interpreted_events_new (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id         TEXT NOT NULL REFERENCES sessions(id),
    athlete_id         TEXT NOT NULL,
    raw_event_id       INTEGER REFERENCES raw_events(id),
    kind               TEXT NOT NULL
        CHECK (kind IN ('ENTERED', 'EXITED', 'STARTED', 'PASSED', 'EXCEPTION')),
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

CREATE INDEX IF NOT EXISTS idx_interpreted_replay
    ON interpreted_events (session_id, detected_at, id);
CREATE INDEX IF NOT EXISTS idx_interpreted_by_raw_event
    ON interpreted_events (raw_event_id);

CREATE TABLE readers_new (
    device_id TEXT NOT NULL,
    reader_id TEXT NOT NULL,
    station   TEXT NOT NULL,
    zone      TEXT,
    mode      TEXT NOT NULL
        CHECK (mode IN ('ENTRY', 'EXIT', 'TOGGLE', 'CHECKPOINT', 'PASSAGE', 'START_FINISH')),
    PRIMARY KEY (device_id, reader_id)
);
INSERT INTO readers_new (device_id, reader_id, station, zone, mode)
SELECT device_id, reader_id, station, zone, mode FROM readers;
DROP TABLE readers;
ALTER TABLE readers_new RENAME TO readers;
