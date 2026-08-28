# HYROX Central Hub

On-site host system for HYROX competition and training sessions. See [CLAUDE.md](CLAUDE.md)
for the full specification and [docs/decisions/](docs/decisions/) for architecture decisions.

## Run

```
cargo run -p hub-server
```

Then open <http://127.0.0.1:8730/live>. The screen is designed for a 2560x1440 projector.

The dev build feeds a scripted class through the real domain instead of MQTT, so the screen
moves without hardware. Clock speed is the `SPEED` constant in `apps/hub-server/src/main.rs`.

State is persisted to `hyrox.db` (override with `HYROX_DB`). Restarting resumes the ARMED
session: athlete state is rebuilt by replaying the stored interpreted events, and already
ingested reads are skipped by their idempotency key. To start clean, delete `hyrox.db*`.

## Test

```
cargo test
```

## Layout

| Path | Contains |
| --- | --- |
| `crates/domain` | Session lifecycle, athlete state, reader interpretation, transition timing. Device and reader identity. No tokio, no axum, no IO. |
| `crates/contract` | What the ESP32 and the hub agreed on: the edge event, the idempotency key, and the ACK-after-commit protocol (ADR 0002, ADR 0005). No broker, no database. |
| `crates/application` | Use cases: ingestion, check-in, recovery, live snapshots. Ports only — no SQL, no HTTP, no MQTT. |
| `crates/transport` | MQTT delivery: topic scheme, device status payloads, rumqttc client behind the default `broker` feature. |
| `crates/simulator` | Emulated ESP32 collectors: presence/re-arm suppression, journal, link faults (CLAUDE.md 25). |
| `crates/storage` | SQLite (WAL), migrations, raw + interpreted event stores, recovery. |
| `apps/hub-server` | Axum server, WebSocket push, scripted event feeder, generated static screens. |
| `design/live` | Screen design sources: station pictograms and the page generator. |

## Regenerating the screens

`apps/hub-server/static/training.html` is generated, not hand-edited:

```
cd design/live && python3 build_screens.py
```

Needs Pillow and NumPy. The generator slices `design/live/icons/*.png` into CSS mask images,
so station glyphs stay recolourable by CSS. The server embeds the result with `include_str!`,
so rebuild the binary after regenerating.
