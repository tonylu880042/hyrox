-- Keep the session the hub currently owns explicit rather than inferring it from status.
-- A READY session can remain in history when a new DRAFT class replaces it, and status
-- ordering would resurrect the wrong class after restart (F07).
CREATE TABLE IF NOT EXISTS active_session (
    singleton  INTEGER PRIMARY KEY CHECK (singleton = 1),
    session_id TEXT NOT NULL REFERENCES sessions(id),
    updated_at INTEGER NOT NULL
);

-- Databases created before this marker have no pointer. Seed the most recent resumable
-- session once; every subsequent session write maintains the pointer transactionally.
INSERT INTO active_session (singleton, session_id, updated_at)
SELECT 1, id, created_at
FROM sessions
WHERE status IN ('DRAFT', 'READY', 'RUNNING', 'PAUSED')
ORDER BY created_at DESC
LIMIT 1;
