# ADR 0001 — Central Hub 介面操作規則

- 狀態：已決議
- 日期：2026-08-27
- 影響範圍：domain（Session 狀態機、Binding、Exception）、application、REST/WS 契約
- 不影響：UI 實作時程（仍排在 CLAUDE.md §26 Milestone 6）

## 背景

CLAUDE.md 定義了 Central Hub 的職責，但沒有定義操作規則。§28 將
「exact Coach correction permission model」列為未決議題。

**主要使用情境經確認為健身房的上課與訓練（TRAINING mode），不是競賽。**
現場步調快、人多、混亂，操作速度優先於流程嚴謹度。競賽模式仍須支援，
但不是日常路徑。

## 介面切分

只有一個寫入面 + 一個窄寫入面，其餘全部唯讀。

| 介面 | 權限 | 用途 |
|---|---|---|
| `/operator` | 寫入 | Session 控制、裝置/Reader 狀態、Exception 清單、人工修正 |
| `/checkin` | 窄寫入 | 只能做 RFID ↔ 選手綁定，不能碰計時 |
| `/coach` | 唯讀 | §23 即時選手資料 |
| `/live` | 唯讀 | 大螢幕 |
| `/result/{id}` | 唯讀 | 成績 |

理由：現場出錯的成本不對稱。唯讀畫面可以隨意發給教練與學員而不必擔心誤觸；
能改資料的入口越少，稽核範圍越小。

## D1 — 權限：不登入，裝置即身分

- **不做登入、不做個人帳號。**
- 每台裝置首次開啟 `/operator` 或 `/checkin` 時取一個名稱
  （例：「櫃檯平板」），存於 localStorage，之後不再詢問。
- **場館安全鎖（2026-09-05 補充）**：為避免共用 Wi-Fi 環境下外人誤入 `/settings`
  關閉主機或重設硬體，針對高風險設定介面加入 4 位數 PIN 鎖（預設 `2018`）。解鎖後記住
  12 小時，大螢幕與選手端依然免密碼。
- 所有寫入 API 帶 `operator_device`，作為 §20 audit 的 `operator` 欄位。
- 網路邊界由部署層負責：Hub 只綁 LAN 介面，不對外開放。
- 破壞性操作（void、改時間、改選手）仍要 `reason`，但以**快速原因鍵**
  提供（誤刷 / 漏刷 / 設備異常 / 其他），一鍵帶入，不強迫打字。

**已知取捨：** 追溯粒度是「裝置」而非「個人」。爭議時需要靠人工知道當時誰持有該裝置。
這是刻意換來零摩擦的代價。若日後需要個人層級追溯，`operator_device` 欄位
可平滑升級為 `operator_identity`，不需改動 audit 結構。

## D2 — Session 狀態機：DRAFT → ARMED → CLOSED

只有三態。選手個別的 READY / ACTIVE / FINISHED 由 §10 Athlete State 承擔，
不在 Session 層重複建模。

| 狀態 | 可否收事件 | 可否編輯設定 | 可否修正 |
|---|---|---|---|
| DRAFT | 否 | 是 | n/a |
| ARMED | 是 | 否 | 是 |
| CLOSED | 否 | 否 | 是 |

轉換規則：

- `DRAFT → ARMED`：operator 觸發。
- `ARMED` 後第一個有效事件開始該選手計時（§11），Session 本身不改狀態。
- `ARMED → DRAFT`：僅當該 Session 尚無任何 interpreted event 時允許。
- `ARMED → CLOSED`：operator 觸發。
- `CLOSED → ARMED`：**允許**，需 `reason`，寫入 audit。
  現場誤觸不該逼使用者開新 Session（無時間窗限制，避免引入魔術常數，見 §29）。

## D3 — 綁定：Tag-first，且可回溯認領

- 任何 Reader 刷到未綁定的 tag → 該 tag 進入「待綁定」，即時推播到 `/checkin`。
- `/checkin` 顯示待綁定 tag 清單 + 選手搜尋框，選定即綁定。
- **未綁定 tag 的事件仍寫入 raw store**（§19），綁定完成後回溯重新解讀，
  已發生的時間不遺失。這條直接服務 §31 第一原則。
- 換腳環 = 舊綁定 unbind + 新綁定，兩筆都寫 audit。
- 不變量：一個 tag 同時間只綁一位選手；一位選手在同一 Session 只有一個有效 tag。

## D4 — Exception Inbox

位置：`/operator` 常駐計數徽章 + 清單。不阻塞其他選手的計時。

進入 inbox 的條件：

- 未知 Reader（`device_id + reader_id` 對應不到 Station）
- 未知 tag（系統中不存在）
- 不可能的狀態轉移（INSIDE 收到同站 ENTRY、OUTSIDE 收到 EXIT）
- 已綁定選手不在本 Session 名單內

**僅競賽模式額外加入：** 站點順序不符 template（§9.1）。
**訓練模式不因順序不同產生 exception**（§9.2 明文規定）。

未綁定 tag **不進 inbox**，改導向 `/checkin`（見 D3）——那是待辦事項，不是錯誤。

處理動作：accept as-is / void / 改判（改站點、改 ENTRY/EXIT、改選手）。
每個動作寫 audit。**只有會改變重播結果的動作才觸發衍生值重算**（§20）——
void 會，accept as-is 不會，因為它一列也沒有從 log 裡拿掉。

**accept as-is（2026-09-03 補上）**：`acknowledged_at` / `acknowledged_by` /
`acknowledge_reason`（migration 0011）。那一列留在 log 與每一次重播裡，離開的只有
inbox 與 badge。這是兩把不同的工具：void 是破壞性的，用來對付「本來就不該算數的
一筆」；一筆真實但無關緊要的重複靠卡不該被抹掉，它只是不該再佔著誰的待辦清單。
不要求原因（沒有東西被改變），有填就記下來。

**原因用一排標籤，不用鍵盤**：教練站著、單手拿平板，虛擬鍵盤會蓋掉他正在看的清單。
五個標籤點一下就送出，底下仍留一個自由輸入框。存進資料庫的一律是自由文字——
稽核值錢的是那句話，不是一個代碼。

## D5 — 資料新鮮度指示器（強制）

所有畫面常駐顯示：WebSocket 連線狀態 + 最後一筆事件距今秒數。
`/operator` 額外顯示每個 Reader 的 `last_seen`。

理由：§31 第一原則是不遺失事件。沒有這個指示器，一個安靜的畫面同時代表
「沒人在跑」和「MQTT 斷了」，現場無法分辨。

## 對既有約束的遵循

- UI 不含業務邏輯（§6、§29）：以上所有判斷都在 application/domain 層，
  UI 只送出意圖、顯示伺服器回傳的狀態。
- 原始事件永不可從 UI 編輯（§19）：UI 只能新增 interpreted event 或 void。
- UI 斷線不影響計時（§6、§31）。

## 本決議產生 / 收斂的未決項目

- Finish rule（§12）尚未定義，因此 `CLOSED` 不代表「所有人都完成」。
  完成判定須做成 Session 設定項，不得寫死。
- 快速原因鍵的實際選項需在真實場館驗證後調整。
- 裝置命名的衝突處理（兩台裝置取同名）尚未定義。

## 補充（2026-08-27，實作第一個切片後）

- **卡片不顯示站內運動進度。** 設計稿原本有 `380 / 500 M` 這類進度條，但 Hub 只從 RFID
  取得進出站時間（§7），站內跑了多少距離／做了幾下是器材遙測，Central Hub 沒有這個資料來源。
  畫面改為顯示**站內用時**（§23 的 Workout Split），課表目標值降為靜態標籤。
  若日後接入器材遙測，這是一個獨立的資料來源，需要新的 ADR。
- **斷線必須明示。** WebSocket 中斷時畫面轉為 `DISCONNECTED` / `LINK DOWN`（紅），
  避免凍住的畫面被誤讀為「現場沒人在跑」。這是 D5 的實作條件，不是選配。
