use crate::crc::crc32;

pub const SECTOR_SIZE: usize = 512;
pub const PAYLOAD_CAPACITY: usize = 494;
pub const MAGIC: [u8; 4] = *b"ETYP"; // doubles as format version 1

pub fn encode(sequence_number: u64, payload: &[u8]) -> [u8; SECTOR_SIZE] {
    assert!(payload.len() <= PAYLOAD_CAPACITY);
    let mut sector = [0u8; SECTOR_SIZE];
    sector[0..4].copy_from_slice(&MAGIC);
    sector[4..12].copy_from_slice(&sequence_number.to_le_bytes());
    sector[12..14].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    sector[14..18].copy_from_slice(&crc32(payload).to_le_bytes());
    sector[18..18 + payload.len()].copy_from_slice(payload);
    sector
}

/// Returns the payload if the sector holds a valid record for this LBA.
pub fn decode(sector: &[u8; SECTOR_SIZE], lba: u64) -> Option<&[u8]> {
    if sector[0..4] != MAGIC {
        return None;
    }
    let sequence_number = u64::from_le_bytes(sector[4..12].try_into().unwrap());
    if sequence_number != lba {
        return None;
    }
    let length = u16::from_le_bytes(sector[12..14].try_into().unwrap()) as usize;
    if length > PAYLOAD_CAPACITY {
        return None;
    }
    let crc = u32::from_le_bytes(sector[14..18].try_into().unwrap());
    let payload = &sector[18..18 + length];
    if crc32(payload) != crc {
        return None;
    }
    Some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let sector = encode(7, b"ink is forever");
        assert_eq!(decode(&sector, 7), Some(&b"ink is forever"[..]));
    }

    #[test]
    fn empty_payload_roundtrip() {
        let sector = encode(0, b"");
        assert_eq!(decode(&sector, 0), Some(&b""[..]));
    }

    #[test]
    fn full_payload_roundtrip() {
        let payload = [b'x'; PAYLOAD_CAPACITY];
        let sector = encode(3, &payload);
        assert_eq!(decode(&sector, 3), Some(&payload[..]));
    }

    #[test]
    fn zeroed_sector_is_invalid() {
        // A fresh scroll disk is all zeros; every sector must decode as None.
        assert_eq!(decode(&[0u8; SECTOR_SIZE], 0), None);
    }

    #[test]
    fn stale_record_at_wrong_lba_is_invalid() {
        // The LBA check stops stale records masquerading as the tail.
        let sector = encode(7, b"old prose");
        assert_eq!(decode(&sector, 8), None);
    }

    #[test]
    fn corrupted_payload_is_invalid() {
        // The CRC catches torn/rotted payloads behind an intact header.
        let mut sector = encode(2, b"about to rot");
        sector[20] ^= 0x01;
        assert_eq!(decode(&sector, 2), None);
    }

    #[test]
    fn oversized_length_is_invalid() {
        let mut sector = encode(0, b"hi");
        sector[12..14].copy_from_slice(&(PAYLOAD_CAPACITY as u16 + 1).to_le_bytes());
        assert_eq!(decode(&sector, 0), None);
    }

    #[test]
    #[should_panic]
    fn encode_rejects_oversized_payload() {
        encode(0, &[0u8; PAYLOAD_CAPACITY + 1]);
    }
}
