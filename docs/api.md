# REST / WebSocket API

The hub's local web service (CLAUDE.md 22). It binds the venue LAN only; there is no
authentication, because ADR 0001 D1 makes the network boundary the deployment layer's job
and the device name the audit identity.

Implemented in `crates/api`. Design rationale: `docs/decisions/0007-http-api-surface.md`.

---

## 1. The surfaces

ADR 0001 cuts the screens into one write surface, one narrow write surface, and three
read-only ones. Every route below belongs to exactly one of them, and the split is
structural — see §5.

| Surface | Writes? | Path space |
| --- | --- | --- |
| `/operator` | yes | `/api/operator/**` |
| `/checkin` | binding only | `/api/checkin/**` |
| `/coach` | no | `/api/coach` |
| `/live` | no | `/api/live`, `/ws` |
| `/result/{id}` | no | `/api/result/{session_id}` |
| (shared read) | no | `/api/session` |

---

## 2. Endpoints

**Writes** are marked ✎. Every ✎ route requires the `x-operator-device` header (§3); the
ones marked ✎! additionally require a `reason` in the body, and answer `422 REASON_REQUIRED`
without one.

### Read-only

| Method | Path | Writes | Requires | Answers |
| --- | --- | --- | --- | --- |
| `GET` | `/api/live` | — | — | `{ freshness, snapshot }` — the big screen |
| `GET` | `/api/coach` | — | — | `{ freshness, snapshot, readers }` — CLAUDE.md 23 |
| `GET` | `/api/session` | — | — | `{ freshness, session, config, class_elapsed_ms, config_editable }` |
| `GET` | `/api/result/{session_id}` | — | — | `{ freshness, results }`; `404 UNKNOWN_SESSION` |
| `GET` | `/ws` | — | — | WebSocket; a `Snapshot` document per frame |

### Check-in — the narrow write surface

| Method | Path | Writes | Requires | Answers |
| --- | --- | --- | --- | --- |
| `GET` | `/api/checkin` | — | — | `{ freshness, pending, athletes }` |
| `POST` | `/api/checkin/bind` | ✎ | `tag_id`, `athlete_id` | `{ freshness, claimed }` |
| `POST` | `/api/checkin/rebind` | ✎! | `tag_id`, `athlete_id`, `reason` | `{ freshness, claimed }` |

`claimed` lists the interpretations produced by replaying reads that happened before anyone
owned the band (ADR 0001 D3). Empty is the ordinary case.

### Operator — the write surface

| Method | Path | Writes | Requires | Answers |
| --- | --- | --- | --- | --- |
| `GET` | `/api/operator` | — | — | session, config, readers, devices, both badges |
| `GET` | `/api/operator/readers` | — | — | `{ freshness, readers }` |
| `POST` | `/api/operator/readers` | ✎ | `device_id`, `reader_id`, `station`, `mode`; optional `zone` | `{ freshness, readers }` |
| `PUT` | `/api/operator/config` | ✎ | `finish_policy`; optional `course` | the new session view |
| `GET` | `/api/operator/exceptions` | — | — | `{ freshness, exceptions }` |
| `POST` | `/api/operator/exceptions/{id}/void` | ✎! | `reason` | the remaining inbox |
| `POST` | `/api/operator/session/arm` | ✎ | — | the new session view |
| `POST` | `/api/operator/session/close` | ✎ | — | the new session view |
| `POST` | `/api/operator/session/reopen` | ✎! | `reason` | the new session view |
| `POST` | `/api/operator/session/draft` | ✎ | — | the new session view |
| `POST` | `/api/operator/session/end-class` | ✎ | — | `{ freshness, finished }` |

### Served by the app, not by `crates/api`

| Method | Path | Writes | Answers |
| --- | --- | --- | --- |
| `GET` | `/` | — | `307` to `/live` |
| `GET` | `/live` | — | the live screen's HTML |

---

## 3. Operator identity (ADR 0001 D1)

There is no login. Each tablet takes a name the first time it opens `/operator` or
`/checkin` — "櫃檯平板", "FRONT DESK TABLET" — keeps it in local storage, and sends it on
every write:

```
x-operator-device: FRONT DESK TABLET
```

That name becomes the `operator` column of the audit trail (CLAUDE.md 20).

**A write with no name, or a blank one, is refused** with `400 OPERATOR_REQUIRED`. It is
never defaulted to an empty string: an audit row naming nobody looks like a record of who
did something and is not one.

Traceability is to a device, not a person. That is D1's deliberate trade for zero friction;
if personal traceability is ever needed, this header becomes an identity without the audit
shape changing.

### Reasons

Destructive actions carry a `reason` in the JSON body. D1 expects the UI to offer quick
reason keys — 誤刷 / 漏刷 / 設備異常 / 其他 — rather than force typing. Whitespace does not
count as a reason.

### Bodies

Every write takes a JSON body. An empty body is read as `{}`, so a `POST` with nothing to
say need not send one. A genuinely required field is still required and its absence is
`400 INVALID_BODY` naming the field — `PUT /api/operator/config` requires `finish_policy`
for exactly this reason: letting it fall to its `Default` would quietly remove a class's
finish rule.

---

## 4. Data freshness (ADR 0001 D5)

**Every** read response embeds a `freshness` object:

```json
{
  "now": 1787940198102,
  "last_event_age_ms": 3984,
  "websocket_path": "/ws",
  "push_interval_ms": 250,
  "subscribers": 2
}
```

| Field | Means |
| --- | --- |
| `now` | the hub's clock; a client that disagrees can see that it does |
| `last_event_age_ms` | age of the newest interpreted event. **`null` is not zero** — it means no event exists yet, and must not be drawn as fresh |
| `websocket_path` | where the live socket is |
| `push_interval_ms` | how often the hub pushes; a screen that has waited much longer has a dead link, without inventing a timeout |
| `subscribers` | how many sockets the hub is pushing to — the server's half of the liveness question |

`/api/coach` and `/api/operator` additionally carry per-reader freshness:
`readers[].last_seen_age_ms`, again `null` for a board the hub has not heard from since it
started. `/api/operator` also carries `devices[]` with each board's own journal warning
(CLAUDE.md 18).

This is safety-critical, not decoration: CLAUDE.md 31's first principle is that no event is
lost, and without the readout a frozen screen and an empty gym are the same picture. The
live screen must show `DISCONNECTED` / `LINK DOWN` when the socket drops (ADR 0001,
2026-08-27 addendum).

---

## 5. How the read/write split is enforced

Three mechanisms, none of which requires reading a handler body.

1. **The state type.** Each router is built with one capability type as its axum state:
   `ReadOnly<S>`, `CheckIn<S>` or `Operator<S>` (`crates/api/src/state.rs`). They hold the
   store and the live session in private fields and expose no accessor for either.
   `ReadOnly` has no method that writes; `CheckIn` has exactly two, `bind` and `rebind`. A
   handler declares its capability in its own signature (`State<ReadOnly<S>>`), so a write
   attempted from `read.rs` is a compile error.
2. **The routing verbs.** `crates/api/src/read.rs` imports only `axum::routing::get`;
   `post`, `put` and `delete` are not in scope in that module.
3. **The path space.** Mutating routes exist only under `/api/operator` and `/api/checkin`.
   A `POST` to a read-only path is a `405` from axum's own method router, before any of our
   code runs — covered by a test that sweeps every read-only path against every write verb.

---

## 6. Status codes

A domain invariant saying no is an answer, not a fault. Only a failed store write is a 500.

| Status | `error` | When |
| --- | --- | --- |
| 400 | `OPERATOR_REQUIRED` | a write with no `x-operator-device` (D1) |
| 400 | `INVALID_BODY` | unparsable body, missing required field, bad tag id, bad reader key |
| 404 | `UNKNOWN_SESSION` | `/api/result/{id}` for a session the store does not hold |
| 404 | `UNKNOWN_ATHLETE` | binding to somebody off the roster |
| 404 | `UNKNOWN_EVENT` | voiding an interpreted event id that does not exist |
| 409 | `ILLEGAL_TRANSITION` | e.g. closing a DRAFT session (ADR 0001 D2) |
| 409 | `HAS_INTERPRETED_EVENTS` | ARMED → DRAFT after something was interpreted (D2) |
| 409 | `SESSION_NOT_EDITABLE` | editing configuration outside DRAFT (D2) |
| 409 | `NO_FINISH_RULE` | ending a class by hand where no finish rule is configured |
| 409 | `TAG_ALREADY_BOUND` | that band is on someone's wrist (D3) |
| 409 | `ATHLETE_ALREADY_BOUND` | that athlete already has a band; rebind to swap |
| 409 | `NOT_BOUND` | there is no binding to change |
| 422 | `REASON_REQUIRED` | a destructive action with no reason (CLAUDE.md 20) |
| 500 | `STORAGE_FAILED` | the store rejected the write |

Body shape:

```json
{ "error": "REASON_REQUIRED", "message": "this action changes recorded data, so it needs a reason (CLAUDE.md 20)" }
```

Branch on `error`. `message` is for whoever reads a log.

---

## 7. Deliberate omissions

These are not oversights. Each of them would require inventing a product rule (CLAUDE.md 28).

* **No ranking on `/api/result/{id}`.** Rows come back in bib order and the payload says so
  (`ordering: "BIB"`). Ranking needs an ordering, every ordering asks who finished first,
  and the competition finish rule is undecided (CLAUDE.md 12). The training rule that *is*
  decided expects most athletes not to complete the course, so ordering by elapsed time
  would rank people who did different amounts of work. `finish_policy` is published so a
  reader can see what "finished" meant here — including that it meant nothing.
* **No reader deletion.** No use case removes a reader, and removal would mean deciding
  what becomes of the events already attributed through it. Re-registering the same
  `(device_id, reader_id)` replaces its mapping, which is what repointing a reader is.
* **No "accept as-is" or "reinterpret" on an exception.** D4 names three actions and only
  `void` has a use case. See `docs/open-issues.md`.
* **No roster editing.** Admitting an athlete needs a `MemberRef` from 健身管, whose contract
  is unknown; `application::UnconfiguredDirectory` is the only implementation that can
  honestly ship. The dev roster comes from the startup plan.
* **No `operator_identity`.** D1 chose device-level traceability on purpose.
