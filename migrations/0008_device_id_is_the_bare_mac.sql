-- `device_id` drops the `esp32-` prefix (ADR 0015).
--
-- The prefix decorated the id without identifying anything: nothing else in this system is
-- twelve hex digits, and the venue's readers are not necessarily ESP32 boards, so the
-- prefix was a claim about hardware that the field could not keep. What actually earns its
-- keep is *normalisation* -- one lower-case, separator-free spelling per MAC -- and that
-- stays exactly as it was.
--
-- Data-only. No table changes shape, and no row is added or removed: the stored ids are
-- rewritten in place so a database written by a prefixed build keeps working. `raw_events`
-- stays immutable in the sense CLAUDE.md 19 means -- this renames the device, it does not
-- reinterpret a read.
--
-- `LIKE 'esp32-%'` guards it: a database already written by an unprefixed build has no
-- matching rows, so re-running this changes nothing.
UPDATE raw_events SET device_id = substr(device_id, 7) WHERE device_id LIKE 'esp32-%';

-- Same rewrite on the reader map. If it were skipped, every reader would keep its old key
-- and every read from the wall would file as UNKNOWN_READER -- silently, at the venue.
UPDATE readers SET device_id = substr(device_id, 7) WHERE device_id LIKE 'esp32-%';
