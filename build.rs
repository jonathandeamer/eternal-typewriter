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
