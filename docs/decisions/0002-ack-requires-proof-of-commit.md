# ADR 0002 — ACK 必須以持久化憑證換取

- 狀態：已決議
- 日期：2026-08-27
- 影響範圍：`crates/mqtt`（新增）、`crates/simulator`（新增）、未來的 Hub ingestion 接線
- 不影響：`domain`、`storage`（本次未接線）
- **2026-08-28 更新（ADR 0005）**：本文寫成時，契約與 ACK 協定都在 `crates/mqtt`。
  之後契約與 ACK 協定移到 `crates/contract`，`crates/mqtt` 改名 `crates/transport`
  且只留 topic / broker client。**型別保證原封不動**：`Ack` 仍無公開建構子、無
  `Deserialize`，仍只能由 `Commit::into_ack` 取得，而 `Commit` 仍只在 `contract::ingest`
  裡、commit 成功的下一行鑄造。以下內文的 `crates/mqtt` 請讀作 `crates/contract`。

## 背景

CLAUDE.md §15 規定：

> Do not ACK before persistent storage commit succeeds.

CLAUDE.md §31 把「No lost RFID events」列為第一優先。

這條規則一旦被違反，失敗是**靜默**的：Hub 先回 ACK、ESP32 釋放事件、
之後 commit 失敗——事件從此不存在於任何地方，而且沒有人會發現。
現場沒有任何補救手段。

以註解或 code review 保證這件事是不夠的：接線的人、之後改 ingestion 迴圈的人、
以及未來加上批次處理的人，都有機會不小心把順序寫反。

## 決議

**把「先 commit 再 ACK」變成型別規則，而不是紀律。**

```text
EventStore::commit() -> Ok(CommitOutcome)   ← port，Hub 之後以 SQLite 實作
        ↓ 只有 ingest() 能在這一行鑄造
      Commit                                 ← 「已持久化」憑證
        ↓ into_ack()
       Ack                                   ← 沒有公開建構子
        ↓
   publish_ack(client, &Ack)                 ← publish 只收 Ack
```

- `Ack` 沒有公開建構子，也沒有 `Deserialize`。
- 線路格式另有 `AckPayload`（可序列化、可反序列化），供邊緣端解析。
  任何人都可以**讀**一則 ACK，只有 commit 可以**造**一則 ACK。
- `IngestError::Storage` 明確代表「事件尚未持久化」，此路徑不產生 `Ack`。

想提早 ACK 的程式碼手上沒有東西可以 ACK。

## 考慮過的替代方案

**A. 只寫註解與測試。**
成本最低，但保護不了未來的修改；而且這個錯誤在測試裡也不容易長期釘住
（新增的 ingestion 路徑不會自動被既有測試覆蓋）。

**B. 讓 `EventStore::commit` 直接回傳 `Ack`。**
可行，但把 ACK 的生成責任推給每一個 store 實作，包含測試用的假 store，
反而多了幾個能造出 `Ack` 的地方。

**C. 執行期斷言（ACK 前查詢資料庫確認該筆存在）。**
每筆事件多一次查詢，且在錯誤發生時已經來不及——事件早已進入 ACK 路徑。

選 B 的變形：憑證由 `ingest` 單點鑄造，store 只回報結果。

## `EventStore` 為何是 async trait

真實 store 是 sqlx / SQLite，本質非同步。若 port 設計成同步，Hub 端只能 block，
在 Tokio 執行緒上是有害的。

使用 Rust 1.75+ 的 async fn in trait，代價是該 trait 不是 dyn-compatible；
呼叫端走泛型（`ingest<S: EventStore>`）即可，Hub 不需要 `dyn EventStore`。

## 為何 `crates/mqtt` 不依賴 `domain` 與 `storage`

CLAUDE.md §3 的依賴方向：基礎設施依賴 domain，反之不可。
線路層是契約，`storage` 是它的 adapter。

> ADR 0005 修正了這一段的前半：契約**確實**依賴 `domain`，但只取身分型別
> （`DeviceId` / `ReaderId`，CLAUDE.md 7.3）。當初在線路層另外複製一份身分型別
> 的做法造成同一概念有兩個型別、兩套驗證，那是錯的。方向仍然朝內，`storage`
> 與 `application` 仍然不可被契約看見。

實務效果：`crates/mqtt` 與 `crates/simulator` 可以在完全沒有 SQLite、
沒有 broker 的情況下編譯與測試（CLAUDE.md 24）。
`crates/mqtt` 的 `broker` feature 關閉後，rumqttc 甚至不會進入建置
（ADR 0005 之後：該 feature 在 `crates/transport`，`crates/contract` 根本不含 rumqttc）。

`storage::RawEvent` 目前以 `String` / `i64` 表示同一組欄位，語意一致
（同樣以 `device_id + boot_id + sequence` 判重），接線時做一次淺層轉換即可。

## 可攜性影響（macOS / Linux）

無平台相依。兩個 crate 都是純運算 + serde + rumqttc，
沒有檔案系統、沒有行程管理、沒有平台 API。
`crates/simulator` 的 journal 是 flash 的**記憶體模型**，不是 flash driver，
目的是讓語意可測試並給韌體一份規格。

## 後續

- Hub 端以 SQLite 實作 `EventStore`，`commit` 回 `Ok` 前必須確定交易已提交。
- 若 10,000 筆 backlog 重送時每筆一則 ACK 造成流量問題，改為批次 ACK cursor；
  屆時 `Commit` 需能代表一段區間，型別保證的形狀不變。
