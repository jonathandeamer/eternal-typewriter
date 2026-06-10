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
  printable key snaps back to the live end of the scroll **and** inks that
  keystroke there — every keystroke is permanent, including the one that ends
  a reading session.
- On each boot, the kernel stamps a dim separator line with the date read from
  the CMOS RTC, making the scroll a journal of sessions. Separator format:
  `— 10 June 2026 —` (em dash, space, day, full month name, year, space,
  em dash) on its own line, marked in-band so the renderer can dim it after
  reload (see Persistence format).
- When the scroll disk is full, the typewriter displays a dignified
  end-of-scroll message and accepts no more ink.
- No other interface exists.

## Target

- **v1:** QEMU (x86_64), BIOS or UEFI boot.
- **Later:** a real PC/old laptop. All v1 interface choices (framebuffer via
  bootloader, PS/2 keyboard, ATA disk) are picked to survive that port: BIOSes
  emulate PS/2 for built-in/USB keyboards, SATA disks run in IDE-compatibility
  mode, and the bootloader's framebuffer works on UEFI machines that have no
  VGA text mode. One port caveat: rewriting the same tail sector on every
  keystroke is harmless in QEMU and on spinning disks, but on an SSD/SD card
  it concentrates writes on one logical sector; wear leveling mostly absorbs
  this, and it should be revisited during real-hardware bring-up.

## Architecture

Six small components, each with one job:

| Component | Job | Key dependency |
|---|---|---|
| Boot | Get from power-on to kernel `main` with a framebuffer and memory map | `bootloader` crate (0.11) |
| Interrupts | IDT setup, PIC remap (8259), PIT timer tick (cursor blink), IRQ1 dispatch | `x86_64`, `pic8259` |
| Page renderer | Bitmap monospace font, warm-white page, margins, line wrapping, cursor blink (timer-driven) | `noto-sans-mono-bitmap` |
| Keyboard | PS/2 driver on IRQ1, scancode → keystroke decoding | `pc-keyboard` |
| Storage | ATA PIO driver, append-only log on a dedicated scroll disk | — |
| Allocator | Heap for the in-RAM scroll | `linked_list_allocator` |

The scroll is held in RAM for rendering. A lifetime of typing is tens of MB;
not a constraint on any machine this will run on.

ATA PIO moves only a few MB/s, so a grown scroll cannot be fully read before
showing the page without breaking the under-a-second boot promise. Boot
therefore reads only the last screenful of sectors before the cursor appears,
then streams the rest of the history into RAM in the background. Typing is
live immediately; PgUp into history not yet loaded waits on the stream
(in practice the stream finishes long before a human reaches for PgUp).

The scroll disk is a **separate disk** from the boot disk (second QEMU drive;
a second physical disk/partition later) so the scroll can never collide with
the kernel image.

## Page layout

- Request a framebuffer of at least 1024x768 via `BootloaderConfig`
  (`minimum_framebuffer_width`/`_height`). The bootloader may fall back to a
  smaller mode, so the actual resolution, stride, and pixel format are taken
  from the `FrameBufferInfo` it passes — never assumed.
- Font: `noto-sans-mono-bitmap`, `FontWeight::Bold`, `RasterHeight::Size24`.
  Glyph width comes from `get_raster_width(Bold, Size24)` and line height from
  `RasterHeight::Size24.val()` — neither is a hardcoded constant.
- Margins: ~24 px on all sides, so the screen reads like a page.
- The text grid is derived at runtime:
  `columns = (width − 2·margin) / glyph_width`,
  `rows = (height − 2·margin) / line_height`.
  At 1024x768 that is on the order of 80 columns by 30 rows. Line-wrap logic
  takes `columns` as a parameter, so it is testable on the host and immune to
  whatever mode the firmware actually provides.

## Reading the clock

The boot date comes from the CMOS RTC via ports `0x70`/`0x71`:

- Wait until the update-in-progress flag (Status Register A, bit 7) is clear,
  then read; re-read until two consecutive reads match, to avoid a value torn
  by an update cycle.
- Status Register B (bit 2) says whether values are BCD or binary; BCD (the
  common case) must be decoded (`((v >> 4) * 10) + (v & 0x0F)`).
- The two-digit year is anchored to 20xx; the century register is not relied
  on. A garbled date degrades to omitting the separator, never to blocking
  boot.

## Persistence format

Pure append-only log on the raw scroll disk. No filesystem.

- Fixed 512-byte records, one per sector:

  | Field | Size | Notes |
  |---|---|---|
  | `magic` | 4 bytes | ASCII `"ETYP"` (`0x45 0x54 0x59 0x50`) |
  | `sequence_number` | 8 bytes | `u64`, monotonically increasing from 0 |
  | `payload_length` | 2 bytes | `u16`, number of valid payload bytes |
  | `payload_crc32` | 4 bytes | CRC-32 (IEEE) of the valid payload bytes |
  | `payload` | 494 bytes | UTF-8 scroll bytes (newlines are ordinary bytes) |

- The magic doubles as a format version: `"ETYP"` is version 1. If the format
  ever changes, the new format gets a new magic (`"ETY2"`, …) and the reader
  learns both. The scroll is eternal; the format must be too.
- A record is **valid** iff the magic matches, `payload_length ≤ 494`,
  `payload_crc32` matches, **and** `sequence_number` equals the record's LBA.
  The CRC catches torn or rotted payloads that a magic/length check alone
  would load as permanent gibberish; the LBA check stops stale records from
  masquerading as the tail.
- Precondition: a scroll disk starts **zeroed** (a fresh QEMU image is; a
  reused physical disk must be zeroed first). Valid records then always form
  a contiguous prefix from LBA 0, which is what makes binary search sound.
- On boot, binary-search the disk for the highest valid record, then load the
  scroll into RAM.
- Tail-sector overwrite flow: each keystroke appends to the in-RAM tail
  buffer, then rewrites the **same** tail sector in place (same
  `sequence_number`, updated `payload_length`, `payload_crc32`, and
  `payload`). When the payload reaches 494 bytes the sector is sealed and the
  next keystroke starts a fresh sector with `sequence_number + 1`. Sectors
  are sealed only when full — newlines do not seal a sector — so a 50 MB disk
  holds ~48 MB of prose (~102,400 sectors), not 100,000 keystrokes.
- Sectors seal at exactly 494 bytes, so a multi-byte UTF-8 character (the
  em-dash separator is one) may split across two sectors. This is correct by
  design: readers concatenate all payloads **before** decoding UTF-8. Do not
  "fix" it by sealing early.
- Separator lines are part of the scroll, marked in-band: a line beginning
  with the byte `0x1E` (ASCII Record Separator — a C0 control, impossible to
  type) is a boot separator. The renderer draws such lines dim; the
  extraction script and serial dump strip the `0x1E`. Prose lines can never
  be mistaken for separators, even if someone types `— 10 June 2026 —`.
- Nothing else is ever overwritten. There is no mutable head-pointer block to
  corrupt; a torn tail-sector write costs at most the unsealed payload and is
  detected by the validity check on boot.
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
