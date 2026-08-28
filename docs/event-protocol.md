# Event Protocol — ESP32 ↔ Central Hub

本檔是 ESP32 邊緣採集器與 Central Hub 之間的線路契約。
**契約變更必須同步更新本檔與測試**（CLAUDE.md 30）。

對應程式碼：`crates/contract/`（契約本身：事件、idempotency key、ACK 協定）、
`crates/transport/`（MQTT topic 與 broker client）、`crates/simulator/`（模擬 ESP32 行為）。
三個 crate 都不依賴 `application` 或 `storage`：邊緣裝置不承載業務語意（CLAUDE.md 8）。
`crates/contract` 只依賴 `domain` 的身分型別（`DeviceId` / `ReaderId`，CLAUDE.md 7.3），
讓線路解碼直接落在 Hub 查表所用的同一個型別上（ADR 0005）。

---

## 1. 身分

### device_id（CLAUDE.md 7.3）

由 ESP32 Base MAC 導出，格式固定：

```text
esp32-a4cf128b3d91
```

`DeviceId::from_mac_str` 接受 `A4:CF:12:8B:3D:91`、`a4-cf-12-8b-3d-91`、`a4cf128b3d91`
三種寫法（給設定檔與人手輸入用），一律正規化為上述形式。不使用隨機 UUID：
重新燒錄不得讓同一台機器變成兩台。

**線路上只接受正規形式**：`device_id` 欄位走 `DeviceId::parse`，大小寫不拘，
但不接受分隔符號——`esp32-a4:cf:12:8b:3d:91` 會被判為 malformed。
韌體送出的就是自己的正規 id，分隔符號只會出現在人寫的設定裡（ADR 0005）。

### reader_id

與 `device_id` 分離。一台 ESP32 未來可掛多個 Reader。

`ReaderId` **不分大小寫**（`RFID-02` 與 `rfid-02` 視為同一個）。
CLAUDE.md §8 內文寫 `RFID-02`、§16 JSON 範例寫 `rfid-02`，折疊大小寫避免
兩種拼法在 Reader 對應表中變成兩列。**此為待人工確認事項**，見第 7 節。

字元限定為英數字、`-`、`_`：這是 Hub 的 Reader 對應表本來就有的規則，
現在線路解碼也套用同一條（ADR 0005），避免一個進得了資料庫、卻永遠查不到
對應的 `reader_id`。空字串一律拒絕。

---

## 2. Event 欄位（CLAUDE.md 16）

```json
{
  "device_id": "esp32-a4cf128b3d91",
  "reader_id": "rfid-02",
  "boot_id": 18,
  "sequence": 10382,
  "tag_id": "E280117000001234",
  "detected_at": 1787734821382,
  "uptime_ms": 382912
}
```

| 欄位 | 型別 | 用途 |
|---|---|---|
| `device_id` | string | 裝置身分 |
| `reader_id` | string | Reader 身分（Hub 據此對應 Station / Zone / Role） |
| `boot_id` | i64 ≥ 0 | 開機序號，開機遞增 |
| `sequence` | i64 ≥ 0 | 該次開機內的事件序號，開機後重新起算 |
| `tag_id` | string 非空 | RFID Tag |
| `detected_at` | i64 ≥ 0 | epoch 毫秒，**官方計時來源** |
| `uptime_ms` | i64 ≥ 0 | 本次開機經過毫秒，診斷用 |

Hub 收件時另外補上 `received_at`，**只用於診斷**（CLAUDE.md 17）。
程式碼以 `ReceivedEvent` 把兩者分開存放，`official_time()` 永遠回傳
`detected_at`，`arrival_lag_ms()` 明確標示為診斷用途。

負數計數器與空 `tag_id` 一律拒收（`WireError`），不進儲存層。

---

## 3. Idempotency Key（CLAUDE.md 16）

```text
device_id + boot_id + sequence
```

以 `EventId` 型別表示，不用鬆散的 tuple 傳遞。

- **重複投遞允許**，重複業務處理不允許。
- `boot_id` 是必要的：`sequence` 會在重開機後歸零，少了 `boot_id` 會與前次
  開機的事件撞號，真實事件會被誤判為重複而遺失（CLAUDE.md 31 第一優先）。

`crates/storage` 的 `raw_events` 以相同三欄位判重，兩邊語意一致。

---

## 4. Topic 配置

```text
hyrox/v1/edge/<device_id>/events    edge → hub   RFID 事件        QoS 1
hyrox/v1/edge/<device_id>/status    edge → hub   裝置健康 / 警告   QoS 1, retained
hyrox/v1/hub/<device_id>/ack        hub  → edge  應用層 ACK       QoS 1
hyrox/v1/hub/time                   hub  → edge  時間同步         QoS 1
```

Hub 訂閱 `hyrox/v1/edge/+/events` 與 `hyrox/v1/edge/+/status`。

`v1` 區段的用途：契約若要變更，是換新 topic，而不是讓舊 topic 悄悄改變語意
（CLAUDE.md 30）。

上下行分屬 `edge/` 與 `hub/` 兩個分支，訂閱不可能混淆方向。

`clean_session = false`：重連時 broker 端 QoS 1 session 不被清掉。

---

## 5. Application ACK（CLAUDE.md 15）

```text
RFID 偵測 → ESP32 journal → MQTT publish → Hub 收件
    → SQLite COMMIT → Application ACK → ESP32 標記已確認
```

**在持久化 commit 成功之前不得 ACK。**

程式碼用型別強制這件事，而非靠自律：

- `EventStore` 是 port（trait），Hub 之後以 SQLite 實作。
  契約只有一句：**持久化 commit 成功之後才回 `Ok`**。
- `Commit` 是「已持久化」的憑證，只有 `mqtt::ingest` 能鑄造，
  而且是在 `EventStore::commit` 回 `Ok` 的下一行。
- `Ack` 沒有公開建構子，唯一取得方式是 `Commit::into_ack()`。
- publish 函式收 `Ack`，不收 `AckPayload`。

想提早 ACK 的程式碼手上沒有東西可以 ACK。

### ACK 訊息

```json
{
  "device_id": "esp32-a4cf128b3d91",
  "boot_id": 18,
  "sequence": 10382,
  "status": "STORED"
}
```

`status` 為 `STORED` 或 `DUPLICATE`。
**兩者都會釋放邊緣裝置上的事件**：`DUPLICATE` 表示 Hub 早已持久化該事件，
與剛存入一樣可靠。重複投遞若不回 ACK，邊緣會永遠重送。

Commit 失敗時不產生 `Ack`，邊緣保留事件並在下次重送。

---

## 6. 邊緣端行為契約

以下由 `crates/simulator` 定義語意、由測試釘住，韌體需照此實作。

### 6.1 抑制：Tag Presence / Re-arm（CLAUDE.md 14）

```text
first_seen                      → SEND
同一 Tag 持續可見                → 抑制
離開超過 absent_timeout          → re-arm
re-arm 後再次出現                → SEND
```

- **不是固定視窗，特別不是 60 秒視窗。**
- `absent_timeout` 預設 4000 ms（CLAUDE.md 14 的 3–5 s 目標中位），
  以 `AbsentTimeout::DEFAULT_MS` 具名常數表示，**可設定且以 Reader 為單位設定**。
- 邊界：離開時間「等於」timeout 尚未 re-arm，需**大於**才 re-arm。
- **Station 停留時間永遠不得當作抑制時間。**
- 每次讀取都會延長 presence，所以 Tag 在天線前停留整站也不會中途 re-arm。
- 重開機清空 presence：該狀態在 ESP32 上是 RAM。

實際數值必須在場地以真實天線驗證後調整。

### 6.2 Journal（CLAUDE.md 18）

append-only log + ACK cursor + ring buffer。

- 預設容量 10,000 筆（`JournalConfig::DEFAULT_CAPACITY`）。
- **未 ACK 的事件永不刪除**。容量用盡且無可回收空間時回報
  `JournalError::Full`，不覆寫。
- 已 ACK 的項目不立即抹除，等到需要空間時**成批**回收
  （預設 256 筆，`DEFAULT_RECLAIM_BATCH`）。
- 重開機保留未 ACK 事件；`boot_id` 遞增、`sequence` 歸零。
- ACK 遺失是安全的：重送即可，Hub 判重。
- 重複 ACK 是安全的：不會破壞 cursor。
- 使用率達門檻（預設 80%）時於 status topic 發布警告
  （`JOURNAL_NEARLY_FULL` / `JOURNAL_FULL`）。

### 6.3 Status 訊息

```json
{
  "device_id": "esp32-a4cf128b3d91",
  "boot_id": 18,
  "pending_events": 8123,
  "journal_capacity": 10000,
  "warning": "JOURNAL_NEARLY_FULL"
}
```

retained：新啟動的 Hub 應立刻看見在它上線前就已告警的裝置（CLAUDE.md 21）。

---

## 7. 未決議 / 待確認（CLAUDE.md 28）

| 項目 | 現況 | 需要誰確認 |
|---|---|---|
| `ReaderId` 折疊大小寫 | 已折疊為小寫 | 產品／韌體：若現場 Reader 標籤大小寫有意義，需改回保留原樣 |
| Topic 命名 `hyrox/v1/...` | 本次提出 | 韌體：需與 ESP32 端一致 |
| 每筆 ACK vs 批次 ACK cursor | 目前每筆一則 ACK | 若 10,000 筆 backlog 重送時流量過大，需改批次 |
| `absent_timeout` 實際值 | 預設 4000 ms | 場地實測 |
| Journal 告警門檻 80% | 本次提出 | 營運：多早通知才來得及處理 |
| ESP32 時間同步訊息格式 | 只保留了 topic，未定義 payload | 韌體 |
| Broker 認證 / ACL | 未處理 | 部署（CLAUDE.md 28 網路設計未決） |
| 已 ACK 事件的保留期 | 僅實作容量回收 | **韌體**：CLAUDE.md 18 寫「當前 + 前一個 Session」，但 §8 規定邊緣不得知道 Session。見 `docs/open-issues.md` |
| `reader_id` 是否為 Reader 自身的 MAC | 目前視為獨立於 `device_id` 的識別碼 | **韌體**：若每個 Reader 各有 MAC，「一台 ESP32 多個 Reader」的模型需重新確認 |

---

## 8. 測試位置

| 主題 | 檔案 |
|---|---|
| 事件契約、idempotency key、官方時間 | `crates/contract/tests/protocol.rs` |
| Topic 配置 | `crates/transport/tests/topics.rs` |
| ACK 協定、commit 前不得 ACK | `crates/contract/tests/ack.rs` |
| Presence / re-arm 抑制 | `crates/simulator/tests/suppression.rs` |
| Journal 語意 | `crates/simulator/tests/journal.rs` |
| 單一裝置：Reader、Tag、重開機 | `crates/simulator/tests/device.rs` |
| 斷線／重送／重複／亂序／ACK 遺失 | `crates/simulator/tests/fleet.rs` |

全部不需要 MQTT broker、不需要 RFID 硬體（CLAUDE.md 24）。
`crates/transport` 的 `broker` feature 關閉後，連 rumqttc 都不會進入建置；
`crates/contract` 本來就沒有 rumqttc。
