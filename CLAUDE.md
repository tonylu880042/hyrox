# CLAUDE.md

# HYROX Central Hub — Claude Development Instructions

## 0. Project Purpose

This repository implements the **HYROX Central Hub**.

The Central Hub is the on-site host system responsible for:

- Member reference synchronization from **健身管**
- RFID Tag ↔ Member binding
- ESP32 / RFID Reader device registration and monitoring
- Reader → Station / Zone / Event-role mapping
- Competition / Training session management
- RFID event ingestion
- Event validation and deduplication
- Athlete state tracking
- Timing calculation
- ROX / Transition time calculation
- Persistent event storage
- Event recovery
- Manual correction with audit trail
- Live monitoring
- Coach / staff browser access
- Leaderboard and result calculation
- Future Tauri operator console

The Central Hub **does not directly connect to RFID readers**.

RFID hardware flow:

```text
RFID Reader
    |
    v
ESP32 Edge Collector
    |
    | MQTT
    v
Central Hub
```

The Central Hub is the source of business interpretation.

---

# 1. Current Development Environment

Development currently runs on:

- macOS
- Rust stable
- local MQTT broker
- local SQLite
- simulated ESP32 / RFID events

However:

> macOS is only the current development environment.

The system must remain portable so that the Central Hub can later run on Linux without rewriting the business logic.

Expected future production environment:

- Linux
- systemd
- local MQTT broker
- SQLite
- local LAN / Wi-Fi
- browser clients
- optional Tauri operator console

---

# 2. Cross-Platform Requirement

This is a **hard architectural requirement**.

Core code must remain OS-independent.

Do not put macOS-specific behavior inside:

- domain logic
- timing logic
- race state logic
- MQTT protocol handling
- storage models
- REST API
- WebSocket logic

Platform-specific behavior must be isolated behind adapters.

Preferred separation:

```text
domain/
application/
infrastructure/
    platform/
        macos/
        linux/
```

or equivalent trait-based abstractions.

Examples of platform-dependent functions:

- service startup
- filesystem paths
- process management
- network interface discovery
- local service installation
- log directories
- auto-start
- power management
- system notifications

Do not use macOS-only APIs unless there is a Linux-compatible abstraction.

When a platform-dependent feature is necessary:

1. define a platform-neutral trait
2. implement macOS adapter
3. leave Linux implementation possible without changing domain/application code
4. document the dependency in `docs/architecture.md`

---

# 3. Architecture Principle

Use this dependency direction:

```text
UI / API / MQTT
       |
       v
Application Layer
       |
       v
Domain Layer
       ^
       |
Infrastructure Adapters
```

The domain layer must not depend on:

- MQTT
- SQLite
- Tauri
- Axum
- Tokio-specific types where avoidable
- macOS APIs
- Linux APIs

Infrastructure depends on the domain, not the reverse.

---

# 4. Recommended Rust Workspace

Prefer a Cargo workspace.

Suggested structure:

```text
hyrox-central-hub/
├── CLAUDE.md
├── Cargo.toml
├── README.md
│
├── crates/
│   ├── domain/
│   ├── application/
│   ├── storage/
│   ├── mqtt/
│   ├── api/
│   ├── platform/
│   └── simulator/
│
├── apps/
│   ├── hub-server/
│   └── operator-ui/        # optional / later
│
├── migrations/
├── config/
├── tests/
│   ├── integration/
│   └── fixtures/
│
└── docs/
    ├── architecture.md
    ├── event-protocol.md
    ├── state-machine.md
    ├── timing-rules.md
    ├── recovery.md
    └── decisions/
```

Avoid unnecessary microservices.

Phase 1 should be a modular monolith.

---

# 5. Technology Direction

Preferred stack:

- Rust stable
- Tokio
- Axum
- WebSocket
- SQLite
- SQLx
- Serde
- Tracing
- MQTT client: `rumqttc` or another mature Rust client

Local MQTT broker during development:

- Mosquitto preferred

Future Linux deployment may use:

- Mosquitto
- systemd
- local SQLite database
- reverse proxy only if later required

Do not require Docker for normal runtime.

Docker may be used for development or integration testing only.

---

# 6. Tauri Rule

Tauri is optional and must remain a client of the Central Hub.

Do not put core business logic inside Tauri commands.

Correct:

```text
Central Hub Rust Service
        |
        +-- REST API
        +-- WebSocket
        |
        +--> Browser
        +--> Tauri
        +--> Live Screen
```

Incorrect:

```text
Tauri
  |
  +-- MQTT
  +-- Race Engine
  +-- SQLite
  +-- Timing
```

The Central Hub service must continue running if:

- Tauri is closed
- Tauri crashes
- the browser disconnects
- no UI is connected

---

# 7. System Responsibilities

The Central Hub owns the following responsibilities.

## 7.1 Member Reference

The complete member profile remains in **健身管**.

Central Hub stores only required references.

Examples:

```text
member_id
display_name
membership_status
```

Do not duplicate the entire 健身管 member database.

---

## 7.2 RFID Binding

Central Hub manages:

```text
RFID Tag ID
    ↕
Member ID
```

Binding must be traceable by Session.

---

## 7.3 Edge Device Identity

ESP32 `device_id` is derived from the ESP32 Base MAC address.

Canonical internal format:

```text
esp32-a4cf128b3d91
```

Do not create random UUIDs as the primary hardware identity.

Keep `reader_id` separate from `device_id`.

One ESP32 may support more than one Reader in the future.

---

# 8. RFID Reader / Station Mapping

ESP32 must not know business meaning.

ESP32 publishes:

```text
device_id
reader_id
tag_id
detected_at
boot_id
sequence
```

Central Hub maps:

```text
device_id + reader_id
        |
        v
Station
Zone
Reader Mode
Event Role
```

Example:

```text
reader_id: RFID-02
station: SKIERG
role: ENTRY
```

Reader modes must support future flexibility.

Required model:

```text
ENTRY
EXIT
TOGGLE
CHECKPOINT
PASSAGE
```

Actual interpretation belongs to Central Hub.

---

# 9. Session Modes

Every Session has a mode.

```text
COMPETITION
TRAINING
```

## 9.1 Competition Mode

Competition follows a defined competition template / rule set.

Central Hub must validate progression according to configured rules.

Unexpected station events:

- preserve the raw event
- mark the interpreted event as exception / unexpected
- do not silently delete source data
- final rule handling must follow the configured competition rule set

Do not invent competition penalties in code unless explicitly specified.

## 9.2 Training Mode

Training allows flexible course definitions.

Training configuration must support:

- training name
- stations
- station order
- repeated stations
- distance
- repetitions
- duration
- optional target values

Training Mode records actual station events.

Do not reject or warn simply because the sequence differs from official competition order.

---

# 10. Athlete State Model

Athlete State must be explicit.

Minimum state:

```text
status:
READY
ACTIVE
FINISHED

current_station
station_state:
OUTSIDE
INSIDE

last_event
expected_station       # competition only
```

## 10.1 Dedicated ENTRY / EXIT Readers

```text
OUTSIDE + SkiErg ENTRY
→ ENTER SkiErg
→ INSIDE
```

```text
INSIDE + SkiErg EXIT
→ EXIT SkiErg
→ OUTSIDE
```

## 10.2 Shared TOGGLE Reader

```text
OUTSIDE + TOGGLE
→ ENTRY
```

```text
INSIDE + same Station TOGGLE
→ EXIT
```

The interpretation must use Athlete State.

Do not rely only on odd/even scan count.

---

# 11. Competition Start Rule

For Competition Mode:

> The first valid RFID event after the Session is ARMED starts timing for that athlete.

A valid event requires:

- known Reader
- known RFID Tag
- RFID bound to participating athlete
- Session ARMED
- event valid for the Session context

Use:

```text
started_at = detected_at
```

Do not use MQTT receive time as official timing.

---

# 12. Finish Rule

Finish behavior is not yet finalized.

Do not hard-code a finish rule.

Implement finish logic behind configurable Session / Competition rules.

Mark this as an open design item in:

```text
docs/timing-rules.md
```

---

# 13. ROX / Transition Time

ROX Zone requires no dedicated RFID reader.

It is derived from two adjacent action events.

General internal model:

```text
Transition Time
=
Next Action Start Time
-
Previous Action Finish Time
```

Example:

```text
Run Finish
10:15:20.500

SkiErg Entry
10:15:36.800
```

Then:

```text
Transition Time = 16.300 s
```

In Competition UI this may be displayed as:

```text
ROX Zone Time
```

In Training Mode use the generic term:

```text
Transition Time
```

Store transition as derived data.

---

# 14. ESP32 Event Suppression Contract

Important:

> Do NOT use a fixed 60-second suppression window.

ESP32 uses Tag Presence / Re-arm logic.

Rules:

```text
first_seen
→ SEND event
```

```text
same Tag continuously visible
→ suppress repeated RF reads
```

```text
Tag absent longer than absent_timeout
→ re-arm Tag
```

```text
Tag appears again after re-arm
→ SEND new event
```

Default target:

```text
absent_timeout = 3–5 seconds
```

The value must:

- be configurable
- be validated in the real venue
- preferably be configurable per Reader

ESP32 handles RF-level suppression.

Central Hub handles business-level deduplication.

Station duration must never be used as RFID suppression duration.

---

# 15. MQTT Reliability

MQTT is transport, not the final reliability guarantee.

Preferred:

```text
MQTT QoS 1
+
Application ACK
+
Persistent Edge Journal
+
Central Hub Deduplication
```

Event flow:

```text
RFID detection
    |
    v
ESP32 persistent journal
    |
    v
MQTT publish
    |
    v
Central Hub receive
    |
    v
SQLite COMMIT
    |
    v
Application ACK
    |
    v
ESP32 marks event acknowledged
```

Do not ACK before persistent storage commit succeeds.

---

# 16. ESP32 Event Protocol

Minimum event fields:

```json
{
  "device_id": "esp32-a4cf128b3d91",
  "reader_id": "rfid-02",
  "boot_id": 18,
  "sequence": 10382,
  "tag_id": "E280117000001234",
  "detected_at": 1787734821382,
  "uptime_ms": 382912
}
```

Central Hub adds:

```text
received_at
```

Official timing:

```text
detected_at
```

Diagnostics:

```text
received_at
```

The combination below must be idempotent:

```text
device_id + boot_id + sequence
```

Duplicate delivery is allowed.

Duplicate business processing is not allowed.

---

# 17. Timing and Clock

Central Hub is the local time authority.

Preferred architecture:

```text
Central Hub
    |
    | local time sync / NTP
    v
ESP32
```

ESP32 events should retain:

```text
detected_at
uptime_ms
boot_id
sequence
```

Do not calculate official competition timing from MQTT arrival latency.

---

# 18. ESP32 Persistent Event Journal

ESP32 must persist unacknowledged events.

This is a Phase 1 reliability requirement.

Minimum design target:

```text
10,000 events per ESP32
```

Prefer:

```text
append-only journal
+
ring buffer
```

Do not erase flash after every ACK.

Maintain ACK cursor / status and reclaim blocks in batches.

Rules:

- UNACKED events must not be deleted
- MQTT reconnect must resend unacknowledged events
- ESP32 reboot must preserve pending events
- ACK loss must be safe
- duplicate resend must be safe

Target retention:

- unacknowledged: until ACK
- acknowledged: current Session + previous Session, or at least 24 hours after Session completion

If storage capacity is close to exhaustion:

- publish critical device warning
- expose warning to Central Hub UI

---

# 19. Central Hub Storage

Use SQLite for Phase 1.

Use WAL mode unless testing proves otherwise.

Store at least:

- Member reference
- RFID binding
- Edge device
- Reader configuration
- Raw RFID Event
- Interpreted Race / Training Event
- Derived timing data
- Corrections / audit records

Raw RFID events must never be destructively edited.

---

# 20. Manual Correction

Phase 1 should preserve maximum operational flexibility.

Authorized staff may:

- add interpreted event
- void interpreted event
- change timestamp
- change station
- change ENTRY / EXIT interpretation
- change athlete assignment
- repair RFID binding

Raw RFID events remain immutable.

Every correction must contain:

```text
operator
timestamp
reason
before
after
```

All affected derived values must be recalculated:

- athlete state
- splits
- transitions
- ROX
- total time
- ranking

---

# 21. Recovery

Central Hub must recover an active Session after restart.

Do not keep critical state only in memory.

Use persisted events / snapshots sufficient to rebuild:

```text
Session
Athlete State
Timing
Leaderboard
```

After restart:

```text
load active Session
rebuild / restore state
resume MQTT ingestion
continue competition / training
```

Design state changes so replay is deterministic.

---

# 22. Web Access

Central Hub must expose a local web service.

Coach / staff devices should be able to connect via local Wi-Fi / LAN.

Preferred interfaces:

```text
REST API
WebSocket
```

Suggested views:

```text
/coach
/live
/result/{id}
```

Coach / staff must be able to use:

- phone
- tablet
- laptop browser

Do not require native mobile apps for Phase 1.

---

# 23. Coach View

Minimum Phase 1 coach data:

```text
Athlete
Current Station
Current State
Elapsed Time
Last Event
Run Split
Workout Split
Transition / ROX
Current Ranking
Device / Reader Warnings
```

Use WebSocket for live updates.

Avoid polling for high-frequency live status.

---

# 24. Testing Policy — TDD Required

This project must use Test-Driven Development.

For business logic:

1. write failing test
2. implement minimum logic
3. make test pass
4. refactor
5. keep tests green

Tests are required for:

- competition start
- training event acceptance
- athlete state transitions
- ENTRY / EXIT Reader interpretation
- TOGGLE Reader interpretation
- duplicate MQTT event
- sequence idempotency
- ROX / transition calculations
- crash recovery
- manual corrections
- leaderboard recalculation
- invalid Reader
- unknown RFID
- event replay

Core business behavior must be testable without:

- actual RFID hardware
- MQTT broker
- Tauri
- macOS UI
- Linux

---

# 25. Simulator Required

Create a simulator early.

The simulator must be able to emulate:

- multiple ESP32 devices
- configurable MAC addresses
- multiple Readers
- multiple RFID Tags
- repeated RF reads
- Tag absence / re-arm
- MQTT disconnect
- resend after reconnect
- duplicated event
- missing ACK
- device reboot
- out-of-order arrival

---

# 26. Development Priorities

Implement in this order unless explicitly instructed otherwise.

## Milestone 1 — Domain Foundation

- Session model
- Competition / Training mode
- Athlete model
- Reader mapping
- RFID binding
- domain event model
- timing types
- transition calculation

## Milestone 2 — Persistence

- SQLite schema
- migrations
- raw event store
- interpreted event store
- recovery tests

## Milestone 3 — MQTT Ingestion

- MQTT client
- event validation
- idempotency
- ACK protocol
- simulator

## Milestone 4 — Race / Training Engine

- start rule
- athlete state
- competition validation
- training record-as-is
- transition / ROX
- recalculation

## Milestone 5 — REST / WebSocket

- session APIs
- reader config
- coach live data
- leaderboard

## Milestone 6 — Operator UI

- browser-based admin first
- Tauri optional after core stability

Do not begin with visual UI work.

---

# 27. Definition of Done

A feature is not done until:

- tests exist
- tests pass
- error cases are covered
- relevant docs are updated
- no business rule exists only in UI code
- behavior is reproducible on macOS
- no unnecessary macOS dependency prevents Linux deployment

---

# 28. Open Issues

Do not invent answers for unresolved product rules.

Maintain unresolved decisions under:

```text
docs/decisions/
```

or:

```text
docs/open-issues.md
```

Current known open issues include:

- exact Finish event definition
- actual venue Reader layout
- whether stations use dedicated ENTRY / EXIT or TOGGLE Readers
- final competition rule exception behavior
- exact 健身管 API contract
- exact Coach correction permission model
- production networking / VLAN design
- Linux service packaging

When blocked by an unresolved product rule:

1. document the assumption
2. isolate it behind configuration / policy
3. do not hard-code speculative behavior

---

# 29. Coding Rules

Prefer:

- explicit types
- deterministic state transitions
- immutable raw events
- pure domain functions
- idempotent handlers
- clear error enums
- small focused modules
- testable application services

Avoid:

- global mutable state
- business logic in HTTP handlers
- business logic in MQTT callbacks
- business logic in Tauri commands
- direct SQL throughout the codebase
- hidden timing assumptions
- silent event deletion
- magic constants
- macOS-only shortcuts in core modules

---

# 30. Claude Working Behavior

Before implementing a feature:

1. inspect existing architecture
2. identify domain rule
3. locate existing tests
4. add / update tests first
5. implement smallest coherent change
6. run relevant tests
7. run full test suite when practical
8. update documentation if contracts changed

For significant architectural changes:

- write an ADR under `docs/decisions/`
- explain alternatives
- explain portability impact
- explain macOS / Linux impact

Never silently change:

- MQTT event contract
- database schema
- Reader semantics
- timing rules
- Athlete State semantics
- recovery behavior

These require explicit documentation and tests.

---

# 31. Key Product Principle

Always optimize in this order:

1. **No lost RFID events**
2. **Correct event interpretation**
3. **Correct timing**
4. **Recoverability**
5. **Operational simplicity**
6. **Live visibility**
7. **UI polish**

The system must remain useful even when:

- Internet is unavailable
- UI is disconnected
- MQTT reconnects
- ESP32 temporarily disconnects
- Central Hub restarts

Phase 1 is successful when the system is:

> stable, traceable, recoverable, deployable, and ready to evolve from macOS development to Linux production without rewriting the core.
