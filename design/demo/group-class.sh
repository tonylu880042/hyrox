#!/usr/bin/env bash
# 團體課程展示的按鈕。一幕一個函式，照順序按，不要在台上打字。
#
#   source design/demo/group-class.sh     # 載入
#   act1_plan                             # 幕 1：教練排課
#   ...
#   acts                                  # 忘記順序時
#
# 劇本（要講什麼、螢幕會出現什麼）：docs/demo-script-group-class.md
# 前提：hub-server 啟動中（不要開 HYROX_DEMO）、mosquitto 在跑、資料庫是乾淨的。
#
# ponytail: 沒有錯誤處理。這是展示用的手動按鈕，每一步的回應本來就要唸出來給觀眾聽，
# 失敗會直接印在畫面上。要自動化驗證請寫測試，不是把這裡撐大。

HUB=${HUB:-http://127.0.0.1:8730}
COACH='x-operator-device: 教練平板'
DESK='x-operator-device: 櫃檯平板'
SESSION=${SESSION:-thu-power-2026-09-04}
DEVICE=a4cf128b3d91

# 主機自己的時鐘（開發時是加速的虛擬時鐘）。刷卡時間一律以它為準。
now() { curl -s "$HUB/api/live" | python3 -c 'import json,sys;print(json.load(sys.stdin)["snapshot"]["now"])'; }
j() { python3 -m json.tool --no-ensure-ascii 2>/dev/null || cat; }

# 一次刷卡。$1 讀卡機 $2 序號 $3 手環 $4 detected_at（毫秒）
read_tag() {
  mosquitto_pub -h 127.0.0.1 -q 1 -t "hyrox/v1/edge/$DEVICE/events" \
    -m "{\"device_id\":\"$DEVICE\",\"reader_id\":\"$1\",\"boot_id\":7,\"sequence\":$2,\"tag_id\":[\"$3\"],\"detected_at\":$4,\"uptime_ms\":30000}"
}

# 按這個順序按。（寫死而不是去 grep 自己，因為這份檔案 bash 和 zsh 都要能 source。）
acts() { cat <<'EOF'
act0_clear      收掉開機自建的課
act1_plan       教練複製系統課表、改成兩輪
act1b_readonly  系統課表改不得
act2_open       開課：編譯成 10 關並快照
act3_checkin    四人報到、發手環
act4_start      開始上課
act5_floor      學員刷卡進出站
act6_pause      暫停（課堂時鐘停住）
act6_resume     恢復
act7_misread    誤刷別站
act7_stages     看系統怎麼判（唯讀）
act7b_void      教練作廢那一筆（要理由）
act7c_trail     原始刷卡與稽核紀錄（唯讀）
act8_end        收工
act9_results    成績
EOF
}

# 幕 0 — 開機時自建的課先收掉，才輪得到教練的課
act0_clear() {
  curl -s -X POST -H "$COACH" -H 'content-type: application/json' \
    -d '{"reason":"開新團課"}' "$HUB/api/operator/session/cancel" | j
}

# 幕 1 — 教練排課：複製系統課表，改成兩輪。
# 課表不綁日期 —— 它是一份可以重複開課的計畫，日期屬於「課程」而不是課表。
act1_plan() {
  curl -s -X POST -H "$COACH" -H 'content-type: application/json' \
    -d '{"new_id":"coach-power","name":"Power 團課","owner_id":"coach-lin"}' \
    "$HUB/api/operator/templates/sys-power/duplicate" > /dev/null
  curl -s "$HUB/api/workout-templates/coach-power" | python3 -c '
import json,sys
t = json.load(sys.stdin)["template"]
t["blocks"][0]["rounds"] = 2
json.dump(t, open("/tmp/hyrox-demo-tpl.json","w"), ensure_ascii=False)'
  curl -s -X PUT -H "$COACH" -H 'content-type: application/json' -d @/tmp/hyrox-demo-tpl.json \
    "$HUB/api/operator/templates/coach-power" | python3 -c '
import json,sys
print([(t["name"], "v%d" % t["version"]) for t in json.load(sys.stdin)["templates"] if t["id"]=="coach-power"])'
}

# 幕 1b — 系統課表改不得
act1b_readonly() {
  curl -s -X PUT -H "$COACH" -H 'content-type: application/json' -d @/tmp/hyrox-demo-tpl.json \
    "$HUB/api/operator/templates/sys-power" | j
}

# 幕 2 — 開課：課表編譯成 10 個關卡，快照到這一堂
act2_open() {
  curl -s -X POST -H "$COACH" -H 'content-type: application/json' \
    -d "{\"template_id\":\"coach-power\",\"session_id\":\"$SESSION\",\"name\":\"週四 19:00 Power 團課\",\"finish_policy\":{\"kind\":\"CLASS_DURATION\",\"limit\":2700000}}" \
    "$HUB/api/operator/class" | python3 -c '
import json,sys
d = json.load(sys.stdin)
print(d.get("error") or (d["session"]["status"], [s["station"] for s in d["config"]["course"]["steps"]]))'
}

# 幕 3 — 報到：四個人現場報名，發四隻手環
act3_checkin() {
  local i=1
  for n in 陳怡君 林建宏 王淑芬 黃培綺; do
    # 報名回傳的 athlete_id 就是那個人的報名編號（ADR 0011）；不要自己拼 id。
    local id=$(curl -s -X POST -H "$DESK" -H 'content-type: application/json' \
      -d "{\"display_name\":\"$n\"}" "$HUB/api/checkin/entrants" \
      | python3 -c 'import json,sys;print(json.load(sys.stdin)["athlete_id"])')
    curl -s -X POST -H "$DESK" -H 'content-type: application/json' \
      -d "{\"tag_id\":\"TAG-G0$i\",\"athlete_id\":\"$id\"}" "$HUB/api/checkin/bind" \
      | python3 -c "import json,sys;d=json.load(sys.stdin);print('$n', '·', '$id', d.get('error') or '· TAG-G0$i 綁定完成')"
    i=$((i+1))
  done
}

# 幕 4 — 開始上課
act4_start() {
  curl -s -X POST -H "$COACH" "$HUB/api/operator/session/ready" > /dev/null
  curl -s -X POST -H "$COACH" "$HUB/api/operator/session/start" | python3 -c '
import json,sys;print(json.load(sys.stdin)["session"]["status"])'
}

# 幕 5 — 上課：刷卡進站、出站。時間戳一律取「現在之前」，分段才會立刻是正數。
act5_floor() {
  local t; t=$(now)
  read_tag rfid-sled_push-entry 101 TAG-G01 $((t-240000))
  read_tag rfid-sled_push-entry 102 TAG-G02 $((t-235000))
  read_tag rfid-sled_push-entry 103 TAG-G03 $((t-230000))
  read_tag rfid-sled_push-exit  104 TAG-G01 $((t-180000))
  read_tag rfid-sled_push-exit  105 TAG-G02 $((t-175000))
  read_tag rfid-sled_pull-entry 106 TAG-G01 $((t-160000))
  read_tag rfid-sled_push-exit  107 TAG-G03 $((t-150000))
  read_tag rfid-sled_pull-entry 108 TAG-G02 $((t-140000))
  read_tag rfid-sled_pull-exit  109 TAG-G01 $((t-90000))
  sleep 2
  curl -s "$HUB/api/live" | python3 -c '
import json,sys
for a in json.load(sys.stdin)["snapshot"]["athletes"]:
    print(a["bib"], a["name"], a["status"], a["station_state"], a.get("station") or "-",
          "分段:", [(s["station"], s["work_ms"], s["transition_ms"]) for s in a.get("splits", [])])'
}

# 幕 6 — 暫停：課堂時鐘停住（多按幾次看數字不動），再恢復
act6_pause()  { curl -s -X POST -H "$COACH" "$HUB/api/operator/session/pause" > /dev/null; elapsed; }
act6_resume() { curl -s -X POST -H "$COACH" "$HUB/api/operator/session/resume" > /dev/null; elapsed; }
elapsed() { curl -s "$HUB/api/session" | python3 -c '
import json,sys;d=json.load(sys.stdin);print(d["session"]["status"], "課堂時鐘", d["class_elapsed_ms"], "ms")'; }

# 幕 7 — 誤刷：刷到今天還沒走到的站
act7_misread() {
  read_tag rfid-wall_balls-entry 301 TAG-G03 $(($(now)-5000))
  sleep 2
  curl -s "$HUB/api/stages" | python3 -c '
import json,sys
for a in json.load(sys.stdin)["athletes"]:
    print(a["name"], "第", a.get("current_stage"), "關", a.get("expectation") or "-")'
}

# 幕 7 附帶 — 只讀不寫，給影片拆格用
act7_stages() {
  curl -s "$HUB/api/stages" | python3 -c '
import json,sys
for a in json.load(sys.stdin)["athletes"]:
    print(a["name"], "第", a.get("current_stage"), "關", a.get("expectation") or "-")'
}
act7c_trail() {
  local db=${HYROX_DB_FILE:-hyrox.db}
  echo "原始刷卡（不可刪除）："
  sqlite3 "$db" "select id,reader_id,tag_id,detected_at from raw_events where sequence=301;"
  echo
  echo "解讀層（打上作廢標記）："
  sqlite3 "$db" "select id,station,kind,voided_by,void_reason from interpreted_events where voided_at is not null;"
  echo
  echo "稽核紀錄："
  sqlite3 "$db" "select operator,action,subject,reason from audit_log order by id desc limit 3;"
}

# 幕 7b — 教練作廢那一筆（要理由）。一般事件的 id 目前只能從資料庫查（介面未做）。
act7b_void() {
  local db=${HYROX_DB_FILE:-hyrox.db}
  local id; id=$(sqlite3 "$db" "select id from interpreted_events where station='WALL BALLS' and voided_at is null order by id desc limit 1;")
  echo "作廢 interpreted_event #$id"
  curl -s -X POST -H "$COACH" -H 'content-type: application/json' -d '{}' \
    "$HUB/api/operator/exceptions/$id/void" | j                       # 先示範沒有理由會被擋
  curl -s -X POST -H "$COACH" -H 'content-type: application/json' \
    -d '{"reason":"誤刷：學員經過牆球區，沒有做"}' "$HUB/api/operator/exceptions/$id/void" > /dev/null
  echo "-- 原始刷卡還在嗎（應為 1）："; sqlite3 "$db" "select count(*) from raw_events where sequence=301;"
  echo "-- 稽核紀錄："; sqlite3 "$db" "select operator,action,subject,reason from audit_log order by id desc limit 1;"
}

# 幕 8 — 收工
act8_end() {
  curl -s -X POST -H "$COACH" "$HUB/api/operator/session/end-class" | python3 -c '
import json,sys;print("判定完成：", json.load(sys.stdin)["finished"])'
  curl -s -X POST -H "$COACH" "$HUB/api/operator/session/complete" | python3 -c '
import json,sys;print(json.load(sys.stdin)["session"]["status"])'
}

# 幕 9 — 成績（用 /leaderboard，不要用 /result，理由見劇本 §已知問題）
act9_results() {
  curl -s "$HUB/api/leaderboard" | python3 -c '
import json,sys
r = json.load(sys.stdin)["results"]
print(r["session_name"], r["status"], "排序依據:", r["ordering"])
for x in r["rows"]:
    print(" ", x["bib"], x["name"], x["status"], "完成關卡", x["stations_completed"], "名次", x["place"])'
}
