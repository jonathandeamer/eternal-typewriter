#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod crc;
pub mod layout;
pub mod record;

/// First byte of a boot-separator line. ASCII Record Separator: a C0
/// control character that can never be typed, so prose can't fake it.
pub const SEPARATOR_MARKER: u8 = 0x1E;
pub const SEPARATOR_MARKER_CHAR: char = '\u{1E}';
