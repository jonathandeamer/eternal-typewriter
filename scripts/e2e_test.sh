#!/usr/bin/env bash
# Boot the typewriter headless, type via the QEMU monitor, kill the power,
# boot again, and verify every keystroke survived.
set -euo pipefail
cd "$(dirname "$0")/.."

IMG=$(cargo run --quiet -- --print-bios-image)
MON=$(mktemp -u /tmp/etyp-monitor.XXXXXX.sock)
SCROLL=e2e-scroll.img

cleanup() {
    if [ -n "${QEMU_PID:-}" ]; then
        kill "$QEMU_PID" 2>/dev/null || true
        wait "$QEMU_PID" 2>/dev/null || true
    fi
    rm -f "$SCROLL" "$MON" "e2e-serial.log"
}
trap cleanup EXIT INT TERM

rm -f "$SCROLL" "$MON" "e2e-serial.log"
truncate -s 50M "$SCROLL" # zeroed: the spec's disk precondition

boot() {
    rm -f e2e-serial.log
    qemu-system-x86_64 \
        -m 512M \
        -drive "format=raw,file=$IMG" \
        -drive "format=raw,file=$SCROLL,if=ide,index=1" \
        -monitor "unix:$MON,server,nowait" \
        -display none -serial file:e2e-serial.log &
    QEMU_PID=$!
    
    # Wait for the kernel to finish booting and restore the scroll
    for ((i=0; i<200; i++)); do
        if grep -q "scroll restored:" e2e-serial.log 2>/dev/null; then
            return
        fi
        sleep 0.1
    done
    echo "FAIL: QEMU boot timeout" >&2
    exit 1
}

mon() {
    python3 -c "import socket; s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect('$MON'); s.sendall(b'$1\n')"
    sleep 0.2
}

type_word() {
    local word=$1
    for ((i = 0; i < ${#word}; i++)); do
        mon "sendkey ${word:i:1}"
    done
}

shutdown() {
    mon "quit"
    wait "$QEMU_PID" 2>/dev/null || true
    QEMU_PID=""
}

echo "--- session 1: type 'hello' ---"
boot
type_word hello
mon "sendkey ret"
sleep 1
shutdown

python3 scripts/extract.py "$SCROLL" | grep -q "hello" || {
    echo "FAIL: 'hello' not on the scroll after session 1"
    exit 1
}

echo "--- session 2: type 'again', expect 'hello' still there ---"
boot
type_word again
sleep 1
shutdown

OUT=$(python3 scripts/extract.py "$SCROLL")
echo "$OUT" | grep -q "hello" || { echo "FAIL: 'hello' lost after reboot"; exit 1; }
echo "$OUT" | grep -q "again" || { echo "FAIL: 'again' not appended"; exit 1; }

echo "E2E PASS"
