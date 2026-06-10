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
