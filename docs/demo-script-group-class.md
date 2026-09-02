# 系統展示劇本 — 團體課程

[docs/demo-script.md](demo-script.md) 演的是**可靠性**（不掉刷卡、斷電復原）。
這份演的是**日常怎麼用**：教練排一堂課、學員報到、上課、中途暫停、有人誤刷、下課出成績。

現場版 **15 分鐘**。每一步都在 2026-09-01 跑過一次，文中的輸出是實測值。

按鈕在 [`design/demo/group-class.sh`](../design/demo/group-class.sh)——**台上不要打字**：

```bash
source design/demo/group-class.sh && acts
```

---

## 1. 這場要證明三件事

| # | 承諾 | 哪一幕 |
|---|---|---|
| 1 | **課是教練自己排的**，不是工程師寫死的 | 幕 1–2 |
| 2 | **現場狀況改得動**：臨時報名、暫停、誤刷 | 幕 3、6、7 |
| 3 | **記錄是誠實的**：不猜、不判罰、不編成績 | 幕 7、9 |

---

## 2. 開演前

```bash
brew services start mosquitto
rm -f hyrox.db*                       # 乾淨的一堂課
cargo run -p hub-server
```

主機預設不帶任何示範資料（`HYROX_DEMO` 沒設），所以這一場的每一次刷卡都由你自己送出，
順序、時間、誰刷到哪一站，全部可控。刷卡用 `mosquitto_pub`——它就是這場的 ESP32，
走的是真的 MQTT 路徑，沒有任何一段被抄近路。

另一個終端機：

```bash
source design/demo/group-class.sh
```

分頁：`/workout`（排課）、`/checkin`（櫃檯）、`/live`（大螢幕）、`/leaderboard`（成績）。

> 開機時主機會自己建一堂空的課。`act0_clear` 把它取消掉，教練的課才排得進來。
> 這是開發用的預設行為，不是產品行為——講一句帶過就好。

---

## 3. 劇本

### 幕 1 — 教練排課（3 分鐘）　`act1_plan` / `act1b_readonly`

先看 `/workout`：四份系統課表，拖拉就能編。

```text
[('Power 團課（縮短版）', 'v2')]
```

複製 `HYROX Power`、把三輪改成兩輪，版本自動變成 v2。

**課表不綁日期**：它是一份可以重複開課的計畫。日期屬於「課程」——同一份課表這週四開一次、下週二再開一次，兩堂課各自有自己的成績。接著故意去改系統課表：

```json
{"error":"TEMPLATE_NOT_EDITABLE","message":"a system template cannot be edited or deleted; duplicate it first"}
```

**講**：系統附的課表是唯讀的，要改先複製一份。教練不會手滑改掉全店共用的範本，
而且能不能改是看**資料庫裡那一筆**，不是看送進來的資料說自己是誰。

### 幕 2 — 開課（2 分鐘）　`act2_open`

```text
('DRAFT', ['SLED PUSH','SLED PULL','FARMERS CARRY','SANDBAG LUNGES','WALL BALLS',
           'SLED PUSH','SLED PULL','FARMERS CARRY','SANDBAG LUNGES','WALL BALLS'])
```

**講**：兩輪五站，被攤平成 10 個關卡，**當場拍成快照釘在這堂課上**。
從這一刻起，這堂課再也不會去讀課表——明天教練改課表，改不到今天的成績。

課還在 DRAFT / READY 的時候可以微調（`PUT /api/operator/config`）；
一旦開始上課就鎖住，回 `SESSION_NOT_EDITABLE`。

### 幕 3 — 報到（3 分鐘）　`act3_checkin`

```text
陳怡君 TAG-G01 綁定完成
林建宏 TAG-G02 綁定完成
王淑芬 TAG-G03 綁定完成
黃培綺 TAG-G04 綁定完成
```

四個人現場報名、現場發手環。沒有會員資料也能上課——會員資料留在健身管，
這裡只需要一個名字和一隻手環。

**講**：每一筆寫入都帶著是哪一台平板做的（`x-operator-device: 櫃檯平板`）。
沒有登入，但有稽核。

> 手環後綁也算數（先刷卡、後綁定，系統回頭補記）是主劇本幕 2 的戲，這裡不重複演。

### 幕 4 — 開始（1 分鐘）　`act4_start`

`/live` 上四個人從 READY 變成準備中。課堂時鐘開始跑。

### 幕 5 — 上課（3 分鐘）　`act5_floor`

刷卡進站、出站。實測輸出：

```text
1 陳怡君 ACTIVE OUTSIDE -          分段: [('SLED PUSH',60000,None), ('SLED PULL',70000,20000)]
2 林建宏 ACTIVE INSIDE SLED PULL   分段: [('SLED PUSH',60000,None), ('SLED PULL',None,35000)]
3 王淑芬 ACTIVE OUTSIDE -          分段: [('SLED PUSH',80000,None)]
4 黃培綺 READY  OUTSIDE -          分段: []
```

**講**：每一站兩個數字——做了多久（60 秒）、上一站到這一站中間走了多久（20 秒）。
**轉場時間不需要多裝一組讀卡機**，它是兩次刷卡之間推算出來的。

黃培綺還沒刷過卡，所以她什麼都沒有。系統不會替她編一個開始時間。

### 幕 6 — 暫停（2 分鐘）　`act6_pause` → `act6_resume`

器材壞了、教練要講解。按暫停，多按幾次 `elapsed`：

```text
PAUSED 課堂時鐘 253728 ms
PAUSED 課堂時鐘 253728 ms      ← 數字不動
RUNNING 課堂時鐘 253848 ms
```

**講**：暫停會真的把課堂時鐘停住，不是畫面停住。暫停時間會被扣掉、會被存起來，
主機這時候重開機，回來還是暫停狀態，累積的暫停時間也還在。

### 幕 7 — 誤刷（3 分鐘）　`act7_misread` → `act7b_void`

讓王淑芬去刷今天還沒走到的牆球站：

```text
王淑芬 第 5 關 OUT_OF_ORDER
```

**講**：系統照實記錄，標記「順序不對」，然後**就這樣**。它不扣分、不判罰、
不假裝沒看到，因為誤刷該怎麼算是規則問題，不是程式該猜的事。

教練確認是誤刷，作廢那一筆。先故意不給理由：

```json
{"error":"REASON_REQUIRED","message":"this action changes recorded data, so it needs a reason"}
```

補上理由之後：

```text
-- 原始刷卡還在嗎（應為 1）：1
-- 稽核紀錄：教練平板|EVENT_VOID|10|誤刷：學員經過牆球區，沒有做
王淑芬 第 2 關（OUT_OF_ORDER 消失，關卡退回去）
```

**講三件事**：原始刷卡永遠不刪，只在解讀層打叉；每一筆修正都記得是誰、什麼時候、
為什麼；改完之後分段、名次、狀態全部自動重算，不會有人手動去對帳。

> 目前只有「例外清單」的事件會回傳 id，一般事件要作廢得從資料庫查（`act7b_void` 幫你查）。
> 操作介面還沒做（M6），這點誠實講。

### 幕 8 — 收工（1 分鐘）　`act8_end`

```text
判定完成： ['...-1', '...-2', '...-3']
COMPLETED
```

三個有刷過卡的人被判定完成；沒下場的黃培綺不在裡面。

### 幕 9 — 成績（2 分鐘）　`act9_results`

```text
週四 19:00 Power 團課 COMPLETED 排序依據: BIB
  1 陳怡君 FINISHED 完成關卡 2 名次 None
  2 林建宏 FINISHED 完成關卡 1 名次 None
  3 王淑芬 FINISHED 完成關卡 1 名次 None
  4 黃培綺 READY   完成關卡 0 名次 None
```

**講**：團課的規則是「時間到就下課」，每個人做完的量不一樣，照時間排名並不誠實，
所以名次留白、按號碼出。想要名次，把完成規則設成「走完全程才算完成」——
那時候系統就會按完成時間排。這是設定，不是改程式。

---

## 3.5 兩分鐘影片版

上面那九幕是**現場版**（15 分鐘、有人在旁邊講）。影片是剪出來的、沒有旁白也要看得懂：
**1 分 59 秒、24 則字幕**。

```bash
python3 design/demo/captions.py group-class                  # 時間軸與片長檢查
python3 design/demo/captions.py group-class --srt > design/demo/captions-group-class.srt
```

### 分鏡與時間預算

| 段 | 時間 | 錄什麼 |
|---|---|---|
| A 教練排課 | 0:00–0:19 | `/workout` 複製課表、三輪改兩輪、版本跳 v2 |
| B 開課 | 0:19–0:35 | 十個關卡的課表出現在大螢幕 |
| C 報到 | 0:35–0:49 | `/checkin` 四個人、四隻手環 |
| D 上課 | 0:49–1:10 | `/live` 分段時間與轉場時間長出來 |
| E 暫停 | 1:10–1:23 | 課堂時鐘凍住的那幾秒（**要錄滿，觀眾要看到數字不動**） |
| F 誤刷 | 1:23–1:48 | `OUT_OF_ORDER` 出現 → 作廢要理由 → 關卡退回去 |
| G 成績 | 1:48–1:59 | `/leaderboard` 名次留白 |

**整段砍掉的**：系統課表唯讀的示範、稽核紀錄的資料庫查詢、沒刷卡的人沒有成績。
這三件在現場版講得完，在 2 分鐘裡只會擠掉別的。

### 錄影三件事

1. **不要錄 `/result`**（見 §4）。成績畫面一律用 `/leaderboard`。
2. **指令全部預先貼好**，或直接用 `group-class.sh` 的函式，只按 Enter。
3. **E 段是唯一需要真實時間的鏡頭**：暫停之後停三秒再按恢復，數字不動才看得出來。
   其他段都可以剪。

---

## 4. 已知問題（展示前必讀）

**不要在這場開 `/result`。** 實測（2026-09-01）：同一堂課、同一時刻，

| 端點 | 陳怡君 |
|---|---|
| `/api/leaderboard` | `FINISHED`，1093212 ms |
| `/api/result/{id}` | `ACTIVE`，80000 ms，`finished_at: null` |

而且重開機之後 `/api/leaderboard` 也會變成 `ACTIVE`——「時間到」判定的完成只活在記憶體裡，
沒有被寫成事件，所以重放看不到它。學員拿到的成績單會顯示他還在跑。

這是真的缺陷，不是展示技巧問題。修法牽涉到「完成要不要記成一筆事件」，
需要先決定再改（見 [docs/open-issues.md](open-issues.md)）。**在修好之前，成績一律用 `/leaderboard`。**

其他還沒做的（跟主劇本共用）：競賽站順序驗證、例外與讀卡機的操作介面、實機出貨驗證、
健身管 API 合約。見 [demo-script.md §6](demo-script.md)。

---

## 5. 出事時

| 症狀 | 處置 |
|---|---|
| `act2_open` 回 `CLASS_IN_PROGRESS` | 忘了 `act0_clear`。 |
| 分段時間是負數 | 刷卡時間戳送到未來了。`act5_floor` 一律用「現在減去 N 秒」，照抄就不會錯。 |
| `act3_checkin` 回 `TAG_ALREADY_BOUND` | 資料庫不乾淨。停掉、`rm -f hyrox.db*`、重來。 |
| 刷卡沒反應 | broker 沒起來，或站名拼錯。讀卡機 id 是站名小寫加底線：`rfid-sled_push-entry`。 |
| 寫入回 `400 OPERATOR_REQUIRED` | 少了 `x-operator-device`。刻意的：沒有署名的稽核紀錄比沒有紀錄更糟。 |
