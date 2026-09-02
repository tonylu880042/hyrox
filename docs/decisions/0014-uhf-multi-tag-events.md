# ADR 0014 — UHF：一次 inventory 一則事件，`tag_id` 為陣列

- 狀態：已決議
- 日期：2026-09-02
- 影響範圍：`crates/contract`（線路契約）、`crates/storage`（`raw_events` 唯一鍵、migration 0007）、
  `crates/application`（ingest 展開）、`crates/simulator`（`rf_inventory`）、
  `docs/event-protocol.md`、`docs/reader-integration.md`
- 不影響：`domain`（`TagId` 與判讀規則未變）、ACK 協定的型別保證（ADR 0002 原封不動）、
  MQTT topic 配置（ADR 0005/0006）

## 背景

現場採用 **UHF RFID**。UHF 的 anti-collision 讓一輪 inventory 同時回報視野內的**多張** Tag，
而不是一次一張。原契約的 `tag_id` 是單一字串（CLAUDE.md §16），代表韌體必須把一輪拆成 N 則訊息、
自行分配 N 個 `sequence`。

CLAUDE.md §30 規定契約不得靜默變更，因此這是一次明示的契約變更。

## 決議

**`tag_id` 改為字串陣列，承載該輪 inventory 中所有「新出現」的 Tag。**

```json
{
  "device_id": "a4cf128b3d91",
  "reader_id": "rfid-02",
  "boot_id": 18,
  "sequence": 10382,
  "tag_id": ["E280117000001234", "E280117000005678"],
  "detected_at": 1787734821382,
  "uptime_ms": 382912
}
```

分成兩個層級來理解：

```text
投遞單位 = 一輪 inventory
  一則訊息、一個 sequence、一則 ACK
  判重鍵仍是 device_id + boot_id + sequence（CLAUDE.md 16 未變）

紀錄單位 = 一張 Tag
  raw_events 一張 Tag 一列
  interpreted_events 一個人一列
```

### 為什麼投遞單位是「輪」

- ACK 的粒度必須等於邊緣 journal 的粒度。韌體 journal 裡的一筆就是一輪，
  一則 ACK 就該釋放一筆，否則邊緣得追蹤「這一輪的五張裡有三張被 ACK 了」的部分狀態。
- 判重鍵不必改。`device_id + boot_id + sequence` 仍然唯一標定一則訊息，
  ADR 0002 的型別保證（`Ack` 只能由 `Commit` 鑄造）一個字都不用動。
- 韌體不需要為了湊足 `sequence` 而把一輪拆開重組。

### 為什麼紀錄單位是「Tag」

`raw_events` 若一輪一列、把 Tag 存成 JSON 陣列，下列既有查詢全部要重寫：

- `idx_raw_by_tag`（0003）
- 事後認領未歸屬讀取（ADR 0001 D3）
- 報到佇列的待綁定 Tag 清單

一張 Tag 一列則全部不動。代價只有一個：`raw_events` 的唯一鍵從
`(device_id, boot_id, sequence)` 加寬為 `(device_id, boot_id, sequence, tag_id)`。

SQLite 無法變更表約束，故 migration 0007 重建 `raw_events`，
沿用 ADR 0008/migration 0004 的「先改名、重建子表、最後刪舊表」配方
（sqlx 在交易內執行，`PRAGMA foreign_keys=OFF` 會被忽略，因此不能直接 `DROP`）。
`id` 逐筆保留：`interpreted_events.raw_event_id` 與稽核紀錄都以它為名（CLAUDE.md 19、20）。

### 展開語意

`application::ingest_read` 的回傳由 `outcome` 改為 `outcomes: Vec<IngestOutcome>`，
每張 Tag 一個，順序即讀取器回報的順序。

- **先全部 commit，才鑄造 ACK。** 中途失敗 → 無 ACK → 邊緣重送整輪 →
  已存在的以判重鍵略過、缺的補上。整輪就此收斂，不需要交易。
- `CommitOutcome::AlreadyStored` 只在**整輪每一張**都已存在時回報。
  半存的輪視為未完成，重送會把它做完。
- 一輪中某張未綁定，不影響同一輪其他張的判讀：他們是不同的人。

### 抑制：presence 必須是「每讀頭 × 每張 Tag」

已是 `crates/simulator` 的實作，本次以測試釘住。若 presence 只記到讀頭層級，
場中持續有人會把已離開者的 re-arm 一直往後推，他的下一次進站永遠不會產生事件——
這正是 CLAUDE.md §31 第一優先禁止的靜默遺失。

`SimDevice::rf_inventory(reader, &[tags], now_ms)` 是韌體的參考行為：
逐張過 presence，把「新出現」的收成一則事件；整輪皆被抑制則不發、且**不消耗 `sequence`**。
`rf_read` 保留為單張 Tag 的一輪。

## 替代方案

| 方案 | 不採用的原因 |
|---|---|
| 維持單張 Tag，一輪拆成 N 則訊息 | 韌體要自行拆輪並分配 N 個 `sequence`；同一輪的同時性在線路上消失，Hub 只能靠 `detected_at` 相等去猜 |
| 陣列 + 每張 Tag 各自的 ACK | ACK 粒度與 journal 粒度不一致，邊緣需維護「一輪中哪幾張已 ACK」的部分狀態 |
| `raw_events` 一輪一列、Tag 存 JSON | `idx_raw_by_tag`、事後認領、報到佇列三處查詢全部要重寫 |
| 同時接受字串與陣列（相容過渡） | 尚無已部署韌體，相容路徑只會讓兩種拼法長期並存。線路上拒收字串形式，整合期就會大聲失敗，而不是在場地安靜失敗 |

## 可攜性影響

無。契約是資料，storage 是 SQLite，兩者都不觸及平台 API（CLAUDE.md 2）。

## 未決

- 事件是否需要 `rssi` 欄位。目前不加：誤讀過濾放在讀取器端（讀取距離、天線角度、RSSI 門檻）。
  Hub 分辨不出「站在天線前的選手」與「路過的人」，事後補救不了。待韌體團隊回覆是否需要。
- 單輪最多幾張 Tag、inventory 週期實測值。見 `docs/reader-integration.md` §8。

## 測試

| 主題 | 檔案 |
|---|---|
| 陣列契約、空陣列與空字串拒收、拒收舊的字串形式 | `crates/contract/tests/protocol.rs` |
| 一輪一則事件、presence 每張 Tag、整輪抑制不消耗序號 | `crates/simulator/tests/device.rs` |
| migration 0007 對既有資料庫、id 保留、同輪多張可存 | `crates/storage/tests/migration_0007.rs` |
| 每張 Tag 各自判讀、一輪一則 ACK、重送半存的輪 | `crates/application/tests/ingest.rs` |
