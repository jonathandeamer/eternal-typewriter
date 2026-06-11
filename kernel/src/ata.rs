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

/// Read one 512-byte sector's worth of data (256 words) from the data port in
/// a single `rep insw`. QEMU services the whole string op in one VM exit, so
/// this is ~256× cheaper than reading word-by-word — the difference between a
/// grown scroll streaming in seconds versus minutes.
unsafe fn insw_sector(buffer: &mut [u8]) {
    debug_assert_eq!(buffer.len(), 512);
    core::arch::asm!(
        "cld",
        "rep insw",
        in("dx") DATA,
        inout("rdi") buffer.as_mut_ptr() => _,
        inout("rcx") 256usize => _,
        options(nostack),
    );
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
    unsafe { insw_sector(buffer) };
    Ok(())
}

/// Read `count` consecutive sectors with a single READ SECTORS command (1–256;
/// pass 256 as `count == 0` is not used here — callers chunk by ≤256). One
/// command amortises the per-command drive latency, so streaming a grown
/// scroll runs at PIO bandwidth instead of one slow command per sector.
/// `buffer` must be exactly `count * 512` bytes.
pub fn read_sectors(lba: u32, count: u16, buffer: &mut [u8]) -> Result<(), AtaError> {
    debug_assert!(count >= 1 && count <= 256);
    debug_assert_eq!(buffer.len(), count as usize * 512);
    wait_not_busy()?;
    select(lba);
    unsafe {
        // SECTOR_COUNT register takes 0 to mean 256; count is ≤ 256.
        Port::<u8>::new(SECTOR_COUNT).write((count & 0xFF) as u8);
        Port::<u8>::new(LBA_LO).write(lba as u8);
        Port::<u8>::new(LBA_MID).write((lba >> 8) as u8);
        Port::<u8>::new(LBA_HI).write((lba >> 16) as u8);
        Port::<u8>::new(STATUS_CMD).write(CMD_READ);
    }
    for sector in 0..count as usize {
        wait_data_request()?;
        let base = sector * 512;
        unsafe { insw_sector(&mut buffer[base..base + 512]) };
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
