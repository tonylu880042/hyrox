# HYROX 場館安全鎖 (Venue PIN Lock) 操作說明手冊

- **預設 PIN 碼**：`2018`（資料庫尚未設定有效 `security.pin` 時的 fallback）
- **受保護介面**：`/settings`（設備設定、讀卡機設置、電源管理、示範資料）
- **免密碼介面**：`/live`（大螢幕投影）、`/workout`（教練日常課表）、`/signup`（學員手機報名與看成績）、`/checkin`（櫃檯報到）

---

## 1. 設計背景與安全原則

在健身房或賽事現場，Central Hub 主機通常架設於場館區域網路（LAN / Wi-Fi）。若未設防護，任何連入 Wi-Fi 的學員手機皆可隨意存取後台，可能導致：
1. **誤觸電源**：意外將主機關機（Poweroff）或重新啟動（Reboot），中斷正在進行的課程或競賽。
2. **誤刪或更動讀卡機**：解除天線綁定，導致選手刷卡無效或變成異常。
3. **覆蓋賽事資料**：誤按「示範資料（Demo Data）」將整場真實學員成績洗掉。

為維持 ADR 0001 的「零摩擦、快節奏」原則，本系統採 **「場館數字安全鎖（Appliance PIN Lock）」**：
- **不強制個人註冊帳號或複雜密碼**。
- **針對高危險設定（`/settings`）加設 4–8 位數 PIN 鎖**。
- **提供 9 宮格大按鈕**，適合平板單手操作；輸入 4–8 位後按「解鎖」或鍵盤 Enter 送出。
- **伺服器 session 最長 12 小時**：session 存在 Hub 記憶體，瀏覽器只保存 HttpOnly cookie，不把登入狀態或 token 放進 localStorage。

---

## 2. 操作說明

### 2.1 解鎖後台設定頁面 (`/settings`)
1. 工作人員或教練在瀏覽器開啟後台設定：`http://<hub-ip>:8730/settings`。
2. 畫面會自動彈出 **「HYROX SECURITY」場館安全鎖** 視窗。
3. 透過螢幕上的 9 宮格大數字鍵盤（或外接實體鍵盤數字鍵）輸入 4–8 位數 PIN 碼：
   - 出廠或初始預設 PIN 碼為：**`2018`**。
4. 按下 **「解鎖」** 或鍵盤 **Enter** 向 Hub 驗證；輸入 4 碼時不會自動送出，方便輸入較長 PIN。
   - **驗證成功**：對話框自動關閉，Hub 發出 HttpOnly、SameSite=Strict session cookie，在 12 小時內可使用受保護 API。
   - **驗證失敗**：指示燈紅字抖動並顯示「PIN 碼不正確」，自動清空讓您重新輸入。
   - 受保護 API 回傳 `401` 時，畫面會立即重新鎖定。PIN 錯誤嘗試有伺服器端速率限制。

### 2.2 自訂 / 修改場館 PIN 碼
為提高場館專屬安全性，建議場館負責人在首次部署後修改 PIN 碼：
1. 進入 `/settings` 並成功解鎖後，點選上方選單的 **「安全鎖」** 分頁。
2. 填寫修改表單：
   - **目前 PIN 碼**：輸入舊密碼（若未曾修改過請輸入 `2018`）。
   - **新 PIN 碼**：輸入 4 至 8 位純數字。
   - **確認新 PIN 碼**：再次輸入新密碼以確認一致。
3. 點選 **「儲存」**。系統完成更新後將記下審計紀錄（`VENUE_SETTING: security.pin`，密碼自動遮罩）。

---

## 3. 緊急恢復與重置機制 (FAQ)

### Q1: 如果現場忘記了自訂的 PIN 碼怎麼辦？
**A: 可由伺服器主機透過以下兩種方式任選其一重置：**

#### 方式一：環境變數 fallback
在啟動 Hub 服務的環境變數設定 `HYROX_PIN`：
```bash
# 透過 systemd 或環境變數啟動
HYROX_PIN=2018 /usr/local/bin/hub-server
```
只有資料庫沒有有效 `security.pin` 時，才會使用有效的 `HYROX_PIN`；資料庫已有有效 PIN 時，環境變數不會覆寫它。若兩者都沒有有效值，才回到 `2018`。

#### 方式二：終端機資料庫重設
直接透過 SQLite 命令列工具將自訂 PIN 碼重置為預設值：
```bash
sqlite3 /var/lib/hyrox/hyrox.db "DELETE FROM venue_settings WHERE key = 'security.pin';"
```
執行後重新啟動 Hub，再開啟網頁；若沒有設定有效 `HYROX_PIN`，PIN 即恢復為預設的 **`2018`**。

---

## 4. API 規格參考

| 端點 | 方法 | 說明 | 認證要求 |
|---|---|---|---|
| `/api/operator/pin/verify` | `POST` | 校驗 PIN 碼並簽發 session cookie | 免登入，Payload: `{"pin": "2018"}`；成功回傳 HttpOnly、SameSite=Strict cookie |
| `/api/operator/pin/change` | `POST` | 變更場館 PIN 碼並撤銷既有 session | 需有效 session cookie 與 `x-operator-device`，Payload: `{"current_pin": "...", "new_pin": "..."}` |

讀卡機、電源、備份、Logo、Demo、例外處理與場館設定寫入等敏感操作都需要有效 session；一般課程控制、讀取介面與報到流程維持既有免 PIN 使用方式。排程備份使用獨立的 machine credential（`Authorization: Bearer`），不使用也不信任可由呼叫端偽造的 `x-operator-device`；該 credential 只授權備份及後續維運介面。
