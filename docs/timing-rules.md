# Timing Rules

本檔記錄 Central Hub 的計時規則現況。**未決議的規則不得寫死在程式碼中**（CLAUDE.md 12、28）。

對應程式碼：`crates/domain/src/time.rs`、`crates/domain/src/athlete.rs`、
`crates/domain/src/finish.rs`、`crates/domain/src/config.rs`。

## 時間基準（CLAUDE.md 11、17）

- 官方計時一律使用事件的 `detected_at`（ESP32 偵測時刻）。
- `received_at`（Hub 收到時刻）只作診斷，**不得**用於計算成績。
- Domain 以 `Instant(i64)` / `Duration(i64)` 兩個 newtype 表示 epoch 毫秒與時間差，
  避免兩者互相誤用。

## Start Rule — 已定義（CLAUDE.md 11）

Session ARMED 之後，該選手的**第一個有效事件**啟動其計時，
`started_at = detected_at`。

「有效」的判定目前由 `domain::decide` 執行：Session 必須為 ARMED、選手尚未 FINISHED、
且該讀取在選手當下狀態中可被解讀。Reader 與 Tag 的已知性由上游負責
（`ReaderRegistry::resolve` 回傳 `UnknownReader`、`BindingLedger` 回傳 `None`）。

測試：`crates/domain/tests/interpretation.rs`
- `first_valid_event_after_arming_starts_timing_at_detected_at`
- `only_the_first_event_starts_the_clock`
- `events_before_arming_are_recorded_as_exceptions_not_dropped`

## Transition / ROX — 已定義（CLAUDE.md 13）

```text
Transition = 下一個 Entry 的 detected_at - 前一個 Exit 的 detected_at
```

第一個站點沒有 transition（沒有可相減的前一筆）。此值為衍生資料，
儲存在 `StationRun::transition_from_prev`，可由 interpreted event 重算。

競賽 UI 顯示為 ROX Zone Time，訓練模式顯示為 Transition Time；
兩者是同一個計算，差別只在名稱。

測試：`transition_is_next_entry_minus_previous_exit`、`first_station_has_no_transition`。

## Finish Rule — 訓練已決議，**競賽仍未決議（OPEN）**

> CLAUDE.md 12 與 28 列為未決的是**競賽**的完成規則。訓練已於 2026-08-27 與使用者確認。

### 現況實作

`domain::FinishPolicy` 是 Session 設定上的策略值：

```rust
pub enum FinishPolicy {
    #[default]
    NotConfigured,                       // 競賽仍在這裡
    ClassDuration { limit: Duration },   // 團課時間到就結束
    CoachDecides,                        // 沒有自動觸發，教練手動結束
}
```

`FinishPolicy::evaluate()` 回傳三值的 `FinishDecision`：

```rust
pub enum FinishDecision { Finished, NotFinished, Undetermined }
```

`NotConfigured` 一律回傳 `Undetermined`。刻意不用 `bool`：
兩值答案會迫使「尚未定義」回報成 `false`，而 `false` 讀起來是
「已經判定、尚未完成」——那是一個被偷渡進來的規則。

**誰會設定 `AthleteStatus::Finished`（ADR 0003）：**

- `application::apply_finish_policy()`：每個 tick 評估一次，只有 `Finished` 才呼叫
  `domain::finish`。`Undetermined` 一律不動作，也不會被當成 `NotFinished`。
- `application::end_class()`：教練手動結束。當策略是 `NotConfigured` 時**拒絕執行**，
  因此競賽不可能經由這條路徑被結束。
- 人工修正（CLAUDE.md 20）。

完成狀態**不寫入 interpreted_events**：它由班級時鐘與既有事件推導，重啟後下一個 tick
會重新推導出相同結果（CLAUDE.md 21）。寫一筆 FINISHED 等於捏造沒有 reader 觀測到的事件。
被時間結束時仍在站內的選手，該站保持未關閉——沒有人回報他離站。

測試：
- `crates/domain/tests/course.rs`：`the_finish_policy_defaults_to_not_configured`、
  `an_unconfigured_finish_policy_never_decides_an_athlete_is_finished`
- `crates/application/tests/finish.rs`：時間到、未到、未進場、重複套用、
  `an_undecided_rule_finishes_nobody_ever`、
  `ending_a_class_by_hand_is_refused_when_no_finish_rule_exists`

### 競賽仍待釐清的問題

1. 完成是由「最後一站 Exit」判定，還是需要專屬的 Finish Reader？
2. 課表（`Course`）跑完是否等於完成？若中途少一站呢？
3. 誰有權把選手標記為完成——自動判定、還是 operator 手動？
4. `Session` CLOSED 是否要連帶處理仍為 ACTIVE 的選手？
   （ADR 0001 已註明 CLOSED **不**代表所有人都完成。）

### 決議後要做的事

1. 在 `FinishPolicy` 增加 variant，並在 `evaluate()` 實作。
2. 先寫測試（CLAUDE.md 24），再實作。
3. 更新本檔，並視影響範圍撰寫 ADR（CLAUDE.md 30）。
4. `crates/storage` 需要新增 finish policy 的持久化與解析（目前 SessionConfig 尚未持久化）。

在此之前，任何模組都**不得**自行推論競賽選手是否完成。

## 其他未決事項

- 競賽模式的站點順序驗證與例外處理（CLAUDE.md 9.1）尚未實作；
  `Course` 可作為 competition template，但目前不做順序強制。
- 訓練模式**永不**因順序不同而產生 exception（CLAUDE.md 9.2、ADR 0001 D4）。
  這是刻意的，不是缺漏。
