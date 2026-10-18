#!/bin/sh
# Linux/ext4 durable-ACK baseline; this does not certify physical power-cut safety.
# Build as the checkout owner first:
#   cargo build --locked -p taypeer-storage --example power_cut
# Obtain https://github.com/josefbacik/log-writes at the exact revision below, then:
#   sudo sh scripts/check-power-cut.sh "$PWD/target/debug/examples/power_cut" /path/to/log-writes
# Requires a real Linux host/VM with dm-log-writes, loop devices and mount capability.
# Orb userspace machines/ordinary containers without those capabilities are unsupported.
# Only private loopfiles are formatted. No device paths or size overrides are accepted.
# Replay drops the recorded tail after each ACK; no SIGKILL is called a power cut.
set -eu
fail() { echo "power-cut: $*" >&2; exit 2; }
[ "$(uname -s)" = Linux ] || fail 'requires Linux with dm-log-writes'
[ "$(id -u)" = 0 ] || fail 'requires root for private loop devices and mounts'
[ "$#" = 2 ] || fail 'usage: check-power-cut.sh /absolute/power_cut /absolute/log-writes-source'
helper=$1
source=$2
case "$helper" in /*) ;; *) fail 'helper path must be absolute' ;; esac
case "$source" in /*) ;; *) fail 'source path must be absolute' ;; esac
[ -f "$helper" ] && [ -x "$helper" ] || fail 'build the storage power_cut example first'
for tool in dmsetup losetup mkfs.ext4 mount umount mountpoint blockdev cp truncate mktemp git sha256sum cc timeout; do
    command -v "$tool" >/dev/null 2>&1 || fail "missing tool: $tool"
done
[ -c /dev/mapper/control ] && [ -c /dev/loop-control ] || fail 'missing device-mapper/loop control devices'
dmsetup targets | awk '$1 == "log-writes" { found=1 } END { exit !found }' || fail 'dm-log-writes target unavailable'
losetup --find >/dev/null || fail 'loop allocation capability unavailable'
revision=7b70d8a6863c5de30933d42a7672d35d01d2dc6c
[ "$(git -c safe.directory="$source" -C "$source" rev-parse HEAD)" = "$revision" ] || fail "log-writes must be checked out at $revision"
name=taypeer-powercut-$$
if dmsetup info "$name" >/dev/null 2>&1; then fail "mapper already exists: $name"; fi
work=$(mktemp -d /tmp/taypeer-powercut.XXXXXXXX)
data= log= replay= mapped=0
cleanup() {
    status=$?
    trap - EXIT HUP INT TERM
    clean=1
    if mountpoint -q "$work/mount"; then umount "$work/mount" || clean=0; fi
    if [ "$mapped" = 1 ]; then dmsetup remove "$name" || clean=0; fi
    for device in "$replay" "$log" "$data"; do
        if [ -n "$device" ]; then losetup -d "$device" || clean=0; fi
    done
    if [ "$clean" = 1 ]; then
        rm -rf "$work"
    else
        echo "power-cut: cleanup incomplete; retained private files at $work" >&2
        status=2
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 2' HUP INT TERM
mkdir "$work/mount" "$work/oracle" "$work/tool"
cp "$helper" "$work/power_cut"
helper=$work/power_cut
uname -srvm
sha256sum "$helper"
# Verify every input to the tiny userspace replayer before compiling it as root.
cat > "$work/sources.sha256" <<'HASHES'
5b26d1a912ba3570057d273f8dc8e63c23e3c6febc2028c383668e1654b61406  log-writes.c
74deb4e1ea8e243178c375be0aa8468c1a0c70d1014a452b443e67ff0161906b  log-writes.h
3fa8581788ba60ac3ec1b845c64f000bb9051ebfc5a9fd26d62b6913ae62bd4b  replay-log.c
HASHES
for file in log-writes.c log-writes.h replay-log.c; do cp "$source/$file" "$work/tool/$file"; done
(cd "$work/tool" && sha256sum -c "$work/sources.sha256")
cc -D_GNU_SOURCE -O2 -o "$work/tool/replay-log" "$work/tool/replay-log.c" "$work/tool/log-writes.c"
# Fixed logical budget: 128 MiB each base/data/replay + 256 MiB log = 640 MiB.
truncate -s 128M "$work/base.img"
mkfs.ext4 -q -F -E lazy_itable_init=0,lazy_journal_init=0,nodiscard "$work/base.img"
cp --sparse=always "$work/base.img" "$work/data.img"
truncate -s 256M "$work/log.img"
data=$(losetup --find --show "$work/data.img")
log=$(losetup --find --show "$work/log.img")
dmsetup create "$name" --table "0 $(blockdev --getsz "$data") log-writes $data $log"
mapped=1
mount -t ext4 -o nodiscard,noatime "/dev/mapper/$name" "$work/mount"
for phase in create replace; do
    timeout 120 "$helper" "$phase" "$work/mount" "$work/oracle/$phase.json"
    dmsetup message "$name" 0 mark "$phase-ack"
done
# This later write must never appear in a replay stopped at an earlier ACK.
printf 'PUBLIC discarded tail\n' > "$work/mount/volatile-tail"
umount "$work/mount"
dmsetup remove "$name"
mapped=0
for phase in create replace; do
    # Never reuse a previously recovered/journal-modified replay image.
    cp --sparse=always "$work/base.img" "$work/replay.img"
    replay=$(losetup --find --show "$work/replay.img")
    "$work/tool/replay-log" --log "$log" --find --end-mark "$phase-ack" >/dev/null
    timeout 120 "$work/tool/replay-log" --log "$log" --replay "$replay" --end-mark "$phase-ack"
    mount -t ext4 -o nodiscard,noatime "$replay" "$work/mount"
    [ ! -e "$work/mount/volatile-tail" ] || fail 'post-ACK tail leaked into replay'
    timeout 120 "$helper" verify "$work/mount" "$work/oracle/$phase.json"
    umount "$work/mount"
    losetup -d "$replay"
    replay=
done
echo 'PASS: ext4 create/replace ACK replay baseline (2 cuts, fresh image/process each)'
echo 'Not covered: intermediate/no-ACK cuts, anchors, credentials, drafts, platform hardware'
