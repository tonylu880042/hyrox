# Roadmap

專案現況與後續規劃。里程碑編號沿用 CLAUDE.md 第 26 節。

**最後更新：2026-09-01**　　510 個測試通過（含真實 broker 端到端）。

---

## 現況一覽

| 里程碑 | 狀態 | 備註 |
|---|---|---|
| M1 Domain Foundation | ✅ 完成 | Session、Athlete State、Reader 對應、RFID 綁定、計時型別、轉換計算 |
| M2 Persistence | ✅ 完成 | SQLite + WAL、三份 migration、復原以事件重放為準 |
| M3 MQTT Ingestion | ✅ 完成 | 已對真實 Mosquitto 端到端驗證 |
| M4 Race / Training Engine | ⚠️ 部分 | 訓練與單人競賽的完成規則已定；競賽的**站點順序驗證**仍未做 |
| M5 REST / WebSocket | ✅ 完成 | `crates/api`，讀寫分離由型別強制 |
| M6 Operator UI | ✅ 完成 | `/live`、`/workout`、`/checkin`、`/signup`、`/settings`（讀卡機／裝置／異常／大螢幕／電源／示範資料，ADR 0013） |
| M7 多國語系 | ⚠️ 部分 | **介面層已完成**（繁中／簡中／英）；資料層本來就支援。剩下的見下方 |
| M8 出貨形態 | ⚠️ 部分 | 第一批＋第二批已完成，**但打包與 kiosk 尚未在實機驗證**。ADR 0009 |

### 系統設定頁與電源控制（ADR 0013，2026-09-02）

- `/settings`：讀卡機設定、邊緣裝置健康、異常清單（可作廢）、電源
- **待設定的讀卡機是推導的**：拿手環碰天線就出現在清單最上面，裝機不用抄 MAC
- 電源三動作，課程進行中拒絕關機／重開機器；重啟主機服務不受限（可復原）
- RFID 讀取器可**手動新增與移除**（ADR 0007 §7 修訂：移除只影響未來的刷卡，歷史存的是站點不是讀取器）
- 場域設定（`venue_settings`）：換頁間隔、每頁人數（固定版型 6/12/20/30，不是任意數字）
- 場館 logo 上傳（`venue_assets`，PNG/JPEG，依 magic bytes 判斷，**不收 SVG**），顯示在大螢幕頁首最左
- 用 **polkit 不用 sudo**——服務的 `NoNewPrivileges=yes` 會擋掉 setuid，實機才發現
- **示範資料變成一顆按鈕**（ADR 0013 §7，2026-09-02）：開機不再自動塞 12 位假選手，
  要 `HYROX_DEMO=1` 才看得到設定頁上的「示範資料」分頁。按下去會建一整場課
  （課表／選手／讀取器／腳環）並經真的 broker 開始模擬刷卡；課程進行中拒絕載入。
  順帶：12 倍速開發時鐘不再套用在場館機器上

### 壞檔偵測與備份（ADR 0012，2026-09-01）

- 開機做 `PRAGMA quick_check`；壞檔就**不啟動**（exit 78，systemd 不重啟），因為半殘地跑會
  一邊 ACK 一邊叫邊緣刪掉副本（ADR 0002）
- `POST /api/operator/backup` 用 `VACUUM INTO` 做線上備份，主機自己做（它是唯一能碰檔案的行程）
- 排程備份一天兩次（12:00 / 21:00，`hyrox-backup.timer`）＋ 夜間更新前一次，保留 **14 天**
- 夜間維護：可以停 → 備份 → 更新 → 驗證 → 關機；**備份失敗就不更新**
- 可選的 USB 隨身碟鏡射（`HYROX_BACKUP_MIRROR`，預設關閉）：掛載點沒掛就報錯，不會假裝有備份
- 尚未做：往場館外送（雲端／異地）、執行中的定期檢查

### 賽事自助報名（ADR 0011，2026-09-01）

模擬賽的非會員可以自己用手機報名，拿到六碼編號與 QR，用來領腳環、查成績。

- `EntryCode`：Crockford base32 去掉 `U`，解析時把 `O`→`0`、`I`/`L`→`1`
- **編號就是 athlete id**，不是另一張對應表；會員仍用 member_id，不發編號
- `POST /api/checkin/signup` 是全站唯一不需要操作者裝置名稱的寫入，稽核記為 `SELF SIGN-UP`；
  只收姓名，號碼布與 member_id 一律忽略
- QR 由主機畫成 SVG（`qrcode` crate），內容是六個字元而不是網址——掃描槍是鍵盤
- `/signup` 一頁三用：報名表、報名證、成績查詢，網址帶編號可截圖可書籤
- `/checkin` 加上編號搜尋（掃描槍／手動）與相機掃碼（`BarcodeDetector`，需 HTTPS，見 open-issues）
- 順帶修掉復原缺陷：resume 沒有把號碼布讀回來，重開機後下一位報名者會拿到已被使用的號碼

### 團課課表模組（ADR 0008，2026-08-29）

Workout Template → 編譯 → Course → 快照 → Class Session。課表是可編輯的物件，
**執行中的課程永遠不讀課表**，所以事後改課表不會影響已經跑過的課。
詳見 [docs/workout-system.md](workout-system.md)。

- Exercise Library（9 項，含 `station_key` 對應現有 reader 與大螢幕圖示）
- WorkoutTemplate / Block / Exercise，SYSTEM 唯讀、COACH 可改、version 自動遞增
- 四份 system preset，開機自動 seed（keyed write，不會累積）
- Session 狀態機擴為六態：`DRAFT → READY → RUNNING ⇄ PAUSED → COMPLETED`，另加 `CANCELLED`
- **PAUSED 會停課堂時鐘**：`ClassClock` 扣除暫停時間並持久化，重啟後仍是暫停狀態
- AthleteStage 為**推導**而非儲存，修正（void）會自動反映
- EXPECTED / OUT_OF_ORDER / UNEXPECTED 只記錄不裁決
- `/workout` 教練介面：課表清單、拖拉編輯器、建立課程

Migration 0004 重建 `sessions` 與三張子表以放寬 status CHECK，並把 `ARMED→RUNNING`、
`CLOSED→COMPLETED`。已對**真實舊資料庫**驗證，並有回歸測試
（`crates/storage/tests/migration_0004.rs`）。

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
| domain | 147 |
| application | 113 |
| simulator | 51 |
| api | 58 |
| contract | 23 |
| transport | 16 |
| storage | 29 |

---

## 已證明的性質

這些不是「有寫」而是**有測試或實測撐著**：

- **不遺失事件**（§31 第一原則）— Hub 關機期間發布的事件，重啟後送達且只解讀一次。以真實 broker 驗證，非記憶體模擬。
- **官方計時不受傳輸影響**（§17）— 上述情境中 `detected_at` 保持原值，`received_at` 是重啟時刻。
- **重複投遞不重複處理**（§16）— 重送回 `DUPLICATE` ACK，raw 與 interpreted 各維持一筆。
- **落地後才 ACK**（§15）— 型別強制：`Ack` 無公開建構子，只能從成功的 commit 產生（ADR 0002）。
- **RF 抑制是 presence/re-arm 不是固定窗**（§14）— 測試跑 120 秒連續在場斷言只發一次。
- **復原完整**（§21）— 重啟取回原本的課表與完成規則，不是預設值。
- **成績時間是事件時刻不是觀測時刻** — 完成判定回傳「何時完成」，遲來的輪詢（或重啟後的第一次輪詢）不會灌水成績。
- **原始事件不可變**（§19）— 修正只在解讀層打 `voided_at`。
- **唯讀介面寫不了**（ADR 0001）— 能力型別讓誤寫變成編譯錯誤。

---

## 下一步

### 立即可做

**~~M6 教練課程編排介面~~** — 已完成（ADR 0008）。

**把 `/live` 與 `/workout` 的 CSS 與字型改為本機提供** — 兩頁目前都從 CDN 載入 Tailwind 與
Google Fonts，斷網時畫面會變成無樣式的裸 HTML，牴觸 §31。資料與 WebSocket 不受影響。
`/live` 本來就有這個問題，`/workout` 是依指示沿用同一風格而延續。

**~~多國語系（介面層）~~** — 已完成（2026-08-29）。兩個畫面、三種語言，見下方 M7。

### 等外部條件

**會員對接** — 健身管的 endpoint、認證、payload 格式未知。`MemberDirectory` port 已定義，目前是 `UnconfiguredDirectory`（一律回報未設定，不假裝知道合約）。已知需求：以 QR 取得會員編號後呼叫，取回性別、年齡、照片，身高體重選填。

**單人競賽介面** — 完成規則已定案並實作（`FinishPolicy::CourseComplete`），不再被擋。剩下的是站點順序驗證（§9.1），那仍未決。

### M7 — 多國語系（i18n）

**目標語系**：繁體中文 `zh-Hant`、簡體中文 `zh-Hans`、英文 `en`。
**介面層已於 2026-08-29 完成**（依指示只做介面層）。

現況已實測（2026-08-29），分成三塊，工作量與風險完全不同：

#### 1. 資料層 — 已經可用，不需要做事

中文資料整條 API 來回都正常，已實測：課表名稱、描述、Block 名稱、動作備註、班級名稱、
學員姓名、修正原因全部原樣進出 SQLite（TEXT 即 UTF-8）。稽核用的 `x-operator-device`
header 也支援中文 —— `crates/api/src/identity.rs` 特意不用 axum 的 `to_str()`，因為那個
函式會拒絕非 ASCII，`櫃檯平板` 會被誤判為「沒有操作者」。

**這塊不屬於 i18n 工作範圍**，它是使用者輸入的內容，不是介面文字。

#### 2. 識別碼 — 不可翻譯，需先確認

`RUN` / `SKIERG` / `WALL BALLS` 這些 `station_key` **同時**是三件事：

- `CourseStep.station`
- reader 註冊的 `station`（`readers` 資料表）
- 大螢幕圖示的 slug 來源（`design/live/icons/*.png`）

它們是識別碼，不是顯示文字。直接改成中文會同時解除全場 reader 對應並讓圖示變空白。

`Exercise` 已經拆成 `code` / `display_name` / `station_key` 三欄（ADR 0008），位置已預留：
**要翻譯的是 `display_name`，`station_key` 與 `code` 永遠不翻。** 大螢幕目前直接顯示
`station_key`，這是唯一需要改資料流的地方 —— `live::CourseStation` 要多帶一個顯示名。

同理不翻譯的還有：`SessionStatus`、`ReaderMode`、`TargetType`、`Unit`、`TemplateCategory`
等所有 enum 的線路值，以及 `ErrorBody.error` 的機器碼。這些是契約，翻譯屬於顯示層。

#### 3. 介面層 — ✅ 已完成（2026-08-29）

單一字典 `apps/hub-server/static/i18n.js`，由 hub 在 `/i18n.js` **本機提供**（不是 CDN——
斷網的場館仍要看得懂自己的語言，CLAUDE.md 31）。兩個畫面共用同一份，所以同一個標籤不會在
投影幕上是一種說法、在教練平板上是另一種。

| 位置 | 做法 |
|---|---|
| `static/i18n.js` | 三語字典 + `t()` / `apply()` / `switcher()`，約 110 個 key |
| `static/workout.html` | 靜態文字用 `data-i18n`，動態產生的用 `I18N.t()`；標頭有語言切換鈕 |
| `static/training.html` | 同上，但字串進 `design/live/build_screens.py`，重新產生 |
| `<html lang>` | 由 `I18N.apply()` 依所選語系設定 |
| 錯誤訊息 | 前端依 `error` 機器碼查字典；後端 `message` 永遠英文，只給讀 log 的人 |

**語系怎麼決定**：`?lang=zh-Hans` 優先 → `localStorage`（每台裝置各自記住，與 `hyrox.device`
同機制）→ `navigator.language`（`zh` 預設繁體，這是台灣場館）→ 繁體。
大螢幕**故意沒有切換鈕**：投影幕不是互動介面，用 `/live?lang=…` 釘住，之後那台機器會記住。

**已翻譯的動態內容**：九個運動項目（以 `Exercise.code` 對照，`station_key` 不動）、單位、
課表類型、區塊型別、課程狀態、Stage 狀態、18 條 API 錯誤碼。大螢幕的站名與
`plan`（「800 M」）也在顯示當下翻譯 —— `plan` 只翻尾端的單位 token，數字就是數字。

**測試**：`apps/hub-server/tests/i18n.rs`，6 個。三份字典 key 必須完全一致、不得重複、
畫面用到的 key 必須存在、每個 enum 值與每條 API 錯誤碼都要有翻譯、字典**不得**以
`station_key` 當 key（那是識別碼，翻了會解除 reader 對應）。少一個 key 就是紅燈，
不是等某天有人看到中文畫面裡夾一句英文才發現。

**繁簡各自維護，不做自動轉換** —— 健身術語兩岸用詞不同（農夫走路／农夫行走、
沙袋弓箭步／沙袋箭步蹲），而且自動轉換會把使用者輸入的資料一起轉掉。

#### 不在範圍內

**測試資料與開發資料維持現狀，不做語系化**（2026-08-29 確認）。

測試裡的中文是**使用者輸入的內容**，不是介面文字 —— `誤觸`、`誤刷`、`停電`、`腳環故障`、
`設備異常`、`重複`、`不要了`、`櫃檯平板` 這些修正原因與裝置名稱，正是 ADR 0001 D1 期望
操作介面提供的快捷原因鍵，它們證明的是「非 ASCII 內容能存能取」這件事本身。把它們參數化
會讓測試不再驗證那件事。

同理不動：`apps/hub-server/src/feeder.rs` 的示範名單與班級名稱、`crates/api/tests/support`
的固定資料。這些是 fixture，不是產品文案。

#### 還沒做（依指示只做介面層）

- **四份 system preset 課表的名稱與說明仍是英文**（"HYROX Engine 800"、"Cardio /
  endurance…"）。那是**資料庫裡的資料列**，不是介面文字 —— 教練可以複製、可以改名。要中文化
  的話有兩條路：seed 時就寫中文（現有安裝的舊資料列不會變），或前端以 preset id 對照字典
  （但複製出來的副本就對不上了）。需要產品決定。
- **`crates/api/src/error.rs` 的 13 條 `message`** 維持英文，且刻意如此：`docs/api.md §6`
  規定前端 branch on `error` 機器碼。這不是缺口。
- **時間與數字格式**未在地化：全系統統一 `mm:ss` 與半形數字。改動會影響成績呈現，屬成績
  規則而非介面翻譯。
- **`/checkin` 與未來的 operator 畫面**還沒有介面，做的時候直接引 `/i18n.js` 即可。

### M8 — 出貨形態（ADR 0009，2026-08-29 決定）

**整機出貨**：客戶不安裝、不選發行版、不會看到終端機。決策與理由見
[ADR 0009](decisions/0009-shipped-as-an-appliance.md)。

#### 作業系統與產線

| 項目 | 選擇 | 為什麼 |
|---|---|---|
| OS | **Ubuntu Server 24.04 LTS**（非 Desktop） | 五年支援；**autoinstall** 一份 YAML 就能無人值守裝出第二十台一模一樣的機器。Debian 的 preseed 較舊較彆扭，而「重現」才是產線的真問題 |
| 瀏覽器 | **Google Chrome `.deb`**（Google 簽章 repo） | Ubuntu 的 chromium 只有 snap，而 **snap 會在背景自我更新** —— 出貨機的瀏覽器版本被自動換掉不可接受。用 `.deb` 就沒這回事 |
| 顯示 | **`cage`**（Wayland kiosk compositor） | 不裝桌面環境、不裝登入畫面。開機就是畫面 |
| 更新 | **自建簽章 apt repo** | GPG 驗證、版本鎖定、回滾都是 apt 現成的，自己寫容易做錯 |
| 設定 | **`/etc/hyrox/hub.env`**（systemd `EnvironmentFile`） | hub 本來就讀環境變數，systemd 本來就會把檔案讀進環境。寫一個設定檔格式與 parser 是白工 |

**Debian 有被認真考慮過**，而且它不複雜 —— Debian 12 起安裝映像已內建非自由韌體，日常操作
跟 Ubuntu 是同一套 apt 與 systemd。它輸在產線重現性，不是輸在難用。

#### 第一批（repo 內）✅ 2026-08-29

- **`synchronous = FULL`** —— WAL 的 `NORMAL` 只擋行程崩潰，**擋不住斷電**，而出貨機一定會被
  直接關電源。ACK 等於叫 ESP32 刪掉唯一的另一份副本。`crates/storage/tests/durability.rs`
  直接對 store 的連線斷言 pragma（注意：`sqlite3` CLI 讀到的是它自己連線的預設值，驗不到這件事）
- **SIGTERM／Ctrl-C 優雅關機**。不需要 flush 任何東西 —— 事件在 ACK 之前就已落地，
  已 commit 未 ACK 的會被邊緣重送並以幂等鍵略過
- **`GET /api/health`** 與 `safe_to_stop`：`CLASS_RUNNING`（READY／RUNNING／PAUSED）與
  `DEVICE_BACKLOG`（裝置 journal 還有未 ACK 事件）。兩個 blocker **一起回報**，修掉一個不會
  遮住另一個。刻意不包 `freshness` 外層 —— 讀它的是 shell script
- **`HYROX_BIND`** 環境變數；程式內**預設仍是 `127.0.0.1:8730`**，出貨的 unit 才開
  `0.0.0.0`（開發機不會因為 `cargo run` 就暴露在咖啡廳的 wifi 上）
- 設定用 systemd `EnvironmentFile=/etc/hyrox/hub.env`，**不寫 parser** —— hub 本來就讀環境變數

#### 第二批（打包與產線）✅ 2026-08-29 — 但**未經實機驗證**

全部在 [`packaging/`](../packaging/)，見 [packaging/README.md](../packaging/README.md)。

| 檔案 | 是什麼 |
|---|---|
| `systemd/hyrox-hub.service` | hub daemon；`Restart=always`、`StateDirectory=hyrox` |
| `systemd/hyrox-kiosk.service` | `cage` + Chrome on tty1；**輪詢 hub 直到回應**才啟動，不用 sleep |
| `systemd/hyrox-maintenance.{service,timer}` | 夜間維護窗 |
| `bin/maintenance` | 問 → 更新 → 驗證 → 關機 |
| `build-deb.sh` / `publish-s3.sh` | 打包與簽章上傳 S3 |
| `autoinstall/user-data` | 產線：USB 無人值守安裝 |

**S3 用公開讀取 bucket + GPG 簽章。** apt 的信任來自 `InRelease` 的簽章，不是傳輸層；改用
IAM 驗證只會讓每台出貨機多一份要輪換的憑證，完整性一點也沒增加。「別人不能下載」是機密性、
不是完整性，那個需求請用 CloudFront signed URL 處理，別動 apt 的信任模型。

**維護時段刻意留成 placeholder**（`OnCalendar=*-*-* 23:30:00`），用
`systemctl edit hyrox-maintenance.timer` 依場館營業時間覆寫。

`Persistent=false` **必須維持** —— `true` 會讓錯過的 timer 在下次開機時補跑，而這個 job 的
結尾是 `poweroff`，等於早上有人開機後機器立刻自己關掉。

##### 維護腳本的六條路徑都實測過

用指令 shim 搭真實 hub 跑過，重點是**沒有一條會在課程進行中關機**：

| 情境 | 結果 |
|---|---|
| 課程 RUNNING | 完全不動作，exit 0，**沒有呼叫 poweroff** |
| 課程結束、無更新 | 關機 |
| 有新版本 | 安裝 → 驗證 → 關機 |
| `apt-get update` 失敗（斷網） | 不更新，仍關機 |
| **hub 沒有回應** | **不關機** —— 沒有回答不等於同意 |
| 更新後起不來 | 回滾；回滾也失敗 → **留機不關**，exit 1（systemd 標紅） |

##### 還沒驗證的

`.deb` 實際建置、`cage` 取得 DRM master、`kiosk` 使用者的 seat 權限、autoinstall 實際跑一遍。
這些需要真機與真投影機，macOS 上驗不到。**第一次建置請當作 bring-up，不要當成可出貨版本。**

#### 維護時段的行為

```text
23:30 timer 觸發
  ├─ 問 /api/health
  ├─ safe_to_stop = false → 記 log，不動作，隔天再說
  ├─ 有更新 → apt install → 重啟 → 驗證起得來 → 關機
  └─ 沒更新 → 關機
```

`unattended-upgrades` 必須**只信任自建 repo**，且關閉自動重開機 —— 課上到一半重開機是這套
系統最糟的失敗模式。

### 需要 domain 擴充

**雙人 / 接力賽制** — 完成的主體是隊伍不是個人，但目前沒有隊伍概念：`AthleteState` 一人一份、綁定一腳環一人。需要隊伍實體、工作站歸屬到隊員、隊伍層級的完成判定。屬 M1 等級擴充，建議獨立排期。

---

## 未決事項

完整記錄在 `docs/open-issues.md`，此處只列擋住什麼。

| 項目 | 擋住 | 誰決定 |
|---|---|---|
| 雙人賽一隊幾個腳環（官方規定待查證） | 雙人／接力賽制 | 產品 |
| 競賽的站點順序驗證與例外處理（§9.1） | 競賽介面的異常判定 | 產品 |
| 健身管 API 合約 | 會員對接 | 外部 |
| ESP32 已 ACK 事件保留期 | 韌體實作 | 韌體團隊 |
| `reader_id` 是否為 Reader 自身的 MAC | 「一台 ESP32 多讀頭」模型 | 韌體團隊 |

**已答覆（2026-08-27～28）**：訓練完成規則（時間到 + 教練手動）、**競賽完成規則（跑完課表，最後一站 EXIT 為成績紀錄點）**、賽制以課表長度區分（全程／半程），會籍不擋計時、識別碼一律轉小寫、topic 命名 `hyrox/v1/...`。

**賽制是資料不是程式分支**：全程與半程的差別只是 `Course` 的長度，新增賽制不該新增 `FinishPolicy` variant。

### 交給韌體團隊

`docs/event-protocol.md` 是交接文件，第 7 節列出所有待確認項。特別需要當面說清楚的兩點：

1. **抑制必須是 tag presence / re-arm，不得用固定時間窗**（§14），而且**不得拿站點時長當抑制時長**。這是最容易被韌體端「合理化」成固定窗的地方。
2. Hub 佔用 MQTT client id `hyrox-hub`，持久 session 需要穩定 id。

---

## 已知缺口

不擋進度但記著：

- **AMRAP / ZONE_ROTATION 只有資料結構，沒有執行模型** — 編譯時具名拒絕，不臆測。
- **Expectation 目前沒有任何規則消費** — 只推導與發布，等競賽例外規則定案。
- **同時只能有一個進行中的課程** — RUNNING/PAUSED 時建立新課會被拒。
- **異常清單只做了 void** — ADR 0001 D4 還要「接受原樣」和「改判」。前者需要新資料庫欄位，並且要先決定被接受的異常還算不算進紅點計數。
- **裝置存活狀態只在記憶體** — 重啟後所有 reader 回報 `last_seen_age_ms: null`，畫面不可將 `null` 畫成新鮮。
- **`/checkin` 的待綁定清單需輪詢** — 推播只給數量，改推清單會動到 `/ws` 格式而破壞現有大螢幕。
- **10,000 筆 backlog 的逐筆 ACK 未壓測**。
- **裝置同名無法區分** — ADR 0001 自己列的未決項；兩台平板取一樣的名字在稽核上分不出來。
- **`transport::client` 的 broker 測試需要本機 Mosquitto**，無 broker 時自動跳過。
- **瀏覽器 `fetch()` 對非 ASCII header 值行為不一致** — 伺服器端已可接受 UTF-8 裝置名，但 M6 做操作介面時可能需要 percent-encode 或改放 body。
