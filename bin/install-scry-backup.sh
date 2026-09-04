#!/usr/bin/env bash
# Idempotent installer for the Scry local dump + off-host backup pipeline.
# Installs from this repository onto the production host:
#   bin/scry-pg-dump                      -> /usr/local/sbin/scry-pg-dump
#   bin/scry-backup-offhost               -> /usr/local/sbin/scry-backup-offhost
#   etc/cron.d/scry-pg-dump               -> /etc/cron.d/scry-pg-dump
#   etc/systemd/scry-backup-*.service/.timer -> /etc/systemd/system/
# then arms the timer and runs one probe pass.
#
# The backup pipeline stays inert until /etc/public-apps/scry-backup.env
# exists; see docs/runbook.md "Off-host backup" for the operator contract.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
host="${SCRY_SSH_HOST:-root@public-apps.tail5f5eb4.ts.net}"
stage="/tmp/scry-backup-install"

paths=(
  bin/scry-pg-dump
  bin/scry-backup-offhost
  bin/retention-preflight.py
  etc/cron.d/scry-pg-dump
  etc/systemd/scry-backup-offhost.service
  etc/systemd/scry-backup-offhost.timer
  etc/systemd/scry-backup-alert.service
)
for rel in "${paths[@]}"; do
  test -f "$repo_root/$rel"
done

revision="$(git -C "$repo_root" rev-parse HEAD)"

# Call 1: stream the tree into a fresh remote stage directory.
# A dedicated directory avoids same-basename collisions (bin/scry-pg-dump
# vs etc/cron.d/scry-pg-dump) that flat scp staging would cause.
ssh "$host" "rm -rf '$stage' && mkdir -p '$stage'"
tar -C "$repo_root" -cf - "${paths[@]}" \
  | ssh "$host" "tar -xf - -C '$stage'"

# Call 2: install from the staged tree, then probe.
ssh "$host" sh -s -- "$revision" "$stage" <<'REMOTE'
set -eu
revision=$1
stage=$2

install -m 0755 "$stage/bin/scry-pg-dump" /usr/local/sbin/scry-pg-dump
install -m 0755 "$stage/bin/scry-backup-offhost" /usr/local/sbin/scry-backup-offhost
install -d -m 0755 /usr/local/lib/scry
install -m 0755 "$stage/bin/retention-preflight.py" /usr/local/lib/scry/retention-preflight.py
install -m 0644 "$stage/etc/cron.d/scry-pg-dump" /etc/cron.d/scry-pg-dump
install -m 0644 "$stage/etc/systemd/scry-backup-offhost.service" /etc/systemd/system/scry-backup-offhost.service
install -m 0644 "$stage/etc/systemd/scry-backup-offhost.timer" /etc/systemd/system/scry-backup-offhost.timer
install -m 0644 "$stage/etc/systemd/scry-backup-alert.service" /etc/systemd/system/scry-backup-alert.service

# Dependency gate BEFORE arming the timer: a replacement host without
# boto3 must fail this install, not pass a missing-env probe and break at
# its first configured backup. Distro package preferred; nothing is piped.
if ! python3 -c 'import boto3' >/dev/null 2>&1; then
  echo "python3-boto3 missing; installing python3-boto3"
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq python3-boto3
fi
python3 -c 'import boto3' >/dev/null || {
  echo "ERROR: python3-boto3 unavailable after install attempt" >&2
  exit 1
}

systemctl daemon-reload
systemctl enable --now scry-backup-offhost.timer
# Propagates: the script's skip path exits 0, so a non-zero rc here means
# a configured run genuinely failed and the install must not claim success.
systemctl start scry-backup-offhost.service
rm -rf "$stage"
REMOTE

echo "Installed from $repo_root at $revision"
