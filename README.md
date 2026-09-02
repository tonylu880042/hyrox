# HYROX 訓練與競賽應用系統

On-site host system for HYROX competition and training sessions. 程式碼、crate 名稱與
ADR 沿用內部名稱 **Central Hub**；面向使用者的畫面與影片一律用「HYROX 訓練與競賽應用系統」
（簡中：HYROX 训练与竞赛应用系统；英文：HYROX TRAINING & COMPETITION SYSTEM）。 See [CLAUDE.md](CLAUDE.md)
for the full specification, [docs/roadmap.md](docs/roadmap.md) for where the build has got to,
and [docs/decisions/](docs/decisions/) for architecture decisions.

## Run

Needs an MQTT broker on `127.0.0.1:1883` — Mosquitto locally (`brew services start mosquitto`,
or `mosquitto -p 1883`). The hub ingests over MQTT and acknowledges over MQTT; without a
broker it starts, serves `/live`, and keeps retrying the connection.

```
cargo run -p hub-server
```

Then open <http://127.0.0.1:8730/live> (the 2560x1440 projector screen) or
<http://127.0.0.1:8730/workout> (the coach's workout builder — see
[docs/workout-system.md](docs/workout-system.md)).

Both screens speak Traditional Chinese, Simplified Chinese and English. The builder has a
switcher in its header; the projector is pinned by URL — `/live?lang=zh-Hans` — and that
machine remembers the choice afterwards. Labels come from `apps/hub-server/static/i18n.js`,
served locally at `/i18n.js` so a venue with no internet still gets its own language.
The REST and WebSocket API sits under `/api` and `/ws` — see `docs/api.md` for the endpoint
table, the operator-identity header, and the freshness readout every read carries.

A hub starts empty: one DRAFT class, no course, no roster, no readers. Start it with
`HYROX_DEMO=1` and the settings screen grows a **Demo data** tab, which loads a whole fixture
venue (course, twelve athletes, readers, bands) and runs an emulated ESP32 in-process
(`crates/simulator`) publishing a scripted class **over the broker** — so the screen moves
without hardware while still exercising the real ingestion path: publish → subscribe →
decode → commit → ACK. That mode also runs the class clock at `SPEED`
(`apps/hub-server/src/main.rs`); without it the hub keeps real time.

| Variable | Default | |
| --- | --- | --- |
| `HYROX_DB` | `sqlite://hyrox.db` | Database file. The appliance uses `/var/lib/hyrox/hyrox.db`. |
| `HYROX_BIND` | `127.0.0.1:8730` | Where the HTTP surface listens. Loopback by default on purpose — `cargo run` must not put an unauthenticated write surface on the café's wifi. The appliance sets `0.0.0.0:8730` in its unit file (ADR 0009). |
| `HYROX_MQTT_HOST` / `HYROX_MQTT_PORT` | `127.0.0.1` / `1883` | Broker address. |
| `HYROX_MQTT_CLIENT_ID` | `hyrox-hub` | Stable on purpose: the broker holds this client's QoS 1 queue while the hub is down. |
| `HYROX_DEMO` | off | `1` offers the demo venue on the settings screen: a fixture class plus an emulated collector, loaded and stopped by hand. `--no-default-features` removes it from the build. |

Restarting resumes the live session (READY, RUNNING or PAUSED): athlete state is rebuilt by replaying the stored
interpreted events, and already ingested reads are skipped by their idempotency key. A class that was PAUSED comes
back paused, with its accumulated pause intact. To start clean, delete `hyrox.db*`.

The exercise library and the four starter workouts are written on every start; the writes are
keyed, so nothing accumulates and a coach's own templates are never touched.

## Test

```
cargo test
```

Everything runs with no broker, no database and no hardware (CLAUDE.md 24) — except
`crates/transport/tests/broker.rs`, which needs Mosquitto and **skips itself** when nothing
answers on `127.0.0.1:1883`. To actually run it:

```
cargo test -p transport --test broker -- --nocapture
```

`--nocapture` matters: a skipped test prints `SKIPPED: no MQTT broker …`, and otherwise looks
exactly like a passing one. Point it elsewhere with `HYROX_TEST_MQTT_HOST` /
`HYROX_TEST_MQTT_PORT`.

## Layout

| Path | Contains |
| --- | --- |
| `crates/domain` | Session lifecycle, athlete state, reader interpretation, transition timing. Device and reader identity. Workout templates, the exercise library and the compile step (ADR 0008). No tokio, no axum, no IO. |
| `crates/contract` | What the ESP32 and the hub agreed on: the edge event, the idempotency key, and the ACK-after-commit protocol (ADR 0002, ADR 0005). No broker, no database. |
| `crates/application` | Use cases: ingestion, check-in, recovery, live snapshots. Ports only — no SQL, no HTTP, no MQTT. |
| `crates/transport` | MQTT delivery: topic scheme, device status payloads, inbound classification, rumqttc client behind the default `broker` feature. |
| `crates/simulator` | Emulated ESP32 collectors: presence/re-arm suppression, journal, link faults (CLAUDE.md 25). Over a real broker behind the `broker` feature. |
| `crates/storage` | SQLite (WAL), migrations, raw + interpreted event stores, recovery. |
| `crates/api` | The REST / WebSocket surface: router, handlers, wire shapes (ADR 0007). Sees `application` and `domain` only — no SQLite, no MQTT. Endpoint table in `docs/api.md`. |
| `apps/hub-server` | The composition root: opens the store, recovers the session, runs the MQTT ingestion loop and the dev venue, serves `crates/api`'s router plus the generated static screens. |
| `design/live` | Screen design sources: station pictograms and the page generator. |
| `packaging` | The appliance: systemd units, the `.deb` build, the signed S3 apt repository, and the production line's autoinstall file (ADR 0009). |

## Regenerating the screens

`apps/hub-server/static/training.html` is generated, not hand-edited — its labels live in
the generator, so a translation goes there and not into the output:

```
cd design/live && python3 build_screens.py
```

Needs Pillow and NumPy. The generator slices `design/live/icons/*.png` into CSS mask images,
so station glyphs stay recolourable by CSS. The server embeds the result with `include_str!`,
so rebuild the binary after regenerating.
