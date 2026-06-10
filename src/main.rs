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
