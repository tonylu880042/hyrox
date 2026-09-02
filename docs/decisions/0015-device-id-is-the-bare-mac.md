# ADR 0015 — `device_id` 就是 MAC，去掉 `esp32-` 前綴

- 狀態：已決議
- 日期：2026-09-02
- 影響範圍：`crates/domain`（`DeviceId`）、migration 0008（既有資料改寫）、
  `CLAUDE.md` §7.3、`docs/event-protocol.md`、`docs/reader-integration.md`
- 不影響：正規化規則（保留）、`ReaderId`、MQTT topic 配置、判重鍵、ACK 協定

## 背景

原本的正規形式是 `esp32-a4cf128b3d91`：前綴 + 12 位小寫 hex、無分隔符號。
其中做了兩件事，價值差很多：

1. **正規化**：折疊大小寫、去掉 `:` `-` `.`。
2. **前綴**：固定加上 `esp32-`。

前綴換來的實質好處找不到。系統中沒有第二種 12 位 hex 的識別碼會跟它混淆；
它在日誌裡也不比裸 MAC 好認。

而現場採用的是 **UHF 讀取器**，裡面不一定有 ESP32。要求韌體在自己的識別碼前面
加上一個錯誤的型號名稱，是一個必須附帶免責聲明才成立的欄位——
`docs/reader-integration.md` 原本就得寫「即使實際硬體不是 ESP32 也照用」。
需要為欄位寫免責聲明，通常代表欄位錯了。

## 決議

**去掉前綴，保留正規化。**

```text
device_id = "a4cf128b3d91"     ← 12 位小寫 hex，無分隔符號
```

### 正規化為什麼不能一起拿掉

這才是有承重的那一半。若原樣接受各種寫法：

```text
韌體送   A4:CF:12:8B:3D:91
設定檔寫 a4-cf-12-8b-3d-91
    ↓
Reader 對應表查不到
    ↓
該讀取器的事件全部變成 UNKNOWN_READER exception
```

失敗是**安靜**的：Hub 不報錯，只是把牆上那台讀取器的每一筆讀取都歸成例外，
要等到有人翻例外清單才會發現。`:` `-` `.` 三種分隔符號 × 大小寫 = 同一台機器六種拼法。

分工維持不變：

| | |
|---|---|
| `DeviceId::from_mac_str` | 給設定檔與人手輸入。吃 `A4:CF:12:8B:3D:91`、`a4-cf-12-8b-3d-91`、`a4.cf.12.8b.3d.91`、裸 hex，一律正規化 |
| `DeviceId::parse` | 給線路。**只**接受正規形式（大小寫不拘），分隔符號拒收 |

舊的 `esp32-` 形式在線路上**拒收**，不做相容接受：兩種拼法並存正是正規形式要防的事，
而且尚無已部署韌體。

`DeviceIdError::MissingPrefix` 與 `DeviceId::mac_hex()` 一併移除——前者無對應錯誤了，
後者與 `as_str()` 變成同一件事。

### 既有資料

migration 0008 就地改寫 `raw_events.device_id` 與 `readers.device_id`，資料層面，不動表結構。
`readers` 那半是關鍵：漏掉的話每台讀取器都會停留在舊鍵，牆上所有讀取靜默變成 UNKNOWN_READER。
以 `WHERE device_id LIKE 'esp32-%'` 護欄，重複執行不會多吃六個字元。

## 替代方案

| 方案 | 不採用的原因 |
|---|---|
| 維持 `esp32-` 前綴 | 換不到東西，且對非 ESP32 硬體是錯誤宣告 |
| 線路上原樣接受 `A4:CF:12:8B:3D:91` | 同一台機器六種拼法，Reader 對應表查不到且失敗是安靜的 |
| 正規形式改成帶冒號的小寫 MAC（`a4:cf:12:8b:3d:91`） | 也是單一拼法，可行；但無分隔符號的形式在 topic、URL、日誌欄寬上都少一層顧慮，而且已是現況 |
| 同時接受新舊兩種形式 | 相容路徑會讓兩種拼法長期並存，正是正規形式要防的事。尚無已部署韌體，不需要 |

## 可攜性影響

無。純資料格式，不觸及平台 API（CLAUDE.md 2）。

## 測試

| 主題 | 檔案 |
|---|---|
| 正規化、大小寫、分隔符號拒收、舊前綴形式拒收、UUID 拒收 | `crates/domain/tests/registry.rs` |
| 線路解碼 | `crates/contract/tests/protocol.rs` |
| migration 0008 改寫既有資料、可重複執行 | `crates/storage/tests/migration_0008.rs` |
