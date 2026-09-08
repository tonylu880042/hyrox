-- Distinguish a policy-derived roster finish from an explicit coach end.
--
-- CourseComplete is replay-derived from the final exit and must disappear when that exit is
-- voided. A manual end-class decision is different: it is an operator decision and remains
-- part of the session's result. Existing rows are left NULL so the storage adapter can use
-- the event log to recognise the old CourseComplete shape while preserving other finishes.
ALTER TABLE session_athletes ADD COLUMN finish_origin TEXT
    CHECK (finish_origin IS NULL OR finish_origin IN ('POLICY', 'MANUAL'));
