# Open issues

Unresolved product rules (CLAUDE.md 28). Nothing here may be guessed in code: document the
assumption, isolate it behind configuration, and wait for a decision.

Answered items stay on this page with their answer and the date, so the reasoning survives.

---

## FIRMWARE TEAM — ESP32 journal retention contradicts the edge's ignorance of business meaning

**Not blocking the hub** (decided 2026-08-27). The hub never observes edge retention: its
contract is receive, commit, then ACK, and that is done and structurally enforced. Resend,
lost ACK, reboot and out-of-order are all covered by tests regardless of what the edge keeps.
Owned by the firmware team; carried here because CLAUDE.md 18 as written cannot be satisfied.


CLAUDE.md 18 says acknowledged events are retained for "current Session + previous Session,
or at least 24 hours after Session completion". CLAUDE.md 8 says the ESP32 must not know
business meaning — and a Session *is* business meaning. The edge cannot implement the rule
as written.

Retaining acknowledged events at all is disaster recovery: if the hub's database is lost or
a range of events is deleted in error, they can be pulled back from the edge.

Options, with the recommendation first:

| | Approach | Cost |
| --- | --- | --- |
| **A** | Time-based: keep acknowledged events for N hours, N configurable, default 48. The edge needs no business concept. | A multi-day event needs N raised. |
| **B** | Hub sends a downlink telling the edge it may reclaim up to a sequence number. | New protocol in both directions; the message is missed while the edge is offline. |
| **C** | Capacity-based only (what `crates/simulator` implements today). | Retention window is unpredictable on a busy day. |

**Needs:** a firmware-team decision, and an edit to CLAUDE.md 18 — the current wording asks
the edge to know what a Session is. `crates/simulator` should match whatever they choose, so
that it stays a faithful stand-in.

---

## OPEN — competition finish rule

CLAUDE.md 12. Training was answered (below); competition was not.

`FinishPolicy::NotConfigured` is the default and evaluates to `Undetermined`, never to
`NotFinished`, so no caller can mistake an undecided rule for a decided negative. No code
path sets `AthleteStatus::Finished` from a competition rule.

Questions still unanswered are listed in `docs/timing-rules.md`.

**Blocks:** the competition screen. Building it without this would mean inventing the rule.

---

## FIRMWARE TEAM — is `reader_id` really a MAC address?

The user stated on 2026-08-27 that `reader_id` is a MAC address. That conflicts with
CLAUDE.md 7.3 and 8, where `device_id` is the MAC-derived ESP32 identity and `reader_id` is
deliberately separate so one ESP32 can host several readers later.

If each reader carries its own MAC, the "one collector, many readers" model needs revisiting.
The casing half of the question is settled (below); this half is not.

**Not blocking the hub** today, but it fixes the wire contract, so settle it with the
firmware team before MQTT ingestion lands. Both questions on this page are in
`docs/event-protocol.md` section 7, which is the handoff sheet for that conversation.

---

## ANSWERED 2026-08-27 — training finish rule

A group class ends when its time is up. In a one-hour class most athletes will not have
completed all eight stations, and that is the normal outcome, not an error. A coach must
also be able to end a class by hand.

Implemented as `FinishPolicy::ClassDuration { limit }` and `FinishPolicy::CoachDecides`.
An athlete caught inside a station keeps that run open: no reader reported them leaving, and
inventing an exit time would fabricate a split nothing observed.

## ANSWERED 2026-08-27 — 健身管 contract shape

The hub calls 健身管, not the reverse. A member id is obtained from a QR code, then used to
fetch the member's basic profile: gender, age, photo, and optionally height and weight.

Modelled as optional fields on `MemberRef`. The exact endpoint, auth and payload are still
unknown, so the client belongs behind a port (Phase 2) with a stub until the real contract
arrives. Age is stored as reported, not as a birth date, so it goes stale.

## ANSWERED 2026-08-27 — membership validity does not gate timing

If 健身管 returns the member, they may be timed. `MembershipStatus` is carried for display
and is deliberately not enforced anywhere; the predicate that invited gating was removed.

## ANSWERED 2026-08-27 — identifier casing

Device and reader identifiers are case-insensitive on the wire and normalised to lowercase
on the way in. This resolves the contradiction between CLAUDE.md 8 (`RFID-02`) and
CLAUDE.md 16 (`rfid-02`): both now denote the same reader.

## ANSWERED 2026-08-27 — MQTT topic scheme

No prior convention exists, so the hub defines it: `hyrox/v1/...`, as documented in
`docs/event-protocol.md`. The firmware must match it.
