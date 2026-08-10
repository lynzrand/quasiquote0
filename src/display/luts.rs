//! Stock UC8251D grayscale waveform table extracted from the Quote/0 firmware.
//!
//! The register ranges are deliberately non-uniform; do not merge or reshape
//! these slices. Their layout is part of the panel's proven refresh sequence.

pub const LUT_GRAY: [u8; 400] = include!("luts_data.rs");

pub fn lut_20() -> &'static [u8] {
    &LUT_GRAY[0..80]
}
pub fn lut_21() -> &'static [u8] {
    &LUT_GRAY[80..136]
}
pub fn lut_22() -> &'static [u8] {
    &LUT_GRAY[136..216]
}
pub fn lut_23() -> &'static [u8] {
    &LUT_GRAY[216..296]
}
pub fn lut_24() -> &'static [u8] {
    &LUT_GRAY[296..376]
}
