#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

mod framebuffer;
mod gdt;
mod interrupts;
mod keyboard;
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
    let mut renderer = framebuffer::Renderer::new(fb);
    serial_println!("grid {}x{}", renderer.columns, renderer.rows);
    for (col, ch) in "the eternal typewriter".chars().enumerate() {
        renderer.draw_char(0, col, ch, framebuffer::INK);
    }
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
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("PANIC: {}", info);
    loop {
        x86_64::instructions::hlt();
    }
}
