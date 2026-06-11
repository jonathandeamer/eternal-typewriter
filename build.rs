use bootloader::{BootConfig, DiskImageBuilder};
use std::{env, path::PathBuf};

fn main() {
    let kernel = PathBuf::from(env::var("CARGO_BIN_FILE_KERNEL").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut builder = DiskImageBuilder::new(kernel);

    // Ask the bootloader for at least an 1024x768 framebuffer. As of
    // bootloader 0.11 this lives in BootConfig (baked into the image here)
    // rather than the kernel's compile-time BootloaderConfig.
    let mut boot_config = BootConfig::default();
    boot_config.frame_buffer.minimum_framebuffer_width = Some(1024);
    boot_config.frame_buffer.minimum_framebuffer_height = Some(768);
    builder.set_boot_config(&boot_config);

    let bios_path = out_dir.join("bios.img");
    builder.create_bios_image(&bios_path).unwrap();
    println!("cargo:rustc-env=BIOS_IMAGE={}", bios_path.display());

    let uefi_path = out_dir.join("uefi.img");
    builder.create_uefi_image(&uefi_path).unwrap();
    println!("cargo:rustc-env=UEFI_IMAGE={}", uefi_path.display());
}
