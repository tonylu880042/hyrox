#!/bin/bash
# Builds hyrox-hub_<version>_amd64.deb (ADR 0009 §4).
#
# Run on Linux (or in a Linux container): the binary inside the package has to be a Linux
# binary. Everything else here is file copying.
#
#   ./packaging/build-deb.sh            # version from apps/hub-server/Cargo.toml
#   ./packaging/build-deb.sh 1.2.3      # or an explicit one
#   HYROX_WITH_DEMO=1 ./packaging/build-deb.sh   # keep the demo venue in the binary
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:-$(awk -F'"' '/^version/ {print $2; exit}' apps/hub-server/Cargo.toml)}"
ARCH=amd64
OUT="dist/hyrox-hub_${VERSION}_${ARCH}.deb"
ROOT="target/deb/hyrox-hub_${VERSION}"

# No dev simulator in a venue build: --no-default-features leaves the emulated ESP32 fleet
# out of the binary entirely, rather than merely switching it off at runtime.
#
# HYROX_WITH_DEMO=1 keeps it in, for the machines we use to demonstrate and integration-test
# the system. Even then the demo is inert until `HYROX_DEMO=1` is in hub.env: two switches,
# because the one that matters -- a customer's machine has no invented athletes -- must not
# depend on remembering an environment variable (ADR 0013 §7).
if [ "${HYROX_WITH_DEMO:-0}" = "1" ]; then
    echo "building WITH the demo venue (a test machine's build, not a venue's)"
    cargo build --release -p hub-server
else
    cargo build --release --no-default-features -p hub-server
fi

rm -rf "$ROOT"
install -Dm755 target/release/hub-server            "$ROOT/usr/bin/hyrox-hub"
install -Dm755 packaging/bin/maintenance            "$ROOT/usr/lib/hyrox/maintenance"
install -Dm755 packaging/bin/backup                 "$ROOT/usr/lib/hyrox/backup"
install -Dm644 packaging/etc/hyrox/hub.env          "$ROOT/etc/hyrox/hub.env"
# What the hub is allowed to ask of the machine (M6). A polkit rule rather than sudo: the
# hub's unit sets NoNewPrivileges, which blocks setuid binaries by design.
install -Dm644 packaging/etc/mosquitto/conf.d/hyrox.conf "$ROOT/etc/mosquitto/conf.d/hyrox.conf"
install -Dm644 packaging/etc/chrony/conf.d/hyrox.conf   "$ROOT/etc/chrony/conf.d/hyrox.conf"
install -Dm644 packaging/etc/polkit-1/rules.d/50-hyrox-power.rules \
    "$ROOT/etc/polkit-1/rules.d/50-hyrox-power.rules"
# The .mount is shipped but not enabled: it belongs to a venue that was given a USB stick
# (ADR 0012), and enabling it on a machine without one would log a failure every boot.
for unit in packaging/systemd/*.service packaging/systemd/*.timer packaging/systemd/*.mount; do
    install -Dm644 "$unit" "$ROOT/lib/systemd/system/$(basename "$unit")"
done

mkdir -p "$ROOT/DEBIAN"
# The kiosk's packages are Recommends, not Depends. The hub is a service: it ingests, times
# and serves HTTP, and it does all of that on a machine with no screen at all. Only
# `hyrox-kiosk.service` needs a browser and a compositor, and that unit is for the box
# wired to the projector (ADR 0009).
#
# apt installs Recommends by default, so a venue build still gets them; `dpkg -i` on a
# headless server no longer fails. It also unblocks a real problem: google-chrome-stable is
# not in Ubuntu's own archive, so as a hard dependency it made the package uninstallable
# until somebody had added Google's apt source.
#
# chrony and avahi-daemon are what a board needs from this host besides the broker: a clock
# to ask, and an answer to `hyrox-hub.local` so no venue's IP has to be flashed into it.
cat > "$ROOT/DEBIAN/control" <<EOF
Package: hyrox-hub
Version: $VERSION
Section: misc
Priority: optional
Architecture: $ARCH
Maintainer: HYROX Central Hub <ops@example.com>
Depends: mosquitto, chrony, avahi-daemon, curl, jq, sqlite3
Recommends: cage, google-chrome-stable
Description: HYROX Central Hub
 On-site host for HYROX competition and training sessions: RFID ingestion over MQTT,
 timing, the live screen and the coaches' web surface.
EOF

# hub.env is the venue's configuration; an upgrade must never overwrite an edited one.
printf '%s\n' /etc/hyrox/hub.env /etc/mosquitto/conf.d/hyrox.conf /etc/chrony/conf.d/hyrox.conf \
    > "$ROOT/DEBIAN/conffiles"

cat > "$ROOT/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
# System users, no login shells. /var/lib/hyrox is created and owned by systemd's
# StateDirectory=, so there is no chown here to get wrong.
adduser --system --group --no-create-home --quiet hyrox   || true
adduser --system --group --no-create-home --quiet kiosk   || true
adduser kiosk video  || true
adduser kiosk input  || true
adduser kiosk render || true

# The hub and scheduled workers share a random machine credential through a root owned file.
# Generate it once; upgrades preserve the existing value so a running worker is never
# invalidated by package installation. `hyrox` needs read access and the backup unit runs as
# root, while the file remains inaccessible to ordinary venue users.
CREDENTIAL_FILE=/etc/hyrox/backup-credential
if [ ! -s "$CREDENTIAL_FILE" ]; then
    umask 077
    temporary=$(mktemp)
    dd if=/dev/urandom bs=32 count=1 status=none | base64 | tr -d '\n' > "$temporary"
    chown hyrox:hyrox "$temporary"
    chmod 600 "$temporary"
    mv -f "$temporary" "$CREDENTIAL_FILE"
else
    chown hyrox:hyrox "$CREDENTIAL_FILE"
    chmod 600 "$CREDENTIAL_FILE"
fi

systemctl daemon-reload
systemctl enable hyrox-hub.service hyrox-maintenance.timer hyrox-backup.timer
# Only where there is something to show it on. Enabling a kiosk on a headless server would
# log a failed unit every boot for a screen nobody is standing in front of.
if command -v google-chrome-stable > /dev/null 2>&1 && command -v cage > /dev/null 2>&1; then
    systemctl enable hyrox-kiosk.service
fi
# Pick up the listener and the LAN time server this package ships. Before the hub, so it
# connects to the broker as configured. Boards reconnect by themselves, and nothing they
# hold is lost: an unacknowledged read stays in their journal (CLAUDE.md 18).
systemctl restart mosquitto.service chrony.service || true
# `restart`, not `start`: this same script runs on upgrade, and the new binary has to be the
# one running before the maintenance job verifies it.
systemctl restart hyrox-hub.service
# The kiosk is left alone on upgrade. Restarting it would blank the projector mid-class, and
# the page reconnects over the WebSocket by itself. Started only where it was enabled just
# above -- on a headless machine there is no screen and no browser, and starting it anyway
# leaves a unit stuck in `activating` forever.
if systemctl is-enabled hyrox-kiosk.service > /dev/null 2>&1; then
    systemctl start hyrox-kiosk.service || true
fi
systemctl start hyrox-maintenance.timer hyrox-backup.timer
EOF
chmod 755 "$ROOT/DEBIAN/postinst"

cat > "$ROOT/DEBIAN/prerm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = remove ]; then
    systemctl disable --now hyrox-kiosk.service hyrox-maintenance.timer hyrox-backup.timer \
        hyrox-hub.service || true
fi
EOF
chmod 755 "$ROOT/DEBIAN/prerm"

mkdir -p dist
dpkg-deb --root-owner-group --build "$ROOT" "$OUT"
echo "built $OUT"
