# 系統展示劇本

給站在投影機前面的人用的。這份演**可靠性**；日常怎麼用（教練排課、報到、上課、
誤刷、出成績）在 [demo-script-group-class.md](demo-script-group-class.md)。

每一條指令、每一個預期畫面都在 2026-09-01 實機跑過一次
（macOS + Mosquitto + `target/debug/hub-server`），沒跑過的東西不寫在這裡。

主線 **25 分鐘**，加演三幕各 3 分鐘。

---

## 1. 這場展示只證明三件事

| # | 承諾 | 哪一幕證明 | 怎麼算證明 |
|---|---|---|---|
| 1 | **不會掉刷卡** | 第 4 幕 | 主機關機期間發布的事件，開機後仍然進帳，且成績時間不變 |
| 2 | **現場改得動** | 第 2、5 幕 | 現場報名、綁手環、補回綁定前刷的紀錄；教練自己排課 |
| 3 | **斷了還能跑** | 第 4、7 幕 | 重開機自動接回課程；沒有網路也讀得到自己的畫面 |

其他都是配菜。講超過這三件事，觀眾記得的反而更少。

不要說的話：「這套系統可以…」。改說「現在發生的是…」，然後指螢幕。

---

## 2. 開演前 T-15 檢查清單

```bash
brew services start mosquitto && mosquitto_sub -h 127.0.0.1 -t 'hyrox/v1/#' -C 1 -W 2
```

沒有 broker，整場只剩靜態畫面。這行如果 2 秒內沒吐東西也沒關係（表示還沒人在發），
重點是不能報連線錯誤。

**調慢展示時鐘。** `apps/hub-server/src/main.rs` 的 `const SPEED: i64 = 12` 是開發用的
——20 分鐘的課 100 秒就跑完，比你講話還快。展示前改成 `3`（約 6.7 分鐘，剛好蓋住第 1–3 幕），
然後：

```bash
cargo build && rm -f hyrox.db*
```

`rm` 是必要的：舊資料庫會被復原機制接回去，開場就不是乾淨的一堂課。

四個分頁先開好、放大到 100%，投影機那台用 `/live`：

| 分頁 | 網址 |
|---|---|
| 大螢幕 | <http://127.0.0.1:8730/live> |
| 排課 | <http://127.0.0.1:8730/workout> |
| 報到 | <http://127.0.0.1:8730/checkin> |
| 排行榜 | <http://127.0.0.1:8730/leaderboard> |

再開一個終端機，字放大，這是「後台」——目前還沒有 operator 網頁介面（M6 未完成），
寫入動作都走 `curl`。誠實講出來即可，見 §6。

---

## 3. 場景

一台機器、四個畫面、一個廣播器（MQTT）。開場先講這張圖，之後每一幕都指得回來：

```text
RFID 手環  →  ESP32 讀卡機  →(MQTT)→  HYROX 訓練與競賽應用系統  →  投影機 / 教練手機 / 櫃檯平板
                                          ↓
                                       SQLite（先落地才回 ACK）
```

展示用的 ESP32 是模擬的，但它走的是**同一條路**：發布 → 訂閱 → 解碼 → 落地 → ACK。
沒有任何一段被抄近路。這句話值得講兩次，因為第 4 幕靠它。

---

## 3.5 兩分鐘影片版

本文件的七幕是**現場展示**用的（25 分鐘、有人在旁邊講解）。影片是另一個東西：
**1 分 59 秒、剪出來的，不是演出來的**，而且沒有旁白也要看得懂。

字幕稿在 [`design/demo/captions.py`](../design/demo/captions.py)：

```bash
python3 design/demo/captions.py                 # 時間軸 + 字數與片長檢查
python3 design/demo/captions.py --srt > design/demo/captions-reliability.srt
```

24 則、1 分 59 秒。`.srt` 直接拖進剪輯軟體或上傳 YouTube，不必安裝任何東西。

### 分鏡與時間預算

| 段 | 時間 | 錄什麼 | 剪掉什麼 |
|---|---|---|---|
| A 開場 | 0:00–0:14 | `/live` 動起來的畫面 | 開機過程、終端機。第一格就是成品 |
| B 現場報名 | 0:14–0:38 | `/checkin` 待綁定 → 綁定 → `/live` 冒出 99 號 | 打字。指令預先貼好，只錄結果 |
| C 拔插頭 | 0:38–1:37 | 殺主機 → 斷線畫面 → `mosquitto_pub` → 重開 → `sqlite3` 兩個時間戳 → 重送後仍是 1 筆 | 編譯輸出、等待。**全片一半的長度給這一段** |
| D 成績 | 1:37–1:52 | `/leaderboard`，名次留白 | 捲動 |
| E 收尾 | 1:52–1:59 | 關掉 Wi-Fi 重新整理，畫面完好 | — |

**整段砍掉的**：排課 `/workout`、收工 `safe_to_stop`、例外處理、多語切換。
2 分鐘塞五件事只會五件都記不住；這些留給現場版和加演。

### 錄影三件事

1. **`SPEED` 保持預設的 `12`。** §2 要求改成 `3` 是給現場展示的；影片剪得動，
   20 分鐘的課 100 秒跑完剛好，`/live` 的數字跳得夠快，畫面才不會像靜止圖。
2. **指令全部預先貼在終端機裡**，只按 Enter。錄打字等於錄空景。
3. **關鍵那一格要壓準**：C 段的「成績只用刷卡的時間」必須壓在 `detected_at` /
   `received_at` 兩個數字出現的那一秒。這是全片唯一非壓不可的對點，
   其他字幕差半秒沒人看得出來。

### 改稿規則

- **寫系統在做什麼，不寫主持人在點什麼。**
- 一行 20 個全形字以內、最多兩行；`captions.py` 會標出超過的。
- **超過 120 秒就砍句子，不要縮秒數。** 縮到讀不完，等於沒有字幕。
  檢查輸出會直接告訴你超出幾秒。
- 團課版的字幕在同一個檔案裡：`python3 design/demo/captions.py group-class`。
- 要簡中或英文版，複製一份 `SCRIPTS` 裡的清單換字串。
- 要旁白就念字幕稿，不要另寫一份——兩份文案會在改稿時分岔。

## 4. 主線（25 分鐘）

### 幕 1 — 開機就是一堂課（3 分鐘）

```bash
cargo run -p hub-server
```

終端機會說：

```text
started new session s-1788230847678
MQTT connected (broker kept our session: true); subscribed to hyrox/v1/edge/+/events
dev simulator: emulated collector a4cf128b3d91 publishing to 127.0.0.1:1883
HYROX Central Hub listening on http://127.0.0.1:8730/live
```

切到 `/live`。12 名學員、8 個站點的課表、每個人在哪一站、站內多久、轉場多久，
自己動起來。

**講**：這台機器插電就是這樣。沒有登入、沒有安裝、沒有雲端。你剛剛看到的每一格，
是一次真的刷卡經過 broker 進到資料庫。

**指一下右上角的新鮮度**（`last_event_age_ms`）：畫面凍住和場館沒人，長得一模一樣，
所以我們把「上一筆事件多久以前」印在螢幕上。

### 幕 2 — 現場報名，手環後綁也算數（5 分鐘）

先讓一個沒登記的手環刷過 SkiErg：

```bash
NOW=$(curl -s localhost:8730/api/live | python3 -c 'import json,sys;print(json.load(sys.stdin)["snapshot"]["now"])')
mosquitto_pub -h 127.0.0.1 -q 1 -t hyrox/v1/edge/a4cf128b3d91/events \
  -m "{\"device_id\":\"a4cf128b3d91\",\"reader_id\":\"rfid-skierg-entry\",\"boot_id\":99,\"sequence\":2,\"tag_id\":\"TAG-WALKIN-01\",\"detected_at\":$NOW,\"uptime_ms\":124000}"
```

切到 `/checkin`：`TAG-WALKIN-01` 出現在待綁定清單。**這時候還沒有人是這隻手環的主人。**

現場報名 + 綁定（實測可用；`bib` 是數字不是字串）：

```bash
H='x-operator-device: 櫃檯平板'
A=$(curl -s -X POST -H "$H" -H 'content-type: application/json' \
  -d '{"display_name":"王小明 (現場報名)","bib":99}' \
  localhost:8730/api/checkin/entrants | python3 -c 'import json,sys;print(json.load(sys.stdin)["athlete_id"])')
curl -s -X POST -H "$H" -H 'content-type: application/json' \
  -d "{\"tag_id\":\"TAG-WALKIN-01\",\"athlete_id\":\"$A\"}" localhost:8730/api/checkin/bind
```

回應會帶回**補記的解讀**：

```json
{"claimed":[{"kind":"ENTERED","station":"SKIERG","at":1788231501822,"started_timing":true}]}
```

`/live` 上 99 號王小明立刻出現，狀態 ACTIVE、站在 SKIERG，**計時從他真正刷卡那一刻算起**，
不是從櫃檯打完字那一刻。

**講**：現場一定會有沒登記就進場的人。系統的選擇不是擋掉他，是先把刷卡留著，
等有人認領再回頭解讀。

**順便指**：`x-operator-device: 櫃檯平板`。沒有登入，但每一筆寫入都記得是哪台平板做的。

### 幕 3 — 教練自己排課（4 分鐘）

切到 `/workout`。四份系統課表（HYROX Complete Short / Engine 800 / Engine Short / Power），
拖拉編輯，系統課表唯讀、要改先複製。

**講**：課表是可以編輯的物件，但**正在跑的課永遠不讀課表**——建立課程的那一刻，
課表被編譯成固定的關卡順序，拍成快照釘在這堂課上。所以明天改課表，不會改到今天的成績。

> ⚠️ **不要在主線中途按「建立課程」。** 實測：新課程的名單是空的，模擬器的刷卡會全部
> 變成 `ATHLETE_NOT_IN_SESSION` 例外（我踩過，一次 47 筆），大螢幕當場停住。
> 要演建立課程，放到加演 A，或用第二台/第二個資料庫。

### 幕 4 — 拔插頭（6 分鐘，全場最重要）

先記下現在的時間戳，然後**當著大家的面把主機殺掉**：

```bash
NOW=$(curl -s localhost:8730/api/live | python3 -c 'import json,sys;print(json.load(sys.stdin)["snapshot"]["now"])')
pkill -f 'target/debug/hub-server'
```

`/live` 立刻顯示斷線。**停三秒讓大家看清楚。**

主機躺著的時候，讀卡機還在刷：

```bash
mosquitto_pub -h 127.0.0.1 -q 1 -t hyrox/v1/edge/a4cf128b3d91/events \
  -m "{\"device_id\":\"a4cf128b3d91\",\"reader_id\":\"rfid-wall_balls-entry\",\"boot_id\":99,\"sequence\":1,\"tag_id\":\"TAG-A1\",\"detected_at\":$NOW,\"uptime_ms\":123456}"
```

開機：

```bash
cargo run -p hub-server
```

```text
resumed session s-1788230847678 (ClassDuration { limit: Duration(1200000) }) with 12 athletes, 72 events already interpreted
MQTT connected (broker kept our session: true)
```

攤開帳本（實測輸出）：

```bash
sqlite3 hyrox.db "select id,boot_id,sequence,tag_id,detected_at,received_at from raw_events where boot_id=99;"
73|99|1|TAG-A1|1788231290790|1788230888870
```

**講三句，不要多**：

1. 那筆刷卡沒有掉——主機不在的時候，broker 幫它排隊（客戶端 id 固定、QoS 1、非乾淨連線）。
2. `detected_at` 還是刷卡當下那一刻。`received_at` 是主機開機才收到的時刻。
   **成績用前者。** 傳輸慢一分鐘，成績不會慢一分鐘。
3. 這堂課接著原本的狀態跑下去，名單、課表、完成規則都是原本那份，不是預設值。

再送一次一模一樣的封包（示範重複投遞）：

```bash
mosquitto_pub -h 127.0.0.1 -q 1 -t hyrox/v1/edge/a4cf128b3d91/events \
  -m "{\"device_id\":\"a4cf128b3d91\",\"reader_id\":\"rfid-wall_balls-entry\",\"boot_id\":99,\"sequence\":1,\"tag_id\":\"TAG-A1\",\"detected_at\":$NOW,\"uptime_ms\":123456}"
sqlite3 hyrox.db "select count(*) from raw_events where boot_id=99;"   # 實測仍然是 1
```

**講**：讀卡機重送是對的行為——ACK 掉了就該重送。重送不能變成兩筆紀錄，這是主機的事。

### 幕 5 — 成績要誠實（3 分鐘）

`/leaderboard`。這堂課的完成規則是「上課時間到就結束」，所以：

```json
{"ordering":"BIB","rows":[{"place":null,"stations_completed":3}]}
```

**講**：時間到才收工的課，每個人做的量不一樣，照時間排名是不誠實的，所以我們**不排**，
按號碼出。只有規則是「走完全程才算完成」的課才會有名次。這是刻意的設計決定（ADR 0010），
不是還沒做。

`/result` 看完賽的那一堂；`?session=<id>` 可以翻出幾小時前那堂。

### 幕 6 — 收工（2 分鐘）

```bash
curl -s localhost:8730/api/health
{"session_status":"RUNNING","class_live":true,"devices_with_backlog":2,"safe_to_stop":false,
 "blocked_by":["CLASS_RUNNING","DEVICE_BACKLOG"]}
```

**講**：機器每天晚上會自己檢查要不要更新、更新完關機。它關機前只問一件事：現在關安全嗎？
安全的定義寫在這裡——課還在跑不能關，還有讀卡機有東西沒送完也不能關。
`blocked_by` 會把**所有**理由列出來，不會修好一個就藏起另一個。

### 幕 7 — 沒有網路也讀得到（2 分鐘）

把 Wi-Fi 關掉，重新整理 `/live`。字型、圖示、翻譯全部還在——都從主機自己出，不走 CDN。
`/live?lang=zh-Hans` 換簡中，`/workout` 右上角有切換鈕（繁中／簡中／英）。

**講**：場館的網路一定會斷。斷網的時候資料是對的、畫面看不懂，是最糟的組合。

---

## 5. 加演（看時間與對象挑）

| | 主題 | 做什麼 | 為什麼值得演 |
|---|---|---|---|
| A | **教練建立明天的課** | 乾淨資料庫 + `POST /api/operator/class`（`template_id` / `session_id` / `name` / `finish_policy`），DRAFT → READY | 完整走過「課表 → 編譯 → 快照 → 開課」 |
| B | **誤刷怎麼修** | `GET /api/operator/exceptions` → `POST .../{id}/void`（**必須帶 `reason`**，否則 422 `REASON_REQUIRED`） | 原始刷卡永遠不改，只在解讀層打叉；每筆修正都要有人、有時間、有理由 |
| C | **讀寫分離是型別擋的** | 對唯讀路徑發 POST → axum 直接 405，程式碼還沒跑到 | 大螢幕那台平板**寫不了東西**，不是因為我們記得檢查 |

---

## 6. 一定會被問到的，先講好答案

誠實比漂亮重要。這些是現在**還沒做**的：

| 問題 | 答案 |
|---|---|
| 「有正式比賽的判罰嗎？」 | 沒有。競賽的站點順序驗證還沒做（M4 部分完成）。順序不對只會標記 `OUT_OF_ORDER`，**記錄但不裁決**——判罰規則沒定案之前，寫進程式碼是猜的。 |
| 「操作介面在哪？」 | 例外清單和讀卡機設定還沒有網頁（M6 部分完成），今天用指令示範。API 已經完備、有測試，接介面是工。 |
| 「這台機器實際出貨過嗎？」 | 沒有。systemd、`.deb`、kiosk、自動安裝檔都寫好了（ADR 0009），但**還沒在實機驗證過**。 |
| 「跟健身管怎麼接？」 | 會員資料留在健身管，這裡只存 `member_id` / 姓名 / 會籍狀態的參照。實際 API 合約還沒拿到（未解議題）。 |
| 「幾個人可以同時跑？」 | 沒有實測過人數上限。有 459 個自動測試，但沒有壓力測試數字——不編。 |
| 「跑完的定義是什麼？」 | 是設定，不是程式。今天這堂是「上課時間到」；也可以設「走完全程」。刻意沒有寫死。 |

---

## 7. 出事時

| 症狀 | 現場處置 |
|---|---|
| 大螢幕不動、右上角年齡一直長 | broker 沒起來。`brew services restart mosquitto`，主機會自己重連（它一直在重試）。 |
| 開機後畫面是上一場的殘骸 | 復原機制在做它該做的事。要乾淨的話：停掉、`rm -f hyrox.db*`、重開。**別在台上做**，講出來就好。 |
| 課跑太快，還沒講完就結束 | `SPEED` 沒改到。重開一次課接著演，或直接跳第 4 幕（第 4 幕不需要課還在跑）。 |
| `curl` 回 `400 OPERATOR_REQUIRED` | 少了 `x-operator-device` 標頭。這是刻意的：沒有署名的稽核紀錄比沒有紀錄更糟。 |
| 一堆 `ATHLETE_NOT_IN_SESSION` 例外 | 你在課中途建了新課程（見幕 3 的警告）。把它變成加演 B：例外沒有被吞掉，全部躺在待處理清單裡。 |

備援：整場最強的一幕（第 4 幕）**不需要模擬器**——只要 broker、一行 `mosquitto_pub`、
一次重開機。真的什麼都壞了，就演這一幕。
