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

## OPEN — should an undecodable payload be quarantined in the database?

Decided for now (ADR 0006): a payload that arrives on one of our topics and does not decode
is **counted and logged** — topic, decode error, and a bounded excerpt of the bytes — and is
neither stored nor acknowledged. Not stored because `raw_events` is keyed by
`device_id + boot_id + sequence` and an undecodable payload has no such key; writing a row
would mean inventing one. Not acknowledged because nothing was made durable, so under
ADR 0002 there is nothing to acknowledge with, and the edge keeps its copy.

A quarantine table would be more durable than a log line. It needs two things this project
does not have: a retention rule for it, and a reason to believe a device will ever produce
one. It also hands a faulty device a way to fill the disk.

**Needs:** evidence from the venue. If undecodable payloads ever appear in the field, the
full bytes are already in hand at `Inbound::Undecodable` and the change is one match arm plus
a migration. Until then, do not build the table.

---

## OPEN — Exception Inbox 的「accept as-is」與「改判」

ADR 0001 D4 列了三個處理動作：accept as-is / void / 改判（改站點、改 ENTRY/EXIT、改選手）。
只有 `void` 有 use case（`application::exceptions::void`），因此 `crates/api` 也只有
`POST /api/operator/exceptions/{id}/void`。另外兩個是刻意缺席，不是遺漏。

**accept as-is** 需要一個地方記下「有人看過這筆例外，決定不動它」。`interpreted_events`
沒有這樣的欄位，而加一個欄位就要先回答：被 accept 的例外還算不算 D4 的 badge？
重算時它如何參與？兩者都是產品規則。

**改判**等於由 operator 產生一筆事件（不同站點、不同 ENTRY/EXIT、不同選手）。
CLAUDE.md 20 明文允許，但它是一條獨立的寫入路徑：`raw_event_id` 為 `None` 的
interpreted event，且必須完整走過 audit 與衍生值重算。它會需要自己的 use case
與自己的測試。

**Needs:** 先確定 accept as-is 之後那筆例外在 badge 與重算裡的身分。在那之前，
現場的替代路徑是 void 加上人工紀錄——不理想，但不會產生一個沒人定義的狀態。

---

## ANSWERED 2026-08-28 — competition finish rule

CLAUDE.md 12. Training was answered (below); competition was not.

`FinishPolicy::NotConfigured` is the default and evaluates to `Undetermined`, never to
`NotFinished`, so no caller can mistake an undecided rule for a decided negative. No code
path sets `AthleteStatus::Finished` from a competition rule: `application::apply_finish_policy`
ignores `Undetermined`, and `application::end_class` refuses to run at all while the policy
is `NotConfigured`, so no operator button can invent the rule either (ADR 0003).

Questions still unanswered are listed in `docs/timing-rules.md`.

Finishing is completing the configured course, and the exit of the final station is the
result's recording point. Implemented as `FinishPolicy::CourseComplete`, which reads the
finish instant off that exit rather than off the tick that noticed it.

Race formats are courses, not rules: a half format is a shorter `Course`, so adding a format
must never mean adding a `FinishPolicy` variant.

**Still open:** whether a dedicated finish reader replaces the last station's exit later. The
current rule needs no extra hardware, and a `FinishReader` variant could coexist if a venue
installs one.

---

## FIRMWARE TEAM — is `reader_id` really a MAC address?

The user stated on 2026-08-27 that `reader_id` is a MAC address. That conflicts with
CLAUDE.md 7.3 and 8, where `device_id` is the MAC-derived ESP32 identity and `reader_id` is
deliberately separate so one ESP32 can host several readers later.

If each reader carries its own MAC, the "one collector, many readers" model needs revisiting.
The casing half of the question is settled (below); this half is not.

**Not blocking the hub** today, but it fixes the wire contract. Both questions on this page
are in `docs/event-protocol.md` section 7, which is the handoff sheet for that conversation.

**2026-08-28:** MQTT ingestion has landed (ADR 0006) without this being settled. Nothing was
guessed — the hub treats `reader_id` exactly as the contract already documented, as an
identifier separate from `device_id`. If each reader turns out to carry its own MAC, what
changes is the reader map and the emulated device's shape, not the wire format.

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
unknown, so the client sits behind `application::MemberDirectory` with `UnconfiguredDirectory`
as the only implementation until the real contract arrives (ADR 0003) -- it reports
`NotConfigured` rather than guessing a URL. Age is stored as reported, not as a birth date,
so it goes stale.

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


## OPEN — doubles and relay: bands per team, and the missing team concept

The user asked what the official HYROX regulation is for bands per doubles team. That needs
checking against the current rulebook; it is not recorded here because it is not yet known.

Independently of that answer, doubles makes the TEAM the timing subject, and the domain has
no team concept: `AthleteState` is per person and a binding is one band to one person.

Two bands also buy less than they appear to. Partners split repetitions inside a station,
which is exactly where RFID sees nothing (the hub knows which zone someone is in, never what
they are doing), so a second band cannot tell us who did what work.

Recommendation on file: two bands, because each partner is a member in their own right with
their own binding and member record, but timing and finishing at the team level.

**Sized as a Milestone 1 extension**, not a finish-policy variant. Relay is structurally the
same problem, so a team model should cover both.

**Blocks:** doubles and relay formats. Does NOT block singles competition.

---

## Workout templates (ADR 0008, 2026-08-29)

* **AMRAP and ZONE_ROTATION have no execution model.** Both are storable as blocks and both
  are refused at compile time by name. How many rounds an athlete gets in an AMRAP is the
  *result*, not the plan, so there is no honest flat course to time it against. Deciding what
  "finished" means for one is a product question.
* **Nothing consumes `Expectation` yet.** EXPECTED / OUT_OF_ORDER / UNEXPECTED is derived and
  published on `/api/stages`; no rule acts on it. It stays that way until the competition
  exception rules are decided (CLAUDE.md 9.1, 28).
* **One active class at a time.** Creating a class while one is RUNNING or PAUSED is refused.
  Scheduling several classes in a day would need a session list and a way to choose which is
  live; nothing has asked for it.
* **Time and calorie targets are labels, not measurements.** The hub learns entry and exit
  from RFID and nothing about what happened on the machine. Verifying them needs the sensor
  adapters (PM5, OCR, manual judge) that Phase 1 deliberately does not implement.
* **`/live` and `/workout` load Tailwind and Google Fonts from a CDN.** With no internet the
  pages still load and the data is still correct, but they render unstyled — which is at odds
  with CLAUDE.md 31's "useful when the Internet is unavailable". Pre-existing on `/live`;
  `/workout` repeats it on the user's instruction to keep the two screens visually identical.
  The fix is to vendor the generated CSS and the four font files and serve them locally.

---

## 多國語系（介面層已於 2026-08-29 完成）

繁體中文、簡體中文、英文。實作與決策見 `docs/roadmap.md` M7。**已決定**：語系跟著裝置走
（`localStorage`，與 `x-operator-device` 同機制），`?lang=` 可覆寫；大螢幕沒有切換鈕，用
`/live?lang=…` 釘住。仍未決的產品問題只剩：

* **四份 system preset 課表的名稱與說明是否中文化。** 它們是資料庫裡的資料列，不是介面文字，
  而且教練可以複製與改名。seed 時寫中文會漏掉既有安裝；用 preset id 對照字典則複製出來的
  副本對不上。需要產品決定。
* **時間與數字格式是否在地化。** 目前全系統統一 `mm:ss` 與半形數字；改動會影響成績呈現，
  屬於成績呈現規則而非介面翻譯。

已確認**不在範圍內**：測試資料與開發 fixture 維持原本的中文（`誤觸`、`櫃檯平板`、
`腳環故障` 等）。那些是使用者輸入的內容，不是介面文案，而且正是「非 ASCII 能存能取」的
證據 —— 參數化會讓測試不再驗證那件事。

已確認**不翻譯**的部分（屬契約，非顯示文字）：`station_key`、`Exercise.code`、所有 enum 的
線路值（`SessionStatus`、`ReaderMode`、`TargetType`、`Unit`、`TemplateCategory`）、
`ErrorBody.error` 機器碼。翻譯屬顯示層。

