# MQTT 上行格式 — RFID 感測端 → Central Hub

本檔只寫**一個方向**：感測端（RFID 讀取器／ESP32）發給 Hub 的 MQTT 訊息長什麼樣。
兩個 topic、兩種 payload，沒有別的。

```text
RFID Reader ──► ESP32 / 採集端 ──MQTT QoS 1──► Central Hub
                                  ▲
                              本檔的範圍
```

- 完整接入流程（journal、抑制、重送、待確認事項）：[reader-integration.md](reader-integration.md)
- Hub 端契約與設計理由：[event-protocol.md](event-protocol.md)
- 欄位形狀由測試釘住：`crates/contract/tests/protocol.rs`、`crates/transport/tests/inbound.rs`

**契約變更必須雙方同步**（CLAUDE.md 30）。

---

## 1. 連線參數

| 項目 | 值 |
|---|---|
| Broker | 場地內網 Mosquitto，預設 `1883` |
| QoS | **1**（events 與 status 都是） |
| `clean_session` | `false` |
| Client ID | 每台裝置固定且唯一，建議直接用 `device_id` |
| Payload 編碼 | UTF-8 JSON，單層物件，無 BOM |
| Retained | events **否**；status **是** |

`clean_session = false` + 固定 client id：Hub 不在線時 broker 會替它排隊，Hub 重啟後照樣送達。
每次開機換 client id 會把這個佇列丟掉。

---

## 2. 上行 Topic（只有這兩個）

```text
hyrox/v1/edge/<device_id>/events     RFID 事件      QoS 1, 不 retained
hyrox/v1/edge/<device_id>/status     裝置健康狀態   QoS 1, retained
```

`<device_id>` 就是第 3 節的 12 位小寫 hex，例：

```text
hyrox/v1/edge/a4cf128b3d91/events
```

- 上行一律走 `edge/` 分支。`hub/` 是 Hub 發給你們的，**不要往那裡發**。
- Hub 以 `hyrox/v1/edge/+/events`、`hyrox/v1/edge/+/status` 訂閱。
- topic 裡的 `<device_id>` 與 payload 的 `device_id` **應該一致**；若不一致，Hub 以 **payload 為準**
  （判重鍵與 ACK 路由都看 payload），不會因此丟掉事件——但請不要靠這個行為。

---

## 3. 事件 payload — `.../events`

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

七個欄位**全部必填**，缺一個整筆拒收。多送的欄位會被忽略，但請不要送。

| 欄位 | JSON 型別 | 規則 | Hub 的正規化 |
|---|---|---|---|
| `device_id` | string | 12 位 hex，**不得含 `:` `-` `.`**，不得有前綴 | 轉小寫 |
| `reader_id` | string | 非空，只允許英數字、`-`、`_` | 去頭尾空白、轉小寫 |
| `boot_id` | integer | ≥ 0，每次開機 +1，存在 NVS／flash | 無 |
| `sequence` | integer | ≥ 0，該次開機內遞增，開機歸零 | 無 |
| `tag_id` | **string 陣列** | 非空陣列，元素不得為空字串或全空白 | 每項去頭尾空白、轉大寫 |
| `detected_at` | integer | ≥ 0，epoch **毫秒**，**偵測當下**的時間 | 無（官方計時來源） |
| `uptime_ms` | integer | ≥ 0，本次開機經過毫秒 | 無（診斷用） |

整數請送 JSON number，**不要加引號**（`"boot_id": "18"` 會被判為 malformed）。

### 3.1 三個必須送對的語意

1. **`detected_at` 是讀到的那一刻，不是發出去的那一刻。**
   壓在 journal 裡 30 秒才補送，`detected_at` 仍是 30 秒前的值。
   Hub 另記 `received_at`，只做診斷，永遠不拿來算成績。
2. **`boot_id` 不可省、不可固定 0。** 判重鍵是 `device_id + boot_id + sequence`；
   少了它，重開機後歸零的 `sequence` 會與上次開機撞號，真實事件被當成重複而消失。
3. **`tag_id` 一定是陣列。** 單一字串形式（`"tag_id": "E28..."`）已不接受，整筆拒收。

### 3.2 一輪 inventory = 一則訊息

UHF 一輪會同時讀到多張 Tag。**整輪放進同一個 `tag_id` 陣列，共用一個 `sequence`**。

```text
發送單位 = 一輪 inventory     一則訊息、一個 sequence、一則 ACK
紀錄單位 = 一張 Tag           Hub 一張一列，各自對到一個人
```

| 情況 | 怎麼做 |
|---|---|
| 該輪有 3 張新 Tag | 一則訊息，`tag_id` 放 3 個元素 |
| 該輪有新 Tag 也有持續在場的舊 Tag | 只放**新出現**的，持續在場的不放 |
| 該輪全部被抑制 | **不發訊息，且不取 `sequence`**（序號有洞看起來像事件遺失） |
| 該輪沒讀到任何 Tag | 不發訊息。空陣列會被拒收 |

---

## 4. 狀態 payload — `.../status`

```json
{
  "device_id": "a4cf128b3d91",
  "boot_id": 18,
  "pending_events": 8123,
  "journal_capacity": 10000,
  "warning": "JOURNAL_NEARLY_FULL"
}
```

| 欄位 | 型別 | 說明 |
|---|---|---|
| `device_id` | string | 同第 3 節 |
| `boot_id` | integer ≥ 0 | 本次開機序號 |
| `pending_events` | integer ≥ 0 | journal 中尚未被 ACK 的筆數 |
| `journal_capacity` | integer ≥ 0 | journal 總容量（目標 10,000） |
| `warning` | string 或 `null` | 健康時送 `null`，**請保留欄位** |

`warning` 目前只有兩個值：`JOURNAL_NEARLY_FULL`（建議 80% 觸發）、`JOURNAL_FULL`。

**必須 retained**：Hub 重啟後要立刻看到它離線期間就已告警的裝置。

---

## 5. 被拒收會怎樣

Hub 收到無法解碼的 payload：

```text
不入資料庫   （判重鍵取不出來）
不回 ACK     （沒有東西被持久化，就沒有 ACK 可發 → 你們保留該筆，稍後重送）
一定留日誌   （topic + 解碼錯誤 + payload 節錄）
不中斷訂閱   （一台壞掉的裝置不得停掉一堂課）
```

所以**送錯格式不會當掉，但也不會有人通知你**——整合期請自己盯 ACK。

### 常見拒收原因

| payload | 結果 |
|---|---|
| `"device_id": "A4:CF:12:8B:3D:91"` | 拒收，有分隔符號 |
| `"device_id": "esp32-a4cf128b3d91"` | 拒收，前綴已取消（ADR 0015） |
| `"device_id": "a4cf128b3d9"` | 拒收，只有 11 位 |
| `"reader_id": ""` 或 `"rfid 02"` | 拒收，空值／含不允許字元（空白） |
| `"tag_id": "E280117000001234"` | 拒收，不是陣列 |
| `"tag_id": []` | 拒收，空輪不是事件 |
| `"tag_id": ["E280...", ""]` | 拒收，含空元素 |
| `"sequence": -1` | 拒收，計數器不得為負 |
| `"boot_id": "18"` | 拒收，整數不要加引號 |
| 少任一欄位 | 拒收 |

`reader_id` 與 `device_id` 大小寫皆可（Hub 轉小寫），但格式不可有分隔符號。

---

## 6. 什麼時候可以把事件從 journal 刪掉

Hub 持久化成功**之後**才回 ACK，發到 `hyrox/v1/hub/<device_id>/ack`：

```json
{ "device_id": "a4cf128b3d91", "boot_id": 18, "sequence": 10382, "status": "STORED" }
```

`status` 為 `STORED` 或 `DUPLICATE`，**兩者都代表該筆可以釋放**。
沒收到 ACK ＝ 沒存成功 ＝ 保留並稍後重送（重送安全，Hub 會判重）。

ACK 的粒度是**一輪**，與你們送出的粒度相同：收到就代表整輪每一張 Tag 都已落地。

抑制、journal、重送的完整行為契約見 [reader-integration.md](reader-integration.md) §6。

---

## 7. 自測

Hub 端跑起來（需要本機 broker，`mosquitto -p 1883`）：

```bash
cargo run -p hub-server
```

一個視窗盯 ACK：

```bash
mosquitto_sub -t 'hyrox/v1/hub/+/ack' -v
```

另一個視窗手動打一筆：

```bash
mosquitto_pub -t 'hyrox/v1/edge/a4cf128b3d91/events' -q 1 -m '{"device_id":"a4cf128b3d91","reader_id":"rfid-02","boot_id":1,"sequence":1,"tag_id":["E280117000001234"],"detected_at":1787734821382,"uptime_ms":382912}'
```

再打一次同樣內容，應該收到 `DUPLICATE`——判重正常。

| 症狀 | 原因 |
|---|---|
| 沒有 ACK，Hub 日誌有 malformed | 格式問題，對照第 5 節 |
| 沒有 ACK，Hub 日誌沒東西 | topic 打錯，或發到了 `hub/` 分支 |
| 一直是 `DUPLICATE` | `sequence` 沒遞增，或重開機時 `boot_id` 沒 +1 |
| 一輪多張只記到一個人 | `tag_id` 只放了一張 |
| 成績時間整體偏移 | `detected_at` 送成發送時間，或裝置未對時 |

---

## 8. 版本

topic 的 `v1` 是契約版本。欄位語意若要改，是開 `v2` 新 topic，
不讓 `v1` 悄悄改變意思。現場混版時舊裝置照樣能繼續跑。

待確認事項（EPC 長度、單輪峰值張數、`rssi` 欄位、對時方式等）見
[reader-integration.md](reader-integration.md) §8 與 [open-issues.md](open-issues.md)。
