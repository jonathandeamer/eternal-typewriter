/// CRC-32 (IEEE 802.3, the zlib polynomial). Bitwise — speed is irrelevant
/// at one 494-byte payload per keystroke, and zero tables keeps it obvious.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_standard_check_value() {
        // The canonical CRC-32 test vector; zlib.crc32 in the Python
        // extraction script must agree with this.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn empty_input() {
        assert_eq!(crc32(b""), 0);
    }
}
