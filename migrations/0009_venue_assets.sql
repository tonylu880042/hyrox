-- The venue's own images (M6 follow-up). Today that is one logo.
--
-- In the database rather than on disk, deliberately:
--
--   * the nightly backup already copies the database (ADR 0012), so a restored machine
--     comes back with the gym's logo on the wall instead of a blank corner;
--   * no directory to create, no permissions to get wrong, no orphaned file to clean up;
--   * the hub is the only process that touches the database, which is the rule the whole
--     appliance is built on (ADR 0009).
--
-- A logo is a few tens of kilobytes. SQLite reads a small BLOB as fast as the filesystem
-- does, and the size is capped above this layer.
CREATE TABLE IF NOT EXISTS venue_assets (
    key        TEXT PRIMARY KEY,
    -- The type it will be served back as. Stored rather than sniffed at read time: what a
    -- browser is told a file is decides how it treats it.
    media_type TEXT NOT NULL,
    bytes      BLOB NOT NULL,
    updated_at INTEGER NOT NULL,
    updated_by TEXT NOT NULL
);
