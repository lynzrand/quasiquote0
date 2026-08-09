//! Native Quote/0 framebuffer: 152 x 296, MSB-first, one bit per pixel.
//!
//! The verified panel polarity is `1 = white`, `0 = black`.

pub const WIDTH: usize = 152;
pub const HEIGHT: usize = 296;
pub const ROW_BYTES: usize = WIDTH / 8;
pub const FRAME_BYTES: usize = ROW_BYTES * HEIGHT;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameError {
    WrongLength,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Frame {
    bytes: [u8; FRAME_BYTES],
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
    pub fn as_bytes_mut(&mut self) -> &mut [u8; FRAME_BYTES] {
        &mut self.bytes
    }
    pub fn clear(&mut self, white: bool) {
        self.bytes.fill(if white { 0xff } else { 0 });
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
}

impl Default for Frame {
    fn default() -> Self {
        Self::white()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pixel_order_is_msb_first() {
        let mut frame = Frame::black();
        frame.set_pixel(0, 0, true);
        frame.set_pixel(151, 295, true);
        assert_eq!(frame.as_bytes()[0], 0x80);
        assert_eq!(frame.as_bytes()[FRAME_BYTES - 1], 0x01);
    }
}
