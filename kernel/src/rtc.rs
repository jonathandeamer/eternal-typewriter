use x86_64::instructions::port::Port;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    pub year: u16,
    pub month: u8, // 1–12
    pub day: u8,   // 1–31
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

fn read_raw() -> (u8, u8, u8) {
    (read_register(0x07), read_register(0x08), read_register(0x09)) // day, month, year
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

    let binary_mode = read_register(0x0B) & 0x04 != 0;
    let decode = |v: u8| -> u8 {
        if binary_mode {
            v
        } else {
            (v >> 4) * 10 + (v & 0x0F)
        }
    };
    let (day, month, year) = (decode(raw.0), decode(raw.1), decode(raw.2));
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) || year > 99 {
        return None;
    }
    Some(Date { year: 2000 + year as u16, month, day })
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
