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
    let fb = boot_info.framebuffer.as_mut().expect("no framebuffer");
    let info = fb.info();
    serial_println!("framebuffer {}x{} {:?}", info.width, info.height, info.pixel_format);
    let mut renderer = framebuffer::Renderer::new(fb);
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
    let mut decoder = pc_keyboard::Keyboard::new(
        pc_keyboard::ScancodeSet1::new(),
        pc_keyboard::layouts::Us104Key,
        pc_keyboard::HandleControl::Ignore,
    );
    let mut cursor_on = true;
    let mut last_toggle = 0u64;
    let mut view_offset: usize = 0;
    render(&mut renderer, &scroll.text, &scroll.layout, cursor_on, view_offset);

    loop {
        let mut dirty = false;
        while let Some(scancode) = keyboard::pop() {
            serial_println!("sc {:#x}", scancode);
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
                        other => {
                            if let Some(ch) = inkable(other) {
                                // Spec: a printable key snaps back to the live
                                // end AND inks there — even mid-reading.
                                view_offset = 0;
                                scroll.append_char(ch);
                                serial_println!("ink {:?} -> {} bytes", ch, scroll.text.len());
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
            if view_offset == 0 {
                dirty = true;
            }
        }
        if dirty {
            render(&mut renderer, &scroll.text, &scroll.layout, cursor_on, view_offset);
        }
        x86_64::instructions::hlt();
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
) {
    renderer.fill(framebuffer::PAPER);
    let lines = layout.lines();
    let end = lines.len().saturating_sub(view_offset);
    let first = end.saturating_sub(renderer.rows);
    for (row, line) in lines[first..end].iter().enumerate() {
        let color = if line.is_separator { framebuffer::DIM } else { framebuffer::INK };
        for (col, ch) in text[line.start..line.end].chars().enumerate() {
            renderer.draw_char(row, col, ch, color);
        }
    }
    if view_offset == 0 {
        let (cursor_line, cursor_col) = layout.cursor();
        if cursor_line >= first {
            renderer.draw_cursor(cursor_line - first, cursor_col, cursor_on);
        }
    } // reading mode: no cursor — you can look, you can't touch
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("PANIC: {}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
