# ADR 0003 — Application 層與 Port 邊界

- 狀態：已決議
- 日期：2026-08-28
- 影響範圍：新增 `crates/application`；`crates/storage`（實作 port、新增 audit 表）、
  `crates/domain`（新增一個 exception reason）、`apps/hub-server`（改為只做接線與傳輸）
- 不影響：MQTT 線路契約、既有資料表結構、計時規則

## 背景

`apps/hub-server` 的 `tick_loop` 直接做了完整的 ingestion 編排：寫 raw、決定是否解讀、
解讀、寫 interpreted、更新 session。這違反 CLAUDE.md §29（不得把業務邏輯放在傳輸層）
與 §3（業務編排屬於 application 層）。

更嚴重的是，那條路徑餵給 `domain::interpret` 的是**已經解好的** `ReaderBinding` 與
陣列索引，等於跳過了現場真正會失敗的兩個步驟：Reader 解析與 Tag 解析。
`ExceptionReason::UnknownReader` 因此從來沒有任何程式碼會產生。

## 決議

### 1. 新增 `crates/application`，依賴只指向內側

```text
hub-server (axum) ──▶ application ──▶ domain
                          │            ▲
                          └─▶ mqtt     │
                                    storage（實作 port）
```

`application` 不依賴 axum、不依賴 `storage`、不依賴任何 OS API，因此整個 crate 可以在
沒有資料庫、沒有 broker、沒有 HTTP 的情況下測試（§24）。
`storage` 反過來依賴 `application` 並實作其 port，依賴箭頭因此指向內側（§3）。

### 2. Ingestion 是一個 use case，且解析步驟是真的

```text
raw edge event
  → commit raw（durable、idempotent，換得 ACK）
  → ReaderRegistry 解析 (device_id, reader_id)
  → BindingLedger 解析 tag
  → domain::decide / apply
  → commit interpreted
```

四種解析結果，沒有任何一種會丟事件（§31 第一原則）：

| 情況 | 結果 |
|---|---|
| Reader 未註冊（含 id 格式不合法） | `UNKNOWN_READER` exception，掛在該選手名下 |
| Tag 未綁定 | **不是錯誤**：進待綁定清單給 `/checkin`，raw 照存（ADR 0001 D3） |
| Tag 綁定給非本 Session 名單的人 | 新增的 `ATHLETE_NOT_IN_SESSION` exception（ADR 0001 D4） |
| Tag id 本身不可用 | `Unattributable`：raw 仍存，但沒有東西可以指認 |

### 3. ACK 的型別保證不被削弱

ADR 0002 規定只有 `mqtt::ingest` 能鑄造 `Ack`。ingestion use case 因此**不自己寫 raw**，
而是把 `HubStore` 包成 `mqtt::EventStore` 交給 `mqtt::ingest`，只是額外把 raw row id
放進一個 atomic 帶回來（用來連結 interpreted 與 raw）。

考慮過的替代方案：把 `EventStore::commit` 的回傳值改成含 row id。那會讓每一個 store
實作（包含測試用假 store）都能造出 ACK，正是 ADR 0002 選 B 變形時排除掉的方向。

`IngestError` 分成兩種失敗：`Storage`（raw 未持久化，**沒有** ACK，邊緣會重送）與
`Interpretation`（raw 已持久化，ACK 有效並回傳，但解讀沒寫進去）。
兩者的現場處置不同，型別上就不該混為一談。

### 4. Finish policy 真的被執行

`FinishPolicy::ClassDuration` 之前只有定義沒有人呼叫。現在 `apply_finish_policy` 每個
tick 評估一次，`Finished` 的選手走 `domain::finish`。

- `Undetermined`（競賽的 `NotConfigured`）一律當成「沒有答案」：不 finish，也不宣告未
  finish（§12、§28）。
- 教練手動結束（`end_class`）在 `NotConfigured` 時**拒絕執行**。一個會把所有競賽選手
  計時停掉的按鈕，就是 §28 禁止的臆測規則。
- finish 結果**不寫入事件**：它是由班級時鐘與既有事件推導出來的，重啟後下一個 tick 會
  重新推導出同樣結果（§21）。寫一筆 FINISHED 進 interpreted_events 等於捏造一個沒有任何
  reader 觀測到的事件。

### 5. 健身管 只有 port 與 stub

方向已確認（Hub 呼叫對方，以 QR 取得的 member id 為 key）。但端點、認證與 payload 仍未知
（docs/open-issues.md），因此只交付 `MemberDirectory` port 與 `UnconfiguredDirectory`
（永遠回報 `NotConfigured`）。依猜測寫出來的 HTTP client 會被整份丟掉，而且看起來像是
專案已經知道答案。`MembershipStatus` 不擋任何事（2026-08-27 已確認）。

### 6. 讀模型（live snapshot）屬於 application

`/live` 用的 snapshot 原本在 hub-server 裡。它是產品決策（教練該看到哪些數字），而不是
傳輸；競賽畫面與 `/coach` 之後要重用同一份，所以移進 `application::live`。
課表的 icon key 與 plan 字串現在由 `domain::Course` 推導，不再由 dev feeder 手打。

## 對既有 crate 的改動

- `domain`：新增 `ExceptionReason::AthleteNotInSession`。ADR 0001 D4 早已列出這個情況，
  但沒有對應的 reason 可用。
- `storage`：實作 `application::HubStore`；新增 migration `0002_audit_log.sql`
  （純新增資料表，0001 未動）與 `save_audit`。§20 要求每筆修正都有
  operator / timestamp / reason / before / after，先前沒有地方可以放。
- `hub-server`：只剩接線、HTTP、WebSocket 與 dev feeder。feeder 現在發布 ESP32 真正會發
  的欄位（device / reader / tag / boot / sequence / detected_at），因此走的是完整管線。

## 可攜性影響（macOS / Linux）

無。`application` 只有純運算、`async fn` in trait 與 serde，沒有檔案系統、沒有行程管理、
沒有平台 API。`async fn` in trait 使這些 port 不是 dyn-compatible，呼叫端走泛型即可。

## 已知缺口

以下兩項已於 2026-08-28 由 ADR 0004 解決，保留原文以留下推理過程：

- ~~Reader 註冊表與綁定帳本仍只在記憶體，Phase 1 沒有對應資料表，重啟後由啟動端重建。~~
  連同 Session 設定一起持久化（migration `0003`）。原本的風險不只是「重建」，而是恢復後
  的 Session 會默默採用呼叫端提供的 Finish 規則。
- ~~未綁定 tag 的既有 raw 事件，在綁定完成後**尚未**自動回溯重新解讀（ADR 0001 D3 要求）。
  事件沒有遺失，但認領目前是人工的。~~ 綁定時自動認領，依 `detected_at` 順序重播。
