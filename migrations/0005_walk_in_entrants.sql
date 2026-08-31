-- Walk-in entrants (ADR 0010).
--
-- A competition takes entries from people the gym has never seen. Until now the only way
-- onto a roster was `admit`, which required a `MemberRef` from 健身管 -- so a non-member
-- could not be entered at all, and the roster was fixed at start-up.
--
-- Additive. One nullable column: an athlete is identified by `athlete_id`, and the member
-- reference becomes provenance rather than a precondition. NULL means a walk-in, which is
-- the question a venue asks first after running an open event.
--
-- SQLite's ALTER TABLE ADD COLUMN rewrites no rows and touches no other table.
ALTER TABLE session_athletes ADD COLUMN member_id TEXT;

-- Bibs are handed out on paper at the door, so two entrants must not be able to share one
-- inside a session. Not a primary key: `athlete_id` still identifies the person, and a bib
-- is a label on a vest that a correction may reassign.
CREATE UNIQUE INDEX IF NOT EXISTS idx_athlete_bib ON session_athletes (session_id, bib);
