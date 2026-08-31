# ADR 0010 — Walk-in entrants, and ranking that follows the finish rule

**Status:** accepted, 2026-08-29
**Amends:** ADR 0001 D3 (the narrow write surface), and the "no ranking" note in `docs/api.md`

## Context

Competitions take entries from people the gym has never seen. Until now there was no way to
put anybody on a roster at all through the API: `admit` required a `MemberRef` from 健身管,
whose contract is unknown and whose only implementation reports that it is not configured.
The roster came from the start-up plan and nothing could change it.

So a non-member could not enter, and neither could a member — the door had no verb.

## Decision

### 1. An athlete is a name and a bib; a member reference is provenance

`Entrant { member_id: Option<String>, display_name, bib: Option<i64> }`.

`athlete_id` identifies the person. When 健身管 knows them, their member id *is* the athlete
id, so a member keeps one identity across every class they ever enter. A walk-in gets an id
scoped to the session they exist in.

`session_athletes` gains one nullable `member_id` column (migration 0005). **Its absence is
the record**, not a gap: "who were the walk-ins" is the first question a venue asks after
running an open event, and without the column it is unanswerable.

Membership status is still never checked. Confirmed 2026-08-27: if 健身管 returns the member
they may be timed, and an expired membership must not stop somebody's clock.

### 2. Bibs are assigned, not counted

Competition bibs are printed in advance, so the door has to be able to name one; a unique
index makes two vests with the same number impossible within a session.

This turned out to be a **correctness fix, not just a feature**. Every read model derived a
bib from the roster's *position* (`enumerate() + 1`). The moment the door could assign 7 to
the second entrant, the check-in list, the live screen and the leaderboard all showed them
as 2. Those now read the assigned bib; results for a session the hub is no longer running
read it back through a new `athlete_bibs` port method, because a bib is a roster fact and is
not replayed from the event log.

### 3. The check-in surface gains one verb, and only one

ADR 0001 D3 made `/checkin` able to bind a band "and nothing else", so a tablet handed to a
helper could never touch timing. Entering somebody is what checking in *is*, and the
alternative was handing the door an operator tablet that can also stop the clock. So
`POST /api/checkin/entrants` joins bind and rebind.

The surface still has no method that reaches the session's clock, and a test sweeps for it.

### 4. Ranking follows the finish rule, and only that rule

`docs/api.md` said results carry no ranking because "the competition finish rule is
undecided". **That is now out of date**: `FinishPolicy::CourseComplete` was settled on
2026-08-28 — finishing is completing the course, timed at the exit of the last station. So
"who finished first" has an answer, and competitors are placed by it.

Under every other rule they are not:

* **`ClassDuration`** stops everyone at the same moment having done different amounts of
  work. Ordering by elapsed time would rank people who did different things.
* **`CoachDecides`**, **`NotConfigured`** — nobody has finished anything.

Those come back in bib order with `place: null`. The payload names its own ordering
(`BIB` / `FINISH_TIME`) so no client mistakes row order for a placing, and the leaderboard
screen says so on the screen rather than showing bib order as though it were a result.

Ties share a place and the next place skips, which is ordinary competition ranking.
`place: null` on a competitor who has not finished is **not last** — they have not finished.

### 5. Three screens

| Path | For |
| --- | --- |
| `/checkin` | the door: enter people, bind bands, see which bands are waiting |
| `/leaderboard` | the projector during a race: placings, progress, finish times |
| `/result` | afterwards: per-athlete splits, work and transition times |

`/leaderboard` reads the live session directly rather than the store — the roster is already
in memory, and a store round trip per poll would answer 404 for a session plainly on the
floor.

## Consequences

* `admit` is gone, replaced by `enter`. Nothing called it in production; it could not be
  reached without a 健身管 client.
* A walk-in's athlete id is session-scoped, so the same person entering two competitions is
  two athletes. Correct today — the hub has no identity for them beyond a name — and the
  place where a "guest profile" would attach later if one is ever wanted.
* `docs/api.md`'s "deliberate omissions" section loses its ranking entry.
