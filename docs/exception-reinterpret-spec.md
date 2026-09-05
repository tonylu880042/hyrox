# 異常改判可靠性開發規格

日期：2026-09-05  
狀態：規格草案，供審閱；尚未實作。  
依據：本次 code review、`CLAUDE.md` 第 19–21 節、ADR 0001 D4。

## 1. 目標與範圍

讓現場操作者修正異常刷卡後，得到正確且可恢復的選手計時資料。修正目前三項問題：

1. 使用目前選手狀態計算歷史事件，導致起跑、轉場與後續衍生資料錯誤。
2. 作廢、新增與審計分別提交，失敗後留下部分完成的改判。
3. `TOGGLE`、`CHECKPOINT`、`PASSAGE` 被默默轉為 ENTRY。

範圍包含 domain 重播、application 改判用例、SQLite 儲存、API 驗證與相關測試。保留現有 API 路徑、成功回應形狀、裝置操作者識別與原因要求。

不包含新 UI、批次改判、登入系統、重新解讀所有 raw reads，或新增比賽規則。原始 RFID 資料不得更動。

## 2. 設計假設與決策狀態

以下是本規格提出的設計方向，不代表已批准的產品規則：

- 改判指令僅表達 ENTRY 或 EXIT；設備本身的 `ReaderMode` 仍保留全部模式。
- 原事件種類、站點、選手與有效時間是重播的事實；`started_timing`、`transition` 是可重新推導的資料。
- 改判後不自動把其他 Exception 轉成有效事件，也不從 raw reads 覆寫先前的人工判定。
- 相同時間的事件沿用現有 `(detected_at, id)` 升冪排序；新增更正取得新 ID，因此排在同時間舊事件之後。
- 對改判所新增的歷史序列衝突，建議拒絕整筆指令並回報衝突，不默默修補其他站點紀錄。此政策須確認，見第 10 節。

## 3. API 契約

`POST /api/operator/exceptions/{id}/reinterpret`

```json
{
  "station": "SKIERG",
  "mode": "ENTRY",
  "athlete_id": "a2",
  "at": 1788573600000,
  "reason": "修正站點與手環歸屬"
}
```

| 欄位 | 規則 |
| --- | --- |
| `station` | 必填；不得為空白。是否要求站點已登錄，沿用專案既有站點政策，不在本次另設限制。 |
| `mode` | 必填；只接受精確字串 `ENTRY`、`EXIT`。 |
| `athlete_id` | 可省略，沿用原異常歸屬；目標必須存在於目前 session roster。 |
| `at` | 可省略，沿用原異常有效時間；單位為 epoch milliseconds。計時運算不得溢位。額外時間範圍限制見第 10 節。 |
| `reason` | 必填且去除首尾空白後不得為空；沿用 `REASON_REQUIRED`。 |

操作者 header 沿用現有規則。成功維持 HTTP 200 與 `ExceptionsResponse`。

模式建議使用 application 專用 `ReinterpretMode`，由 wire DTO 引用或轉換；不得縮減設備 `ReaderMode`。非支援模式、大小寫錯誤或不合法 JSON 回 HTTP 400 `INVALID_BODY`，不得产生任何儲存變更。application 型別也不得提供隱含轉 ENTRY 的途徑。

保留既有 `OPERATOR_REQUIRED`、`REASON_REQUIRED`、`UNKNOWN_ATHLETE`、`UNKNOWN_EVENT` 狀態映射。待確認的新增錯誤為 HTTP 409 `CORRECTION_CONFLICT`，供歷史序列衝突使用；不可表示儲存故障。

## 4. 歷史重算規則

### 4.1 重播來源與順序

以目前 session 內未作廢的 interpreted events 為輸入，按 `(detected_at, id)` 升冪重播。納入更正事件、排除原異常後，從目標選手初始狀態重建。可先沿用全 session 重建，無須為本次引入增量快取。

新事件及其後續事件都必須重新推導計時欄位，不能只修正新增事件。否則後續原 ENTRY 的 `started_timing = true` 仍會覆蓋更早的起跑時間。

### 4.2 計時與狀態

- 第一個有效 ENTRY 決定 `started_at`，之後 ENTRY 不得再次重設起跑時間。
- 第一個 ENTRY 的 `transition_from_prev` 為 `None`。
- 後續 ENTRY 的轉場時間取有效時間減去歷史序列中的前一個有效 EXIT；不得使用指令執行當下的 `last_exit_at`。
- EXIT 對應當時尚未關閉的相同站點 run，重新推導 split 與站點狀態。
- Exception 本身不推進選手狀態；accepted exception 的既有語意不變。
- 計時使用安全算術，無法表示的時間差拒絕整筆指令，不得 panic、溢位或直接歸零。
- 依既有 finish policy 重建受影響的完成狀態、總時間與排名；保留教練結束與班級時限等獨立結束事實。不可用舊的課程完成快取遮蔽修正後的結果。

建議將「既定事件套用並推導計時」放在 domain 純函式，改判、重启恢復及後續相關重播共用。不要直接以目前 session 狀態呼叫讀卡 `decide` 重新判斷所有歷史 raw reads。

若保留資料庫的 `started_timing`、`transition_ms` 欄位，必須明確當作快取而非重播權威；所有讀取路徑須使用一致結果，不能只有改判回應算對而重啟後還原舊值。實作時盤點欄位消費者並決定統一重算或在交易內更新快取。

### 4.3 具體案例

原紀錄為 10:00 UnknownReader、10:02 EXIT 異常、10:05 ENTRY、10:10 EXIT。操作者將 10:00 改成 ENTRY 時，10:05 ENTRY 會形成重複進站；依建議衝突政策，整筆拒絕，原資料不變。

無衝突案例：原紀錄為 10:00 異常、10:05 ENTRY、10:10 EXIT，將異常改成 10:12 ENTRY，重播後起跑仍是 10:05、轉場為兩分鐘。再以其他測試覆蓋補入較早 EXIT 對後續 ENTRY 轉場的影響。

測試純計時重播時，必須另外驗證多個儲存事件帶有舊 `started_timing = true` 時，只採第一個有效 ENTRY 起跑，確保不再依賴過期欄位。

## 5. 原子提交與失敗語意

在 `HubStore` 定義一次完成改判的用例級操作；application 不得再串接三個獨立提交的儲存方法。具體 Rust 簽名在後續實作設計定案，契約必須包含 session、來源 ID、更正内容及審計資料，並回傳新事件 ID 與一致的重建結果。

SQLite 必須在同一 transaction 內：

1. 確認來源屬於指定 session，且仍是未作廢、未接受的待處理 Exception。
2. 建立更正後候選歷史，完成必要驗證與衍生狀態重算。
3. 將原異常標記作廢，插入更正事件，保留 `raw_event_id` 來源連結。
4. 寫入 `EVENT_REINTERPRET` 審計，必要時同步更新衍生快取。
5. 全部成功才 commit。任何錯誤都 rollback。

候選重播必須採用最終事件排序；可在交易內插入後驗證，再於失敗時回滾。不得在交易外驗證後，忽略中間狀態變更。

延續既有 Hub session lock 保護寫入。重建结果在 transaction 內準備完成，只有 commit 成功後才替換記憶體；不可 commit 後還需要另一個可能失敗的資料庫重算才能更新記憶體。程序若在 commit 後、記憶體替換前終止，重啟須由已提交紀錄恢復同樣結果。

重試語意：

- 明確 rollback 的失敗：原異常仍可見且可再次改判。
- 已成功提交後重送：來源不再可改判，回 `UNKNOWN_EVENT`；不得建立第二筆更正。
- 回應遺失或提交結果不確定：用戶端刷新 inbox 判斷，不承諾相同請求重送必回 200。本次不引入 idempotency key。
- 成功後取得 inbox 回應失敗，不代表 transaction 已回滾；仍須維持記憶體與持久化結果一致。

審計保留 operator、操作時間、reason、before、after。before/after 採可解析結構，包含原／新事件 ID、選手、有效時間、事件種類、站點及 raw_event_id；操作時間與有效時間分開記錄。

## 6. 專案位置與程式慣例

技術基礎為現有 Rust 2021 workspace（宣告最低 Rust 1.83）、Tokio、Axum、Serde 與 SQLx／SQLite，版本沿用 Cargo.lock，不新增框架。

| 位置 | 責任 |
| --- | --- |
| `crates/domain/src/athlete.rs`、相關 domain 模組 | 純歷史重播與計時推導；無 SQL、HTTP 或 IO。 |
| `crates/application/src/exceptions.rs`、`ports.rs`、`operator.rs` | 專用模式、改判契約與用例錯誤。 |
| `crates/storage/src/hub_store.rs`、`lib.rs` | transaction、條件更新、恢復路徑與 domain 重播整合。 |
| `crates/api/src/wire.rs`、`operator.rs`、`error.rs`、`state.rs` | request 驗證、錯誤映射與記憶體提交整合。 |
| 各 crate 的 `tests/` | 純邏輯、用例、SQLite 與 HTTP 回歸測試。 |
| `docs/api.md`、`docs/open-issues.md` | 正式契約、限制及完成狀態。 |

沿用 snake_case、型別化錯誤、`Result` 及既有 rustfmt 格式。模式處理應完整列舉，例如：

```rust
match spec.mode {
    ReinterpretMode::Entry => build_entry(/* historical context */),
    ReinterpretMode::Exit => build_exit(/* historical context */),
}
```

此片段只示意型別與分支風格，不指定尚不存在的函式介面。

## 7. 測試與驗收矩陣

| 編號 | 情境 | 驗收結果 |
| --- | --- | --- |
| T01 | Ready 選手異常改 ENTRY | 正確起跑、建立 run、清除 badge。 |
| T02 | 現在已出站，補入較早 ENTRY／EXIT | 使用歷史前綴推導；不從最新 last_exit_at 產生錯誤負轉場。 |
| T03 | 改判改變後續 ENTRY 的前一個 EXIT | 後續 transition 一併更新。 |
| T04 | 重播輸入含過期起跑旗標 | 第一個有效 ENTRY 決定起跑；後續旗標不覆蓋。 |
| T05 | 轉移給另一選手並覆寫 at | 新選手歷史正確，原選手其他紀錄不變。 |
| T06 | 同時間事件與新更正 | SQLite、FakeStore、domain 遵守相同排序，結果確定。 |
| T07 | TOGGLE／CHECKPOINT／PASSAGE／未知 mode | HTTP 400 INVALID_BODY，原事件、審計與記憶體皆不變。 |
| T08 | 缺 operator、空白 reason、未知來源／選手 | 保留契約錯誤，零部分寫入。 |
| T09 | 作廢後、新增後、審計或 commit 前注入失敗 | 真實 SQLite rollback；inbox 原異常仍存在，重試可成功。 |
| T10 | 同一來源兩次或競爭改判 | 最多一筆成功更正，另一筆不新增資料。 |
| T11 | 成功後关闭並重開資料庫 | 選手狀態、split、transition、總時間、badge 與成功時一致。 |
| T12 | 課程完成／班級時限／教練結束 | 各自沿用既有結束規則，受影響結果與排名一致。 |
| T13 | 極端 i64 時間差 | 無 panic 或溢位；拒絕時零寫入。 |
| T14 | 新增序列衝突 | 依確認後政策處理；採拒絕方案時回 409 且完整回滾。 |
| T15 | 審計内容 | 原／新事件、有效時間、操作時間、歸屬與原因均可追查。 |

使用既有 Rust tests 與 `#[tokio::test]`。domain 測試覆蓋排序後的純計時推導，application 測試覆蓋用例與失敗行為，API 測試覆蓋 wire 契約。交易、排序與重啟驗收必須使用真實 SQLite；僅 FakeStore 通過不足以完成驗收。

修正 application FakeStore 按插入順序重播的差異；測試資料必須包含新增時間晚於歷史有效時間的更正。

## 8. 驗證指令

在 repository root 執行：

```sh
cargo fmt --all -- --check
cargo test -p domain -p application -p storage -p api
cargo build --workspace
git diff --check
```

上述是實作完成後的驗收指令，本規格產出不代表已執行或通過這些新增測試。需要手動啟動時使用 `cargo run -p hub-server`，MQTT broker 需求沿用 README。

## 9. 開發邊界與完成條件

必須做到：保留 raw 資料、既有審計歷史與設備模式；沿用分層依賴；失敗不留下部分改判；改判成功結果與重啟恢復一致。

需要另行確認：第 10 節產品政策，以及超出本規格的功能範圍。若實作需要 schema migration，須先記錄資料相容性與回復方式；不得為方便直接清空資料庫。

禁止：默默將其他模式轉 ENTRY、依目前狀態計算歷史計時、分次提交一筆改判、重新解讀 raw reads 而抹去人工判定、移除失敗測試以通過驗收。

完成條件：T01–T15 依確認後政策通過；第 8 節驗證通過；API 文件更新；原有 accept／void／讀卡行為無非預期回歸；文件僅在實作及驗收完成後標記已完成。

## 10. 待確認的產品決策

1. **新增歷史衝突的處理：**建議拒絕改判，回 409 並提供可定位的衝突事件，不自動改寫其他事件。需要區分既存衝突與本次新增衝突，避免舊資料問題讓所有修正都被封鎖。
2. **有效時間範圍：**目前只要求可安全計算；是否禁止早於班級開始或晚於班級結束，須由人工補錄規則決定，不能自行以 server now 取代。
3. **同時間排序：**建議維持 `(detected_at, id)`；若要求更正取代原異常的同時間位置，需另定穩定來源順序與可能的 migration。

本文件先界定需求及验收；上述政策定案後，再產出逐步實作計畫與工作拆分。
