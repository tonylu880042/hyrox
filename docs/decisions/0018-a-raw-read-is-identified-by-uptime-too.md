# ADR 0018 — 判重要比對 `uptime_ms`：被清除過的裝置會重用判重鍵

- 狀態：**已實作**（2026-10-13）
- 日期：2026-10-13
- 影響範圍：`raw_events`（migration 0018）、`crates/storage`（判重查詢）、
  `crates/application`（`RawRead` 多一個欄位）、`docs/event-protocol.md` §3
- 不影響：MQTT 上行格式、ACK 格式與語意、邊緣與 ACK 使用的判重鍵
  （仍是 `device_id + boot_id + sequence`）、`detected_at` 為官方時間的原則
- 來源：韌體協議文件 v1.1 第 3 節、`hub-followup.md` B3

## 背景

`boot_id` 存在裝置的 NVS。裝置的 flash 被整片清除後，`boot_id` 從 1 重新計數，
它之後送出的事件就會帶著 Hub **已經存過**的 `(device_id, boot_id, sequence)`。

改動前，`raw_events` 的唯一性是 `(device_id, boot_id, sequence, tag_id)`，判重查詢也只比這四欄。
所以清除後的新讀取只要剛好是同一張卡，Hub 就會：

1. 找到舊的那一列，判定為重送；
2. 回 `DUPLICATE`；
3. 裝置收到 ACK，把事件從 journal 刪掉。

這筆讀取從此不存在，而且沒有任何錯誤——正是 CLAUDE.md 31 排在第一位、不允許發生的事。
同一張卡撞上同一個鍵聽起來罕見，但 `boot_id = 1, sequence = 0` 是每台裝置出廠後的第一筆讀取，
安裝測試時刷的往往就是同一張測試卡。

`uptime_ms` 一直都在線路上，而 Hub 解碼後直接丟掉，沒有存進 `raw_events`。

## 決定

**一筆讀取 = 判重鍵 + `tag_id` + `uptime_ms`。**

| 收到的事件 | 處理 | ACK |
|---|---|---|
| 鍵、卡、`uptime_ms` 都與已存的相同 | 不再存 | `DUPLICATE` |
| 鍵與卡相同，`uptime_ms` 不同 | **存成新的一列**，並在 log 標記 | `STORED` |
| 鍵與卡相同，已存的那列沒有 `uptime_ms`（migration 之前寫入的） | 視為相同，不再存 | `DUPLICATE` |

- **不比對 `detected_at`。** 裝置是在送出的那一刻才由 `uptime_ms` 換算 `detected_at`，
  兩次送出之間若發生對時修正，重送的值會差幾毫秒到幾十毫秒（韌體協議 4.2 第 5 點）。
  已存的那個時間不會被重送覆蓋。
- **邊緣與 ACK 講的鍵沒有變。** ACK 仍然只帶 `device_id + boot_id + sequence`，
  `uptime_ms` 只在 Hub 內部用來分辨「同一輪」與「撞號」。
- **標記**是 hub 的一行 log（`raw event key reused: ...`）。持久的紀錄就是資料本身：
  同一組鍵與卡底下有超過一列。
- `raw_events.uptime_ms` 對舊資料是 `NULL`，不是 0：沒有人記下它，補一個值等於捏造讀取內容
  （CLAUDE.md 19）。

## 沒有採用的做法

- **只記 log、仍回 `DUPLICATE`。** 讀取還是遺失，只是多了一行沒人會看的 log。
- **撞號時不回 ACK。** 事件不會遺失，但會永遠卡在裝置的 journal 裡每 10 秒重送一次，
  而且從未被解讀——選手的這一筆等於不存在。
- **以 status 的 `boot_id` 倒退偵測裝置被清除，再把舊資料標成另一個世代。**
  需要 Hub 持久記住每台裝置見過的最大 `boot_id`，而且 status 與事件的到達順序不保證。
  `uptime_ms` 已經在每一則事件裡，直接比對即可。

## Migration 0018

放寬資料表層級的 `UNIQUE` 在 SQLite 必須重建資料表。沿用 0007 的做法：
舊表改名讓出名字（子表的外鍵會跟著指向 `raw_events_old`）→ 建新表並原樣複製 →
重建兩個子表（`interpreted_events`、`raw_interpretation_context`）→ 刪舊表。
每一列、每一個 id 都原樣保留。

順帶修復一個既有問題：0007 用 `CREATE INDEX IF NOT EXISTS idx_raw_by_tag` 建索引時，
舊表還佔著這個名字，所以那一句什麼都沒做，索引隨後跟著舊表被刪掉。
0007 之後的每個資料庫都沒有這個索引，以卡號查讀取（報到佇列、追溯認領）一直是全表掃描。
0018 在最後重新建立它。

## 可攜性

沒有任何平台相依：只動 SQL、`crates/storage` 與一個資料欄位，macOS 與 Linux 行為相同。

## 測試

- `crates/storage/tests/migration_0018.rs`：對已有讀取、解讀、解讀情境的資料庫套用 0018，
  id 與關聯不變、外鍵檢查乾淨、撞號可存、真正的重複仍被擋下、`idx_raw_by_tag` 存在。
- `crates/storage/tests/port.rs`：撞號是新讀取；`detected_at` 不同仍是同一筆；
  沒有 `uptime_ms` 的舊資料仍然判重。
- `crates/application/tests/ingest.rs`：撞號的事件會被解讀，ACK 為 `STORED`。
