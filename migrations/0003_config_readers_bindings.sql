-- Configuration, reader map and binding ledger (CLAUDE.md 8, 19, 21; ADR 0004).
--
-- Additive: nothing in 0001 or 0002 changes, and raw_events is untouched.
--
-- Before this, a restart rebuilt all three from whatever the caller supplied at startup, so
-- a resumed class could silently adopt a different course or a different finish rule from
-- the one it was armed under.

-- The course and the policies a session was armed with. One JSON document rather than a
-- table per part: the course is a nested, ordered, repeatable structure whose shape is the
-- domain's business (CLAUDE.md 9.2), the hub reads it whole and writes it whole, and no
-- query needs to reach inside it. Normalising it would buy joins nobody performs.
CREATE TABLE IF NOT EXISTS session_configs (
    session_id  TEXT PRIMARY KEY REFERENCES sessions(id),
    config_json TEXT NOT NULL
);

-- (device_id, reader_id) -> station / zone / mode (CLAUDE.md 8). Venue configuration, not
-- session data: the readers on the wall outlive any one class, so this is not scoped by
-- session. A reader absent from here is an UNKNOWN_READER exception, before and after a
-- restart alike.
CREATE TABLE IF NOT EXISTS readers (
    device_id TEXT NOT NULL,
    reader_id TEXT NOT NULL,
    station   TEXT NOT NULL,
    zone      TEXT,
    mode      TEXT NOT NULL
        CHECK (mode IN ('ENTRY', 'EXIT', 'TOGGLE', 'CHECKPOINT', 'PASSAGE')),
    PRIMARY KEY (device_id, reader_id)
);

-- Append-only, like the domain ledger it mirrors (CLAUDE.md 7.2, 20; ADR 0001 D3). Ending a
-- binding stamps unbound_at; who held the tag is never rewritten, because a correction has
-- to be able to answer "who was wearing this band at 10:15" afterwards. bound_at is part of
-- the key so the same band can come back to the same athlete later and still be two rows.
--
-- No foreign key to sessions: tag uniqueness is checked across every session (one band, one
-- wrist), so the ledger legitimately holds rows for classes this hub did not run.
CREATE TABLE IF NOT EXISTS tag_bindings (
    session_id TEXT NOT NULL,
    tag_id     TEXT NOT NULL,
    athlete_id TEXT NOT NULL,
    bound_at   INTEGER NOT NULL,
    unbound_at INTEGER,
    PRIMARY KEY (session_id, tag_id, bound_at)
);

CREATE INDEX IF NOT EXISTS idx_bindings_active ON tag_bindings (tag_id, unbound_at);

-- Retroactive claim reads raw_events by tag and time (ADR 0001 D3), and the check-in queue
-- is derived the same way.
CREATE INDEX IF NOT EXISTS idx_raw_by_tag ON raw_events (tag_id, detected_at, id);
