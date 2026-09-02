# RFID Reader 接入指南（給讀取器／韌體團隊）

本檔是**你們唯一需要照著實作的東西**：把一次 Tag 讀取，變成一則 JSON，發到一個 MQTT topic。
Hub 端的完整契約與理由寫在 [event-protocol.md](event-protocol.md)，本檔是它的實作面摘要。

**契約變更必須雙方同步**（CLAUDE.md 30）。有疑問的欄位請先看第 8 節「待確認」，不要自行猜測。

---

## 1. 一句話

```text
Tag 被讀到
  → 讀取器／ESP32 寫入本機 journal
  → 以 QoS 1 發到 hyrox/v1/edge/<device_id>/events
  → Hub 存進資料庫
  → Hub 回 ACK 到 hyrox/v1/hub/<device_id>/ack
  → 你才可以把這筆從 journal 釋放
```

Hub **不直接連讀取器**，只收 MQTT。讀取器**不需要知道**什麼是 Station、什麼是比賽、
誰是選手——那些對應全部在 Hub 端做（CLAUDE.md 8）。你只要誠實回報：
**哪一台裝置、哪一個讀頭、哪一張 Tag、什麼時候讀到的。**

---

## 2. 身分：`device_id` 就是 MAC

`device_id` = **Base MAC 的 12 位十六進位小寫、不含任何分隔符號**。沒有前綴。

| MAC | `device_id` |
|---|---|
| `A4:CF:12:8B:3D:91` | `a4cf128b3d91` |
| `a4-cf-12-8b-3d-91` | `a4cf128b3d91` |

規則：

- **一台硬體一個 id，重新燒錄不得改變。** 不要用隨機 UUID、不要用開機時間、不要用 DHCP 得到的 IP。
- 線路上**只接受正規形式**：大小寫皆可解析（Hub 會轉小寫），但**不接受 `:`、`-`、`.` 分隔符號**。
  `A4:CF:12:8B:3D:91` 會被判為 malformed 而**整筆拒收**。

錯誤示範（皆會被拒）：

```text
A4:CF:12:8B:3D:91       ← 有分隔符號
a4cf128b3d9             ← 只有 11 位
a4cf128b3dzz            ← 非十六進位
esp32-a4cf128b3d91      ← 舊格式，前綴已取消（ADR 0015）
```

### 為什麼要去掉分隔符號

不是潔癖。`:` `-` `.` 三種分隔符號 × 大小寫 = 同一台機器**六種拼法**。
Hub 用 `device_id + reader_id` 查 Station 對應表，拼法不一致就查不到，
而失敗是**安靜**的——不會報錯，只是那台讀取器的每一筆讀取都被歸成 `UNKNOWN_READER` 例外，
要等到有人翻例外清單才會發現。

人寫的設定檔可以照你們習慣打 `A4:CF:12:8B:3D:91`，Hub 端的設定介面會正規化。
**線路上請送正規形式。**

---

## 3. 身分：`reader_id` —— 哪一個讀頭

`reader_id` 與 `device_id` **分開**，因為一台裝置未來可能掛多個讀頭／天線。

- 字元限定：**英數字、`-`、`_`**，不可為空。其他字元整筆拒收。
- **不分大小寫**：`RFID-02` 與 `rfid-02` 是同一個（Hub 一律轉小寫）。
- 建議用固定編號，寫在韌體設定裡，不要動態產生：`rfid-01`、`rfid-02`……
- 這個值是 Hub 查對應表的鍵：

```text
device_id + reader_id  →  Station / Zone / Reader Mode（ENTRY、EXIT、TOGGLE…）
```

  所以**它一旦上線就不能改**，改了等於換一台讀頭，Hub 會查不到對應。

### 3.1 如果「一台讀取器就是一台獨立裝置」（各自有 MAC，沒有匯聚板）

照樣可用，對應方式：

```text
device_id = 該讀取器自己的 MAC → 12 位小寫 hex
reader_id = "rfid-01"（該裝置上唯一的讀頭）
```

不要把 MAC 塞進 `reader_id`。`reader_id` 是**裝置內**的讀頭編號，MAC 只出現在 `device_id`。
若你們的架構真的是一機一讀頭，請在第 8 節那一列跟我們確認一次。

---

## 4. 事件 JSON（唯一必須送對的東西）

Topic：

```text
hyrox/v1/edge/<device_id>/events        QoS 1
```

例：`hyrox/v1/edge/a4cf128b3d91/events`

Payload：

```json
{
  "device_id": "a4cf128b3d91",
  "reader_id": "rfid-02",
  "boot_id": 18,
  "sequence": 10382,
  "tag_id": ["E280117000001234"],
  "detected_at": 1787734821382,
  "uptime_ms": 382912
}
```

| 欄位 | 型別 | 必填 | 說明 |
|---|---|---|---|
| `device_id` | string | ✔ | 第 2 節。必須與 topic 中的 `<device_id>` 一致 |
| `reader_id` | string | ✔ | 第 3 節 |
| `boot_id` | 整數 ≥ 0 | ✔ | **開機序號**，每次開機 +1，存在 NVS／flash，不可歸零 |
| `sequence` | 整數 ≥ 0 | ✔ | 該次開機內遞增的事件序號，**開機後從 0 重新起算** |
| `tag_id` | **非空字串陣列** | ✔ | 該輪 inventory 中所有新出現的 Tag。EPC 原樣，建議大寫 hex；Hub 會 trim 並轉大寫後比對 |
| `detected_at` | 整數 ≥ 0 | ✔ | **偵測當下**的 epoch 毫秒。這是**官方計時來源** |
| `uptime_ms` | 整數 ≥ 0 | ✔ | 本次開機經過毫秒，診斷用 |

七個欄位全部必填，**缺一個就整筆拒收**。多送的欄位會被忽略，但請不要送。

### 4.1 三件最容易做錯的事

1. **`detected_at` 是「讀到的那一刻」，不是「發出去的那一刻」。**
   事件在 journal 裡壓了 30 秒才補送，`detected_at` 仍然是 30 秒前那個值。
   Hub 另外記自己的收件時間，只用來診斷，**永遠不拿來算成績**（CLAUDE.md 17）。

2. **`boot_id` 不可以省、不可以固定為 0。**
   `device_id + boot_id + sequence` 是判重的鍵。少了 `boot_id`，重開機後 `sequence` 歸零
   會與上一次開機的事件撞號，**真實事件會被當成重複而永遠消失**——這是本專案第一優先要防的事。

3. **負數一律拒收。** 計數器來自單調遞增的硬體，出現負值代表資料損毀。

---

## 4.2 UHF：一次讀到多張 Tag 怎麼送

UHF 的 anti-collision 讓一輪 inventory 同時回報多張 Tag。**一輪就是一則訊息，`tag_id` 放整輪。**

```json
{
  "device_id": "a4cf128b3d91",
  "reader_id": "rfid-02",
  "boot_id": 18,
  "sequence": 10382,
  "tag_id": ["E280117000001234", "E280117000005678", "E28011700000ABCD"],
  "detected_at": 1787734821382,
  "uptime_ms": 382912
}
```

兩個層級不要混淆：

```text
你送出的單位 = 一輪 inventory     一則訊息、一個 sequence、一則 ACK
Hub 紀錄的單位 = 一張 Tag         一張一列，各自對到一個人
```

| 事項 | 規則 |
|---|---|
| `sequence` | **一輪一個**，不是一張一個 |
| `detected_at` | 整輪共用該輪的偵測時刻 |
| `tag_id` 內容 | 只放**新出現**的 Tag。仍持續在場、被抑制的不要放進去 |
| 空陣列 | **不可送**。整輪沒有新 Tag 就不要發訊息 |
| 空字串項目 | **不可送**。整則會被拒收 |
| ACK | 一輪一則。收到就代表整輪都已持久化，可以把這一筆從 journal 釋放 |
| 整輪被抑制 | 不發訊息，且**不消耗 `sequence`** —— 序號有洞看起來就像事件遺失 |

舊的單一字串形式（`"tag_id": "E28..."`）**已不再接受**，會被判為 malformed 而整筆拒收。
這是刻意的：讓它在整合期大聲失敗，而不是在場地安靜失敗。

Hub 端會把一輪展開成每張 Tag 各自的判讀。一輪中某張未綁定，不影響同一輪的其他張——
他們是不同的人。

### 抑制必須是「每個讀頭 × 每張 Tag」

這是 UHF 最容易做錯的地方：

```text
正確：presence[reader][tag] = last_seen
錯誤：presence[reader]      = last_seen   ← 場中還有人，就會壓住已經離開的那張
```

團體課十個人站在天線前，其中一個先離開再回來，他必須要能產生新事件。
若抑制狀態只記到讀頭層級，其他九個人持續在場會把他的 re-arm 一直往後推，
他的第二次進站就永遠不會被記錄——這是本專案第一優先禁止的事（CLAUDE.md 31）。

一輪的處理順序：

```text
對該輪每一張 Tag 各自過 presence
  → 收集判定為「新出現」的那些
  → 若非空：取一個 sequence，發一則訊息，tag_id 放這些
  → 若全空：不發，不取號
```

模擬器已按此實作並由測試釘住（`crates/simulator/tests/device.rs`）：
`a_uhf_inventory_round_travels_as_one_event`、
`presence_is_per_tag_so_one_tag_leaving_a_crowd_re_arms_alone`、
`a_round_where_every_tag_is_suppressed_consumes_no_sequence_number`。
`SimDevice::rf_inventory(reader, &[tags], now_ms)` 就是這段邏輯的參考實作。

### 讀取距離：UHF 真正的風險

UHF 讀得遠，所以會讀到**不該讀到的人**——走過旁邊的、在隔壁站休息的、
背包裡有別人 Tag 的。Hub 無法分辨「站在天線前的選手」與「路過的人」，
它只看得到一則事件，**過濾必須在讀取器端做**：

- 用 **RSSI 門檻**擋掉遠處的 Tag，門檻要可設定、要在場地實測。
- 用功率／天線角度限制讀取範圍，而不是靠軟體事後補救。
- 寧可**漏讀一次可補**，也不要**誤讀一次進錯站**：誤讀會產生錯誤的成績且很難察覺，
  漏讀會被現場立刻發現並人工修正（Hub 有修正與稽核機制，CLAUDE.md 20）。

目前事件 JSON **沒有 `rssi` 欄位**。若你們判斷 Hub 端也需要看到 RSSI 來調參，
請提出，我們再加（加欄位＝改契約，要同步改測試與本文件）。

### 量的估算

一站 20 人的團體課，每人進站一次、出站一次，一輪 inventory 每 200 ms 一次：
每人整站只會產生 **2 則事件**，不是每輪一則——presence 抑制吃掉了中間所有重複讀取。
10,000 筆的 journal 容量對這個量級綽綽有餘。

---

## 5. ACK：什麼時候可以把事件從 journal 刪掉

Hub 存進資料庫**之後**才回 ACK，發到：

```text
hyrox/v1/hub/<device_id>/ack            QoS 1
```

```json
{
  "device_id": "a4cf128b3d91",
  "boot_id": 18,
  "sequence": 10382,
  "status": "STORED"
}
```

`status` 只有兩個值，**兩者都代表可以釋放該筆**：

| status | 意思 |
|---|---|
| `STORED` | 新存入 |
| `DUPLICATE` | Hub 早就存過同一個鍵，一樣可靠 |

沒收到 ACK ＝ 沒存成功 ＝ **保留該筆，稍後重送**。

- 重送是安全的，Hub 會判重。
- 重複收到 ACK 是安全的，不要因此破壞 cursor。
- 收到不認得的 `boot_id`／`sequence` 的 ACK，忽略即可，不要當成錯誤。

---

## 6. 抑制與 journal（行為契約，不是建議）

### 6.1 讀取抑制：Presence / Re-arm（CLAUDE.md 14）

```text
第一次看到 Tag              → 送出事件
同一張 Tag 持續在天線前      → 抑制，不重複送
Tag 消失超過 absent_timeout → re-arm
re-arm 後再次出現           → 送出新事件
```

- **不是固定時間視窗，特別不是 60 秒。**
- `absent_timeout` 預設 **4000 ms**，必須**可設定**，且**以讀頭為單位**可設定。
- 邊界：消失時間**等於** timeout 尚未 re-arm，要**大於**才 re-arm。
- 每次讀到都會延長 presence，所以選手在天線前站整站也不會中途誤發第二筆。
- **絕對不可以拿「一站需要多久」當抑制時間。** 抑制是 RF 層的事，站點時長是 Hub 的事。
- 重開機時 presence 清空（它是 RAM 狀態），這是預期行為。

實際數值要在場地用真實天線量過再定。請把它做成可調參數，不要編死。

### 6.2 未 ACK 事件必須落地（CLAUDE.md 18）

- 容量目標：**每台 10,000 筆**。
- **未 ACK 的事件永遠不可刪除。** 空間用盡且無可回收空間時回報錯誤，**不要覆寫**。
- 已 ACK 的不要一筆一筆抹除，等需要空間時**成批**回收（建議 256 筆一批）。
- 斷線重連要重送未 ACK 的；重開機要保留未 ACK 的。
- 重開機後：`boot_id` +1、`sequence` 歸零，**未 ACK 的舊事件照原本的 `boot_id`／`sequence` 重送**。

### 6.3 健康狀態

Topic：

```text
hyrox/v1/edge/<device_id>/status        QoS 1, retained
```

```json
{
  "device_id": "a4cf128b3d91",
  "boot_id": 18,
  "pending_events": 8123,
  "journal_capacity": 10000,
  "warning": "JOURNAL_NEARLY_FULL"
}
```

`warning` 健康時送 `null`（請保留欄位）。目前定義的值：
`JOURNAL_NEARLY_FULL`（建議用量 80% 觸發）、`JOURNAL_FULL`。

**必須 retained**：Hub 重啟後要立刻看見它離線期間就已告警的裝置。

---

## 7. 自測

Hub 端：`cargo run -p hub-server`，需要本機 broker（`mosquitto -p 1883`）。

手動打一筆進去：

```bash
mosquitto_pub -t 'hyrox/v1/edge/a4cf128b3d91/events' -q 1 -m '{"device_id":"a4cf128b3d91","reader_id":"rfid-02","boot_id":1,"sequence":1,"tag_id":["E280117000001234"],"detected_at":1787734821382,"uptime_ms":382912}'
```

同時在另一個視窗看 ACK 有沒有回來：

```bash
mosquitto_sub -t 'hyrox/v1/hub/+/ack' -v
```

沒有 ACK，就對照下表：

| 症狀 | 原因 |
|---|---|
| 完全沒有 ACK，Hub 日誌有 malformed | JSON 欄位缺漏／型別錯／`device_id` 或 `reader_id` 格式不合／`tag_id` 送成字串而不是陣列 |
| 完全沒有 ACK，Hub 日誌沒東西 | topic 打錯，或發到了 `hub/` 分支（上行一律是 `edge/`） |
| 一直收到 `DUPLICATE` | `boot_id`／`sequence` 沒有遞增，或重開機時 `boot_id` 沒有 +1 |
| 一輪多張，只有一個人被記錄 | `tag_id` 只放了一張。整輪的新 Tag 都要放進同一個陣列 |
| 成績時間看起來全部偏移 | `detected_at` 送的是發送時間或裝置未對時 |

Hub 對**無法解碼的 payload 不回 ACK 也不入庫**，但一定留下日誌（含 topic 與錯誤原因），
且不會因為一筆壞資料就停止收件——一台壞掉的裝置不得停掉一堂課。

---

## 8. 待你們確認

| 項目 | Hub 目前的假設 | 需要你們回答 |
|---|---|---|
| `reader_id` 是不是讀頭自己的 MAC | 視為**裝置內的編號**，與 `device_id` 無關 | 若每個讀頭各有 MAC，我們要重新確認「一台裝置多個讀頭」的模型 |
| 一台 ESP32 掛幾個讀頭 | 支援 1..N | 實際幾個？ |
| `tag_id` 格式 | 陣列；元素不限長度與字元集，只 trim + 轉大寫 | 實際 EPC 長度與編碼 |
| 單輪最多幾張 Tag | 無上限假設 | 實際峰值（決定 journal 單筆大小） |
| `absent_timeout` | 預設 4000 ms，可調 | 場地實測值 |
| 時間同步 | 只保留了 topic `hyrox/v1/hub/time`，**payload 未定義** | 你們希望的對時方式（NTP？還是走這個 topic？） |
| 每筆一則 ACK | 目前如此 | 若 10,000 筆 backlog 重送時流量吃不消，改批次 cursor |
| 事件是否需要 `rssi` 欄位 | 目前**沒有**此欄位 | 若 Hub 端也要看 RSSI 調參，提出後再加（改契約） |
| inventory 輪詢週期 | 無假設，抑制邏輯與週期無關 | 實際週期與單輪最多回報幾張 Tag |
| RSSI 門檻 | 讀取器端自行過濾，Hub 不介入 | 場地實測值 |
| Broker 認證／ACL | 未處理 | 部署時決定 |

不確定的請直接問，不要先實作再對答案。未決事項見 [open-issues.md](open-issues.md)。
