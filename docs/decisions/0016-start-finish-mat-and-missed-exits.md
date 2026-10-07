# ADR 0016 — 起終點感應墊（START_FINISH）與漏讀出站

- 狀態：已決議
- 日期：2026-10-08
- 影響範圍：`crates/domain`（`ReaderMode`、`Interpreted`、`StationRun`、`FinishPolicy`、
  `expectation`）、`crates/storage`（migration 0016、重播）、`crates/application`
  （例外改判）、比賽畫面的分段名稱
- 不影響：MQTT 上行格式、ESP32 韌體、判重鍵、ACK 協定、`raw_events`

## 背景

場館 RFID 配置（2026-10-08 團隊討論圖）：

```text
1S 起終點感應墊 ─跑1─▶ 2S SkiErg 2N ─跑2─▶ 3S … 8N ─跑8─▶ 9S 拋藥球 ─▶ 9N（= 1S 同一片墊）
```

站 1–7 有專屬的進／出站閘門，現有的 ENTRY / EXIT 可直接對應。三件事做不到：

1. **起跑**：只有 `Entered` 能啟動計時。1S 不論設成哪種模式，都會變成例外或讓選手
   「待在起點站內」，第 1 段跑步因此遺失。
2. **同一片墊，兩種意思**：同一個 `(device, reader)` 只能有一筆註冊。這片墊在選手出發前
   是起跑，在選手位於拋藥球站內時是該站出站兼完賽；拋藥球站沒有自己的出站閘門。
3. **漏讀出站會卡死**：2N 漏讀後，選手停在 SkiErg 站內；之後每一筆都是
   `IMPOSSIBLE_TRANSITION`，選手永遠無法完賽。RF 漏讀在現場一定會發生。

另已確認（使用者，2026-10-08）：**跑步路線會經過起終點感應墊**。

## 決議

### 1. 新讀取器角色 `START_FINISH`

註冊時的 `station` 填這片墊**負責收尾的站**（例：`WALL_BALLS`）。解讀依選手狀態：

| 選手狀態 | 解讀 | 效果 |
|---|---|---|
| READY | `Started` | 開始計時；起跑時刻同時作為「上一次出站」 |
| 在 `station` 站內 | `Exited { station }` | 由既有的 `CourseComplete` 結算完賽 |
| 其他（含已完賽） | `Passed { station }` | 只記錄，不改狀態，不進例外收件匣 |

起跑時刻作為 `last_exit_at`，於是「第 1 段跑步 = 2S − 1S」由既有的 transition 公式
直接算出，不需要另一套算法。

中途路過記成 `Passed` 而不是例外：路線會經過墊子，若記成例外，每位選手每圈都會在
收件匣產生一筆，真正需要處理的例外會被淹沒。原始讀取照常保存（CLAUDE.md 19）。

### 2. 漏讀出站：繼續計時，標示「出站時間不明」

選手在站 A 內，卻在**另一站** B 被讀到進站（ENTRY 或 TOGGLE）：

- 記錄 B 的 `Entered`，`transition = None`（上一站的出站時刻不知道，無從相減）。
- A 這一趟標記 `exit_missed`，`exited_at` 維持 `None`——不捏造任何讀取器沒看到的時刻
  （CLAUDE.md 19）。A 的站內時間與其後的跑步分段留白。
- `exit_missed` 的一趟仍算「已完成的站」，所以 `CourseComplete` 與站點順序判斷不會卡住。

**同一站**在站內時再讀到進站，仍是 `IMPOSSIBLE_TRANSITION`：那是重複讀取，不是漏讀。

不需要新增欄位：`exit_missed` 由重播時「站內又進另一站」推導，與即時解讀走同一個函式
（`domain::rederive`），重播與即時路徑因此不會分歧（CLAUDE.md 21）。

### 3. 比賽畫面把分段叫「跑步 k」

這個配置沒有 Roxzone 感應墊，「上一站 N → 下一站 S」是跑步加上 Roxzone 移動。
在比賽模式標示為 ROX 會誤導；改稱「跑步 k」。訓練模式維持「轉換」。

## 結構變更

- `interpreted_events.kind` 新增 `STARTED`、`PASSED`；`readers.mode` 新增
  `START_FINISH`。SQLite 不能修改 CHECK，migration 0016 以重建表的方式完成，所有 id 與
  欄位原樣搬移（同 migration 0007 的做法）。

## 考慮過的替代方案

- **借用 `PASSAGE`**：不需要新模式，但在讀取器設定畫面上「通過 @ 拋藥球」看不出它是
  起終點，而 `PASSAGE` 未來可能需要別的語意。選擇明確的名稱。
- **漏讀時以下一站進站時刻補出站**：計時會連續，但站內時間會吃掉整段跑步，
  而且寫入了一個沒有任何讀取器觀測到的時刻。否決。
- **維持例外、全部人工處理**：最保守，但人數一多操作人員跟不上。否決。

## 仍未決（記於 `docs/open-issues.md`）

- 9S 也漏讀時，衝線只會記成 `Passed`：路線經過墊子時，系統分辨不出「路過」與「衝線」。
  由操作人員在例外改判處理。
- 漏讀**進站**（3S 未讀到、直接讀到 3N）維持例外；是否比照做「進站時間不明」待決。

## 可攜性

純 domain 與 SQL 變更，不涉及任何作業系統 API；macOS 與 Linux 行為相同。
