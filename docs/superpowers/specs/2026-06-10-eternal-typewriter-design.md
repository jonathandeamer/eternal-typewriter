# The Eternal Typewriter — Design

**Date:** 2026-06-10
**Status:** Approved for planning

## Concept

A from-scratch x86_64 operating system in Rust whose entire purpose is to be a
typewriter. Power on → under a second later you're looking at a page with a
blinking cursor. Every keystroke is permanent ink: no backspace, no delete, no
cursor movement, no menus, no escape. The scroll survives reboots forever.

Nothing like it exists: writerdeck projects (Tinker WriterDeck OS,
writerdeckOS, TypeWriterOS) are stripped Linux distros booting into ordinary
editors with ordinary files. No prior project combines a from-scratch kernel
with an append-only eternal scroll. The name "Eternal Typewriter" is unclaimed
in app stores and on GitHub as of 2026-06-10.

## Behavior

- Printable keys append to the scroll. Enter starts a new line. Long lines wrap.
- No backspace, no delete, no editing of any kind. Typos are fossils.
- When the page fills, text scrolls upward like paper feeding.
- PgUp/PgDn scrolls back through the entire history, read-only. Typing any
  printable key snaps back to the live end of the scroll.
- On each boot, the kernel stamps a dim separator line with the date read from
  the CMOS RTC (e.g. `— 10 June 2026 —`), making the scroll a journal of
  sessions.
- When the scroll disk is full, the typewriter displays a dignified
  end-of-scroll message and accepts no more ink.
- No other interface exists.

## Target

- **v1:** QEMU (x86_64), BIOS or UEFI boot.
- **Later:** a real PC/old laptop. All v1 interface choices (framebuffer via
  bootloader, PS/2 keyboard, ATA disk) are picked to survive that port: BIOSes
  emulate PS/2 for built-in/USB keyboards, SATA disks run in IDE-compatibility
  mode, and the bootloader's framebuffer works on UEFI machines that have no
  VGA text mode.

## Architecture

Five small components, each with one job:

| Component | Job | Key dependency |
|---|---|---|
| Boot | Get from power-on to kernel `main` with a framebuffer and memory map | `bootloader` crate (0.11) |
| Page renderer | Bitmap monospace font, warm-white page, margins, line wrapping, cursor blink (timer-driven) | `noto-sans-mono-bitmap` |
| Keyboard | PS/2 driver on IRQ1, scancode → keystroke decoding | `pc-keyboard` |
| Storage | ATA PIO driver, append-only log on a dedicated scroll disk | — |
| Allocator | Heap for the in-RAM scroll | `linked_list_allocator` |

The scroll is held in RAM for rendering. A lifetime of typing is tens of MB;
not a constraint on any machine this will run on.

The scroll disk is a **separate disk** from the boot disk (second QEMU drive;
a second physical disk/partition later) so the scroll can never collide with
the kernel image.

## Persistence format

Pure append-only log on the raw scroll disk. No filesystem.

- Fixed 512-byte records: magic number, monotonically increasing sequence
  number, payload length, UTF-8 payload bytes.
- On boot, binary-search the disk for the highest valid sequence number, then
  load the scroll into RAM.
- Nothing is ever overwritten except the current tail sector as it fills.
  There is no mutable head-pointer block to corrupt.
- Flush policy: every keystroke is flushed to disk. At human typing speed one
  sector write per keypress is negligible. Power loss costs at most the
  keystroke in flight.

## Getting text out

1. A host-side script reads the scroll disk image and emits plain text
   (covers QEMU now; covers a pulled disk from real hardware later).
2. F12 in the kernel dumps the entire scroll out the serial port (QEMU
   redirects serial to a host file).

## Failure handling

- Kernel panic → full-screen red "the typewriter is broken" page rendering
  the panic message. The scroll is already on disk; prose is never lost.
- Disk write error → retry; on repeated failure, a persistent warning glyph
  in the page margin.

## Testing

- Line wrapping and the log format are written as `no_std`-agnostic pure
  logic, unit-tested on the host with `cargo test`.
- QEMU integration tests (isa-debug-exit device, blog_os pattern) verify
  boot → type → reboot → scroll restored, end to end.
- Dev loop: `cargo run` launches QEMU.

## Milestones

1. **It boots:** kernel boots, page renders, cursor blinks.
2. **It types:** keyboard input appears with wrapping and scrolling (RAM-only).
3. **It's eternal:** scroll disk works; reboot restores every word.
4. **Polish:** boot date stamps, scroll-back reading, panic page, extraction
   script, serial dump.

## Out of scope (v1)

- Backspace or any editing (by design, forever)
- Files, documents, sessions as separate objects — there is one scroll
- Networking, USB drivers, sound
- Proportional/anti-aliased typography (possible v2 polish)
- Real-hardware bring-up (v2)
