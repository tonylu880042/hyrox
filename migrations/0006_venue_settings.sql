-- Venue settings: the few numbers a site adjusts for itself (M6 follow-up).
--
-- Not session configuration. `session_configs` describes one class and is snapshotted so a
-- later edit cannot rewrite what happened (ADR 0008); this describes the room, and outlives
-- every class in it -- like `readers` does.
--
-- Key/value because the list is short and the alternative is a migration per number. What
-- keeps it honest is that the application refuses a key it does not define: an unknown key
-- is a typo, and a stored typo is a setting somebody will swear they changed.
CREATE TABLE IF NOT EXISTS venue_settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    -- The operator device that changed it (ADR 0001 D1). The audit log holds the history;
    -- this is here so the settings screen can say who last touched a value without a join.
    updated_by TEXT NOT NULL
);
