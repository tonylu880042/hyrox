#!/bin/bash
# Builds hyrox-hub_<version>_amd64.deb (ADR 0009 §4).
#
# Run on Linux (or in a Linux container): the binary inside the package has to be a Linux
# binary. Everything else here is file copying.
#
#   ./packaging/build-deb.sh            # version from apps/hub-server/Cargo.toml
#   ./packaging/build-deb.sh 1.2.3      # or an explicit one
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:-$(awk -F'"' '/^version/ {print $2; exit}' apps/hub-server/Cargo.toml)}"
ARCH=amd64
OUT="dist/hyrox-hub_${VERSION}_${ARCH}.deb"
ROOT="target/deb/hyrox-hub_${VERSION}"

# No dev simulator in a venue build: --no-default-features leaves the emulated ESP32 fleet
# out of the binary entirely, rather than merely switching it off at runtime.
cargo build --release --no-default-features -p hub-server

rm -rf "$ROOT"
install -Dm755 target/release/hub-server            "$ROOT/usr/bin/hyrox-hub"
install -Dm755 packaging/bin/maintenance            "$ROOT/usr/lib/hyrox/maintenance"
install -Dm644 packaging/etc/hyrox/hub.env          "$ROOT/etc/hyrox/hub.env"
for unit in packaging/systemd/*.service packaging/systemd/*.timer; do
    install -Dm644 "$unit" "$ROOT/lib/systemd/system/$(basename "$unit")"
done

mkdir -p "$ROOT/DEBIAN"
cat > "$ROOT/DEBIAN/control" <<EOF
Package: hyrox-hub
Version: $VERSION
Section: misc
Priority: optional
Architecture: $ARCH
Maintainer: HYROX Central Hub <ops@example.com>
Depends: mosquitto, curl, jq, cage, google-chrome-stable
Description: HYROX Central Hub
 On-site host for HYROX competition and training sessions: RFID ingestion over MQTT,
 timing, the live screen and the coaches' web surface.
EOF

# hub.env is the venue's configuration; an upgrade must never overwrite an edited one.
echo "/etc/hyrox/hub.env" > "$ROOT/DEBIAN/conffiles"

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

systemctl daemon-reload
systemctl enable hyrox-hub.service hyrox-kiosk.service hyrox-maintenance.timer
# `restart`, not `start`: this same script runs on upgrade, and the new binary has to be the
# one running before the maintenance job verifies it.
systemctl restart hyrox-hub.service
# The kiosk is left alone on upgrade. Restarting it would blank the projector mid-class, and
# the page reconnects over the WebSocket by itself.
systemctl start hyrox-kiosk.service    || true
systemctl start hyrox-maintenance.timer
EOF
chmod 755 "$ROOT/DEBIAN/postinst"

cat > "$ROOT/DEBIAN/prerm" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = remove ]; then
    systemctl disable --now hyrox-kiosk.service hyrox-maintenance.timer hyrox-hub.service || true
fi
EOF
chmod 755 "$ROOT/DEBIAN/prerm"

mkdir -p dist
dpkg-deb --root-owner-group --build "$ROOT" "$OUT"
echo "built $OUT"
