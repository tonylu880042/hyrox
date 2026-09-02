# ADR 0005 — 契約獨立成 crate，傳輸層改名，身分型別單一化

- 狀態：已決議
- 日期：2026-08-28
- 影響範圍：新增 `crates/contract`；`crates/mqtt` → `crates/transport`（只留 topic / status /
  broker client）；`crates/application` 移除對傳輸層的依賴；`crates/domain`（`DeviceId` /
  `ReaderId` 新增 `Deserialize` 與錯誤的 `Display`）；`crates/simulator`、`crates/storage`、
  `apps/hub-server` 只改 import
- 不影響：線路格式、topic 名稱、ACK 協定的型別保證、資料表結構、計時規則、任何測試的斷言

## 背景

依賴圖長成這樣：

```text
application  ──▶ domain
             ──▶ mqtt      ← 違反 CLAUDE.md §3
```

`application` 是 use case 層，不該依賴傳遞機制。它之所以依賴 `crates/mqtt`，
只為了四個名字：`ingest`、`IngestError`、`EventStore`、`CommitOutcome`。

看過 `crates/mqtt` 的內容之後，問題不在設計而在**放錯地方**：

- `event.rs`（`EdgeEvent` / `ReceivedEvent` / `EventId` / 編解碼）
- `ack.rs`（`EventStore` / `Commit` / `Ack` / `ingest`）

這兩個檔案裡沒有任何 MQTT：沒有 topic、沒有 rumqttc。它們是**兩個系統之間的契約**
（CLAUDE.md 15、16），只是當初和 topic、client 寫在同一個 crate 裡。
真正屬於 MQTT 的只有 `topic.rs`、`client.rs`、`status.rs`。

同一段歷史還留下第二個問題：`DeviceId` / `ReaderId` 在 `domain/src/device.rs` 與
`mqtt/src/id.rs` 各有一份，語意相同、驗證不同。

## 決議

### 1. 契約與 ACK 協定移到 `crates/contract`

```text
domain      ──▶ （無）
contract    ──▶ domain                       事件契約 + idempotency key + ACK 協定
application ──▶ domain, contract             use case
transport   ──▶ domain, contract [, rumqttc] topic / status / broker client
simulator   ──▶ contract, transport          模擬 ESP32
storage     ──▶ domain, application, contract
hub-server  ──▶ domain, application, contract, storage
```

`application` 現在只依賴 `domain`、`contract`、serde、thiserror。

### 2. 為何不是「全部塞進 `application`」

考慮過把 `event.rs` 與 `ack.rs` 直接放進 `crates/application`：不必新增 crate，
而且 ingest use case 的輸入邊界確實是 `EdgeEvent`。**否決的理由是 `crates/simulator`**。

模擬器模擬的是**另一台機器**。它需要 `EdgeEvent`（發事件）、`AckPayload`（讀 ACK），
而且 `InMemoryHub` 實作 `EventStore`、`Bench::flush` 呼叫 `ingest`——那是在模擬
「Hub 先 commit 才回 ACK」這條迴圈，也就是韌體真正在等的行為。若契約放進
`application`，模擬器就得依賴 Hub 的 use case 層，`LiveSession`、`HubStore`、
`ingest_read` 全部對它可見。這正好推翻 `crates/simulator` 現有的邊界宣告：
「a real edge collector carries no business meaning (CLAUDE.md 8), so this one must not
be able to either」。

### 3. 為何 ACK 協定也留在 `contract`，而不是 `application`

另一個選項是契約進新 crate、ACK 協定進 `application`。這個組合**同時**多一個 crate
**又**讓模擬器依賴 `application`（因為 `ingest` / `EventStore` 在那裡），
比選項一嚴格更差，所以不選。

理由不只是「模擬器需要」。ACK 是**雙方**的協定，不是 Hub 的內部政策：

- `AckPayload` 是韌體要解析的線路格式（CLAUDE.md 15 的流程圖裡，
  「ESP32 marks event acknowledged」是邊緣端的動作）。
- ADR 0002 的保證要成立，`Ack` 的鑄造點必須是**單一**一處。放在契約 crate 裡，
  Hub 與模擬器走的是同一條路徑，而不是各自有一條。

`application` 因此**使用**契約（`contract::ingest`），而不是**擁有**它——這正是
CLAUDE.md §3 的方向。

### 4. 身分型別以 `domain` 為準

刪除 `mqtt/src/id.rs` 的 `DeviceId` / `ReaderId` / `IdError`，線路直接解碼成
`domain::DeviceId` / `domain::ReaderId`。`crates/contract` 從根部 re-export 這兩個型別，
邊緣端程式碼（模擬器、韌體對應路徑）因此仍然只需要認得一個 crate。

兩份實作的驗證差異，以及處理方式：

| 情況 | 舊 `mqtt` 版 | `domain` 版（現行） | 處理 |
|---|---|---|---|
| `reader_id` 字元 | 任何非空字串 | 限英數字 / `-` / `_` | **採用 domain**：Hub 查表本來就用這條規則，寬鬆的那一份只會讓 id 進得了 raw store 卻永遠對不到 Reader |
| `device_id` 大小寫 | 前綴必須小寫 `esp32-` | 全字串折疊大小寫 | 採用 domain，較寬鬆且無歧義 |

> **2026-09-02 更新（ADR 0015）**：`esp32-` 前綴已取消，正規形式改為裸的 12 位小寫 hex。
> 折疊大小寫與拒收分隔符號的規則不變，本節其餘內容照舊。
| `device_id` 分隔符號 | 線路上接受 `a4:cf:…` | 線路上只接受正規形式；分隔符號只有 `from_mac_str` 接受 | 採用 domain：韌體送的是自己的正規 id，分隔符號屬於人手輸入 |

沒有任何驗證被丟掉：唯一「舊版接受、新版拒絕」的是帶分隔符號的 `device_id`，
而那不是驗證，是輸入正規化，`DeviceId::from_mac_str`（設定檔與人手輸入的入口）
仍然接受。

為了讓線路能解碼，`domain` 的兩個身分型別新增手寫的 `Deserialize`（走 `parse`，
在邊界驗證而非信任），以及兩個錯誤型別的 `Display` / `Error`。序列化形式不變。

### 5. `crates/mqtt` → `crates/transport`

留下的東西只剩傳遞機制：topic 配置、裝置健康狀態 payload、rumqttc client。
`broker` feature 維持預設開啟，關掉後 rumqttc 不進入建置（CLAUDE.md 24）。
`apps/hub-server` 不再依賴傳輸層——它目前用的是腳本 feeder，等 Milestone 3 接上
真的 broker 時再加回來，屆時加的是 `transport`，而不是「順便帶進 rumqttc」。

## ADR 0002 的保證如何確認未被削弱

搬家不得動到「commit 之後才有 ACK」這條型別規則。確認方式：

- `Ack` 仍是 `pub struct Ack(AckPayload)`，欄位私有、只 derive `Debug`（無 `Deserialize`）。
- `Commit::new` 仍是 `pub(crate)`。
- `grep -rn "Commit::new\|into_ack" crates apps` 的結果全部落在
  `crates/contract/src/ack.rs`，而且 `Commit::new` 只出現在 `ingest` 裡
  `store.commit(...)` 成功之後的下一行。
- `crates/contract/tests/ack.rs` 原樣搬移，斷言一字未改。

## 可攜性影響（macOS / Linux）

無。全部是 crate 邊界與 import 的移動，沒有新增平台相依，也沒有檔案系統或行程管理。

## 後續

- 接上真的 MQTT 時，`transport::client` 是唯一需要動的地方；`contract` 與 `application`
  不必改。
- `DeviceStatus` 目前放在 `transport`（它是綁在 status topic 上的健康回報）。
  若之後韌體改用別的通道回報健康狀態，它應該跟著移到 `contract`。
