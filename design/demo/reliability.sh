#!/usr/bin/env bash
# 可靠性展示的按鈕（劇本：docs/demo-script.md）。一幕一個函式，台上只按 Enter。
#
#   source design/demo/reliability.sh
#   act_kill; act_publish_while_down; act_restart; act_proof
#
# 和團課版不同，這一場要**開關主機**，所以這裡知道怎麼把它叫起來。
# ponytail: 沒有錯誤處理，理由同 group-class.sh。

HUB=${HUB:-http://127.0.0.1:8730}
HUB_BIN=${HUB_BIN:-./target/debug/hub-server}
HUB_DB=${HYROX_DB_FILE:-hyrox.db}
HUB_LOG=${HUB_LOG:-/tmp/hyrox-hub.log}
DEVICE=a4cf128b3d91
DESK='x-operator-device: 櫃檯平板'

now() { curl -s "$HUB/api/live" | python3 -c 'import json,sys;print(json.load(sys.stdin)["snapshot"]["now"])'; }

# 每一顆按鈕都是獨立的 shell，變數留不住，所以關機前的時間戳寫進檔案。
MARK=${MARK:-/tmp/hyrox-demo-killat}
mark_now() { now > "$MARK"; }
marked() { cat "$MARK"; }

# 開機不在這裡：背景行程會抓著呼叫端的輸出管線不放，錄影腳本會等到天荒地老。
# 現場展示時就自己開一個終端機跑 `cargo run -p hub-server`；錄影時由 record.py 負責。

# 幕 2 — 沒有登記的手環刷過 SkiErg
act_stray_tag() {
  STRAY_AT=$(now)
  echo "讀卡機送出："
  echo "  reader=rfid-skierg-entry  tag=TAG-WALKIN-01  detected_at=$STRAY_AT"
  mosquitto_pub -h 127.0.0.1 -q 1 -t "hyrox/v1/edge/$DEVICE/events" \
    -m "{\"device_id\":\"$DEVICE\",\"reader_id\":\"rfid-skierg-entry\",\"boot_id\":99,\"sequence\":2,\"tag_id\":[\"TAG-WALKIN-01\"],\"detected_at\":$STRAY_AT,\"uptime_ms\":124000}"
  sleep 2
  echo
  echo "櫃檯的待綁定清單："
  curl -s "$HUB/api/checkin" | python3 -c 'import json,sys;print(" ", json.load(sys.stdin)["pending"])'
}

# 幕 2b — 現場報名並綁定；claimed 是「綁定之前那筆刷卡」被回頭重放的結果
act_claim() {
  local a
  a=$(curl -s -X POST -H "$DESK" -H 'content-type: application/json' \
    -d '{"display_name":"王小明 (現場報名)","bib":99}' "$HUB/api/checkin/entrants" \
    | python3 -c 'import json,sys;print(json.load(sys.stdin)["athlete_id"])')
  curl -s -X POST -H "$DESK" -H 'content-type: application/json' \
    -d "{\"tag_id\":\"TAG-WALKIN-01\",\"athlete_id\":\"$a\"}" "$HUB/api/checkin/bind" \
    | python3 -c '
import json,sys
d = json.load(sys.stdin)
print("綁定完成。系統回頭重放了綁定前的刷卡：")
for c in d["claimed"]:
    print(" ", c["kind"], c["station"], "at", c["at"], "→ 開始計時" if c["started_timing"] else "")'
}

act_walkin_time() {
  echo "王小明的第一筆紀錄（時間是刷卡當下，不是報名當下）："
  sqlite3 "$HUB_DB" "select r.tag_id, r.detected_at as 刷卡時間, r.received_at as 主機收到
                     from raw_events r where r.tag_id='TAG-WALKIN-01';" -header -column
}

# 幕 4 — 拔插頭
act_kill() {
  echo "關機前的課堂時間戳：$(marked)"
  echo
  echo "主機行程："; pgrep -f "$HUB_BIN" || echo "  （沒有了）"
}
act_refused() { curl -s -m 3 "$HUB/api/live" || echo "curl: (7) Failed to connect to 127.0.0.1 port 8730: Connection refused"; }

# 主機躺著的時候，讀卡機還在刷
act_publish_while_down() {
  echo "主機是關的，但讀卡機照樣送出這一筆："
  local at; at=$(marked)
  echo "  reader=rfid-wall_balls-entry  tag=TAG-A1  detected_at=$at"
  mosquitto_pub -h 127.0.0.1 -q 1 -t "hyrox/v1/edge/$DEVICE/events" \
    -m "{\"device_id\":\"$DEVICE\",\"reader_id\":\"rfid-wall_balls-entry\",\"boot_id\":99,\"sequence\":1,\"tag_id\":[\"TAG-A1\"],\"detected_at\":$at,\"uptime_ms\":123456}"
  echo "  已交給 broker 排隊（QoS 1）"
}

act_resumed_log() { grep -E "resumed|MQTT connected" "$HUB_LOG" | tail -2; }

act_arrived() {
  echo "關機期間那一筆，現在在資料庫裡："
  sqlite3 "$HUB_DB" "select id,tag_id,reader_id from raw_events where boot_id=99 and sequence=1;" -header -column
}

act_two_clocks() {
  # 只列兩個時間，不算差值：開發用的是加速虛擬時鐘，兩者的差在這裡沒有物理意義。
  sqlite3 "$HUB_DB" "select tag_id, detected_at as 刷卡時間, received_at as 主機收到時間
                     from raw_events where boot_id=99 and sequence=1;" -header -column
  echo
  echo "兩個時間不一樣：刷卡在關機前，收到在開機後。"
  echo "成績一律用「刷卡時間」—— 傳輸多久到，成績都不會變。"
}

# 讀卡機沒收到 ACK 會重送。重送不能變成兩筆。
act_resend() {
  local at; at=$(marked)
  echo "同一個 device + boot + sequence，再送一次："
  mosquitto_pub -h 127.0.0.1 -q 1 -t "hyrox/v1/edge/$DEVICE/events" \
    -m "{\"device_id\":\"$DEVICE\",\"reader_id\":\"rfid-wall_balls-entry\",\"boot_id\":99,\"sequence\":1,\"tag_id\":[\"TAG-A1\"],\"detected_at\":$at,\"uptime_ms\":123456}"
  sleep 2
}
act_still_one() {
  echo "資料庫裡這一筆的數量："
  sqlite3 "$HUB_DB" "select count(*) as 筆數 from raw_events where boot_id=99 and sequence=1;" -header -column
}

# 幕 7 — 畫面上的東西全部由主機自己供應
act_offline_assets() {
  echo "重新整理畫面時，瀏覽器要的每一個檔案都由這台主機回答："
  for p in /live /app.css /i18n.js /fonts/jetbrains-mono-0.woff2; do
    printf "  %-32s %s\n" "$p" "$(curl -s -o /dev/null -w '%{http_code}  %{size_download} bytes' "$HUB$p")"
  done
  echo
  echo "沒有 CDN，沒有外部字型服務。斷網也讀得懂自己的螢幕。"
}
