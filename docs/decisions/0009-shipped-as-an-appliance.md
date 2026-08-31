# ADR 0009 — Shipped as an appliance: browser kiosk on Ubuntu Server, no Tauri

**Status:** accepted, 2026-08-29
**Amends:** CLAUDE.md §6 (Tauri), §5 (deployment shape)

## Context

The hub ships as a **complete machine**. A customer never installs anything, never picks a
distribution, and never sees a terminal. The venue box drives the projector directly and
runs the coaches' web surface on the same LAN.

That settles questions that were open while "someone installs this on their own hardware"
was still on the table, and it changes which trade-offs matter: reproducibility on the
production line matters, ease of first-time installation does not.

Four requirements were stated (2026-08-29):

1. fast boot to a live screen;
2. no lost RFID events;
3. room to grow;
4. update prompting, and a scheduled check that shuts the machine down afterwards.

## Decision

### 1. The screens stay web. Tauri is not used.

CLAUDE.md §6 already required Tauri to be a *client* of the hub. We go further and do not
ship it at all, because on inspection it buys nothing here:

* **It would not be native rendering.** Tauri on Linux renders in WebKitGTK, not Chromium.
  The same HTML/CSS/JS runs either way; only the window frame changes.
* **It would be a rendering *risk*.** The live screen slices pictograms into CSS
  `mask-image` layers (`design/live/build_screens.py`). That is verified on Chromium.
  Re-verifying it on WebKitGTK is work with no upside.
* **It would not boot faster.** `hub-server` is a Rust binary that starts in milliseconds.
  Time from power-on to a live screen is firmware POST, kernel and compositor; a WebView
  wrapper does not touch any of them.

**Tauri stays on the table for one thing only: an operator console that must reach
hardware** — the PM5 over USB, a camera, BLE/FTMS, a judge's input device (workout brief
§23). A browser cannot do those. When that day comes, the console reads those devices and
**posts results to the hub's REST API**; it does not open the database.

The rule, stated once: **needs hardware → native shell. Displays and operates → browser.**

### 2. Nothing but `hub-server` touches the database

Restating CLAUDE.md §6's prohibition with the reasons it is right, because "let the UI open
SQLite directly" is the shortcut that will be proposed again:

* **Two sources of truth.** `hub-server` holds `LiveSession` in memory and rebuilds it from
  the interpreted log only at startup. A second writer would leave that copy silently stale
  — the live screen and the finish policy would be deciding on a state that no longer
  matches the log until the next restart.
* **A UI could block ingestion.** SQLite WAL is multi-process safe, but writes are
  exclusive. A UI holding a write transaction gives the ingestion path `SQLITE_BUSY`.
  CLAUDE.md §31's first principle is that no event is lost; nothing that renders a screen
  may be able to stall the path that records events.
* **The ACK contract assumes one writer.** ADR 0002 makes `Ack` unconstructable except from
  a successful commit. That guarantee is about *this* process's commits.

### 3. Ubuntu Server 24.04 LTS

Debian was considered first, for one honest reason: Ubuntu's `chromium` is snap-only, and
**snap refreshes itself in the background** — a browser version changing under a machine
whose behaviour must be identical forever is not acceptable. Debian 12+ also fixed its old
non-free-firmware problem, and day-to-day operation of the two is the same apt and the same
systemd.

Ubuntu wins anyway, on production-line reproducibility:

| | |
| --- | --- |
| **Ubuntu Server 24.04 LTS** | not Desktop; five years of support |
| **autoinstall** | one cloud-init YAML, unattended from a USB stick, version-controlled. Debian's preseed is older and fiddlier, and building the twentieth identical machine is the actual problem |
| **Google Chrome `.deb`** | from Google's signed apt repo. Sidesteps snap entirely — the objection to Ubuntu costs one `late-command` |
| **`cage`** | Wayland kiosk compositor. No desktop environment, no display manager, no login screen |
| **`unattended-upgrades`** | configured to trust **our** repo only, so nothing upstream can move under a running venue |

### 4. Updates are a signed apt repository

`.deb` packages from our own signed repo, rather than a bespoke downloader: GPG verification,
version pinning and rollback all come from `apt`, and each of those is easy to get wrong by
hand. The package carries the binary, the systemd units and `/etc/hyrox/hub.env`.

**Migrations run on start** (`Store::open`), so installing a package upgrades the schema
with no separate step. Migration 0004 is the precedent that this is safe on real data.

### 5. Configuration is an `EnvironmentFile`, not a parser

`/etc/hyrox/hub.env`, referenced by the systemd unit. The hub already reads its settings
from the environment; systemd already reads files into it. Adding a configuration file
format, a parser and a precedence order would be new code that does nothing the platform
does not already do.

The shipped unit sets what a venue needs and a developer must not get by default:

```ini
Environment=HYROX_BIND=0.0.0.0:8730
Environment=HYROX_DB=sqlite:///var/lib/hyrox/hyrox.db
```

`HYROX_BIND` defaults to `127.0.0.1:8730` in code. Safe by default: a developer's laptop is
not exposed to the café's wifi because they ran `cargo run`. The appliance opts in, and the
network boundary is the deployment layer's job (ADR 0001 D1) — there is no login.

### 6. Nothing stops or updates during a class

The scheduled window must never take a machine down mid-class. `GET /api/health` answers it:

```json
{ "version": "…", "session_status": "RUNNING", "class_live": true,
  "devices_with_backlog": 1, "safe_to_stop": false,
  "blocked_by": ["CLASS_RUNNING", "DEVICE_BACKLOG"] }
```

Two blockers, both from state the hub already keeps:

* **`CLASS_RUNNING`** — `Session::is_live()`: READY, RUNNING or PAUSED. A paused class is a
  coffee break, not an ending.
* **`DEVICE_BACKLOG`** — an edge device still reporting unacknowledged events in its journal
  (CLAUDE.md 18). Stopping now means those events wait on flash until the hub is back. That
  is *survivable* — the journal is exactly what it is for — but it is not something to do
  deliberately when tomorrow will do.

The maintenance timer asks first, and does nothing on a `false`.

### 7. `synchronous = FULL`

Changed from `NORMAL`. In WAL mode `NORMAL` makes a commit durable against a **process
crash** but not against **power loss** — the WAL is only fsynced at checkpoints.

That was an acceptable development setting and is not acceptable here. This machine gets
switched off at the wall. The ACK contract tells an ESP32 to delete its only other copy of
an event:

```text
ESP32 → hub commits → ACK → ESP32 drops the event from its journal
                ↑
        if that commit was still in the OS page cache,
        a pulled plug loses an event we promised was safe
```

The cost is one fsync per commit, against a write volume of roughly two rows per athlete per
station. Correctness wins trivially here.

## Portability

Nothing above reaches into the core. `hub-server` is the only crate that learns about the
bind address or signals; `domain`, `application` and `storage` are untouched by the
deployment shape, which is what CLAUDE.md §2 asks for. The hub still runs on macOS for
development with no appliance machinery involved.

## Consequences

* A second artefact to build and sign: the `.deb` and the apt repository behind it.
* The autoinstall YAML becomes production-line equipment and belongs under version control
  with everything else.
* `unattended-upgrades` must be pinned to our repo, and reboot-on-upgrade disabled: a
  machine rebooting mid-class is the worst failure this system has.
* A venue with no route to the update repo simply never updates. That is a safe default and
  an offline path (USB) can be added without changing anything decided here.
