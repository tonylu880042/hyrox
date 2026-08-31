#!/bin/bash
# Publishes dist/*.deb to the apt repository on S3 (ADR 0009 §4).
#
# The bucket is public-read on purpose. An apt repository's integrity comes from the GPG
# signature on `InRelease`, not from the transport: a public bucket serving signed packages
# cannot be tampered with, and IAM-authenticated apt would put rotatable credentials on
# every appliance for no gain. If the software itself must not be downloadable by outsiders,
# that is confidentiality, not integrity -- put CloudFront signed URLs in front and leave
# apt's trust model alone.
#
#   HYROX_APT_BUCKET=hyrox-apt HYROX_GPG_KEY=ops@example.com ./packaging/publish-s3.sh
set -euo pipefail
cd "$(dirname "$0")/.."

BUCKET="${HYROX_APT_BUCKET:?set HYROX_APT_BUCKET}"
KEY="${HYROX_GPG_KEY:?set HYROX_GPG_KEY to the signing key uid}"
SUITE="${HYROX_APT_SUITE:-stable}"
REPO="target/apt"

command -v apt-ftparchive >/dev/null || {
    echo "apt-ftparchive is missing (apt-utils). Run this on Linux." >&2; exit 1; }

rm -rf "$REPO"
mkdir -p "$REPO/pool/main" "$REPO/dists/$SUITE/main/binary-amd64"
cp dist/*.deb "$REPO/pool/main/"

cd "$REPO"
apt-ftparchive packages pool/main > "dists/$SUITE/main/binary-amd64/Packages"
gzip -9kf "dists/$SUITE/main/binary-amd64/Packages"

apt-ftparchive \
    -o "APT::FTPArchive::Release::Origin=HYROX" \
    -o "APT::FTPArchive::Release::Label=HYROX Central Hub" \
    -o "APT::FTPArchive::Release::Suite=$SUITE" \
    -o "APT::FTPArchive::Release::Codename=$SUITE" \
    -o "APT::FTPArchive::Release::Architectures=amd64" \
    -o "APT::FTPArchive::Release::Components=main" \
    release "dists/$SUITE" > "dists/$SUITE/Release"

# InRelease is the inline-signed form modern apt prefers; Release.gpg is kept for anything
# older. Signing is the whole security model here -- if this step is skipped, every
# appliance will refuse the repository, which is the correct failure.
gpg --default-key "$KEY" --clearsign -o "dists/$SUITE/InRelease"   "dists/$SUITE/Release"
gpg --default-key "$KEY" -abs        -o "dists/$SUITE/Release.gpg" "dists/$SUITE/Release"

# Metadata last: a machine that syncs mid-upload must never see a Release naming a package
# that is not there yet.
aws s3 sync pool  "s3://$BUCKET/pool"  --acl public-read --delete
aws s3 sync dists "s3://$BUCKET/dists" --acl public-read --delete --cache-control "max-age=60"

echo "published $SUITE to s3://$BUCKET"
echo "appliances read it via /etc/apt/sources.list.d/hyrox.list"
