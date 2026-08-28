# ADR 0004 — Session 設定、Reader 註冊表與綁定帳本必須持久化

- 狀態：已決議
- 日期：2026-08-28
- 影響範圍：新增 migration `0003_config_readers_bindings.sql`；`application`（新增 port
  方法、`recover` 改寫、新增 `readers` use case、`checkin` 回溯認領）；`storage`（實作新
  port）；`domain`（設定型別可反序列化、`BindingLedger::restore`）；`apps/hub-server`
  （啟動改由 use case 佈建）
- 不影響：MQTT 線路契約、raw event 結構、既有資料表、計時規則、Finish 規則本身

## 背景

CLAUDE.md §21 要求重啟後必須重建 Session、Athlete State、計時與排行榜。

Athlete State 早已達成：它由 interpreted event log 重播而來，所以重啟後不可能與 log 不
一致。但另外三樣東西只存在於記憶體，由啟動端在開機時重新提供（ADR 0003「已知缺口」）：

- Session 設定（課表 + Finish 規則）
- Reader 註冊表
- Tag 綁定帳本

後果不是「資料遺失」，而是更難察覺的一種：**恢復後的 Session 會默默採用呼叫端當下提供的
設定**。一堂以 `ClassDuration { limit }` 開課的課，在 Hub 重啟後可能改以另一條規則結束，
而畫面上沒有任何跡象。CLAUDE.md §12 明文禁止把 Finish 規則寫死；讓它在重啟時被替換掉，
等於用另一種方式把規則交給了偶然。

## 決議

### 1. 設定以 JSON 欄位儲存，不做正規化

`session_configs(session_id, config_json)`。

課表是巢狀、有序、可重複的結構（CLAUDE.md §9.2），每一站帶自己的目標值。Hub 永遠整份
讀、整份寫，沒有任何查詢需要進到它裡面。拆成 `courses` / `course_steps` /
`station_targets` 三張表只會製造沒有人會下的 JOIN，而且每次 domain 改一個欄位就要動
schema。

代價寫在這裡：SQL 問不出「哪些課用過 SKIERG」。真的需要時，另外建索引欄位即可，不必動
這個欄位。

設定在 ARM 時寫入。ADR 0001 D2 規定只有 DRAFT 可以編輯設定，所以進行中的課不會被抽換
規則——這正是儲存它的意義。

### 2. 恢復時，儲存的設定優先於呼叫端的設定

`resume_or_start` 讀回 `session_config`：

| 情況 | 結果 |
|---|---|
| 有儲存的設定 | `Recovery::Resumed`，使用儲存的課表與 Finish 規則 |
| 沒有（ADR 0004 之前的資料庫） | `Recovery::ResumedWithoutStoredConfig`，退回 plan 的設定 |

第二種情況**必須被說出來**，不能靜默退回。hub-server 印出 WARNING。

考慮過的替代方案：沒有設定就拒絕恢復。否決，因為那會讓一個舊資料庫上的進行中課程完全
無法接續——為了避免一個可能的錯誤而製造一個確定的損失，違反 §31 的優先順序。

### 3. Reader 註冊表屬於場館，不屬於 Session

`readers(device_id, reader_id, station, zone, mode)`，主鍵是 `(device_id, reader_id)`。
牆上的 Reader 比任何一堂課活得久，所以這張表不以 session 分割。

重新註冊只有在對映真的改變時才寫 audit。每次開機都會重新註冊一輪，把那些寫進 audit 只
會淹沒唯一重要的那一行：某天晚上有人把 Reader 改指到別站（CLAUDE.md §20）。

### 4. 綁定帳本以 append-only 的方式持久化

`tag_bindings(session_id, tag_id, athlete_id, bound_at, unbound_at)`，主鍵含 `bound_at`。

domain 的帳本刻意是 append-only：換綁是「關閉舊列 + 開新列」，不是改寫舊列。儲存層用同一
條規則——UPSERT 只允許補上 `unbound_at`，永遠不更新 `athlete_id`。**已關閉的綁定必須一起
恢復**，否則「10:15 這個腳環戴在誰身上」在重啟後就永遠答不出來（CLAUDE.md §20）。

這張表刻意沒有指向 `sessions` 的外鍵：Tag 唯一性是跨 Session 檢查的（一個腳環同時只在一
隻手腕上），所以帳本合理地會持有本 Hub 沒有跑過的班級的列。

### 5. 待綁定清單用推導，不用儲存

待綁定 tag = 「本堂課開始後被任一 Reader 讀到、且目前沒有有效綁定」的 tag。
由 `raw_events` 推導（新增 port method `raw_tags_since`）。

推導而非儲存，是因為它本來就是兩個已持久化事實的函數。儲存它會多出一個可能與事實不符的
狀態：例如有人在別的裝置上完成綁定後，一份存下來的清單仍會把那個 tag 留在待辦上。

### 6. 回溯認領（ADR 0001 D3）改為自動

綁定完成後，該 tag 已儲存但**尚未被任何 interpreted event 指向**的 raw 讀取，會依
`detected_at` 順序被解讀並歸戶。

- 冪等性來自 `NOT EXISTS (SELECT 1 FROM interpreted_events WHERE raw_event_id = ...)`。
  認領過的讀取不會被認領第二次，換幾次腳環都一樣。被 operator void 掉的也算已認領——那是
  刻意移除的，重新產生一筆等於推翻他的修正。
- 順序就是全部的規則。`transition` 由前一次離站時間推導（CLAUDE.md §13），只有依
  `detected_at` 重播才會得到正確的值。
- 認領與即時解讀共用同一個 `attribute_read`，所以「晚綁」與「早綁」的結果相等不是靠對齊
  兩段程式碼，而是結構上同一段。測試直接驗證這個等價性。

### 7. 順帶修掉的記憶體漂移

先前 `domain::interpret`（decide + apply 一次做完）在寫入 interpreted event **之前**就推
進了記憶體中的 Athlete State。寫入失敗時，記憶體宣稱選手在某一站，而 log 從未記載——兩者
要到下次重啟才會一致。

改為 decide → 寫入 → apply。decide 是純函數，提早做不會失去任何東西；apply 才是必須等待
的那一步。計數器（`interpreted_event_count`、exception badge）同樣移到寫入之後。

保留下來的較小落差：寫入 interpreted 成功、但接著更新 sessions 列失敗時，
`interpreted_event_count` 會少一。它只用於把關 ARMED → DRAFT（ADR 0001 D2）；Athlete
State 與 exception badge 都是重新推導的，不受影響。這一項記在 `application` 的 crate 文件
的「Known gaps」。

## 依賴方向

不變（CLAUDE.md §3）。新增的持久化需求全部是 `application::HubStore` 上的 port method，
由 `crates/storage` 實作。`application` 仍然不依賴 sqlx、不依賴 storage，整層仍可在沒有
資料庫的情況下以記憶體假物件測試（CLAUDE.md §24）。

`domain` 的設定型別加上 `Deserialize`。這是讓設定能被讀回來的最小改動，沒有引入任何
infrastructure 依賴——serde 本來就在。

## 可攜性影響（macOS / Linux）

無。新增的是三張 SQLite 資料表與純運算，沒有檔案系統路徑、沒有平台 API。

## 已知缺口

- Reader 註冊、綁定與換綁目前只有 use case，沒有 UI 入口（Milestone 6）。hub-server 以
  dev feeder 呼叫它們。
- 沒有「刪除 Reader」的 use case。改指到別站可以，移除不行——先確認現場需要哪一種語意。
- 認領只看本堂課開始之後的 raw 讀取。開課前刷到的腳環不會被認領，那是刻意的：那些讀取
  不屬於這堂課。
