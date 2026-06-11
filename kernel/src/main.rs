#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

extern crate alloc;

mod allocator;
mod ata;
mod framebuffer;
mod gdt;
mod interrupts;
mod keyboard;
mod memory;
mod rtc;
mod scroll;
mod serial;

use bootloader_api::{
    config::{BootloaderConfig, Mapping},
    entry_point, BootInfo,
};
use core::panic::PanicInfo;
use x86_64::VirtAddr;

static PANIC_FRAMEBUFFER: spin::Mutex<Option<(usize, usize, bootloader_api::info::FrameBufferInfo)>> =
    spin::Mutex::new(None);

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    // Framebuffer minimum size now lives in BootConfig (set in build.rs).
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    serial_println!("eternal typewriter: boot");
    let physical_memory_offset = VirtAddr::new(
        boot_info.physical_memory_offset.into_option().expect("no physical memory mapping"),
    );
    let mut mapper = unsafe { memory::init(physical_memory_offset) };
    let mut frame_allocator =
        unsafe { memory::BootInfoFrameAllocator::init(&boot_info.memory_regions) };
    allocator::init_heap(&mut mapper, &mut frame_allocator).expect("heap init failed");
    let fb = boot_info.framebuffer.as_mut().expect("no framebuffer");
    let info = fb.info();
    serial_println!("framebuffer {}x{} {:?}", info.width, info.height, info.pixel_format);
    let mut renderer = framebuffer::Renderer::new(fb);
    {
        let (ptr, len, info) = renderer.raw_parts();
        *PANIC_FRAMEBUFFER.lock() = Some((ptr as usize, len, info));
    }
    serial_println!("grid {}x{}", renderer.columns, renderer.rows);
    interrupts::init();

    let mut scroll = scroll::Scroll::load(renderer.columns).expect("scroll disk unreadable");
    serial_println!("scroll restored: {} bytes", scroll.text.len());
    if let Some(date) = rtc::read_date() {
        let mut separator = alloc::string::String::new();
        // The marker must start its own line.
        if !scroll.text.is_empty() && !scroll.text.ends_with('\n') {
            separator.push('\n');
        }
        // Spec format: `— 10 June 2026, 14:32 —`, marked in-band with 0x1E so the
        // renderer can dim it after any future reload. Time is a snapshot of the
        // machine's clock at boot (zero-padded 24h); no timezone is asserted.
        separator.push(scroll_core::SEPARATOR_MARKER_CHAR);
        use core::fmt::Write;
        write!(
            separator,
            "\u{2014} {} {} {}, {:02}:{:02} \u{2014}\n",
            date.day,
            rtc::month_name(date.month),
            date.year,
            date.hour,
            date.minute
        )
        .unwrap();
        scroll.append_str(&separator);
    } // None: omit the separator, never block boot
    let mut decoder = pc_keyboard::Keyboard::new(
        pc_keyboard::ScancodeSet1::new(),
        pc_keyboard::layouts::Us104Key,
        pc_keyboard::HandleControl::Ignore,
    );
    let mut cursor_on = true;
    let mut last_toggle = 0u64;
    let mut view_offset: usize = 0;
    render(&mut renderer, &scroll.text, &scroll.layout, cursor_on, view_offset, scroll.full);

    loop {
        let mut dirty = false;
        while let Some(scancode) = keyboard::pop() {
            if let Ok(Some(event)) = decoder.add_byte(scancode) {
                if let Some(key) = decoder.process_keyevent(event) {
                    match key {
                        pc_keyboard::DecodedKey::RawKey(pc_keyboard::KeyCode::PageUp) => {
                            let page = renderer.rows.saturating_sub(1);
                            let max = scroll.layout.lines().len().saturating_sub(renderer.rows);
                            let new_offset = view_offset.saturating_add(page).min(max);
                            if new_offset != view_offset {
                                view_offset = new_offset;
                                dirty = true;
                            }
                        }
                        pc_keyboard::DecodedKey::RawKey(pc_keyboard::KeyCode::PageDown) => {
                            let new_offset = view_offset.saturating_sub(renderer.rows.saturating_sub(1));
                            if new_offset != view_offset {
                                view_offset = new_offset;
                                dirty = true;
                            }
                        }
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
            }
        }
        let ticks = interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
        if ticks.wrapping_sub(last_toggle) >= 9 {
            cursor_on = !cursor_on;
            last_toggle = ticks;
            if view_offset == 0 && !scroll.full {
                dirty = true;
            }
        }
        if dirty {
            render(&mut renderer, &scroll.text, &scroll.layout, cursor_on, view_offset, scroll.full);
        }
        if scroll.fully_loaded() {
            // Idle: sleep until the next interrupt (keystroke or blink tick).
            x86_64::instructions::hlt();
        } else {
            // Stream history in behind the live page as fast as the disk
            // allows — don't hlt(), or the timer would throttle the load to
            // ~18 Hz and a grown scroll would take a minute. Keystrokes are
            // still serviced each pass at the top of the loop.
            if scroll.pump(renderer.columns).unwrap_or(false) {
                serial_println!("scroll fully loaded: {} bytes", scroll.text.len());
                render(&mut renderer, &scroll.text, &scroll.layout, cursor_on, view_offset, scroll.full);
            }
        }
    }
}

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
    view_offset: usize,
    full: bool,
) {
    renderer.fill(framebuffer::PAPER);
    let lines = layout.lines();
    let end = lines.len().saturating_sub(view_offset);
    let max_text_rows = if full { renderer.rows.saturating_sub(1) } else { renderer.rows };
    let first = end.saturating_sub(max_text_rows);
    for (row, line) in lines[first..end].iter().enumerate() {
        let color = if line.is_separator { framebuffer::DIM } else { framebuffer::INK };
        for (col, ch) in text[line.start..line.end].chars().enumerate() {
            renderer.draw_char(row, col, ch, color);
        }
    }
    if view_offset == 0 && !full {
        let (cursor_line, cursor_col) = layout.cursor();
        if cursor_line >= first {
            renderer.draw_cursor(cursor_line - first, cursor_col, cursor_on);
        }
    } // reading mode or full scroll: no cursor
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
    renderer.present();
}

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
        r.fill((140, 30, 30)); // Red background
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
                r.draw_char(*row, *col, ch, (250, 240, 230)); // Off-white text
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
