# Packaging the appliance

The hub ships as a complete machine: Ubuntu Server 24.04 LTS, one browser in a kiosk
compositor, updates from a signed apt repository on S3. Decisions and reasoning:
[ADR 0009](../docs/decisions/0009-shipped-as-an-appliance.md).

> **Status.** The build scripts are syntax-checked and the units are written against
> documented systemd behaviour, but **none of this has run on real hardware yet**. The
> kiosk half in particular — `cage` taking DRM master, the `kiosk` user's seat access —
> needs a real machine and a real projector before it ships. Treat the first build as a
> bring-up exercise, not a release.

---

## What is here

| Path | Is |
| --- | --- |
| `systemd/hyrox-hub.service` | the hub daemon; owns the database, MQTT and HTTP |
| `systemd/hyrox-kiosk.service` | `cage` + Chrome on tty1, waits for the hub to answer |
| `systemd/hyrox-maintenance.{service,timer}` | the nightly window |
| `bin/maintenance` | ask → update → verify → power off |
| `etc/hyrox/hub.env` | the venue's configuration, read by systemd |
| `build-deb.sh` | produces `dist/hyrox-hub_<version>_amd64.deb` |
| `publish-s3.sh` | signs the repository and syncs it to S3 |
| `autoinstall/user-data` | production line: unattended Ubuntu install from a USB stick |

---

## Building a release

Both scripts are Linux-only — the binary in the package has to be a Linux binary, and
`apt-ftparchive` is a Debian tool. On a Mac, run them in a container.

```bash
./packaging/build-deb.sh                 # version from apps/hub-server/Cargo.toml
HYROX_APT_BUCKET=hyrox-apt \
HYROX_GPG_KEY=ops@example.com \
  ./packaging/publish-s3.sh
```

The venue build is `--no-default-features`, so the emulated ESP32 fleet is not in the
binary at all — not merely switched off at runtime.

### The signing key

The repository's integrity is the GPG signature and nothing else, so:

* the private key lives wherever you build releases, and **nowhere on an appliance**;
* the public key is at `s3://<bucket>/hyrox.gpg`, and every machine installs it at
  provisioning time (`autoinstall/user-data`);
* a machine that cannot verify the signature refuses the repository. That is the correct
  failure — it will keep running the version it has.

### Why the bucket is public-read

An apt repository's trust comes from the signature on `InRelease`, not from the transport.
A public bucket serving signed packages cannot be tampered with, and IAM-authenticated apt
would put rotatable credentials on every appliance for no gain in integrity.

"Nobody else should be able to download our software" is a different requirement —
confidentiality, not integrity. Put CloudFront signed URLs in front of the bucket and leave
apt's trust model alone.

---

## The nightly window

```text
timer fires
  ├─ GET /api/health
  │    safe_to_stop = false  → log, do nothing, try again tomorrow
  ├─ apt-get update fails    → power off without updating
  ├─ no new version          → power off
  └─ new version
       ├─ install → verify it answers /api/health → power off
       └─ verification fails → roll back → verify → power off
                              └─ rollback also fails → STAY UP for inspection
```

Three things in there are deliberate:

**It asks the hub, never the clock.** `safe_to_stop` is false while the session is READY,
RUNNING or PAUSED, or while an edge device still reports unacknowledged events
(CLAUDE.md 18). No answer at all also counts as "no".

**A failed update is rolled back, not left in place.** A venue opening to a machine that
will not start is the worst outcome this script can produce. The previous `.deb` is still in
apt's cache, which is what makes the rollback possible. If *both* versions fail to serve,
the machine deliberately stays powered on — powering off would hide it.

**`Persistent=false` on the timer, and it must stay that way.** `Persistent=true` runs a
missed timer at the next boot, which for a job ending in `poweroff` means the machine shuts
itself down moments after somebody switches it on in the morning.

### Scheduling

The `OnCalendar` in the shipped timer is a placeholder. It belongs to the venue's opening
hours, so set it per customer without editing the packaged file:

```bash
systemctl edit hyrox-maintenance.timer
```

```ini
[Timer]
OnCalendar=
OnCalendar=*-*-* 23:30:00
```

The empty `OnCalendar=` first is required — systemd *adds* schedules otherwise, and you
would get both.

---

## Provisioning a machine

Put `autoinstall/user-data` and the empty `meta-data` in a `nocloud` directory on a USB
stick beside the Ubuntu Server ISO, boot, walk away.

Replace before the first build: the password hash, the SSH key, and the S3 bucket name.

---

## Checking a machine

```bash
curl -s http://127.0.0.1:8730/api/health | jq
systemctl status hyrox-hub hyrox-kiosk
systemctl list-timers hyrox-maintenance.timer
journalctl -t hyrox-maintenance --since yesterday
```

`/api/health` is the one endpoint with no `freshness` envelope: it is read by a shell
script, and `safe_to_stop` has to be the whole answer.

---

## Still open

* **The maintenance schedule** is a placeholder pending the customer conversation.
* **Offline venues** are not handled. Nothing breaks — a machine that cannot reach S3 keeps
  running the version it has — but there is no USB update path.
* **Fleet visibility.** `/api/health` answers for one machine to itself. Knowing which
  venues are on which version, remotely, is not built.
