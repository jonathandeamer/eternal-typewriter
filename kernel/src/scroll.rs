use crate::ata::{self, AtaError};
use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use scroll_core::layout::Layout;
use scroll_core::record::{self, PAYLOAD_CAPACITY, SECTOR_SIZE};

/// Set when a sector write keeps failing; the main loop renders a red `!`
/// in the margin. Never cleared — distrust, once earned, is permanent.
pub static WRITE_TROUBLE: AtomicBool = AtomicBool::new(false);

pub struct Scroll {
    pub text: String,
    pub layout: Layout,
    sealed: u64,    // count of full, never-touched-again sectors
    tail: Vec<u8>,  // bytes in the one rewritable tail sector (< 494)
    total_sectors: u64,
    pub full: bool,
    /// Sealed sectors [0, end) not yet in RAM, streamed oldest-first by the
    /// idle loop. None = fully loaded.
    pending: Option<Loader>,
}

pub struct Loader {
    prefix: Vec<u8>, // payloads of sectors [0, cursor) — oldest prose
    cursor: u64,     // next LBA to read
    end: u64,        // first LBA already in RAM
}

/// ~31 KB ≈ a dozen screenfuls: enough to fill the page instantly at boot
/// while the rest of a grown scroll streams in behind the cursor.
const TAIL_SECTORS: u64 = 64;

impl Scroll {
    /// Boot: binary-search the highest valid record, then load only the last
    /// `TAIL_SECTORS` before returning so the cursor appears in well under a
    /// second even on a grown scroll. The older history streams in afterwards
    /// via `pump`. Valid records form a contiguous prefix from LBA 0 (zeroed-
    /// disk precondition + LBA check), which is what makes the search sound.
    pub fn load(columns: usize) -> Result<Scroll, AtaError> {
        let total_sectors = u32::try_from(ata::identify().ok_or(AtaError)?).map_err(|_| AtaError)? as u64;
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

        // Payloads may split a UTF-8 char across sectors; decode only after
        // concatenating all of them (spec: don't "fix" by sealing early).
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let layout = Layout::from_text(&text, columns);
        let full = sealed == total_sectors;
        let pending = (first_loaded > 0).then(|| Loader {
            prefix: Vec::new(),
            cursor: 0,
            end: first_loaded,
        });
        Ok(Scroll { text, layout, sealed, tail, total_sectors, full, pending })
    }

    pub fn fully_loaded(&self) -> bool {
        self.pending.is_none()
    }

    /// Stream one chunk of history from disk. Call from the idle loop; returns
    /// true when the stream just finished (layout was rebuilt — re-render).
    /// Reads up to `CHUNK` sectors per ATA command so a grown scroll loads at
    /// PIO bandwidth rather than one slow command per sector.
    pub fn pump(&mut self, columns: usize) -> Result<bool, AtaError> {
        const CHUNK: u64 = 256; // max sectors per READ SECTORS command
        let Some(loader) = &mut self.pending else { return Ok(false) };
        let count = (loader.end - loader.cursor).min(CHUNK);
        let mut batch = alloc::vec![0u8; count as usize * SECTOR_SIZE];
        ata::read_sectors(loader.cursor as u32, count as u16, &mut batch)?;
        for i in 0..count {
            let lba = loader.cursor + i;
            let start = i as usize * SECTOR_SIZE;
            let sector: &[u8; SECTOR_SIZE] =
                batch[start..start + SECTOR_SIZE].try_into().unwrap();
            let payload = record::decode(sector, lba).ok_or(AtaError)?;
            loader.prefix.extend_from_slice(payload);
        }
        loader.cursor += count;
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
        if !self.tail.is_empty() {
            let sector = record::encode(self.sealed, &self.tail);
            self.write(self.sealed, &sector);
        }
    }

    fn write(&mut self, lba: u64, sector: &[u8; SECTOR_SIZE]) {
        if WRITE_TROUBLE.load(Ordering::Relaxed) {
            return;
        }
        for _attempt in 0..3 {
            if ata::write_sector(lba as u32, sector).is_ok() {
                return;
            }
        }
        WRITE_TROUBLE.store(true, Ordering::Relaxed);
    }
}
