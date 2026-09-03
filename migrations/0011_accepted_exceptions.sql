-- Accepting an exception as it stands (ADR 0001 D4, second action).
--
-- The inbox holds reads the hub could not turn into progress. Some of them need nothing
-- doing: a band that brushed an antenna twice, a read from a reader that was being moved.
-- Until now the only button was `void`, which takes the interpretation out of every replay
-- -- the destructive action, applied to a row that is a perfectly true record of what the
-- antenna saw. So operators either erased honest data or let the badge climb until they
-- stopped reading it, and an inbox nobody reads is where the next real exception goes to
-- die (CLAUDE.md 31 principle 6).
--
-- Acknowledging changes nothing that replays. The row keeps its place in the log and its
-- effect on athlete state (which, for an exception, is none); what changes is whether it is
-- still somebody's outstanding work.
--
-- Nullable and never set for anything but an exception. `acknowledged_by` is the operator's
-- device name, the same identity every other write carries (ADR 0001 D1).
ALTER TABLE interpreted_events ADD COLUMN acknowledged_at INTEGER;
ALTER TABLE interpreted_events ADD COLUMN acknowledged_by TEXT;
ALTER TABLE interpreted_events ADD COLUMN acknowledge_reason TEXT;
