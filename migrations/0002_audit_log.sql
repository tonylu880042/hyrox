-- Operator audit trail (CLAUDE.md 20; ADR 0001 D1).
--
-- Additive: nothing in 0001 changes. Session reopen, manual class end and tag binding all
-- land here, so "who changed what, when, and why" survives the session that produced it.
-- `operator` is a device name today (D1 accepted device-level traceability); the column can
-- carry a person later without a shape change.
CREATE TABLE IF NOT EXISTS audit_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    at           INTEGER NOT NULL,          -- epoch ms, same clock as detected_at
    operator     TEXT NOT NULL,
    action       TEXT NOT NULL,             -- SESSION_ARM, SESSION_REOPEN, TAG_BIND, ...
    subject      TEXT NOT NULL,             -- session id, athlete id, tag id
    reason       TEXT,                      -- required by the use case for destructive actions
    before_state TEXT,
    after_state  TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_subject ON audit_log (subject, at);
