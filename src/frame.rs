//! Canonical Quote/0 landscape framebuffer.
//!
//! The UC8251D controller exposes 152 x 296 portrait RAM, but Quote/0 is
//! normally viewed as a 296 x 152 landscape display. This type is the single
//! wire and persistent representation used by USB and HTTP. Projection to the
//! controller representation happens only at the EPD boundary.

use crate::display::Frame as ControllerFrame;
use sha2::{Digest, Sha256};

pub const WIDTH: usize = 296;
pub const HEIGHT: usize = 152;
pub const ROW_BYTES: usize = WIDTH / 8;
pub const FRAME_BYTES: usize = ROW_BYTES * HEIGHT;

pub type Sha256Digest = [u8; 32];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    WrongLength,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Frame {
    bytes: [u8; FRAME_BYTES],
}

/// The fixed mapping between the user-visible landscape canvas and UC8251D
/// portrait RAM. `Clockwise` matches the Quote/0 mount: landscape content
/// upright with the device-owned status bar along the viewer's right edge,
/// which corresponds to the first canvas columns before the content area.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Projection {
    Clockwise,
    CounterClockwise,
}

impl Default for Frame {
    fn default() -> Self {
        Self::white()
    }
}

impl Frame {
    pub const fn black() -> Self {
        Self {
            bytes: [0; FRAME_BYTES],
        }
    }

    pub const fn white() -> Self {
        Self {
            bytes: [0xff; FRAME_BYTES],
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.len() != FRAME_BYTES {
            return Err(FrameError::WrongLength);
        }
        let mut frame = Self::black();
        frame.bytes.copy_from_slice(bytes);
        Ok(frame)
    }

    pub fn as_bytes(&self) -> &[u8; FRAME_BYTES] {
        &self.bytes
    }

    pub fn sha256(&self) -> Sha256Digest {
        Sha256::digest(&self.bytes).into()
    }

    pub fn sha256_hex(&self) -> String {
        digest_hex(&self.sha256())
    }

    pub fn pixel_is_white(&self, x: usize, y: usize) -> Option<bool> {
        if x >= WIDTH || y >= HEIGHT {
            return None;
        }
        Some(self.bytes[y * ROW_BYTES + x / 8] & (0x80 >> (x % 8)) != 0)
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, white: bool) -> bool {
        if x >= WIDTH || y >= HEIGHT {
            return false;
        }
        let byte = &mut self.bytes[y * ROW_BYTES + x / 8];
        let mask = 0x80 >> (x % 8);
        if white {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
        true
    }

    /// Project this landscape frame into the UC8251D's portrait RAM order.
    pub fn project(&self, projection: Projection) -> ControllerFrame {
        let mut output = ControllerFrame::white();
        for source_y in 0..HEIGHT {
            for source_x in 0..WIDTH {
                let white = self
                    .pixel_is_white(source_x, source_y)
                    .expect("source coordinates are in range");
                let (controller_x, controller_y) = match projection {
                    // Calibrated against the mounted Quote/0 panel: the
                    // landscape canvas renders with its origin at the
                    // viewer's top-left (x rightward, y downward), so
                    // server-authored images are neither mirrored nor
                    // flipped. The reserved status columns sit at the
                    // canvas's right edge.
                    Projection::Clockwise => (source_y, WIDTH - 1 - source_x),
                    Projection::CounterClockwise => (HEIGHT - 1 - source_y, source_x),
                };
                output.set_pixel(controller_x, controller_y, white);
            }
        }
        output
    }
}

pub fn digest_hex(digest: &Sha256Digest) -> String {
    use core::fmt::Write;

    let mut output = String::with_capacity(64);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_is_the_exact_landscape_wire_size() {
        assert_eq!(ROW_BYTES, 37);
        assert_eq!(FRAME_BYTES, 5_624);
    }

    #[test]
    fn clockwise_projection_moves_landscape_corners_correctly() {
        let mut frame = Frame::black();
        frame.set_pixel(0, 0, true);
        frame.set_pixel(WIDTH - 1, HEIGHT - 1, true);

        let projected = frame.project(Projection::Clockwise);
        assert_eq!(projected.pixel_is_white(0, WIDTH - 1), Some(true));
        assert_eq!(projected.pixel_is_white(HEIGHT - 1, 0), Some(true));
    }

    #[test]
    fn digest_hex_is_stable() {
        let digest = Frame::white().sha256();
        assert_eq!(digest_hex(&digest).len(), 64);
        assert!(digest_hex(&digest)
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
    }
}
