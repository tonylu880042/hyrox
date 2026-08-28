# ADR 0006 — MQTT ingestion 接線：訂閱迴圈的位置、無法解碼的 payload、開發用模擬器

- 狀態：已決議
- 日期：2026-08-28
- 影響範圍：`crates/transport`（新增 `inbound.rs`、`client::next_inbound` / `subscribe_acks`、
  `topic::device_of_ack`）、`crates/simulator`（新增 `broker` feature 與 `mqtt.rs`）、
  `apps/hub-server`（新增 `mqtt.rs` 訂閱迴圈與 `sim.rs` 開發車隊；`feeder.rs` 由「產生事件」
  縮回「描述場地與腳本」）
- 不影響：線路格式、topic 名稱、ACK 協定的型別保證、資料表結構、計時規則、
  `crates/application` 與 `crates/domain` 的任何一行

## 背景

Milestone 3 之前，`apps/hub-server` 用計時器在行程內憑空造出 `ReceivedEvent`，
而 `crates/transport/src/client.rs` 因為沒有 broker 可用，測試覆蓋率是零。
本次把真的 MQTT 接上：訂閱 → 解碼 → `application::ingest_read` → 發布 ACK。

## 決議

### 1. 訂閱迴圈放在組合根，分類放在傳輸層

迴圈本身在 `apps/hub-server/src/mqtt.rs`（組合根），因為它需要同時看見
`application`、`storage` 與 broker，而那是組合根才有的視野（CLAUDE.md 3）。

它只做三件事，且**一件都不決定**：

```text
decode (transport::classify) → hand off (application::ingest_read) → publish the ACK it was given
```

看起來像規則的部分全都屬於下層：

| 看起來的規則 | 真正住在哪裡 |
|---|---|
| 重複投遞也要 ACK | `IngestOutcome::Duplicate` 本身就帶著一則 ACK |
| 解讀失敗仍要 ACK | `IngestError::Interpretation { ack, .. }` 把 ACK 交回來（raw read 已持久） |
| 未持久化就不准 ACK | `IngestError::Storage` 手上根本沒有 ACK 可交（ADR 0002） |

迴圈裡沒有任何一條路徑能自己造出 `Ack`，這是型別決定的，不是自律（CLAUDE.md 29）。

解碼與分類則放在 `crates/transport/src/inbound.rs`，**不在 `broker` feature 底下**：
它是純函式（topic + bytes → `Inbound`），因此連 rumqttc 都不必進建置就能測完所有分支，
包含用 broker 很難重現的那些（外來 topic、截斷的 payload）。

`Inbound` 不判斷「payload 裡的 `device_id` 與 topic 上的 device 是否相符」。
payload 才是契約（CLAUDE.md 16），idempotency key 由它組成，ACK 也回發給
**payload** 指名的裝置，兩者不可能各走各的；為了一個定址上的不一致丟掉真實讀取，
違反 CLAUDE.md 31 第一優先。

### 2. 無法解碼的 payload：記錄在營運日誌，不入庫、不 ACK

**不 ACK**：什麼都沒有持久化，依 ADR 0002 手上就沒有 ACK 可發，邊緣保留該筆並重送。

**不入 raw store**：`raw_events` 以 `device_id + boot_id + sequence` 為鍵（CLAUDE.md 16），
而「無法解碼」的定義就是沒有這組鍵；硬要寫一列等於**發明**一個鍵。

**留下紀錄**：訂閱迴圈以計數器 + 一行 stderr 記錄 topic、解碼錯誤、以及 payload 的
有界節錄（256 bytes，`transport::payload_excerpt`，escape 過，避免偽造日誌行）。
有界是因為「留下紀錄」不該變成讓壞掉的裝置淹沒日誌的手段。

考慮過但未採用的替代方案：**quarantine 資料表**。它比日誌可靠，但需要新的資料表
（schema 變更，CLAUDE.md 30）以及一條本專案還沒有答案的保留期規則，
而且會讓一台故障裝置把磁碟寫滿。列為未決議項目（`docs/open-issues.md`），
等現場真的出現這個症狀再決定——屆時 `Inbound::Undecodable` 已經把完整 payload
交在手上，改成寫表只需要動這一個 match arm。

**一台壞掉的裝置不得停掉一堂課**：迴圈在任何 payload 上都不 return、不 panic、
不停止 poll。這條由 `broker_a_payload_that_is_not_an_event_does_not_stop_the_subscriber`
釘住（壞 payload 之後緊接著一筆好事件，必須照常收到）。

### 3. Hub 的 MQTT client id 固定，`clean_session = false`

`hyrox-hub`（可用 `HYROX_MQTT_CLIENT_ID` 覆寫）。broker 因此會在 Hub 不在線時
保留這個 client 的 QoS 1 佇列與訂閱，Hub 重啟後那些事件仍會送達（CLAUDE.md 15、21）。
每次啟動換一個 id 會把佇列丟掉，等於用組態抵銷掉可靠性設計。

實測（2026-08-28）：Hub 停止 → 發布一筆事件 → Hub 重啟，該事件在重啟後送達、
存一列、解讀一次、回 `STORED`；同一筆再重送兩次，回兩則 `DUPLICATE`，
raw 與 interpreted 都仍是各一列。

### 4. 開發用模擬器跑在 hub-server 行程內，預設開啟

要求是「`cargo run -p hub-server` 一個指令就要有會動的 `/live`」。
真的 MQTT ingestion 之後，Hub 自己不再產生任何事件，所以必須有東西在發布。

選擇：`crates/simulator` 的 `SimDevice`（同一份 journal、同一份 presence/re-arm 抑制）
掛上真的 broker，在 hub-server 內以背景 task 執行。

- 為什麼不是另一個 binary：那樣單一指令就不成立，開發者每次都要開兩個終端機。
- 為什麼不是保留舊 feeder 當 fallback：那會留下第二條不經過 broker 的 ingestion 路徑，
  而「螢幕上的東西到底走過哪一條路」正是這個里程碑要消除的疑問。
- 為什麼安全：`dev-simulator` feature 預設開啟但可關（`--no-default-features` 讓模擬器
  完全不進建置），執行期另有 `HYROX_SIM=off`。場地部署用前者。

`crates/simulator` 因此新增 `broker` feature（預設關閉）。既有的 `Bench` / `Link`
行程內測試一行未動——那是 CLAUDE.md 24 要求的「沒有 broker 也能測」的所在，
新增的只是同一台裝置的另一條輸出管道。

`apps/hub-server/src/feeder.rs` 隨之縮小：它現在只描述「哪張卡在什麼時候被哪個 reader 讀到」。
boot id、sequence、journal 與抑制全部回到裝置模型手上，也就是真實韌體的位置（CLAUDE.md 16、25）。

### 5. 模擬器的重送計時器

韌體不能只在重連時重送：ACK 可能在連線完全正常的情況下遺失。開發車隊每 5 秒檢查一次，
若仍有未 ACK 的事件就整批重送（`MqttDevice::resend_pending`）。這是邊緣端實作細節，
不是產品規則；實際數值屬於韌體團隊。Hub 以 `device_id + boot_id + sequence` 判重，
所以重送的代價是一則 `DUPLICATE` ACK，不是一筆事件（CLAUDE.md 16、18）。

## 測試策略

`crates/transport/tests/broker.rs` 是**整個 workspace 唯一需要 broker 的測試**。
它在 `127.0.0.1:1883` 沒有回應時自己跳過（TCP connect 探測，印出 `SKIPPED: …`），
所以沒有 broker 的機器上 `cargo test --workspace` 仍然全綠，只是證明得比較少。
五個案例共用一個 broker 與一棵 topic 樹，而 Hub 訂閱是萬用字元，因此以一個
`tokio::sync::Mutex` 序列化。

## 可攜性影響（macOS / Linux）

無。新增的全部是 rumqttc + tokio，沒有檔案系統路徑、沒有行程管理、沒有平台 API。
broker 位址與 client id 走環境變數，Linux 部署不需要改任何一行程式碼。

## 後續

- 無法解碼的 payload 是否需要 quarantine 資料表（`docs/open-issues.md`）。
- 目前每筆事件一則 ACK。若 10,000 筆 backlog 重送造成流量問題，改批次 ACK cursor
  （ADR 0002 已經預留了形狀）。
- Broker 認證 / ACL 仍未處理（CLAUDE.md 28 網路設計未決）。
- `DeviceStatus` 目前只寫進 Hub 日誌，尚未進入 coach view 的裝置警告欄（CLAUDE.md 23）。
