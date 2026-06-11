use x86_64::instructions::port::Port;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    pub year: u16,
    pub month: u8,  // 1–12
    pub day: u8,    // 1–31
    pub hour: u8,   // 0–23 (normalized to 24h regardless of chip mode)
    pub minute: u8, // 0–59
}

fn read_register(reg: u8) -> u8 {
    unsafe {
        Port::<u8>::new(0x70).write(0x80 | reg); // bit 7: NMI stays disabled during the read
        let val = Port::<u8>::new(0x71).read();
        // Re-enable NMIs by clearing bit 7 (writing register index with MSB=0)
        Port::<u8>::new(0x70).write(reg);
        val
    }
}

fn update_in_progress() -> bool {
    read_register(0x0A) & 0x80 != 0
}

fn read_raw() -> (u8, u8, u8, u8, u8) {
    // hour, minute, day, month, year — read together so the snapshot is coherent.
    (
        read_register(0x04),
        read_register(0x02),
        read_register(0x07),
        read_register(0x08),
        read_register(0x09),
    )
}

/// Spec: wait out the update flag, read until two consecutive reads match
/// (no torn values), BCD-decode if Status B says so, anchor year to 20xx.
/// Any weirdness returns None — a garbled date must never block boot.
pub fn read_date() -> Option<Date> {
    // 10,000 iterations is plenty: checking UIP on 1-2us port reads takes ~10-20ms.
    for _ in 0..10_000 {
        if !update_in_progress() {
            break;
        }
    }
    if update_in_progress() {
        return None;
    }

    let mut raw = read_raw();
    let mut stable = false;
    for _ in 0..10 {
        let again = read_raw();
        if again == raw {
            stable = true;
            break;
        }
        raw = again;
    }
    if !stable {
        return None;
    }

    let status_b = read_register(0x0B);
    let binary_mode = status_b & 0x04 != 0;
    let hour_24 = status_b & 0x02 != 0;
    let decode = |v: u8| -> u8 {
        if binary_mode {
            v
        } else {
            (v >> 4) * 10 + (v & 0x0F)
        }
    };

    // In 12h mode the PM flag is bit 7 of the raw hour byte; mask it off before
    // BCD decoding, then fold AM/PM into a 24h value (12am→0, 12pm→12).
    let raw_hour = raw.0;
    let hour = if hour_24 {
        decode(raw_hour)
    } else {
        let pm = raw_hour & 0x80 != 0;
        let h12 = decode(raw_hour & 0x7F);
        match (pm, h12) {
            (false, 12) => 0,
            (false, h) => h,
            (true, 12) => 12,
            (true, h) => h + 12,
        }
    };
    let minute = decode(raw.1);
    let (day, month, year) = (decode(raw.2), decode(raw.3), decode(raw.4));
    if !(1..=31).contains(&day)
        || !(1..=12).contains(&month)
        || year > 99
        || hour > 23
        || minute > 59
    {
        return None;
    }
    Some(Date { year: 2000 + year as u16, month, day, hour, minute })
}

const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];

pub fn month_name(month: u8) -> &'static str {
    if month >= 1 && month <= 12 {
        MONTHS[(month - 1) as usize]
    } else {
        "Unknown"
    }
}
