-- Preserve the decision context captured with a raw read (CLAUDE.md 15, 16, 21).
--
-- A raw row earns the edge ACK before its interpretation is written. If the hub stops in that
-- gap, retrying against today's reader map or today's tag binding could move the historical
-- read to a different station or athlete. The context is written in the same transaction as
-- the raw row and stores the already-decided event as JSON. NULL planned_json means the read
-- was intentionally left for check-in (or was malformed), not that recovery may guess.
CREATE TABLE IF NOT EXISTS raw_interpretation_context (
    raw_event_id       INTEGER PRIMARY KEY REFERENCES raw_events(id),
    session_id         TEXT NOT NULL,
    athlete_id         TEXT,
    bound_session_id   TEXT,
    on_roster          INTEGER NOT NULL,
    reader_station     TEXT,
    reader_mode        TEXT,
    planned_json       TEXT
);

CREATE INDEX IF NOT EXISTS idx_raw_context_pending
    ON raw_interpretation_context (session_id, raw_event_id);
