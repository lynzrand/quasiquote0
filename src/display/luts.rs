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
const fn encode(timings: &[Timing], levels: &[[Level; 4]]) -> [u8; 80] {
    let mut bytes = [0; 80];
    let mut group = 0;
    while group < timings.len() {
        let timing = &timings[group];
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

const VCOM: [u8; 80] = encode(&TIMING, &[[Reference; 4]; 4]);
const WHITE: [u8; 80] = encode(&TIMING, &WHITE_LEVELS);
const BLACK: [u8; 80] = encode(&TIMING, &BLACK_LEVELS);

// Experimental direct write: one 48-frame pulse in the stock final-drive
// polarity, followed by three neutral frames. No erase/alternation groups.
// At 160 Hz, 51 programmed frames plus the two-frame ending take 331.25 ms.
#[cfg(feature = "experimental-fast")]
const FAST_TIMING: [Timing; 1] = [Timing {
    repeats: 1,
    frames: [48, 1, 1, 1],
    state_repeats: [1, 1],
}];
#[cfg(feature = "experimental-fast")]
const FAST_ZERO: [u8; 80] = encode(&FAST_TIMING, &[[Reference; 4]]);
#[cfg(feature = "experimental-fast")]
const FAST_WHITE: [u8; 80] = encode(&FAST_TIMING, &[[Low, Reference, Reference, Reference]]);
#[cfg(feature = "experimental-fast")]
const FAST_BLACK: [u8; 80] = encode(&FAST_TIMING, &[[High, Reference, Reference, Reference]]);

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

/// Zero source-drive levels for unchanged pixels, with the selected timings.
/// The bytes match VCOM's table, but selector 00 means 0 V for pixel outputs.
/// R21 takes the first 56 bytes; R24 takes all 80. Keeping the timing fields
/// avoids terminating these tables before the changed-pixel patterns finish.
#[cfg(feature = "experimental-differential")]
pub fn lut_unchanged() -> &'static [u8] {
    #[cfg(feature = "experimental-fast")]
    {
        &FAST_ZERO
    }
    #[cfg(not(feature = "experimental-fast"))]
    {
        &VCOM
    }
}

/// All five differential payloads must share timings, including VCOM and idle
/// pixels. Stock and fast profiles are kept distinct for hardware comparison.
#[cfg(feature = "experimental-differential")]
pub fn differential() -> [&'static [u8]; 5] {
    #[cfg(feature = "experimental-fast")]
    {
        [
            lut_unchanged(),
            &lut_unchanged()[..56],
            &FAST_WHITE,
            &FAST_BLACK,
            lut_unchanged(),
        ]
    }
    #[cfg(not(feature = "experimental-fast"))]
    {
        [
            lut_20(),
            &lut_unchanged()[..56],
            lut_22(),
            lut_23(),
            lut_unchanged(),
        ]
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "experimental-fast")]
    #[test]
    fn fast_profile_has_one_short_directional_pulse_and_synchronized_idle_tables() {
        let tables = differential();
        for (index, (table, length)) in tables.iter().zip([80, 56, 80, 80, 80]).enumerate() {
            assert_eq!(table.len(), length);
            let packed = match index {
                2 => 0x80,
                3 => 0x40,
                _ => 0,
            };
            assert_eq!(&table[..8], &[1, packed, 48, 1, 1, 1, 1, 1]);
            assert!(table[8..].iter().all(|&byte| byte == 0));
        }
    }

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
