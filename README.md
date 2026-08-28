# HYROX Central Hub

On-site host system for HYROX competition and training sessions. See [CLAUDE.md](CLAUDE.md)
for the full specification and [docs/decisions/](docs/decisions/) for architecture decisions.

## Run

Needs an MQTT broker on `127.0.0.1:1883` — Mosquitto locally (`brew services start mosquitto`,
or `mosquitto -p 1883`). The hub ingests over MQTT and acknowledges over MQTT; without a
broker it starts, serves `/live`, and keeps retrying the connection.

```
cargo run -p hub-server
```

Then open <http://127.0.0.1:8730/live>. The screen is designed for a 2560x1440 projector.

The dev build also runs an emulated ESP32 in-process (`crates/simulator`) which publishes a
scripted class **over the broker**, so the screen moves without hardware while still
exercising the real ingestion path: publish → subscribe → decode → commit → ACK. Clock speed
is the `SPEED` constant in `apps/hub-server/src/main.rs`.

| Variable | Default | |
| --- | --- | --- |
| `HYROX_DB` | `sqlite://hyrox.db` | Database file. |
| `HYROX_MQTT_HOST` / `HYROX_MQTT_PORT` | `127.0.0.1` / `1883` | Broker address. |
| `HYROX_MQTT_CLIENT_ID` | `hyrox-hub` | Stable on purpose: the broker holds this client's QoS 1 queue while the hub is down. |
| `HYROX_SIM` | on | `off` disables the emulated collector. `--no-default-features` removes it from the build. |

Restarting resumes the ARMED session: athlete state is rebuilt by replaying the stored
interpreted events, and already ingested reads are skipped by their idempotency key. To start
clean, delete `hyrox.db*`.

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
| `crates/domain` | Session lifecycle, athlete state, reader interpretation, transition timing. Device and reader identity. No tokio, no axum, no IO. |
| `crates/contract` | What the ESP32 and the hub agreed on: the edge event, the idempotency key, and the ACK-after-commit protocol (ADR 0002, ADR 0005). No broker, no database. |
| `crates/application` | Use cases: ingestion, check-in, recovery, live snapshots. Ports only — no SQL, no HTTP, no MQTT. |
| `crates/transport` | MQTT delivery: topic scheme, device status payloads, inbound classification, rumqttc client behind the default `broker` feature. |
| `crates/simulator` | Emulated ESP32 collectors: presence/re-arm suppression, journal, link faults (CLAUDE.md 25). Over a real broker behind the `broker` feature. |
| `crates/storage` | SQLite (WAL), migrations, raw + interpreted event stores, recovery. |
| `apps/hub-server` | Axum server, WebSocket push, the MQTT ingestion loop, the dev venue and its emulated collector, generated static screens. |
| `design/live` | Screen design sources: station pictograms and the page generator. |

## Regenerating the screens

`apps/hub-server/static/training.html` is generated, not hand-edited:

```
cd design/live && python3 build_screens.py
```

Needs Pillow and NumPy. The generator slices `design/live/icons/*.png` into CSS mask images,
so station glyphs stay recolourable by CSS. The server embeds the result with `include_str!`,
so rebuild the binary after regenerating.
