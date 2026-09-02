#!/usr/bin/env bash
# 比賽展示的按鈕：賽制設定 → 報到 → 開賽 → 完賽成績。
#
#   source design/demo/competition.sh
#   race_track; race_open; race_checkin; race_start; race_run; race_results
#
# 賽制的差別只有一個地方：完成規則是 COURSE_COMPLETE（走完全程才算完成），
# 所以成績才排得出名次。團課用 CLASS_DURATION，時間到就下課，名次留白。
# ponytail: 沒有錯誤處理，理由同 group-class.sh。

HUB=${HUB:-http://127.0.0.1:8730}
DIRECTOR='x-operator-device: 賽事平板'
DESK='x-operator-device: 報到處平板'
RACE=${RACE:-race-2026-09-06}
DEVICE=a4cf128b3d91

now() { curl -s "$HUB/api/live" | python3 -c 'import json,sys;print(json.load(sys.stdin)["snapshot"]["now"])'; }
read_tag() {
  mosquitto_pub -h 127.0.0.1 -q 1 -t "hyrox/v1/edge/$DEVICE/events" \
    -m "{\"device_id\":\"$DEVICE\",\"reader_id\":\"$1\",\"boot_id\":8,\"sequence\":$2,\"tag_id\":[\"$3\"],\"detected_at\":$4,\"uptime_ms\":40000}"
}

# 1 — 賽道：三站，SkiErg 500m → 推雪橇 25m → 牆球 50 下
race_track() {
  curl -s -X POST -H "$DIRECTOR" -H 'content-type: application/json' \
    -d '{"new_id":"race-sprint-3","name":"HYROX 三站衝刺賽","owner_id":"race-director"}' \
    "$HUB/api/operator/templates/sys-power/duplicate" > /dev/null
  python3 - <<'PY' > /tmp/hyrox-race.json
import json, urllib.request
t = json.load(urllib.request.urlopen("http://127.0.0.1:8730/api/workout-templates/race-sprint-3"))["template"]
t["name"] = "HYROX 三站衝刺賽"
t["description"] = "SkiErg 500m → 推雪橇 25m → 牆球 50 下"
t["blocks"] = [{
    "name": "賽道", "block_type": "SEQUENTIAL", "rounds": None, "duration": None, "rest": None,
    "exercises": [
        {"exercise_code": "SKIERG", "target": {"target_type": "DISTANCE", "value": 500, "unit": "METER"},
         "weight": None, "time_limit": None, "notes": None},
        {"exercise_code": "SLED_PUSH", "target": {"target_type": "DISTANCE", "value": 25, "unit": "METER"},
         "weight": None, "time_limit": None, "notes": None},
        {"exercise_code": "WALL_BALL", "target": {"target_type": "REPS", "value": 50, "unit": "REPS"},
         "weight": None, "time_limit": None, "notes": None},
    ],
}]
print(json.dumps(t, ensure_ascii=False))
PY
  curl -s -X PUT -H "$DIRECTOR" -H 'content-type: application/json' -d @/tmp/hyrox-race.json \
    "$HUB/api/operator/templates/race-sprint-3" | python3 -c '
import json,sys
t = [x for x in json.load(sys.stdin)["templates"] if x["id"]=="race-sprint-3"][0]
print("賽道：", t["name"], "· v%d" % t["version"])'
}

# 2 — 開賽事：完成規則 = 走完全程
race_open() {
  curl -s -X POST -H "$DIRECTOR" -H 'content-type: application/json' \
    -d "{\"template_id\":\"race-sprint-3\",\"session_id\":\"$RACE\",\"name\":\"HYROX 三站衝刺賽\",\"mode\":\"COMPETITION\",\"finish_policy\":{\"kind\":\"COURSE_COMPLETE\"}}" \
    "$HUB/api/operator/class" | python3 -c '
import json,sys
d = json.load(sys.stdin)
print("賽事狀態：", d["session"]["status"], "·", d["session"]["mode"])
print("完成規則：", d["config"]["finish_policy"]["kind"], "（走完全程才算完成，成績才排得出名次）")
print("賽道：", " → ".join(s["station"] for s in d["config"]["course"]["steps"]))'
}

# 3 — 報到發手環
race_checkin() {
  local i=1
  for n in 陳怡君 林建宏 王淑芬 黃培綺; do
    # 報名回傳的 athlete_id 就是選手的報名編號（ADR 0011）；不要自己拼 id。
    local id=$(curl -s -X POST -H "$DESK" -H 'content-type: application/json' \
      -d "{\"display_name\":\"$n\"}" "$HUB/api/checkin/entrants" \
      | python3 -c 'import json,sys;print(json.load(sys.stdin)["athlete_id"])')
    curl -s -X POST -H "$DESK" -H 'content-type: application/json' \
      -d "{\"tag_id\":\"TAG-R0$i\",\"athlete_id\":\"$id\"}" "$HUB/api/checkin/bind" > /dev/null
    echo "  $n  ·  編號 $id  ·  號碼 0$i  ·  手環 TAG-R0$i"
    i=$((i+1))
  done
}

race_start() {
  curl -s -X POST -H "$DIRECTOR" "$HUB/api/operator/session/ready" > /dev/null
  curl -s -X POST -H "$DIRECTOR" "$HUB/api/operator/session/start" | python3 -c '
import json,sys;print("賽事狀態：", json.load(sys.stdin)["session"]["status"])'
}

# 4 — 比賽過程。分三波送出刷卡，大螢幕才看得到「進行中」，而不是一秒全部完賽。
# 每位選手每站進出各一次；最後一站的出場就是完賽。時間戳一律取「現在之前」。
RACE_PACE_FILE=${RACE_PACE_FILE:-/tmp/hyrox-race-start}

race_wave1() {                    # 四人出發，進 SkiErg
  local t; t=$(now); echo $((t - 900000)) > "$RACE_PACE_FILE"
  local at; at=$(cat "$RACE_PACE_FILE")
  for i in 1 2 3 4; do read_tag rfid-skierg-entry $((210 + i)) "TAG-R0$i" $((at + i * 4000)); done
  sleep 2; race_board
}

race_wave2() {                    # 陸續出 SkiErg、進推雪橇
  local at; at=$(cat "$RACE_PACE_FILE")
  for i in 1 2 3 4; do
    local pace=$((150 + i * 25))
    read_tag rfid-skierg-exit    $((220 + i)) "TAG-R0$i" $((at + pace * 1000))
    read_tag rfid-sled_push-entry $((230 + i)) "TAG-R0$i" $((at + (pace + 20) * 1000))
  done
  sleep 2; race_board
}

race_wave3() {                    # 推雪橇 → 牆球 → 完賽
  local at; at=$(cat "$RACE_PACE_FILE")
  for i in 1 2 3 4; do
    local pace=$((150 + i * 25))
    local s=$((pace * 2 + 20))
    read_tag rfid-sled_push-exit  $((240 + i)) "TAG-R0$i" $((at + s * 1000))
    read_tag rfid-wall_balls-entry $((250 + i)) "TAG-R0$i" $((at + (s + 20) * 1000))
    read_tag rfid-wall_balls-exit  $((260 + i)) "TAG-R0$i" $((at + (s + 20 + pace) * 1000))
  done
  sleep 3; race_board
}

race_board() {
  curl -s "$HUB/api/live" | python3 -c '
import json,sys
for a in json.load(sys.stdin)["snapshot"]["athletes"]:
    print("  %s 號 %s  %s  %s  完成 %d 站" % (
        a["bib"], a["name"], a["status"], a.get("station") or "-", a["completed"]))'
}

# 完賽當下的畫面文字版（大螢幕看得到的同一件事）
race_run_summary() {
  curl -s "$HUB/api/live" | python3 -c '
import json,sys
d = json.load(sys.stdin)["snapshot"]
print("賽道：", " → ".join(c["name"] for c in d["course"]))
print()
for a in d["athletes"]:
    print("  %s 號 %s   %s   完成 %d 站" % (a["bib"], a["name"], a["status"], a["completed"]))'
}

race_results() {
  curl -s "$HUB/api/leaderboard" | python3 -c '
import json,sys
r = json.load(sys.stdin)["results"]
print(r["session_name"], "·", r["status"], "· 排序依據:", r["ordering"])
for x in r["rows"]:
    t = x["elapsed_ms"]
    clock = "--:--" if t is None else "%02d:%02d" % (t // 60000, t % 60000 // 1000)
    print("  第 %s 名   %s 號  %s   %s" % (x["place"] or "-", x["bib"], x["name"], clock))'
}
