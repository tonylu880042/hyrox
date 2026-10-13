# Open issues

Unresolved product rules (CLAUDE.md 28). Nothing here may be guessed in code: document the
assumption, isolate it behind configuration, and wait for a decision.

Answered items stay on this page with their answer and the date, so the reasoning survives.

---

## 韌體協議 v1.1 對照（2026-10-13）

對照韌體團隊的 `hub-protocol-v1.1.md` 與 `hub-followup.md`（韌體 0.10.0／0.11.0）。

**Hub 已配合（2026-10-13）**

* status 的 `warning` 是開放字串，未知值不再讓整則 status 被拒收；新增 `TIME_NOT_SYNCED`。
* Hub 每 2 秒發布 `hyrox/v1/hub/time`（QoS 0、不 retained）。
* 判重比對 `uptime_ms`，裝置被清除後重用的鍵存成新讀取（ADR 0018）。
* retained 的 status 不算「收到裝置訊息」；裝置靜默門檻改為配合 60 秒一則的 status。
* appliance 套件帶 mosquitto 區網 listener（匿名）、chrony（NTP）、avahi（`hyrox-hub.local`）。
  **這三項尚未在實體 appliance 上驗證**，接實機前要確認：另一台電腦連得上 1883、
  `ping hyrox-hub.local` 解析得到、裝置的 `/api/station` 顯示 `time_synced: true`。

**要回覆韌體團隊的更正**

* Hub 主機是 **Ubuntu Server appliance**（ADR 0009），開發機是 macOS，**不是 Windows PC**。
  他們文件的 2.1.1、7.3 與「Hub 主機檢查清單」不適用。
* 他們對照的是 2026-09-21／23 的規格，沒有 ADR 0016、0017。

**接受的偏離（韌體現況與 CLAUDE.md 不同）**

* journal 容量 **1024 筆**，不是 CLAUDE.md 18 的 10,000（裝置 RAM 放不下）。
  夠不夠取決於事件率與 Hub 最長離線時間——**待提供**：每場人數、讀頭數、每人讀取次數、
  Hub 最長可能離線多久。
* 一機一讀頭；一則事件最多 3 張卡，超過拆成多則（各自的 `sequence`）。Hub 不需要改。
* 整次開機都沒對到時的事件，裝置重開機後**丟棄**（那個 `boot_id` 完全沒有事件）。
  Hub 無從補救，只能確保時間來源一直都在。
* broker 匿名、無 TLS：韌體不支援帳密。邊界是場地網路；production networking／VLAN 設計仍未定（CLAUDE.md 28）。

**OPEN — 這版韌體是「登入站」，不是計時感應站**

按著按鍵才掃描、一輪一張卡、15 dBm；持續掃描與一輪多卡「日後再做」，
也沒有 `presence: DEPARTED`。所以無人值守的 ENTRY／EXIT 地墊與 ZONE 讀頭（ADR 0017）
目前沒有韌體可以配合。需要韌體團隊排入持續掃描與離開事件。

**OPEN — 同一張卡沒離開就再次上報**

韌體在裝置重開機、journal 滿後恢復、抑制表溢出時，會把還在場的卡再報一次（新的 `sequence`，
不是重送）；按鍵隔 7 秒以上再按也會。TOGGLE 讀頭會把它解讀成出站。CLAUDE.md 14 把業務層去重
交給 Hub，但 TOGGLE 目前沒有最短間隔保護（只有 ZONE 有 `min_visit`）。
需要決定：最短間隔是多少、是否每個讀頭可設。在決定之前不寫死任何數值。

**OPEN — 電量 topic `hyrox/v1/edge/<device_id>/battery`**

Hub 目前不訂閱。確定現場用電池供電後再加（`LOW`／`CRITICAL` 告警；`UNKNOWN` 不是低電量）。

**待答** 計時精度要求（各裝置時間差容許多少）。韌體實測 SNTP +40 ms、`hub/time` −8～−18 ms。

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

## FIRMWARE TEAM — 站區讀頭的「離開」事件（ADR 0017，2026-10-08）

主機端與模擬器已支援 `"presence": "DEPARTED"`（`docs/mqtt-uplink-format.md` §3.3）。
韌體需實作：缺席超過 `absent_timeout` 時送出，`detected_at` 為最後一次讀到的時間。
在韌體完成前，站區讀頭只能測到進站。

現場必須驗證：

* 15 秒防護的預設值（`readers.min_visit_ms`）是否適合每一站的器材遮擋情況。
* 相鄰站區讀取範圍不重疊、跑道不穿過站區範圍（RSSI 門檻、功率、天線角度）。
* 站區讀頭的 `absent_timeout`（建議 8–10 秒）。

---

## OPEN — 起終點感應墊與漏讀的邊界情況（ADR 0016，2026-10-08）

ADR 0016 已處理起跑、終點、中途路過與漏讀出站。以下兩件刻意不猜：

* **9S（拋藥球進站）也漏讀**：選手衝線時系統不知道他在拋藥球站內，墊子只會記成
  `PASSED`。路線會經過墊子，所以分辨不出「路過」與「衝線」。目前由操作人員改判處理；
  若現場常發生，可考慮「已完成前 N−1 站且離開第 N−1 站後經過墊子」的推定規則，
  但那是在推測選手做了什麼，需要產品決定。
* **漏讀進站**（3S 未讀到、直接讀到 3N）：維持 `IMPOSSIBLE_TRANSITION` 例外。
  是否比照做成「進站時間不明」待決。

---

## OPEN — 個人運動紀錄要放在雲端，不放在場館

**方向已定，設計未定**（使用者，2026-09-01）：使用者的運動紀錄未來從雲端取得，不從場館主機
取得，理由是應用端的管理比較單純。

這推翻了一個還沒動工的提案：先前規劃的「本機跨課程個人歷史」（`session_athletes.athlete_id`
加索引、逐場重算、`GET /api/athlete/{id}/history`）**不要做**。歷史查詢的家在雲端，
在場館再蓋一套等於兩份真相。

不會因為上雲而改變的事：

* **場館必須在沒有網路時照常運作**（CLAUDE.md 31）。所以切法是：
  **現場即時（今天這堂課）＝本機**，**歷史查詢＝雲端**。今天的課不能依賴雲端回應。
* 場館端仍然是紀錄的**產生者**與第一手保存者。雲端是複本與查詢介面，不是計時的權威。
* 備份（ADR 0012）不是同步機制。備份是為了還原這台機器，不是為了把資料送給別人；
  兩者的頻率、格式與失敗處理都不一樣。

需要決定的（不要在程式碼裡先猜）：

| 問題 | 為什麼難 |
| --- | --- |
| 上傳什麼粒度 | 原始刷卡／解讀事件／每場結果。愈原始愈能重算，也愈難去識別化 |
| 身分怎麼跨場館 | 場館內是報名編號或 member_id（ADR 0010、0011）。雲端要的是一個跨場館的人，兩者怎麼對上 |
| 修正怎麼同步 | 本機作廢一筆解讀會重算成績（ADR 0001 D4）。雲端要能收回已經送上去的成績 |
| 離線時的降級 | 沒網路時 `/signup` 的成績頁要顯示什麼？今天的課本機有，上個月的沒有 |
| 個資與保留 | 誰持有、保留多久、非會員（walk-in）的資料能不能上傳 |
| 誰先寫入 | 雲端是唯讀鏡像，還是也能被編輯？後者需要衝突解決，而我們現在完全沒有 |

**前置條件（已完成，2026-09-02）**：完成判定的持久化缺陷——`ClassDuration` 的完成只活在
記憶體，重啟後成績退回 `ACTIVE`——已修。完成時刻存在 `session_athletes.finished_at`
（migration 0010），重建時在重播之後套用，`/leaderboard` 與 `/result/{id}` 不再各說各話。
往雲端送一份「每堂團課都還在跑」的歷史，會比沒有歷史更糟，所以這一步先過了。

---

## OPEN — 相機掃碼需要 HTTPS，出貨形態目前是 http

`/checkin` 的相機掃碼用瀏覽器內建的 `BarcodeDetector`（ADR 0011），不需要任何函式庫，
但**瀏覽器只在安全內容下開放相機**。主機在場館區網上是 `http://<ip>:8730`，所以平板上
的相機掃碼不會動；畫面會說明並要求改用掃描槍或手動輸入六碼，這是可用的路徑。

要讓相機掃碼在場館可用，得先決定 TLS：

| | 做法 | 代價 |
| --- | --- | --- |
| **A** | 掃描槍當標準配備，不做 TLS | 每個報到處要一支槍（兩三千元）。目前的預設。 |
| **B** | 自簽憑證 | 每一台平板第一次都要手動信任，換機器就要再來一次。 |
| **C** | 真實網域 + 憑證 | 要有網域與更新機制，而機器可能整場沒有對外網路。 |

**Needs:** 一個出貨決定（ADR 0009 的延伸）。在那之前，掃描槍是官方答案。

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

## ANSWERED 2026-09-04 — Exception Inbox 的「改判」（void / accept as-is / reinterpret 全部完成）

ADR 0001 D4 列了三個處理動作：accept as-is / void / 改判（改站點、改 ENTRY/EXIT、改選手）。

* **accept as-is 已實作**（2026-09-03，`POST /api/operator/exceptions/{id}/accept`，migration 0011）。
* **改判（reinterpret）已實作**（2026-09-04，`POST /api/operator/exceptions/{id}/reinterpret`）。
  操作者提供目標 station、mode（ENTRY/EXIT）、可選的 athlete_id 轉移、以及可選的有效時間戳記 `at`
  （預設沿用原始刷卡的 `detected_at`），並要求填寫審計原因。執行時將舊異常作廢（`voided`）、
  提交新的解讀（保留原始 `raw_event_id`），並記錄 `EVENT_REINTERPRET` 審計日誌後觸發 `recalculate` 重建選手成績。

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


## UHF 讀取器（ADR 0014，2026-09-02）

線路契約已改為一輪 inventory 一則事件、`tag_id` 為陣列。仍待韌體／場地確認：

* **事件是否需要 `rssi` 欄位。** 目前不加。誤讀過濾放在讀取器端（功率、天線角度、RSSI 門檻），
  因為 Hub 分辨不出「站在天線前的選手」與「路過的人」，事後補救不了。
  若讀取器端調參需要 Hub 也看得到 RSSI，這是一次契約變更（CLAUDE.md 30）。
* **單輪最多幾張 Tag、inventory 週期。** 目前無上限假設。實際峰值決定邊緣 journal 的單筆大小，
  進而決定 10,000 筆容量是否仍然足夠。
* **RSSI 門檻與 `absent_timeout` 的實測值。** 兩者都必須在場地以真實天線量過。

完整清單見 `docs/reader-integration.md` §8。
