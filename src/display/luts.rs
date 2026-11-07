//! Stock UC8251D waveform reconstructed from the Quote/0 firmware.
//!
//! The register ranges are deliberately non-uniform; do not merge or reshape
//! these slices. Their layout is part of the panel's proven refresh sequence.
//!
//! UC8251d A0.1 sections 14-18 define eight bytes per group: group repeats,
//! four packed levels, two frame counts and a repeat for each of two states.
//! See docs/WAVEFORM.md for the decoded drive sequences and source reference.
//! The original stock bytes remain in luts_data.rs as the test reference.

#[derive(Clone, Copy)]
#[repr(u8)]
enum Level {
    // The same code selects 0 V in pixel LUTs and VCOM_DC in the VCOM LUT.
    Reference = 0,
    High = 1,
    Low = 2,
}

struct Timing {
    repeats: u8,
    // State 1 uses phases 0/1; state 2 uses phases 2/3.
    frames: [u8; 4],
    state_repeats: [u8; 2],
}

// All five stock tables use these timings. A group runs both repeated states
// before repeating the whole group; the first group therefore lasts 20 frames.
const TIMING: [Timing; 4] = [
    Timing {
        repeats: 5,
        frames: [1, 1, 1, 1],
        state_repeats: [1, 1],
    },
    Timing {
        repeats: 1,
        frames: [14, 14, 1, 1],
        state_repeats: [1, 1],
    },
    Timing {
        repeats: 1,
        frames: [24, 24, 14, 14],
        state_repeats: [1, 1],
    },
    Timing {
        repeats: 1,
        frames: [35, 35, 1, 1],
        state_repeats: [1, 1],
    },
];

use Level::{High, Low, Reference};

// White-to-white and black-to-white share one drive pattern in the stock LUT.
const WHITE_LEVELS: [[Level; 4]; 4] = [
    [Low, High, Reference, Reference],
    [High, Reference, Reference, Reference],
    [Low, High, Reference, Reference],
    [Low, Reference, Reference, Reference],
];

// White-to-black and black-to-black also share their drive pattern.
const BLACK_LEVELS: [[Level; 4]; 4] = [
    [Low, High, Reference, Reference],
    [Reference, Low, Reference, Reference],
    [Reference, High, Low, Reference],
    [Reference, High, Reference, Reference],
];

/// Encode the stock groups into a controller table, preserving its zero tail.
/// A zero group-repeat byte terminates the sequence; those bytes are not idle
/// phases. These const calls do all encoding at compile time, without a buffer
/// or extra stack allocation in the panel's refresh path.
const fn encode(levels: [[Level; 4]; 4]) -> [u8; 80] {
    let mut bytes = [0; 80];
    let mut group = 0;
    while group < TIMING.len() {
        let timing = &TIMING[group];
        let packed = (levels[group][0] as u8) << 6
            | (levels[group][1] as u8) << 4
            | (levels[group][2] as u8) << 2
            | levels[group][3] as u8;
        let offset = group * 8;
        bytes[offset] = timing.repeats;
        bytes[offset + 1] = packed;
        bytes[offset + 2] = timing.frames[0];
        bytes[offset + 3] = timing.frames[1];
        bytes[offset + 4] = timing.state_repeats[0];
        bytes[offset + 5] = timing.frames[2];
        bytes[offset + 6] = timing.frames[3];
        bytes[offset + 7] = timing.state_repeats[1];
        group += 1;
    }
    bytes
}

const VCOM: [u8; 80] = encode([[Reference; 4]; 4]);
const WHITE: [u8; 80] = encode(WHITE_LEVELS);
const BLACK: [u8; 80] = encode(BLACK_LEVELS);

pub fn lut_20() -> &'static [u8] {
    &VCOM
}
pub fn lut_21() -> &'static [u8] {
    // R21 has seven groups; the other registers have ten.
    &WHITE[..56]
}
pub fn lut_22() -> &'static [u8] {
    &WHITE
}
pub fn lut_23() -> &'static [u8] {
    &BLACK
}
pub fn lut_24() -> &'static [u8] {
    &BLACK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_register_tables_match_the_stock_firmware_capture() {
        // Preserve the working waveform while replacing opaque constants.
        // Captured at VA 0x3c0f37a4 in the original firmware; SHA-256 of the
        // 376 transmitted bytes is 03bbfbf8ebf77e17b633b5c7515d2b112b7bd203a4ebcb415d7182ba01dec34e.
        let stock: [u8; 400] = include!("luts_data.rs");
        let tables = [lut_20(), lut_21(), lut_22(), lut_23(), lut_24()];
        let mut offset = 0;
        for (index, (table, length)) in tables.iter().zip([80, 56, 80, 80, 80]).enumerate() {
            assert_eq!(table.len(), length, "register 0x{:02x}", 0x20 + index);
            assert_eq!(
                *table,
                &stock[offset..offset + length],
                "register 0x{:02x}",
                0x20 + index
            );
            offset += length;
        }
        // The last 24 captured bytes are adjacent firmware data, not LUT data.
        assert_eq!(offset, 376);
    }

    #[test]
    fn decoded_stock_timings_match_the_reconstructed_sequence() {
        let frames = TIMING.each_ref().map(|group| {
            u32::from(group.repeats)
                * ((u32::from(group.frames[0]) + u32::from(group.frames[1]))
                    * u32::from(group.state_repeats[0])
                    + (u32::from(group.frames[2]) + u32::from(group.frames[3]))
                        * u32::from(group.state_repeats[1]))
        });
        assert_eq!(frames, [20, 30, 76, 72]);
        // The documented two-frame ending makes 200 frames at nominal 160 Hz.
        assert_eq!((frames.iter().sum::<u32>() + 2) * 1_000 / 160, 1_250);
    }
}
