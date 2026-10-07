-- Recovery and retroactive claim both ask whether a raw row already has an interpretation.
-- Corrections may append another interpretation for the same raw row, so this must remain a
-- non-unique index.
CREATE INDEX IF NOT EXISTS idx_interpreted_by_raw_event
    ON interpreted_events (raw_event_id);
