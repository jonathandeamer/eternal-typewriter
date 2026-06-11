#!/usr/bin/env bash
#
# Record the README demo GIF (docs/demo.gif).
#
# Boots the typewriter twice on a fresh scroll and captures the real
# framebuffer via QEMU's `screendump` monitor command (no screen recorder),
# then assembles the frames into a square GIF with ffmpeg. The two boots use
# `-rtc base` to stamp the date-separator with the real timestamps of the
# project's first night commit and first morning commit, so the page tells
# the overnight-session story by itself.
#
# Requires: qemu-system-x86_64, python3, ffmpeg. Run from anywhere.
set -euo pipefail
cd "$(dirname "$0")/.."

# --- what the demo shows (edit these to re-theme the GIF) --------------------
LINE1="the ink is dry before you lift your finger"
LINE2="and it will still be here tomorrow"
RTC1="2026-06-10T22:22:00"   # first night commit:   docs: add design spec
RTC2="2026-06-11T07:30:00"   # first morning commit: fix: address code review
OUT="docs/demo.gif"
# Square crop from the top-left of the 1280x720 framebuffer, scaled to 600px.
CROP="crop=620:620:0:0,scale=600:600:flags=lanczos"
# -----------------------------------------------------------------------------

WORK=$(mktemp -d /tmp/etyp-demo.XXXXXX)
FRAMES="$WORK/frames"
mkdir -p "$FRAMES"
trap 'rm -rf "$WORK"' EXIT

BIOS=$(cargo run --quiet -- --print-bios-image)
dd if=/dev/zero of="$WORK/scroll.img" bs=1M count=8 2>/dev/null

boot_and_drive () {
  local start=$1 text="$2" intro=$3 final=$4 rtcbase=$5
  rm -f "$WORK/mon.sock" "$WORK/boot.log"
  qemu-system-x86_64 -m 512M -rtc "base=$rtcbase" \
    -drive "format=raw,file=$BIOS" \
    -drive "format=raw,file=$WORK/scroll.img,if=ide,index=1" \
    -monitor "unix:$WORK/mon.sock,server,nowait" -display none \
    -serial "file:$WORK/boot.log" &
  local qpid=$!
  local i
  for i in $(seq 1 40); do
    grep -q "scroll restored" "$WORK/boot.log" 2>/dev/null && break || true
    sleep 0.25
  done
  sleep 0.4
  python3 "$(dirname "$0")/record_demo.py" \
    "$WORK/mon.sock" "$FRAMES" "$start" "$text" "$intro" "$final"
  wait "$qpid" 2>/dev/null || true
}

echo "phase 1: night boot ($RTC1), type line 1"
N1=$(boot_and_drive 0 "$LINE1" 4 5 "$RTC1")
echo "phase 2: morning reboot ($RTC2), line 1 persists, type line 2"
N2=$(boot_and_drive "$N1" "$LINE2" 7 6 "$RTC2")
echo "captured $(ls "$FRAMES" | wc -l | tr -d ' ') frames"

mkdir -p "$(dirname "$OUT")"
ffmpeg -y -framerate 10 -i "$FRAMES/f%04d.ppm" \
  -vf "$CROP,split[a][b];[a]palettegen=stats_mode=full[p];[b][p]paletteuse=dither=none" \
  -loop 0 "$OUT" 2>/dev/null

echo "wrote $OUT"
