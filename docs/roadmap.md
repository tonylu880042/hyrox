# Roadmap

專案現況與後續規劃。里程碑編號沿用 CLAUDE.md 第 26 節。

**最後更新：2026-08-28**　　286 個測試通過、零警告。

---

## 現況一覽

| 里程碑 | 狀態 | 備註 |
|---|---|---|
| M1 Domain Foundation | ✅ 完成 | Session、Athlete State、Reader 對應、RFID 綁定、計時型別、轉換計算 |
| M2 Persistence | ✅ 完成 | SQLite + WAL、三份 migration、復原以事件重放為準 |
| M3 MQTT Ingestion | ✅ 完成 | 已對真實 Mosquitto 端到端驗證 |
| M4 Race / Training Engine | ⚠️ 部分 | 訓練模式完整；競賽的順序驗證未做（等完成規則） |
| M5 REST / WebSocket | ✅ 完成 | `crates/api`，讀寫分離由型別強制 |
| M6 Operator UI | ⬜ 未開始 | 目前只有 `/live` 訓練大螢幕 |

### Crate 結構與相依方向

```
domain      →  (無)                          實體與規則
contract    →  domain                        邊緣事件協定 + ACK 政策
application →  domain, contract              用例與 port
transport   →  contract, domain, rumqttc     MQTT
simulator   →  contract, transport           模擬 ESP32
storage     →  domain, application, contract SQLite adapter
api         →  domain, application, axum     HTTP / WebSocket
hub-server  →  全部                            組裝根
```

箭頭全部向內（ADR 0005）。`application` 碰不到 axum、sqlx、rumqttc。

| Crate | 測試數 |
|---|---|
| domain | 69 |
| application | 80 |
| simulator | 51 |
| api | 34 |
| contract | 23 |
| transport | 16 |
| storage | 13 |

---

## 已證明的性質

這些不是「有寫」而是**有測試或實測撐著**：

- **不遺失事件**（§31 第一原則）— Hub 關機期間發布的事件，重啟後送達且只解讀一次。以真實 broker 驗證，非記憶體模擬。
- **官方計時不受傳輸影響**（§17）— 上述情境中 `detected_at` 保持原值，`received_at` 是重啟時刻。
- **重複投遞不重複處理**（§16）— 重送回 `DUPLICATE` ACK，raw 與 interpreted 各維持一筆。
- **落地後才 ACK**（§15）— 型別強制：`Ack` 無公開建構子，只能從成功的 commit 產生（ADR 0002）。
- **RF 抑制是 presence/re-arm 不是固定窗**（§14）— 測試跑 120 秒連續在場斷言只發一次。
- **復原完整**（§21）— 重啟取回原本的課表與完成規則，不是預設值。
- **原始事件不可變**（§19）— 修正只在解讀層打 `voided_at`。
- **唯讀介面寫不了**（ADR 0001）— 能力型別讓誤寫變成編譯錯誤。

---

## 下一步

### 立即可做

**M6 教練課程編排介面** — API 已就緒（`PUT /api/operator/config`），課表模型支援重複站點與各站目標值。不被任何未決問題卡住，建議先做。

### 等外部條件

**會員對接** — 健身管的 endpoint、認證、payload 格式未知。`MemberDirectory` port 已定義，目前是 `UnconfiguredDirectory`（一律回報未設定，不假裝知道合約）。已知需求：以 QR 取得會員編號後呼叫，取回性別、年齡、照片，身高體重選填。

**競賽介面** — 見下方未決事項。

### 需要 domain 擴充

**雙人 / 接力賽制** — 完成的主體是隊伍不是個人，但目前沒有隊伍概念：`AthleteState` 一人一份、綁定一腳環一人。需要隊伍實體、工作站歸屬到隊員、隊伍層級的完成判定。屬 M1 等級擴充，建議獨立排期。

---

## 未決事項

完整記錄在 `docs/open-issues.md`，此處只列擋住什麼。

| 項目 | 擋住 | 誰決定 |
|---|---|---|
| 完成的觸發點：最後一站 EXIT vs 專屬 Finish Reader | 競賽介面、場館佈線 | 產品 |
| 雙人賽一隊幾個腳環 | 雙人賽制 | 產品 |
| 健身管 API 合約 | 會員對接 | 外部 |
| ESP32 已 ACK 事件保留期 | 韌體實作 | 韌體團隊 |
| `reader_id` 是否為 Reader 自身的 MAC | 「一台 ESP32 多讀頭」模型 | 韌體團隊 |

**已答覆（2026-08-27～28）**：訓練完成規則（時間到 + 教練手動）、會籍不擋計時、識別碼一律轉小寫、topic 命名 `hyrox/v1/...`、競賽賽制以課表長度區分（全程／半程）。

### 交給韌體團隊

`docs/event-protocol.md` 是交接文件，第 7 節列出所有待確認項。特別需要當面說清楚的兩點：

1. **抑制必須是 tag presence / re-arm，不得用固定時間窗**（§14），而且**不得拿站點時長當抑制時長**。這是最容易被韌體端「合理化」成固定窗的地方。
2. Hub 佔用 MQTT client id `hyrox-hub`，持久 session 需要穩定 id。

---

## 已知缺口

不擋進度但記著：

- **異常清單只做了 void** — ADR 0001 D4 還要「接受原樣」和「改判」。前者需要新資料庫欄位，並且要先決定被接受的異常還算不算進紅點計數。
- **裝置存活狀態只在記憶體** — 重啟後所有 reader 回報 `last_seen_age_ms: null`，畫面不可將 `null` 畫成新鮮。
- **`/checkin` 的待綁定清單需輪詢** — 推播只給數量，改推清單會動到 `/ws` 格式而破壞現有大螢幕。
- **10,000 筆 backlog 的逐筆 ACK 未壓測**。
- **裝置同名無法區分** — ADR 0001 自己列的未決項；兩台平板取一樣的名字在稽核上分不出來。
- **`transport::client` 的 broker 測試需要本機 Mosquitto**，無 broker 時自動跳過。
- **瀏覽器 `fetch()` 對非 ASCII header 值行為不一致** — 伺服器端已可接受 UTF-8 裝置名，但 M6 做操作介面時可能需要 percent-encode 或改放 body。
