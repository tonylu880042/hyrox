# ADR 0007 — HTTP / WebSocket 介面層：`crates/api`、能力型別、寫入身分

- 狀態：已決議
- 日期：2026-08-29
- 影響範圍：新增 `crates/api`（router、handler、wire DTO）；`apps/hub-server` 縮回純組合根；
  `crates/application/src/ports.rs`（port 方法改寫為 `-> impl Future<..> + Send`）；
  `domain::Session::accepts_config_edits`、`domain::ReaderMode` 新增 `Deserialize`、
  `DeviceWarning` 由 `transport` 移入 `domain`（`transport` 仍再匯出）；新增
  `application::checkin_view` 與 `application::last_event_age_ms`
- 不影響：線路格式、topic、ACK 協定、資料表結構、計時規則、Athlete State 語意、
  復原行為、`/live` 畫面與 `/ws` 的 frame 內容

## 背景

Milestone 5（CLAUDE.md 26）要求 REST 與 WebSocket。在此之前路由寫在
`apps/hub-server/src/main.rs`：兩條靜態路由加一個 `/ws`，足以餵一個大螢幕，
不足以支撐 ADR 0001 定義的五個介面。

ADR 0001 已經把介面切成「一個寫入面 + 一個窄寫入面 + 三個唯讀面」，並在 D1、D5
給了兩條強制規則。這份 ADR 決定那個切分**在程式碼裡長什麼樣子**。

## 決議

### 1. 介面層自成一個 crate，依賴只向內

`crates/api` 只看得到 `application` 與 `domain`。它**看不到** `storage`，也**看不到**
`transport`：store 以泛型參數 `S: application::HubStore` 傳入，與 use case 的作法一致
（CLAUDE.md 3；ADR 0003、0005）。

代價是每個 handler 都帶泛型參數。換到的是三件事：

- handler 裡沒有任何運算式碰得到 SQLite 或 MQTT；
- 整個 HTTP 介面能用記憶體 fake 測，不需要資料庫、不需要 broker、不需要 listener
  （CLAUDE.md 24）；
- `apps/hub-server` 縮回組合根，只剩開 store、復原、MQTT 訂閱、開發用模擬器、
  tick loop 與接線。它是唯一能同時看見所有層的地方，也因此是唯一不准長出規則的地方。

**被否決的替代方案：**把路由留在 `hub-server`。省一個 crate，但介面層就能直接拿到
`storage::Store` 的具體型別，「handler 不得含業務決策」（CLAUDE.md 29）就只剩自律。

### 2. 讀寫切分是型別，不是約定

三個能力型別，一個 router 各配一個（`crates/api/src/state.rs`）：

```text
ReadOnly<S>   讀              /ws  /api/live  /api/coach  /api/session  /api/result/{id}
CheckIn<S>    讀 + 綁定        /api/checkin/**
Operator<S>   讀 + 全部寫入     /api/operator/**
```

三者都把 `Hub<S>`（live session 的鎖 + store + clock + 廣播）放在**私有欄位**，
且不提供任何取回它的方法。`ReadOnly` 身上沒有任何會寫入的方法，`CheckIn` 身上剛好只有
`bind` 與 `rebind`。handler 在自己的簽章裡宣告需要哪個能力（`State<ReadOnly<S>>`），
所以在 `read.rs` 裡寫一個寫入操作是**編譯錯誤**，不是 code review 的發現。

三道防線，缺一不可，因為它們擋的是不同的錯誤：

| | 機制 | 擋掉什麼 |
| --- | --- | --- |
| 1 | 能力型別（私有欄位 + 無 accessor） | 唯讀 handler 寫入 |
| 2 | `read.rs` 只 import `get` | 在唯讀模組註冊寫入路由 |
| 3 | 寫入路由只存在於兩個前綴之下 | 唯讀畫面的 URL 空間出現寫入端點 |

第 3 條有測試橫掃：每個唯讀路徑 × 每個寫入 verb，全部必須是 405。

理由沿用 ADR 0001：唯讀畫面要能隨手發給教練與學員。**能改資料的入口越少，
稽核範圍越小**——而「越少」必須是可驗證的事實，不是註解裡的承諾。

**被否決的替代方案：**單一 state + 命名慣例（`read_*` / `write_*`）。零型別成本，
但「這條路由會不會寫入」要讀 handler 內文才知道，正是這份 ADR 想消除的狀況。

### 3. 沒有身分就拒絕，不預設空字串

ADR 0001 D1：不登入，裝置即身分。每個寫入帶 `x-operator-device` header，
那個名字就是 CLAUDE.md 20 audit 的 `operator` 欄位。

**沒帶、或只帶空白，一律 `400 OPERATOR_REQUIRED`。** 不預設空字串：一筆誰都沒指到的
audit row 比一個被拒絕的請求更糟——它長得像紀錄，但不是紀錄。爭議發生時，
「當時沒人簽名」與「當時沒有這筆操作」必須看得出差別。

用 header 而不是 body 欄位：寫入的 body 有些是領域文件（course、reader registration），
不該為了帶身分而焊上一個 operator 欄位；header 對所有寫入一視同仁，
也讓「每個寫入 handler 的簽章裡都有 `OperatorDevice`」成為可掃描的事實。

### 4. 領域回絕是答案，不是故障

use case 說不，對應 404 / 409 / 422，各自帶一個穩定的機器碼。只有 store 寫入失敗是 500。

現場的人必須分得出「規則不允許」與「機器壞了」——這兩件事的下一步完全不同。
完整對照表在 `docs/api.md` §6。

### 5. 每個讀取回應都帶新鮮度

ADR 0001 D5 是強制的，所以它是**封套的一部分**，不是一個要另外呼叫的端點：
少呼叫一次就少顯示一格的設計，遲早會少顯示。

`freshness` 帶 `now`、`last_event_age_ms`、`websocket_path`、`push_interval_ms`、
`subscribers`。`last_event_age_ms` 為 `null` 表示「還沒有任何事件」，
**不是零**，畫面不得畫成新鮮。`push_interval_ms` 讓 client 不必自己猜一個 timeout
就能判斷 socket 死了——那個 timeout 會是 CLAUDE.md 29 禁止的魔術常數。

`/api/coach` 與 `/api/operator` 另外帶每個 reader 的 `last_seen_age_ms` 與該板子自己回報的
journal 警告（CLAUDE.md 18）。為了讓這個欄位真的有值，`apps/hub-server/src/mqtt.rs`
現在每收到一則訊息就呼叫 `note_device_seen` / `note_device_status`——
時間戳用 hub 自己的時鐘（`api::Clock`），與畫面讀的是同一個，
否則開發用的虛擬時鐘會讓 reader 的秒數與選手的秒數不能相比。

### 6. `crates/api` 不做排名

`/api/result/{id}` 原樣輸出 `application::results`，列以 bib 排序並在 payload 裡自陳
（`ordering: "BIB"`）。競賽 finish rule 未決（CLAUDE.md 12、28），任何排序都會蘊含一個
還沒有人做過的決定。`finish_policy` 一併輸出，讓讀的人看得到這場的「完成」是什麼意思——
包含它什麼意思都不是。

### 7. 連帶改動

- **port 方法改寫成 `-> impl Future<..> + Send`。** trait 裡的 `async fn` 不承諾回傳的
  future 是 `Send`，而每個 use case 都對 port 泛型；handler await 它時就無法證明能在
  多執行緒 executor 上跑，`crates/api` 會編不過。adapter 仍然寫一般的 `async fn`，
  多的只是那個承諾。
- **`DeviceWarning` 從 `transport` 移到 `domain`**（`transport` 再匯出）：operator 的
  reader 健康視圖需要它，而介面層不得依賴傳輸層。
- **`Session::accepts_config_edits()`**：D2 的「只有 DRAFT 可編輯設定」原本在
  `application::config` 裡以 `status != Draft` 表達，現在畫面也要知道
  （`config_editable`）。與其讓 UI 自己重推一次規則（CLAUDE.md 6、29），
  不如讓 domain 講一次，兩邊讀同一句話。
- **`application::checkin_view` 與 `last_event_age_ms`**：`/checkin` 的待辦清單與
  D5 的秒數都是讀模型，不是 handler 的推導。放在 application 並各自有測試。

## 可攜性

`crates/api` 只用 axum、tokio 的 `sync`、serde。沒有任何 macOS API，
沒有檔案系統假設，沒有 OS 特定的網路行為。監聽位址由組合根決定。
Linux 部署不需要改這個 crate 的任何一行（CLAUDE.md 2）。

## 本決議未處理

- **D4 的另外兩個動作**（accept as-is、改判）沒有 use case，因此沒有端點。
  記在 `docs/open-issues.md`，不在介面層半做。
- **名單編輯**需要 健身管 的 `MemberRef`，而該契約未定（ADR 0003）。
- **裝置同名衝突**（ADR 0001 自陳未決）仍未決：兩台叫同一個名字的平板，
  audit 上分不出來。
- **CLAUDE.md 23 的 Current Ranking** 沒有欄位可填，理由見 §6。
