# The Eternal Typewriter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A from-scratch x86_64 Rust kernel that boots straight into an append-only typewriter whose scroll survives reboots forever.

**Architecture:** A `no_std` kernel crate (boot via `bootloader` 0.11, framebuffer page renderer, PS/2 keyboard on IRQ1, ATA PIO scroll disk, heap allocator) plus a `scroll-core` crate of pure logic (CRC-32, record format, line layout) that is unit-tested on the host. A root builder/runner crate assembles the disk image and launches QEMU.

**Tech Stack:** Rust nightly, `bootloader`/`bootloader_api` 0.11, `x86_64` 0.15, `pic8259` 0.11, `pc-keyboard` 0.7, `noto-sans-mono-bitmap` 0.3, `linked_list_allocator` 0.10, `uart_16550` 0.3, `spin` 0.9, QEMU, Python 3 (host extraction script).

**Spec:** `docs/superpowers/specs/2026-06-10-eternal-typewriter-design.md` — read it before starting any task.

**Prerequisites (one-time, host machine):**

```bash
brew install qemu socat
rustup toolchain install nightly
```

**Milestone map:** M1 "It boots" = Tasks 1–6. M2 "It types" = Tasks 7–8. M3 "It's eternal" = Tasks 9–11. M4 "Polish" = Tasks 12–17.

---

## File structure

```
Cargo.toml                  # workspace + root builder/runner package
build.rs                    # builds BIOS/UEFI disk images from the kernel artifact
src/main.rs                 # runner: creates scroll.img, launches QEMU
rust-toolchain.toml         # nightly + x86_64-unknown-none target
.cargo/config.toml          # enables -Z bindeps
scroll-core/                # pure logic, no_std, host-tested
  src/lib.rs
  src/crc.rs                # CRC-32 (IEEE)
  src/record.rs             # 512-byte record encode/decode
  src/layout.rs             # line wrapping / scroll layout
kernel/                     # the OS itself, no_std + alloc
  src/main.rs               # entry point, main loop, panic handler
  src/serial.rs             # COM1 for debug output and F12 dump
  src/framebuffer.rs        # Renderer: page, glyphs, cursor
  src/gdt.rs                # GDT + TSS (double-fault stack)
  src/interrupts.rs         # IDT, PIC remap, PIT tick, IRQ1
  src/keyboard.rs           # scancode ring buffer
  src/memory.rs             # paging init + frame allocator
  src/allocator.rs          # heap (linked_list_allocator)
  src/ata.rs                # ATA PIO driver (primary channel, slave drive)
  src/rtc.rs                # CMOS RTC boot date
  src/scroll.rs             # Scroll: RAM text + layout + persistence
scripts/
  extract.py                # host-side scroll.img → plain text
  e2e_test.sh               # boot → type → reboot → verify, via QEMU monitor
```

Design rule from the spec: everything in `scroll-core` is pure (no I/O, no statics) so it runs under `cargo test` on the host. The kernel only wires that logic to hardware.

---

### Task 1: Scaffolding — kernel boots in QEMU and paints the screen

**Files:**
- Create: `rust-toolchain.toml`, `.cargo/config.toml`, `Cargo.toml`, `build.rs`, `src/main.rs`, `.gitignore`
- Create: `kernel/Cargo.toml`, `kernel/src/main.rs`, `kernel/src/serial.rs`

- [ ] **Step 1: Write the toolchain and cargo config**

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "nightly"
targets = ["x86_64-unknown-none"]
```

`.cargo/config.toml`:

```toml
[unstable]
bindeps = true
```

`.gitignore`:

```
/target
scroll.img
serial.log
```

- [ ] **Step 2: Write the workspace / builder crate**

`Cargo.toml` (repo root):

```toml
[package]
name = "eternal-typewriter"
version = "0.1.0"
edition = "2021"

[workspace]
members = ["kernel", "scroll-core"]

[build-dependencies]
bootloader = "0.11"
kernel = { path = "kernel", artifact = "bin", target = "x86_64-unknown-none" }
```

`build.rs`:

```rust
use bootloader::DiskImageBuilder;
use std::{env, path::PathBuf};

fn main() {
    let kernel = PathBuf::from(env::var("CARGO_BIN_FILE_KERNEL").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let builder = DiskImageBuilder::new(kernel);

    let bios_path = out_dir.join("bios.img");
    builder.create_bios_image(&bios_path).unwrap();
    println!("cargo:rustc-env=BIOS_IMAGE={}", bios_path.display());

    let uefi_path = out_dir.join("uefi.img");
    builder.create_uefi_image(&uefi_path).unwrap();
    println!("cargo:rustc-env=UEFI_IMAGE={}", uefi_path.display());
}
```

`src/main.rs` (the QEMU runner):

```rust
use std::{env, fs, process::Command};

const SCROLL_IMG: &str = "scroll.img";
const SCROLL_SIZE: u64 = 50 * 1024 * 1024; // zeroed by creation — spec precondition

fn main() {
    let bios = env!("BIOS_IMAGE");
    if env::args().any(|a| a == "--print-bios-image") {
        println!("{bios}");
        return;
    }
    if fs::metadata(SCROLL_IMG).is_err() {
        let f = fs::File::create(SCROLL_IMG).unwrap();
        f.set_len(SCROLL_SIZE).unwrap();
    }
    let status = Command::new("qemu-system-x86_64")
        .args(["-m", "512M"])
        .args(["-drive", &format!("format=raw,file={bios}")])
        .args(["-drive", &format!("format=raw,file={SCROLL_IMG},if=ide,index=1")])
        .args(["-serial", "stdio"])
        .status()
        .expect("qemu-system-x86_64 not found — brew install qemu");
    std::process::exit(status.code().unwrap_or(1));
}
```

Boot disk is IDE primary **master** (index 0), the scroll disk primary **slave** (index 1). The ATA driver in Task 9 talks only to the slave, so the scroll can never collide with the kernel image.

- [ ] **Step 3: Write the minimal kernel**

`kernel/Cargo.toml`:

```toml
[package]
name = "kernel"
version = "0.1.0"
edition = "2021"

[dependencies]
bootloader_api = "0.11"
x86_64 = "0.15"
spin = "0.9"
uart_16550 = "0.3"
scroll-core = { path = "../scroll-core" }
```

(`scroll-core` doesn't exist until Task 2 — create an empty placeholder now so this compiles: `cargo new scroll-core --lib`, then replace `scroll-core/src/lib.rs` with `#![cfg_attr(not(test), no_std)]` as its only line, and add `edition = "2021"` is already set by cargo new.)

`kernel/src/serial.rs`:

```rust
use spin::{Lazy, Mutex};
use uart_16550::SerialPort;

pub static SERIAL1: Lazy<Mutex<SerialPort>> = Lazy::new(|| {
    let mut port = unsafe { SerialPort::new(0x3F8) };
    port.init();
    Mutex::new(port)
});

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    x86_64::instructions::interrupts::without_interrupts(|| {
        SERIAL1.lock().write_fmt(args).ok();
    });
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => ($crate::serial::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => ($crate::serial_print!("{}\n", format_args!($($arg)*)));
}
```

`kernel/src/main.rs`:

```rust
#![no_std]
#![no_main]

mod serial;

use bootloader_api::{
    config::{BootloaderConfig, Mapping},
    entry_point, BootInfo,
};
use core::panic::PanicInfo;

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config.frame_buffer.minimum_framebuffer_width = Some(1024);
    config.frame_buffer.minimum_framebuffer_height = Some(768);
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    serial_println!("eternal typewriter: boot");
    let fb = boot_info.framebuffer.as_mut().expect("no framebuffer");
    let info = fb.info();
    serial_println!("framebuffer {}x{} {:?}", info.width, info.height, info.pixel_format);
    for byte in fb.buffer_mut() {
        *byte = 0xE8; // rough light grey in any pixel format — proves we own the screen
    }
    loop {
        x86_64::instructions::hlt();
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("PANIC: {}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
```

- [ ] **Step 4: Build and run**

Run: `cargo run`
Expected: QEMU window opens, screen turns light grey within ~1s, terminal shows `eternal typewriter: boot` and a `framebuffer 1024x768 …` line. Close QEMU window to exit. A 50 MB `scroll.img` now exists.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: kernel boots in QEMU and paints the framebuffer"
```

---

### Task 2: scroll-core — CRC-32

**Files:**
- Create: `scroll-core/Cargo.toml`, `scroll-core/src/lib.rs`, `scroll-core/src/crc.rs`

- [ ] **Step 1: Crate skeleton**

`scroll-core/Cargo.toml`:

```toml
[package]
name = "scroll-core"
version = "0.1.0"
edition = "2021"

[dependencies]
```

`scroll-core/src/lib.rs`:

```rust
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod crc;
pub mod layout;
pub mod record;

/// First byte of a boot-separator line. ASCII Record Separator: a C0
/// control character that can never be typed, so prose can't fake it.
pub const SEPARATOR_MARKER: u8 = 0x1E;
pub const SEPARATOR_MARKER_CHAR: char = '\u{1E}';
```

Create empty `scroll-core/src/layout.rs` and `scroll-core/src/record.rs` files so the module declarations compile (they're filled in Tasks 3–4).

- [ ] **Step 2: Write the failing test**

`scroll-core/src/crc.rs`:

```rust
/// CRC-32 (IEEE 802.3, the zlib polynomial). Bitwise — speed is irrelevant
/// at one 494-byte payload per keystroke, and zero tables keeps it obvious.
pub fn crc32(_data: &[u8]) -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_standard_check_value() {
        // The canonical CRC-32 test vector; zlib.crc32 in the Python
        // extraction script must agree with this.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn empty_input() {
        assert_eq!(crc32(b""), 0);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p scroll-core crc`
Expected: FAIL — `matches_the_standard_check_value` asserts `0 == 0xCBF43926`.

- [ ] **Step 4: Implement**

Replace the `crc32` body in `scroll-core/src/crc.rs`:

```rust
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p scroll-core crc`
Expected: 2 passed.

- [ ] **Step 6: Commit**

```bash
git add scroll-core
git commit -m "feat: scroll-core crate with CRC-32"
```

---

### Task 3: scroll-core — the 512-byte record format

**Files:**
- Modify: `scroll-core/src/record.rs`

The on-disk format, exactly as specced: magic `"ETYP"` (4) + sequence_number `u64` LE (8) + payload_length `u16` LE (2) + payload_crc32 `u32` LE (4) + payload (494) = 512. A record is valid iff magic matches, length ≤ 494, CRC matches, **and** sequence_number == LBA.

- [ ] **Step 1: Write the failing tests**

`scroll-core/src/record.rs`:

```rust
use crate::crc::crc32;

pub const SECTOR_SIZE: usize = 512;
pub const PAYLOAD_CAPACITY: usize = 494;
pub const MAGIC: [u8; 4] = *b"ETYP"; // doubles as format version 1

pub fn encode(_sequence_number: u64, _payload: &[u8]) -> [u8; SECTOR_SIZE] {
    [0u8; SECTOR_SIZE]
}

/// Returns the payload if the sector holds a valid record for this LBA.
pub fn decode(_sector: &[u8; SECTOR_SIZE], _lba: u64) -> Option<&[u8]> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let sector = encode(7, b"ink is forever");
        assert_eq!(decode(&sector, 7), Some(&b"ink is forever"[..]));
    }

    #[test]
    fn empty_payload_roundtrip() {
        let sector = encode(0, b"");
        assert_eq!(decode(&sector, 0), Some(&b""[..]));
    }

    #[test]
    fn full_payload_roundtrip() {
        let payload = [b'x'; PAYLOAD_CAPACITY];
        let sector = encode(3, &payload);
        assert_eq!(decode(&sector, 3), Some(&payload[..]));
    }

    #[test]
    fn zeroed_sector_is_invalid() {
        // A fresh scroll disk is all zeros; every sector must decode as None.
        assert_eq!(decode(&[0u8; SECTOR_SIZE], 0), None);
    }

    #[test]
    fn stale_record_at_wrong_lba_is_invalid() {
        // The LBA check stops stale records masquerading as the tail.
        let sector = encode(7, b"old prose");
        assert_eq!(decode(&sector, 8), None);
    }

    #[test]
    fn corrupted_payload_is_invalid() {
        // The CRC catches torn/rotted payloads behind an intact header.
        let mut sector = encode(2, b"about to rot");
        sector[20] ^= 0x01;
        assert_eq!(decode(&sector, 2), None);
    }

    #[test]
    fn oversized_length_is_invalid() {
        let mut sector = encode(0, b"hi");
        sector[12..14].copy_from_slice(&(PAYLOAD_CAPACITY as u16 + 1).to_le_bytes());
        assert_eq!(decode(&sector, 0), None);
    }

    #[test]
    #[should_panic]
    fn encode_rejects_oversized_payload() {
        encode(0, &[0u8; PAYLOAD_CAPACITY + 1]);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p scroll-core record`
Expected: FAIL — `roundtrip`, `empty_payload_roundtrip`, `full_payload_roundtrip`, and `encode_rejects_oversized_payload` fail; the invalid-input tests pass vacuously (stub returns `None`).

- [ ] **Step 3: Implement**

Replace `encode` and `decode` bodies:

```rust
pub fn encode(sequence_number: u64, payload: &[u8]) -> [u8; SECTOR_SIZE] {
    assert!(payload.len() <= PAYLOAD_CAPACITY);
    let mut sector = [0u8; SECTOR_SIZE];
    sector[0..4].copy_from_slice(&MAGIC);
    sector[4..12].copy_from_slice(&sequence_number.to_le_bytes());
    sector[12..14].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    sector[14..18].copy_from_slice(&crc32(payload).to_le_bytes());
    sector[18..18 + payload.len()].copy_from_slice(payload);
    sector
}

pub fn decode(sector: &[u8; SECTOR_SIZE], lba: u64) -> Option<&[u8]> {
    if sector[0..4] != MAGIC {
        return None;
    }
    let sequence_number = u64::from_le_bytes(sector[4..12].try_into().unwrap());
    if sequence_number != lba {
        return None;
    }
    let length = u16::from_le_bytes(sector[12..14].try_into().unwrap()) as usize;
    if length > PAYLOAD_CAPACITY {
        return None;
    }
    let crc = u32::from_le_bytes(sector[14..18].try_into().unwrap());
    let payload = &sector[18..18 + length];
    if crc32(payload) != crc {
        return None;
    }
    Some(payload)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p scroll-core record`
Expected: 8 passed.

- [ ] **Step 5: Commit**

```bash
git add scroll-core/src/record.rs
git commit -m "feat: append-only log record format with CRC and LBA validation"
```

---

### Task 4: scroll-core — line layout (wrapping, newlines, separators)

**Files:**
- Modify: `scroll-core/src/layout.rs`

`Layout` maps the scroll text to display lines: incremental (one `append` per typed char, no re-scan), rebuildable from a full string on boot, and parameterised by `columns` so it's host-testable and immune to whatever framebuffer mode the firmware provides. Wrapping is per-character (a typewriter doesn't reflow words). Lines are byte ranges into the scroll text; a line whose first byte is the `0x1E` marker is a separator (the marker byte is excluded from the range so rendering never sees it).

- [ ] **Step 1: Write the failing tests**

`scroll-core/src/layout.rs`:

```rust
use crate::SEPARATOR_MARKER_CHAR;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    /// Byte range into the scroll text. Excludes the trailing '\n' of a
    /// newline-terminated line and the leading 0x1E of a separator line.
    pub start: usize,
    pub end: usize,
    pub is_separator: bool,
}

pub struct Layout {
    columns: usize,
    lines: Vec<LineSpan>,
    current_columns: usize, // chars on the last (open) line
}

impl Layout {
    pub fn new(columns: usize) -> Self {
        unimplemented!()
    }

    /// Rebuild from the whole scroll (used at boot).
    pub fn from_text(text: &str, columns: usize) -> Self {
        let mut layout = Self::new(columns);
        let mut offset = 0;
        for ch in text.chars() {
            layout.append(offset, ch);
            offset += ch.len_utf8();
        }
        layout
    }

    /// Record one appended char. `byte_offset` is where its bytes start in
    /// the scroll text.
    pub fn append(&mut self, _byte_offset: usize, _ch: char) {
        unimplemented!()
    }

    pub fn lines(&self) -> &[LineSpan] {
        &self.lines
    }

    /// Where the cursor sits: (line index, column).
    pub fn cursor(&self) -> (usize, usize) {
        (self.lines.len() - 1, self.current_columns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &str, columns: usize) -> Vec<(&str, bool)> {
        Layout::from_text(text, columns)
            .lines()
            .iter()
            .map(|l| (&text[l.start..l.end], l.is_separator))
            .collect()
    }

    #[test]
    fn empty_scroll_is_one_empty_line() {
        assert_eq!(spans("", 10), [("", false)]);
    }

    #[test]
    fn long_lines_wrap_at_columns() {
        assert_eq!(
            spans("abcdefgh", 3),
            [("abc", false), ("def", false), ("gh", false)]
        );
    }

    #[test]
    fn newline_starts_a_new_line() {
        assert_eq!(spans("ab\ncd", 10), [("ab", false), ("cd", false)]);
    }

    #[test]
    fn trailing_newline_leaves_an_open_empty_line() {
        assert_eq!(spans("ab\n", 10), [("ab", false), ("", false)]);
    }

    #[test]
    fn exactly_full_line_then_newline_does_not_double_break() {
        assert_eq!(spans("abc\nd", 3), [("abc", false), ("d", false)]);
    }

    #[test]
    fn separator_marker_dims_the_line_and_is_excluded() {
        let text = "hi\n\u{1E}— 10 June 2026 —\nmore";
        assert_eq!(
            spans(text, 40),
            [("hi", false), ("— 10 June 2026 —", true), ("more", false)]
        );
    }

    #[test]
    fn multibyte_chars_count_as_one_column() {
        // Em dash is 3 bytes but one glyph cell.
        assert_eq!(spans("——a", 2), [("——", false), ("a", false)]);
    }

    #[test]
    fn cursor_tracks_line_and_column() {
        let layout = Layout::from_text("ab\ncd", 10);
        assert_eq!(layout.cursor(), (1, 2));
    }

    #[test]
    fn cursor_after_wrap_is_on_the_new_line() {
        let layout = Layout::from_text("abcd", 3);
        assert_eq!(layout.cursor(), (1, 1));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p scroll-core layout`
Expected: FAIL — every test panics on `unimplemented!()`.

- [ ] **Step 3: Implement**

Replace the `new` and `append` bodies:

```rust
    pub fn new(columns: usize) -> Self {
        let mut lines = Vec::new();
        lines.push(LineSpan { start: 0, end: 0, is_separator: false });
        Layout { columns, lines, current_columns: 0 }
    }

    pub fn append(&mut self, byte_offset: usize, ch: char) {
        if ch == '\n' {
            let after = byte_offset + 1;
            self.lines.push(LineSpan { start: after, end: after, is_separator: false });
            self.current_columns = 0;
            return;
        }
        if ch == SEPARATOR_MARKER_CHAR {
            // Marker is always the first byte of its line (the writer
            // guarantees a preceding '\n'); flag the line and skip the byte.
            let line = self.lines.last_mut().unwrap();
            line.is_separator = true;
            line.start = byte_offset + ch.len_utf8();
            line.end = line.start;
            return;
        }
        if self.current_columns == self.columns {
            self.lines.push(LineSpan { start: byte_offset, end: byte_offset, is_separator: false });
            self.current_columns = 0;
        }
        let line = self.lines.last_mut().unwrap();
        line.end = byte_offset + ch.len_utf8();
        self.current_columns += 1;
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p scroll-core layout`
Expected: 9 passed. Also run the full suite once: `cargo test -p scroll-core` — all green.

- [ ] **Step 5: Commit**

```bash
git add scroll-core/src/layout.rs
git commit -m "feat: incremental line layout with wrapping and separator lines"
```

---

### Task 5: Page renderer

**Files:**
- Create: `kernel/src/framebuffer.rs`
- Modify: `kernel/Cargo.toml`, `kernel/src/main.rs`

**Note on font features:** `noto-sans-mono-bitmap` gates weights, sizes, and unicode ranges behind features. If a feature name below doesn't exist in the version that resolves, `cargo build` will say so — the names track the crate's README. The em dash (U+2014) lives in the general-punctuation range; `draw_char` falls back to `'-'` for any glyph the font lacks, so a missing range degrades, never panics.

- [ ] **Step 1: Add the font dependency**

In `kernel/Cargo.toml` under `[dependencies]` add:

```toml
noto-sans-mono-bitmap = { version = "0.3", default-features = false, features = [
    "bold",
    "size_24",
    "unicode-basic-latin",
    "unicode-latin-1-supplement",
    "unicode-general-punctuation",
] }
```

- [ ] **Step 2: Write the renderer**

`kernel/src/framebuffer.rs`:

```rust
use bootloader_api::info::{FrameBuffer, FrameBufferInfo, PixelFormat};
use noto_sans_mono_bitmap::{
    get_raster, get_raster_width, FontWeight, RasterHeight, RasterizedChar,
};

pub type Rgb = (u8, u8, u8);

pub const PAPER: Rgb = (245, 240, 230); // warm white
pub const INK: Rgb = (40, 38, 34);
pub const DIM: Rgb = (165, 155, 138); // separator lines
pub const ALERT: Rgb = (170, 40, 40); // margin warning glyph

const WEIGHT: FontWeight = FontWeight::Bold;
const HEIGHT: RasterHeight = RasterHeight::Size24;
const MARGIN: usize = 24;

fn raster(ch: char) -> RasterizedChar {
    // Fall back to '-' for glyphs outside the compiled-in ranges; basic
    // latin is always compiled in, so the unwrap cannot fail.
    get_raster(ch, WEIGHT, HEIGHT)
        .unwrap_or_else(|| get_raster('-', WEIGHT, HEIGHT).unwrap())
}

pub struct Renderer {
    buffer: &'static mut [u8],
    info: FrameBufferInfo,
    glyph_width: usize,
    line_height: usize,
    pub columns: usize,
    pub rows: usize,
}

impl Renderer {
    pub fn new(framebuffer: &'static mut FrameBuffer) -> Self {
        let info = framebuffer.info();
        // Spec: glyph metrics come from the font API, the grid from the
        // actual mode the bootloader gave us — nothing is assumed.
        let glyph_width = get_raster_width(WEIGHT, HEIGHT);
        let line_height = HEIGHT.val();
        let columns = (info.width - 2 * MARGIN) / glyph_width;
        let rows = (info.height - 2 * MARGIN) / line_height;
        let mut renderer = Renderer {
            buffer: framebuffer.buffer_mut(),
            info,
            glyph_width,
            line_height,
            columns,
            rows,
        };
        renderer.fill(PAPER);
        renderer
    }

    /// Raw parts for the panic handler (Task 15).
    pub fn raw_parts(&mut self) -> (*mut u8, usize, FrameBufferInfo) {
        (self.buffer.as_mut_ptr(), self.buffer.len(), self.info)
    }

    pub unsafe fn from_raw_parts(ptr: *mut u8, len: usize, info: FrameBufferInfo) -> Self {
        let buffer = core::slice::from_raw_parts_mut(ptr, len);
        let glyph_width = get_raster_width(WEIGHT, HEIGHT);
        let line_height = HEIGHT.val();
        let columns = (info.width - 2 * MARGIN) / glyph_width;
        let rows = (info.height - 2 * MARGIN) / line_height;
        Renderer { buffer, info, glyph_width, line_height, columns, rows }
    }

    fn put_pixel(&mut self, x: usize, y: usize, (r, g, b): Rgb) {
        if x >= self.info.width || y >= self.info.height {
            return;
        }
        let offset = (y * self.info.stride + x) * self.info.bytes_per_pixel;
        let pixel = &mut self.buffer[offset..offset + self.info.bytes_per_pixel];
        match self.info.pixel_format {
            PixelFormat::Rgb => {
                pixel[0] = r;
                pixel[1] = g;
                pixel[2] = b;
            }
            PixelFormat::Bgr => {
                pixel[0] = b;
                pixel[1] = g;
                pixel[2] = r;
            }
            _ => {
                let grey = ((r as u16 + g as u16 + b as u16) / 3) as u8;
                pixel[0] = grey;
            }
        }
    }

    pub fn fill(&mut self, color: Rgb) {
        for y in 0..self.info.height {
            for x in 0..self.info.width {
                self.put_pixel(x, y, color);
            }
        }
    }

    fn cell_origin(&self, row: usize, col: usize) -> (usize, usize) {
        (MARGIN + col * self.glyph_width, MARGIN + row * self.line_height)
    }

    fn fill_cell(&mut self, row: usize, col: usize, color: Rgb) {
        let (x0, y0) = self.cell_origin(row, col);
        for dy in 0..self.line_height {
            for dx in 0..self.glyph_width {
                self.put_pixel(x0 + dx, y0 + dy, color);
            }
        }
    }

    pub fn draw_char(&mut self, row: usize, col: usize, ch: char, ink: Rgb) {
        if row >= self.rows || col >= self.columns {
            return;
        }
        let glyph = raster(ch);
        let (x0, y0) = self.cell_origin(row, col);
        for (dy, raster_row) in glyph.raster().iter().enumerate() {
            for (dx, &alpha) in raster_row.iter().enumerate() {
                let color = (
                    mix(PAPER.0, ink.0, alpha),
                    mix(PAPER.1, ink.1, alpha),
                    mix(PAPER.2, ink.2, alpha),
                );
                self.put_pixel(x0 + dx, y0 + dy, color);
            }
        }
    }

    pub fn draw_cursor(&mut self, row: usize, col: usize, on: bool) {
        if row >= self.rows || col >= self.columns {
            return;
        }
        self.fill_cell(row, col, if on { INK } else { PAPER });
    }

    /// Persistent disk-trouble warning in the top-right margin (Task 14).
    pub fn draw_warning_glyph(&mut self) {
        let glyph = raster('!');
        let x0 = self.info.width - MARGIN + 2;
        for (dy, raster_row) in glyph.raster().iter().enumerate() {
            for (dx, &alpha) in raster_row.iter().enumerate() {
                let color = (
                    mix(PAPER.0, ALERT.0, alpha),
                    mix(PAPER.1, ALERT.1, alpha),
                    mix(PAPER.2, ALERT.2, alpha),
                );
                self.put_pixel(x0 + dx, 4 + dy, color);
            }
        }
    }
}

/// Linear blend of paper and ink by glyph coverage.
fn mix(paper: u8, ink: u8, alpha: u8) -> u8 {
    let p = paper as i32;
    let i = ink as i32;
    (p + (i - p) * alpha as i32 / 255) as u8
}
```

Then delete the leftover `blend` closure inside `draw_char` (it was scaffolding while writing `mix`) — final `draw_char` inner loop is just:

```rust
            for (dx, &alpha) in raster_row.iter().enumerate() {
                let color = (
                    mix(PAPER.0, ink.0, alpha),
                    mix(PAPER.1, ink.1, alpha),
                    mix(PAPER.2, ink.2, alpha),
                );
                self.put_pixel(x0 + dx, y0 + dy, color);
            }
```

- [ ] **Step 3: Wire it into kernel_main**

In `kernel/src/main.rs`, add `mod framebuffer;` and replace the body of `kernel_main` after the serial prints:

```rust
    let fb = boot_info.framebuffer.as_mut().expect("no framebuffer");
    let mut renderer = framebuffer::Renderer::new(fb);
    serial_println!("grid {}x{}", renderer.columns, renderer.rows);
    for (col, ch) in "the eternal typewriter".chars().enumerate() {
        renderer.draw_char(0, col, ch, framebuffer::INK);
    }
    renderer.draw_cursor(1, 0, true);
    loop {
        x86_64::instructions::hlt();
    }
```

- [ ] **Step 4: Build and run**

Run: `cargo run`
Expected: warm-white page with ~24 px margins, "the eternal typewriter" in dark bold monospace on the first line, a solid block cursor on the second line. Serial shows a grid around `60x30` (24 px Noto bold glyphs are ~13–16 px wide; the exact count comes from `get_raster_width` — anything in the 60–80 column range is correct).

- [ ] **Step 5: Commit**

```bash
git add kernel
git commit -m "feat: page renderer with bitmap font, margins, and cursor"
```

---

### Task 6: Interrupts — GDT, IDT, PIC, timer tick, blinking cursor

**Files:**
- Create: `kernel/src/gdt.rs`, `kernel/src/interrupts.rs`
- Modify: `kernel/Cargo.toml`, `kernel/src/main.rs`

- [ ] **Step 1: Add dependencies and feature gate**

In `kernel/Cargo.toml` add `pic8259 = "0.11"` under `[dependencies]`. At the top of `kernel/src/main.rs` (line 1–2 area, with the other crate attributes) add:

```rust
#![feature(abi_x86_interrupt)]
```

- [ ] **Step 2: Write the GDT (double-fault stack)**

`kernel/src/gdt.rs`:

```rust
use spin::Lazy;
use x86_64::instructions::tables::load_tss;
use x86_64::registers::segmentation::{Segment, CS};
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

static TSS: Lazy<TaskStateSegment> = Lazy::new(|| {
    let mut tss = TaskStateSegment::new();
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
        const STACK_SIZE: usize = 4096 * 5;
        static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
        let start = VirtAddr::from_ptr(&raw const STACK);
        start + STACK_SIZE as u64
    };
    tss
});

static GDT: Lazy<(GlobalDescriptorTable, SegmentSelector, SegmentSelector)> = Lazy::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    let code = gdt.append(Descriptor::kernel_code_segment());
    let tss_sel = gdt.append(Descriptor::tss_segment(&TSS));
    (gdt, code, tss_sel)
});

pub fn init() {
    GDT.0.load();
    unsafe {
        CS::set_reg(GDT.1);
        load_tss(GDT.2);
    }
}
```

- [ ] **Step 3: Write the IDT/PIC/timer**

`kernel/src/interrupts.rs`:

```rust
use core::sync::atomic::{AtomicU64, Ordering};
use pic8259::ChainedPics;
use spin::{Lazy, Mutex};
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
const TIMER_VECTOR: u8 = PIC_1_OFFSET; // IRQ0
const KEYBOARD_VECTOR: u8 = PIC_1_OFFSET + 1; // IRQ1

static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

/// PIT ticks since boot, default 18.2 Hz. The cursor blinks off this.
pub static TICKS: AtomicU64 = AtomicU64::new(0);

static IDT: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut idt = InterruptDescriptorTable::new();
    unsafe {
        idt.double_fault
            .set_handler_fn(double_fault_handler)
            .set_stack_index(crate::gdt::DOUBLE_FAULT_IST_INDEX);
    }
    idt[TIMER_VECTOR].set_handler_fn(timer_handler);
    idt[KEYBOARD_VECTOR].set_handler_fn(keyboard_handler);
    idt
});

pub fn init() {
    crate::gdt::init();
    IDT.load();
    unsafe {
        let mut pics = PICS.lock();
        pics.initialize();
        // Unmask only IRQ0 (timer) and IRQ1 (keyboard).
        pics.write_masks(0b1111_1100, 0b1111_1111);
    }
    x86_64::instructions::interrupts::enable();
}

extern "x86-interrupt" fn timer_handler(_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    unsafe { PICS.lock().notify_end_of_interrupt(TIMER_VECTOR) };
}

extern "x86-interrupt" fn keyboard_handler(_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;
    let scancode: u8 = unsafe { Port::new(0x60).read() };
    crate::keyboard::push(scancode);
    unsafe { PICS.lock().notify_end_of_interrupt(KEYBOARD_VECTOR) };
}

extern "x86-interrupt" fn double_fault_handler(frame: InterruptStackFrame, code: u64) -> ! {
    panic!("DOUBLE FAULT ({}): {:#?}", code, frame);
}
```

- [ ] **Step 4: Stub the scancode queue**

`kernel/src/keyboard.rs` (filled out properly in Task 8 — for now the handler needs the symbol):

```rust
pub fn push(_scancode: u8) {}

pub fn pop() -> Option<u8> {
    None
}
```

- [ ] **Step 5: Blink the cursor from the tick counter**

In `kernel/src/main.rs` add `mod gdt; mod interrupts; mod keyboard;` and replace `kernel_main`'s tail (everything from `renderer.draw_cursor…` down) with:

```rust
    interrupts::init();
    let mut cursor_on = true;
    let mut last_toggle = 0u64;
    renderer.draw_cursor(1, 0, cursor_on);
    loop {
        let ticks = interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
        if ticks.wrapping_sub(last_toggle) >= 9 {
            // 9 ticks at 18.2 Hz ≈ a half-second blink phase
            cursor_on = !cursor_on;
            last_toggle = ticks;
            renderer.draw_cursor(1, 0, cursor_on);
        }
        x86_64::instructions::hlt();
    }
```

- [ ] **Step 6: Build and run**

Run: `cargo run`
Expected: same page as Task 5, but the cursor now blinks roughly twice a second. **Milestone 1 complete: it boots, page renders, cursor blinks.**

- [ ] **Step 7: Commit**

```bash
git add kernel
git commit -m "feat: IDT/PIC/PIT interrupts and a blinking cursor (milestone 1)"
```

---

### Task 7: Heap allocator

**Files:**
- Create: `kernel/src/memory.rs`, `kernel/src/allocator.rs`
- Modify: `kernel/Cargo.toml`, `kernel/src/main.rs`

- [ ] **Step 1: Add dependency**

In `kernel/Cargo.toml` add `linked_list_allocator = "0.10"` under `[dependencies]`.

- [ ] **Step 2: Paging and frame allocator**

`kernel/src/memory.rs`:

```rust
use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use x86_64::structures::paging::{
    FrameAllocator, OffsetPageTable, PageTable, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

/// Safety: physical memory must be fully mapped at `physical_memory_offset`
/// (the bootloader config requests this) and this must be called once.
pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    let (frame, _) = x86_64::registers::control::Cr3::read();
    let virt = physical_memory_offset + frame.start_address().as_u64();
    let table: *mut PageTable = virt.as_mut_ptr();
    OffsetPageTable::new(&mut *table, physical_memory_offset)
}

pub struct BootInfoFrameAllocator {
    memory_regions: &'static MemoryRegions,
    next: usize,
}

impl BootInfoFrameAllocator {
    /// Safety: all `Usable` regions must really be unused.
    pub unsafe fn init(memory_regions: &'static MemoryRegions) -> Self {
        BootInfoFrameAllocator { memory_regions, next: 0 }
    }

    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> + '_ {
        self.memory_regions
            .iter()
            .filter(|r| r.kind == MemoryRegionKind::Usable)
            .map(|r| r.start..r.end)
            .flat_map(|r| r.step_by(4096))
            .map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}
```

- [ ] **Step 3: Heap init**

`kernel/src/allocator.rs`:

```rust
use linked_list_allocator::LockedHeap;
use x86_64::structures::paging::{
    mapper::MapToError, FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB,
};
use x86_64::VirtAddr;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub const HEAP_START: u64 = 0x4444_4444_0000;
/// The whole scroll lives in RAM. A lifetime of prose is tens of MB;
/// 256 MiB leaves room for the layout's line table on top.
pub const HEAP_SIZE: u64 = 256 * 1024 * 1024;

pub fn init_heap(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), MapToError<Size4KiB>> {
    let page_range = {
        let start = VirtAddr::new(HEAP_START);
        let end = start + HEAP_SIZE - 1u64;
        Page::range_inclusive(Page::containing_address(start), Page::containing_address(end))
    };
    for page in page_range {
        let frame = frame_allocator
            .allocate_frame()
            .ok_or(MapToError::FrameAllocationFailed)?;
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        unsafe { mapper.map_to(page, frame, flags, frame_allocator)?.flush() };
    }
    unsafe {
        ALLOCATOR.lock().init(HEAP_START as *mut u8, HEAP_SIZE as usize);
    }
    Ok(())
}
```

- [ ] **Step 4: Wire into kernel_main and smoke-test**

In `kernel/src/main.rs`: add `extern crate alloc;` under the crate attributes, add `mod allocator; mod memory;`, and insert at the **top** of `kernel_main` (before the framebuffer code):

```rust
    let physical_memory_offset = VirtAddr::new(
        boot_info.physical_memory_offset.into_option().expect("no physical memory mapping"),
    );
    let mut mapper = unsafe { memory::init(physical_memory_offset) };
    let mut frame_allocator =
        unsafe { memory::BootInfoFrameAllocator::init(&boot_info.memory_regions) };
    allocator::init_heap(&mut mapper, &mut frame_allocator).expect("heap init failed");
    {
        let mut probe = alloc::vec::Vec::new();
        for i in 0..100_000u64 {
            probe.push(i);
        }
        serial_println!("heap ok: vec of {} elements", probe.len());
    }
```

Add `use x86_64::VirtAddr;` to the imports.

- [ ] **Step 5: Build and run**

Run: `cargo run`
Expected: boots as before; serial shows `heap ok: vec of 100000 elements` before the framebuffer lines. Boot may pause ~a second while 256 MiB of heap pages map.

- [ ] **Step 6: Commit**

```bash
git add kernel
git commit -m "feat: paging and 256 MiB kernel heap"
```

---

### Task 8: Keyboard — typing onto the page (RAM-only)

**Files:**
- Modify: `kernel/Cargo.toml`, `kernel/src/keyboard.rs`, `kernel/src/main.rs`

- [ ] **Step 1: Add dependency**

In `kernel/Cargo.toml` add `pc-keyboard = "0.7"` under `[dependencies]`.

- [ ] **Step 2: Real scancode ring buffer**

Replace `kernel/src/keyboard.rs`:

```rust
use spin::Mutex;

/// Fixed ring so the IRQ handler never allocates. 64 pending scancodes is
/// far beyond human typing speed; overflow drops the oldest-unread input.
struct Ring {
    buf: [u8; 64],
    head: usize,
    len: usize,
}

static QUEUE: Mutex<Ring> = Mutex::new(Ring { buf: [0; 64], head: 0, len: 0 });

/// Called from the IRQ1 handler (interrupts already disabled there).
pub fn push(scancode: u8) {
    let mut q = QUEUE.lock();
    if q.len < q.buf.len() {
        let tail = (q.head + q.len) % q.buf.len();
        q.buf[tail] = scancode;
        q.len += 1;
    }
}

/// Called from the main loop. Disables interrupts around the lock so the
/// IRQ handler can never deadlock against us.
pub fn pop() -> Option<u8> {
    x86_64::instructions::interrupts::without_interrupts(|| {
        let mut q = QUEUE.lock();
        if q.len == 0 {
            return None;
        }
        let scancode = q.buf[q.head];
        q.head = (q.head + 1) % q.buf.len();
        q.len -= 1;
        Some(scancode)
    })
}
```

- [ ] **Step 3: Type into a RAM scroll**

Replace `kernel_main`'s tail in `kernel/src/main.rs` (from the `"the eternal typewriter"` demo text through the end of the loop) with:

```rust
    interrupts::init();

    let mut text = alloc::string::String::new();
    let mut layout = scroll_core::layout::Layout::new(renderer.columns);
    let mut decoder = pc_keyboard::Keyboard::new(
        pc_keyboard::ScancodeSet1::new(),
        pc_keyboard::layouts::Us104Key,
        pc_keyboard::HandleControl::Ignore,
    );
    let mut cursor_on = true;
    let mut last_toggle = 0u64;
    render(&mut renderer, &text, &layout, cursor_on);

    loop {
        let mut dirty = false;
        while let Some(scancode) = keyboard::pop() {
            if let Ok(Some(event)) = decoder.add_byte(scancode) {
                if let Some(key) = decoder.process_keyevent(event) {
                    if let Some(ch) = inkable(key) {
                        let offset = text.len();
                        text.push(ch);
                        layout.append(offset, ch);
                        dirty = true;
                    }
                }
            }
        }
        let ticks = interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
        if ticks.wrapping_sub(last_toggle) >= 9 {
            cursor_on = !cursor_on;
            last_toggle = ticks;
            dirty = true;
        }
        if dirty {
            render(&mut renderer, &text, &layout, cursor_on);
        }
        x86_64::instructions::hlt();
    }
```

Add these two functions at the bottom of `kernel/src/main.rs`:

```rust
/// The typewriter's whole input policy: printable chars and Enter make ink;
/// everything else (backspace, escape, control chars) does not exist.
fn inkable(key: pc_keyboard::DecodedKey) -> Option<char> {
    match key {
        pc_keyboard::DecodedKey::Unicode('\n') | pc_keyboard::DecodedKey::Unicode('\r') => {
            Some('\n')
        }
        pc_keyboard::DecodedKey::Unicode(ch) if !ch.is_control() => Some(ch),
        _ => None,
    }
}

/// Draw the last `rows` lines of the scroll (the page scrolls upward like
/// paper feeding) plus the cursor at the live end.
fn render(
    renderer: &mut framebuffer::Renderer,
    text: &str,
    layout: &scroll_core::layout::Layout,
    cursor_on: bool,
) {
    renderer.fill(framebuffer::PAPER);
    let lines = layout.lines();
    let first = lines.len().saturating_sub(renderer.rows);
    for (row, line) in lines[first..].iter().enumerate() {
        let color = if line.is_separator { framebuffer::DIM } else { framebuffer::INK };
        for (col, ch) in text[line.start..line.end].chars().enumerate() {
            renderer.draw_char(row, col, ch, color);
        }
    }
    let (cursor_line, cursor_col) = layout.cursor();
    renderer.draw_cursor(cursor_line - first, cursor_col, cursor_on);
}
```

- [ ] **Step 4: Build, run, and try to break it**

Run: `cargo run`
Expected, verified by hand in the QEMU window:
- Typing appears immediately at the cursor; Enter starts a new line.
- Typing past the right edge wraps mid-word to the next line.
- Filling the page scrolls earlier lines up and off.
- **Backspace, Delete, arrows, Escape, Tab do nothing at all.** Typos stay.
- **Milestone 2 complete: it types (RAM-only).**

- [ ] **Step 5: Commit**

```bash
git add kernel
git commit -m "feat: PS/2 typing with wrap and scroll, no editing (milestone 2)"
```

---

### Task 9: ATA PIO driver

**Files:**
- Create: `kernel/src/ata.rs`
- Modify: `kernel/src/main.rs`

The driver talks **only** to the primary-channel **slave** drive (the second QEMU `-drive`, i.e. the scroll disk). LBA28 addresses 128 GiB — plenty.

- [ ] **Step 1: Write the driver**

`kernel/src/ata.rs`:

```rust
use x86_64::instructions::port::Port;

const DATA: u16 = 0x1F0;
const SECTOR_COUNT: u16 = 0x1F2;
const LBA_LO: u16 = 0x1F3;
const LBA_MID: u16 = 0x1F4;
const LBA_HI: u16 = 0x1F5;
const DRIVE: u16 = 0x1F6;
const STATUS_CMD: u16 = 0x1F7;

const STATUS_ERR: u8 = 1 << 0;
const STATUS_DRQ: u8 = 1 << 3;
const STATUS_DF: u8 = 1 << 5;
const STATUS_BSY: u8 = 1 << 7;

const CMD_READ: u8 = 0x20;
const CMD_WRITE: u8 = 0x30;
const CMD_FLUSH: u8 = 0xE7;
const CMD_IDENTIFY: u8 = 0xEC;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtaError;

fn status() -> u8 {
    unsafe { Port::<u8>::new(STATUS_CMD).read() }
}

/// ~400ns settle after touching the drive register: four status reads.
fn settle() {
    for _ in 0..4 {
        status();
    }
}

fn wait_not_busy() -> Result<(), AtaError> {
    for _ in 0..1_000_000 {
        let s = status();
        if s & STATUS_BSY == 0 {
            if s & (STATUS_ERR | STATUS_DF) != 0 {
                return Err(AtaError);
            }
            return Ok(());
        }
    }
    Err(AtaError)
}

fn wait_data_request() -> Result<(), AtaError> {
    for _ in 0..1_000_000 {
        let s = status();
        if s & (STATUS_ERR | STATUS_DF) != 0 {
            return Err(AtaError);
        }
        if s & STATUS_BSY == 0 && s & STATUS_DRQ != 0 {
            return Ok(());
        }
    }
    Err(AtaError)
}

/// Select the slave drive with the top LBA28 bits. 0xF0 = LBA mode | slave.
fn select(lba: u32) {
    unsafe { Port::<u8>::new(DRIVE).write(0xF0 | ((lba >> 24) as u8 & 0x0F)) };
    settle();
}

fn issue(lba: u32, command: u8) -> Result<(), AtaError> {
    wait_not_busy()?;
    select(lba);
    unsafe {
        Port::<u8>::new(SECTOR_COUNT).write(1);
        Port::<u8>::new(LBA_LO).write(lba as u8);
        Port::<u8>::new(LBA_MID).write((lba >> 8) as u8);
        Port::<u8>::new(LBA_HI).write((lba >> 16) as u8);
        Port::<u8>::new(STATUS_CMD).write(command);
    }
    Ok(())
}

/// Total sectors on the scroll disk, or None if it's absent.
pub fn identify() -> Option<u64> {
    wait_not_busy().ok()?;
    unsafe { Port::<u8>::new(DRIVE).write(0xB0) }; // slave, for IDENTIFY
    settle();
    unsafe {
        Port::<u8>::new(SECTOR_COUNT).write(0);
        Port::<u8>::new(LBA_LO).write(0);
        Port::<u8>::new(LBA_MID).write(0);
        Port::<u8>::new(LBA_HI).write(0);
        Port::<u8>::new(STATUS_CMD).write(CMD_IDENTIFY);
    }
    if status() == 0 {
        return None; // no drive
    }
    wait_data_request().ok()?;
    let mut words = [0u16; 256];
    let mut data = Port::<u16>::new(DATA);
    for word in words.iter_mut() {
        *word = unsafe { data.read() };
    }
    // Words 60–61: total addressable LBA28 sectors.
    Some(words[60] as u64 | ((words[61] as u64) << 16))
}

pub fn read_sector(lba: u32, buffer: &mut [u8; 512]) -> Result<(), AtaError> {
    issue(lba, CMD_READ)?;
    wait_data_request()?;
    let mut data = Port::<u16>::new(DATA);
    for chunk in buffer.chunks_exact_mut(2) {
        let word = unsafe { data.read() };
        chunk[0] = word as u8;
        chunk[1] = (word >> 8) as u8;
    }
    Ok(())
}

pub fn write_sector(lba: u32, buffer: &[u8; 512]) -> Result<(), AtaError> {
    issue(lba, CMD_WRITE)?;
    wait_data_request()?;
    let mut data = Port::<u16>::new(DATA);
    for chunk in buffer.chunks_exact(2) {
        let word = chunk[0] as u16 | ((chunk[1] as u16) << 8);
        unsafe { data.write(word) };
    }
    flush()
}

/// Spec flush policy: every keystroke reaches the platter.
pub fn flush() -> Result<(), AtaError> {
    wait_not_busy()?;
    unsafe { Port::<u8>::new(STATUS_CMD).write(CMD_FLUSH) };
    wait_not_busy()
}
```

- [ ] **Step 2: Smoke-test in kernel_main**

In `kernel/src/main.rs` add `mod ata;` and insert after `interrupts::init();`:

```rust
    {
        let sectors = ata::identify().expect("scroll disk missing");
        serial_println!("scroll disk: {} sectors ({} MiB)", sectors, sectors * 512 / 1024 / 1024);
        let pattern = [0xA5u8; 512];
        let mut readback = [0u8; 512];
        ata::write_sector(0, &pattern).expect("ata write failed");
        ata::read_sector(0, &mut readback).expect("ata read failed");
        assert_eq!(pattern, readback, "ata readback mismatch");
        // Restore the spec's zeroed-disk precondition before Task 10 trusts it.
        ata::write_sector(0, &[0u8; 512]).expect("ata zero failed");
        serial_println!("ata ok");
    }
```

- [ ] **Step 3: Build and run**

Run: `cargo run`
Expected: serial shows `scroll disk: 102400 sectors (50 MiB)` then `ata ok`, and the typewriter behaves as in Task 8.

- [ ] **Step 4: Remove the smoke test**

Delete the block added in Step 2 except keep nothing — Task 10 replaces it with the real scroll load. (Leaving a self-test that scribbles on sector 0 would eat the first record.)

- [ ] **Step 5: Commit**

```bash
git add kernel
git commit -m "feat: ATA PIO driver for the scroll disk"
```

---

### Task 10: Persistence — the scroll becomes eternal

**Files:**
- Create: `kernel/src/scroll.rs`
- Modify: `kernel/src/main.rs`

- [ ] **Step 1: Write the Scroll**

`kernel/src/scroll.rs`:

```rust
use crate::ata::{self, AtaError};
use alloc::{string::String, vec::Vec};
use scroll_core::layout::Layout;
use scroll_core::record::{self, PAYLOAD_CAPACITY, SECTOR_SIZE};

pub struct Scroll {
    pub text: String,
    pub layout: Layout,
    sealed: u64,    // count of full, never-touched-again sectors
    tail: Vec<u8>,  // bytes in the one rewritable tail sector (< 494)
    total_sectors: u64,
    pub full: bool,
}

impl Scroll {
    /// Boot: binary-search the highest valid record, then load everything.
    /// Valid records form a contiguous prefix from LBA 0 (zeroed-disk
    /// precondition + LBA check), which is what makes the search sound.
    pub fn load(columns: usize) -> Result<Scroll, AtaError> {
        let total_sectors = ata::identify().ok_or(AtaError)?;
        let mut sector = [0u8; SECTOR_SIZE];

        let mut lo: u64 = 0;
        let mut hi: u64 = total_sectors;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            ata::read_sector(mid as u32, &mut sector)?;
            if record::decode(&sector, mid).is_some() {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let valid = lo;

        let mut bytes: Vec<u8> = Vec::new();
        let mut last_len = 0usize;
        for lba in 0..valid {
            ata::read_sector(lba as u32, &mut sector)?;
            let payload = record::decode(&sector, lba).ok_or(AtaError)?;
            last_len = payload.len();
            bytes.extend_from_slice(payload);
        }

        let (sealed, tail) = if valid == 0 {
            (0, Vec::new())
        } else if last_len == PAYLOAD_CAPACITY {
            (valid, Vec::new())
        } else {
            (valid - 1, bytes[bytes.len() - last_len..].to_vec())
        };

        // Payloads may split a UTF-8 char across sectors; decode only after
        // concatenating all of them (spec: don't "fix" by sealing early).
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let layout = Layout::from_text(&text, columns);
        let full = sealed == total_sectors;
        Ok(Scroll { text, layout, sealed, tail, total_sectors, full })
    }

    pub fn append_char(&mut self, ch: char) {
        let mut utf8 = [0u8; 4];
        let encoded = ch.encode_utf8(&mut utf8);
        self.append_raw(encoded.as_bytes());
    }

    pub fn append_str(&mut self, s: &str) {
        self.append_raw(s.as_bytes());
    }

    fn append_raw(&mut self, bytes: &[u8]) {
        if self.full {
            return;
        }
        // RAM first: text + layout.
        let appended = core::str::from_utf8(bytes).expect("append_raw gets whole chars");
        for ch in appended.chars() {
            let offset = self.text.len();
            self.text.push(ch);
            self.layout.append(offset, ch);
        }
        // Then ink: byte-at-a-time so a char may split across sectors.
        for &byte in bytes {
            self.tail.push(byte);
            if self.tail.len() == PAYLOAD_CAPACITY {
                let sector = record::encode(self.sealed, &self.tail);
                self.write(self.sealed, &sector);
                self.sealed += 1;
                self.tail.clear();
                if self.sealed == self.total_sectors {
                    self.full = true;
                    return;
                }
            }
        }
        // Rewrite the (same) tail sector in place — the only overwrite that
        // ever happens.
        let sector = record::encode(self.sealed, &self.tail);
        self.write(self.sealed, &sector);
    }

    fn write(&mut self, lba: u64, sector: &[u8; SECTOR_SIZE]) {
        // Retry-with-warning lands in Task 14; for now fail loudly.
        ata::write_sector(lba as u32, sector).expect("scroll write failed");
    }
}
```

- [ ] **Step 2: Use it in kernel_main**

In `kernel/src/main.rs` add `mod scroll;`. Replace the `text`/`layout` locals and the key-handling arm:

```rust
    let mut scroll = scroll::Scroll::load(renderer.columns).expect("scroll disk unreadable");
    serial_println!("scroll restored: {} bytes", scroll.text.len());
```

In the input loop, replace the three lines that pushed into `text`/`layout` with:

```rust
                    if let Some(ch) = inkable(key) {
                        scroll.append_char(ch);
                        dirty = true;
                    }
```

and change every `&text, &layout` argument to `&scroll.text, &scroll.layout` (the `render` function signature is unchanged).

- [ ] **Step 3: Build, run, reboot, verify by hand**

Run: `rm -f scroll.img && cargo run` — type `first session`, close QEMU.
Run: `cargo run` again.
Expected: `first session` is on the page before you touch a key, serial shows `scroll restored: 13 bytes`. Type more; close; run again; everything is there. **Milestone 3 core behavior works.**

- [ ] **Step 4: Commit**

```bash
git add kernel
git commit -m "feat: scroll persists across reboots via append-only log (milestone 3)"
```

---

### Task 11: Extraction script + end-to-end test

**Files:**
- Create: `scripts/extract.py`, `scripts/e2e_test.sh`

- [ ] **Step 1: Extraction script**

`scripts/extract.py`:

```python
#!/usr/bin/env python3
"""Read a scroll disk image, emit the prose. Validates exactly like the
kernel: magic, seq==LBA, length, CRC-32 (zlib's crc32 is the same IEEE
polynomial as scroll-core's). Separator marker bytes (0x1E) are stripped."""
import struct
import sys
import zlib

MAGIC = b"ETYP"
PAYLOAD_CAPACITY = 494


def payloads(path):
    with open(path, "rb") as f:
        lba = 0
        while True:
            sector = f.read(512)
            if len(sector) < 512 or sector[:4] != MAGIC:
                return
            (seq,) = struct.unpack_from("<Q", sector, 4)
            (length,) = struct.unpack_from("<H", sector, 12)
            (crc,) = struct.unpack_from("<I", sector, 14)
            if seq != lba or length > PAYLOAD_CAPACITY:
                return
            payload = sector[18 : 18 + length]
            if zlib.crc32(payload) & 0xFFFFFFFF != crc:
                return
            yield payload
            lba += 1


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: extract.py scroll.img")
    data = b"".join(payloads(sys.argv[1])).replace(b"\x1e", b"")
    sys.stdout.write(data.decode("utf-8", errors="replace"))


if __name__ == "__main__":
    main()
```

Run: `chmod +x scripts/extract.py`

- [ ] **Step 2: Verify extraction against the Task 10 image**

Run: `python3 scripts/extract.py scroll.img`
Expected: exactly what you typed in Task 10's manual test, as plain text.

- [ ] **Step 3: End-to-end test script**

The spec's intent is boot → type → reboot → scroll restored, verified mechanically. We drive QEMU's monitor with `sendkey` (real PS/2 scancodes through the real driver) instead of an isa-debug-exit test kernel — same coverage, no special test build.

`scripts/e2e_test.sh`:

```bash
#!/usr/bin/env bash
# Boot the typewriter headless, type via the QEMU monitor, kill the power,
# boot again, and verify every keystroke survived. Requires: qemu, socat.
set -euo pipefail
cd "$(dirname "$0")/.."

IMG=$(cargo run --quiet -- --print-bios-image)
MON=/tmp/etyp-monitor.sock
SCROLL=e2e-scroll.img

rm -f "$SCROLL" "$MON"
truncate -s 50M "$SCROLL" # zeroed: the spec's disk precondition

boot() {
    qemu-system-x86_64 \
        -m 512M \
        -drive "format=raw,file=$IMG" \
        -drive "format=raw,file=$SCROLL,if=ide,index=1" \
        -monitor "unix:$MON,server,nowait" \
        -display none -serial none &
    QEMU_PID=$!
    sleep 8 # generous: kernel is at the cursor in well under a second
}

mon() {
    echo "$1" | socat - "UNIX-CONNECT:$MON"
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

rm -f "$SCROLL"
echo "E2E PASS"
```

Run: `chmod +x scripts/e2e_test.sh`

- [ ] **Step 4: Run it**

Run: `./scripts/e2e_test.sh`
Expected: the two session banners, then `E2E PASS`. **Milestone 3 complete and machine-verified.**

- [ ] **Step 5: Commit**

```bash
git add scripts
git commit -m "test: host extraction script and boot-type-reboot e2e test"
```

---

### Task 12: Boot date separator from the CMOS RTC

**Files:**
- Create: `kernel/src/rtc.rs`
- Modify: `kernel/src/main.rs`

- [ ] **Step 1: Write the RTC reader**

`kernel/src/rtc.rs`:

```rust
use x86_64::instructions::port::Port;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    pub year: u16,
    pub month: u8, // 1–12
    pub day: u8,   // 1–31
}

fn read_register(reg: u8) -> u8 {
    unsafe {
        Port::<u8>::new(0x70).write(0x80 | reg); // bit 7: NMI stays disabled during the read
        Port::<u8>::new(0x71).read()
    }
}

fn update_in_progress() -> bool {
    read_register(0x0A) & 0x80 != 0
}

fn read_raw() -> (u8, u8, u8) {
    (read_register(0x07), read_register(0x08), read_register(0x09)) // day, month, year
}

/// Spec: wait out the update flag, read until two consecutive reads match
/// (no torn values), BCD-decode if Status B says so, anchor year to 20xx.
/// Any weirdness returns None — a garbled date must never block boot.
pub fn read_date() -> Option<Date> {
    for _ in 0..1_000_000 {
        if !update_in_progress() {
            break;
        }
    }
    if update_in_progress() {
        return None;
    }

    let mut raw = read_raw();
    for _ in 0..10 {
        let again = read_raw();
        if again == raw {
            break;
        }
        raw = again;
    }

    let binary_mode = read_register(0x0B) & 0x04 != 0;
    let decode = |v: u8| -> u8 {
        if binary_mode {
            v
        } else {
            (v >> 4) * 10 + (v & 0x0F)
        }
    };
    let (day, month, year) = (decode(raw.0), decode(raw.1), decode(raw.2));
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) || year > 99 {
        return None;
    }
    Some(Date { year: 2000 + year as u16, month, day })
}

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];

pub fn month_name(month: u8) -> &'static str {
    MONTHS[(month - 1) as usize]
}
```

- [ ] **Step 2: Stamp the separator at boot**

In `kernel/src/main.rs` add `mod rtc;` and insert directly after the `Scroll::load` lines:

```rust
    if let Some(date) = rtc::read_date() {
        let mut separator = alloc::string::String::new();
        // The marker must start its own line.
        if !scroll.text.is_empty() && !scroll.text.ends_with('\n') {
            separator.push('\n');
        }
        // Spec format: `— 10 June 2026 —`, marked in-band with 0x1E so the
        // renderer can dim it after any future reload.
        separator.push(scroll_core::SEPARATOR_MARKER_CHAR);
        use core::fmt::Write;
        write!(
            separator,
            "\u{2014} {} {} {} \u{2014}\n",
            date.day,
            rtc::month_name(date.month),
            date.year
        )
        .unwrap();
        scroll.append_str(&separator);
    } // None: omit the separator, never block boot
```

- [ ] **Step 3: Build, run, verify**

Run: `cargo run` — a dim `— 10 June 2026 —` line (today's real date) appears under the restored prose, cursor below it. Close, run `python3 scripts/extract.py scroll.img`: the separator line appears as plain text with no `0x1E` and each boot added exactly one. Run `cargo run` once more: the previous separator is still dim after reload (the marker round-tripped through disk).

- [ ] **Step 4: Commit**

```bash
git add kernel
git commit -m "feat: dim boot-date separator stamped from the CMOS RTC"
```

---

### Task 13: Scroll-back (PgUp/PgDn) with snap-and-ink

**Files:**
- Modify: `kernel/src/main.rs`

- [ ] **Step 1: Add a view offset to the main loop**

In `kernel_main`, add a local alongside `cursor_on`:

```rust
    let mut view_offset: usize = 0; // lines scrolled back from the live end
```

Replace the key-handling `if let Some(key) = …` body with:

```rust
                if let Some(key) = decoder.process_keyevent(event) {
                    match key {
                        pc_keyboard::DecodedKey::RawKey(pc_keyboard::KeyCode::PageUp) => {
                            let page = renderer.rows.saturating_sub(1);
                            let max = scroll.layout.lines().len().saturating_sub(renderer.rows);
                            view_offset = (view_offset + page).min(max);
                            dirty = true;
                        }
                        pc_keyboard::DecodedKey::RawKey(pc_keyboard::KeyCode::PageDown) => {
                            view_offset = view_offset.saturating_sub(renderer.rows.saturating_sub(1));
                            dirty = true;
                        }
                        other => {
                            if let Some(ch) = inkable(other) {
                                // Spec: a printable key snaps back to the live
                                // end AND inks there — even mid-reading.
                                view_offset = 0;
                                scroll.append_char(ch);
                                dirty = true;
                            }
                        }
                    }
                }
```

- [ ] **Step 2: Make render window-aware**

Change `render`'s signature and window math:

```rust
fn render(
    renderer: &mut framebuffer::Renderer,
    text: &str,
    layout: &scroll_core::layout::Layout,
    cursor_on: bool,
    view_offset: usize,
) {
    renderer.fill(framebuffer::PAPER);
    let lines = layout.lines();
    let end = lines.len() - view_offset.min(lines.len());
    let first = end.saturating_sub(renderer.rows);
    for (row, line) in lines[first..end].iter().enumerate() {
        let color = if line.is_separator { framebuffer::DIM } else { framebuffer::INK };
        for (col, ch) in text[line.start..line.end].chars().enumerate() {
            renderer.draw_char(row, col, ch, color);
        }
    }
    if view_offset == 0 {
        let (cursor_line, cursor_col) = layout.cursor();
        renderer.draw_cursor(cursor_line - first, cursor_col, cursor_on);
    } // reading mode: no cursor — you can look, you can't touch
}
```

Update both call sites to pass `view_offset`. Also gate the blink: only mark `dirty` for a cursor toggle when `view_offset == 0`.

- [ ] **Step 3: Build, run, verify**

Run: `cargo run` — type several screenfuls (hold a key down). PgUp pages back through history with no cursor; PgDn returns; typing `x` while scrolled back instantly snaps to the end with the `x` inked at the live position.

- [ ] **Step 4: Commit**

```bash
git add kernel
git commit -m "feat: read-only scroll-back with snap-and-ink on typing"
```

---

### Task 14: Disk-full dignity and write-error retries

**Files:**
- Modify: `kernel/src/scroll.rs`, `kernel/src/main.rs`

- [ ] **Step 1: Retry writes, surface persistent trouble**

In `kernel/src/scroll.rs`, add at the top:

```rust
use core::sync::atomic::{AtomicBool, Ordering};

/// Set when a sector write keeps failing; the main loop renders a red `!`
/// in the margin. Never cleared — distrust, once earned, is permanent.
pub static WRITE_TROUBLE: AtomicBool = AtomicBool::new(false);
```

Replace the `write` method:

```rust
    fn write(&mut self, lba: u64, sector: &[u8; SECTOR_SIZE]) {
        for _attempt in 0..3 {
            if ata::write_sector(lba as u32, sector).is_ok() {
                return;
            }
        }
        WRITE_TROUBLE.store(true, Ordering::Relaxed);
    }
```

- [ ] **Step 2: End-of-scroll message and warning glyph**

In `render` in `kernel/src/main.rs`, add a `full: bool` parameter (pass `scroll.full` at the call sites) and append at the end of the function:

```rust
    if full {
        // The dignified end: rendered, never written — there is nowhere
        // left to write it.
        let message = "\u{2014} the scroll is full. the typewriter rests. \u{2014}";
        let row = renderer.rows - 1;
        for (col, ch) in message.chars().enumerate() {
            renderer.draw_char(row, col, ch, framebuffer::DIM);
        }
    }
    if scroll::WRITE_TROUBLE.load(core::sync::atomic::Ordering::Relaxed) {
        renderer.draw_warning_glyph();
    }
```

When `full`, also skip drawing the cursor (wrap the existing cursor block in `if view_offset == 0 && !full`).

- [ ] **Step 3: Verify with a tiny scroll disk**

A 50 MB disk takes a lifetime to fill, so test with a doll-sized one:

```bash
rm -f scroll.img && truncate -s 5120 scroll.img   # 10 sectors ≈ 4.9 KB of ink
cargo run
```

Hold a key until ~4,940 characters are inked (a minute or so of key-repeat). Expected: at capacity the cursor vanishes and the dim end-of-scroll line appears on the last row; further typing changes nothing. Reboot (`cargo run` again): the full scroll renders, message still there, still no ink accepted. Then restore reality: `rm -f scroll.img`.

- [ ] **Step 4: Commit**

```bash
git add kernel
git commit -m "feat: end-of-scroll message and disk write-error warning"
```

---

### Task 15: The panic page

**Files:**
- Modify: `kernel/src/main.rs`

- [ ] **Step 1: Stash framebuffer raw parts for the panic handler**

In `kernel/src/main.rs` add a static near the top:

```rust
static PANIC_FRAMEBUFFER: spin::Mutex<Option<(usize, usize, bootloader_api::info::FrameBufferInfo)>> =
    spin::Mutex::new(None);
```

In `kernel_main`, right after constructing the renderer:

```rust
    {
        let (ptr, len, info) = renderer.raw_parts();
        *PANIC_FRAMEBUFFER.lock() = Some((ptr as usize, len, info));
    }
```

- [ ] **Step 2: Render the red page on panic**

Replace the panic handler:

```rust
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("PANIC: {}", info);
    // Single-threaded kernel: if the lock is held we panicked mid-render;
    // aliasing the buffer is fine because we never return.
    let stashed = unsafe {
        PANIC_FRAMEBUFFER.force_unlock();
        *PANIC_FRAMEBUFFER.lock()
    };
    if let Some((ptr, len, fb_info)) = stashed {
        let mut r = unsafe { framebuffer::Renderer::from_raw_parts(ptr as *mut u8, len, fb_info) };
        r.fill((140, 30, 30));
        let mut row = 1;
        let mut col = 0;
        let mut put = |s: &str, row: &mut usize, col: &mut usize| {
            for ch in s.chars() {
                if ch == '\n' || *col >= r.columns {
                    *row += 1;
                    *col = 0;
                    if ch == '\n' {
                        continue;
                    }
                }
                r.draw_char(*row, *col, ch, (250, 240, 230));
                *col += 1;
            }
        };
        put("the typewriter is broken.", &mut row, &mut col);
        row += 2;
        col = 0;
        let mut message = alloc::string::String::new();
        use core::fmt::Write;
        write!(message, "{}", info).ok();
        put(&message, &mut row, &mut col);
        row += 2;
        col = 0;
        put("your words are safe on the scroll.", &mut row, &mut col);
    }
    loop {
        x86_64::instructions::hlt();
    }
}
```

Note: `draw_char` blends against `PAPER`, so glyphs on the red page get paper-coloured anti-aliasing fringes. On a broken-typewriter page that's acceptable; do not complicate the renderer for it.

- [ ] **Step 3: Verify with a deliberate crash**

Temporarily add to the input loop: panic when `'!'` is typed:

```rust
                            if ch == '!' {
                                panic!("deliberate test panic");
                            }
```

Run: `cargo run`, type `!` (shift-1). Expected: full red page, "the typewriter is broken.", the panic message with file:line, "your words are safe on the scroll." Then `python3 scripts/extract.py scroll.img` — everything typed before the `!` is on the scroll. **Delete the temporary panic lines.**

- [ ] **Step 4: Build once more and commit**

Run: `cargo run` (confirm `!` types normally again).

```bash
git add kernel
git commit -m "feat: red panic page; the scroll survives the crash"
```

---

### Task 16: F12 serial dump

**Files:**
- Modify: `kernel/src/main.rs`

- [ ] **Step 1: Handle F12**

In the key-handling `match` (Task 13's version), add an arm above `other =>`:

```rust
                        pc_keyboard::DecodedKey::RawKey(pc_keyboard::KeyCode::F12) => {
                            serial_println!("=== SCROLL BEGIN ===");
                            // Strip in-band separator markers, same as extract.py.
                            for ch in scroll.text.chars() {
                                if ch != scroll_core::SEPARATOR_MARKER_CHAR {
                                    serial_print!("{}", ch);
                                }
                            }
                            serial_println!();
                            serial_println!("=== SCROLL END ===");
                        }
```

- [ ] **Step 2: Verify**

Run: `cargo run`, type a few words, press F12. Expected: the terminal running cargo (QEMU's `-serial stdio`) prints the whole scroll between the BEGIN/END markers, separators as plain text. (To capture to a file instead: run QEMU with `-serial file:serial.log` — the runner's stdio default is the dev loop; the e2e script already shows the file form.)

- [ ] **Step 3: Commit**

```bash
git add kernel
git commit -m "feat: F12 dumps the scroll out the serial port"
```

---

### Task 17: Streaming boot load — under a second to the cursor, always

**Files:**
- Modify: `kernel/src/scroll.rs`, `kernel/src/main.rs`

ATA PIO moves a few MB/s; a grown scroll cannot be fully read before first paint without breaking the boot promise. Per the spec: load a tail screenful before the cursor appears, stream the rest in the background, let PgUp wait on the stream.

- [ ] **Step 1: Split load into tail-load + back-loader**

In `kernel/src/scroll.rs`, add to the `Scroll` struct:

```rust
    /// Sealed sectors [0, loader_next) not yet in RAM, streamed oldest-first
    /// by the idle loop. None = fully loaded.
    pending: Option<Loader>,
```

and below the struct:

```rust
pub struct Loader {
    prefix: Vec<u8>,   // payloads of sectors [0, cursor) — oldest prose
    cursor: u64,       // next LBA to read
    end: u64,          // first LBA already in RAM
}
```

Replace `Scroll::load` with a version that reads only the last `TAIL_SECTORS` before returning:

```rust
const TAIL_SECTORS: u64 = 64; // ~31 KB ≈ a dozen screenfuls: instant boot

    pub fn load(columns: usize) -> Result<Scroll, AtaError> {
        let total_sectors = ata::identify().ok_or(AtaError)?;
        let mut sector = [0u8; SECTOR_SIZE];

        let mut lo: u64 = 0;
        let mut hi: u64 = total_sectors;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            ata::read_sector(mid as u32, &mut sector)?;
            if record::decode(&sector, mid).is_some() {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let valid = lo;
        let first_loaded = valid.saturating_sub(TAIL_SECTORS);

        let mut bytes: Vec<u8> = Vec::new();
        let mut last_len = 0usize;
        for lba in first_loaded..valid {
            ata::read_sector(lba as u32, &mut sector)?;
            let payload = record::decode(&sector, lba).ok_or(AtaError)?;
            last_len = payload.len();
            bytes.extend_from_slice(payload);
        }

        let (sealed, tail) = if valid == 0 {
            (0, Vec::new())
        } else if last_len == PAYLOAD_CAPACITY {
            (valid, Vec::new())
        } else {
            (valid - 1, bytes[bytes.len() - last_len..].to_vec())
        };

        let text = String::from_utf8_lossy(&bytes).into_owned();
        let layout = Layout::from_text(&text, columns);
        let pending = (first_loaded > 0).then(|| Loader {
            prefix: Vec::new(),
            cursor: 0,
            end: first_loaded,
        });
        let full = sealed == total_sectors;
        Ok(Scroll { text, layout, sealed, tail, total_sectors, full, pending })
    }
```

(In Task 10's original full-load path, `pending` didn't exist; add `pending: None` there is no longer needed — this replaces it.)

- [ ] **Step 2: The idle-loop pump**

Add to `impl Scroll`:

```rust
    pub fn fully_loaded(&self) -> bool {
        self.pending.is_none()
    }

    /// Stream one chunk of history. Call from the idle loop; returns true
    /// if the stream just finished (layout was rebuilt — re-render).
    pub fn pump(&mut self, columns: usize) -> Result<bool, AtaError> {
        const CHUNK: u64 = 32;
        let Some(loader) = &mut self.pending else { return Ok(false) };
        let mut sector = [0u8; SECTOR_SIZE];
        let stop = (loader.cursor + CHUNK).min(loader.end);
        while loader.cursor < stop {
            ata::read_sector(loader.cursor as u32, &mut sector)?;
            let payload = record::decode(&sector, loader.cursor).ok_or(AtaError)?;
            loader.prefix.extend_from_slice(payload);
            loader.cursor += 1;
        }
        if loader.cursor < loader.end {
            return Ok(false);
        }
        // Done: splice history before the live tail, rebuild layout once.
        let loader = self.pending.take().unwrap();
        let mut all = loader.prefix;
        all.extend_from_slice(self.text.as_bytes());
        self.text = String::from_utf8_lossy(&all).into_owned();
        self.layout = Layout::from_text(&self.text, columns);
        Ok(true)
    }
```

- [ ] **Step 3: Pump from the main loop, gate PgUp**

In `kernel_main`'s loop, before the `hlt()`:

```rust
        if !scroll.fully_loaded() {
            if scroll.pump(renderer.columns).unwrap_or(false) {
                serial_println!("scroll fully loaded: {} bytes", scroll.text.len());
                dirty = true;
            }
        }
```

The PgUp arm needs no change: it already clamps to `lines().len()`, which only grows when the stream finishes, so while loading PgUp simply stops at the oldest line already in RAM. That satisfies the spec's "waits on the stream" — the spec defines no loading indicator and the stream finishes in seconds.

- [ ] **Step 4: Verify**

Seed a big scroll, then boot:

```bash
rm -f scroll.img && truncate -s 50M scroll.img
cargo run   # type a line, close — then pad the scroll from the host:
python3 - <<'EOF'
# Append ~20 MB of sealed filler records after the existing ones, so boot
# has real history to stream. Reuses the kernel's exact format.
import struct, zlib
PAY = 494
img = open("scroll.img", "r+b")
# find first invalid sector
lba = 0
while True:
    img.seek(lba * 512)
    s = img.read(512)
    if s[:4] != b"ETYP" or struct.unpack_from("<Q", s, 4)[0] != lba:
        break
    lba += 1
# the kernel's unsealed tail (if any) sits at lba-1; seal everything by
# rewriting from lba-1 as full records of filler
start = max(lba - 1, 0)
filler = (b"all work and no play makes jack a dull boy. " * 12)[:PAY]
for i in range(start, start + 40000):
    rec = b"ETYP" + struct.pack("<QHI", i, PAY, zlib.crc32(filler)) + filler
    img.seek(i * 512)
    img.write(rec)
img.close()
print("padded")
EOF
cargo run
```

Expected: the page with the filler tail appears immediately (same subjective instant as an empty disk), typing works at once; within a few seconds serial prints `scroll fully loaded: …`; PgUp then pages back through all ~20 MB of history. Run `./scripts/e2e_test.sh` once more — still `E2E PASS`. Then `rm -f scroll.img`.

- [ ] **Step 5: Commit**

```bash
git add kernel
git commit -m "feat: instant boot via tail-first load with background history stream"
```

---

## Spec coverage checklist (self-review, all covered)

- Boot < 1s to cursor: Tasks 1, 6, 17 (streaming load)
- Append-only typing, wrap, no editing: Tasks 4, 8 (`inkable` filter)
- Page scroll upward: Task 8 `render`
- PgUp/PgDn read-only + snap-and-ink: Task 13
- Boot-date separator, exact format, dim, in-band 0x1E marker: Tasks 4, 12
- Disk full → dignified message, no more ink: Task 14
- QEMU BIOS target, second IDE drive, PS/2, framebuffer: Tasks 1, 5, 8, 9
- Six components incl. interrupts: Tasks 1, 5–9
- Derived grid, font metrics from API, margins: Task 5
- RTC: UIP wait, double-read, BCD, 20xx anchor, degrade-to-omit: Task 12
- Record format: magic/seq/len/CRC/payload, validity = magic+len+CRC+LBA, zeroed-disk precondition, binary search, tail rewrite, seal at 494, UTF-8 split across sectors: Tasks 3, 10
- Flush per keystroke: Task 9 (`write_sector` flushes)
- Extraction script + F12 serial dump (both strip 0x1E): Tasks 11, 16
- Panic page: Task 15
- Write-error retry + margin warning glyph: Task 14
- Host unit tests for wrap + log format: Tasks 2–4; QEMU end-to-end: Task 11
- Format versioning: magic-as-version noted in Task 3; nothing to build
- Out of scope (editing, files, networking, real hardware): not present anywhere
