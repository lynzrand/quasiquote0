//! UC8251D status-strip window in controller coordinates.
//!
//! The calibrated clockwise projection maps landscape columns 280..295 to
//! native rows 15..0. Each row is 19 bytes, so this window is exactly 304 bytes.
use super::{Frame, ROW_BYTES, WIDTH};

pub const ROWS: usize = 16;
pub const BYTES: usize = ROW_BYTES * ROWS;

// 0x90: horizontal start/end (byte aligned), vertical start/end (inclusive),
// PT_SCAN=0 to scan gates only inside the selected window.
pub const COMMAND: [u8; 7] = [0, (WIDTH - 1) as u8, 0, 0, 0, (ROWS - 1) as u8, 0];

pub fn plane(frame: &Frame) -> &[u8] {
    &frame.as_bytes()[..BYTES]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Frame as LandscapeFrame;

    #[test]
    fn status_window_contains_all_reserved_columns_and_no_content() {
        let mut landscape = LandscapeFrame::white();
        for y in 0..crate::frame::HEIGHT {
            for x in crate::status_bar::CONTENT_WIDTH..crate::frame::WIDTH {
                landscape.set_pixel(x, y, false);
            }
        }
        let native = landscape.project();
        assert_eq!(plane(&native).len(), 304);
        assert!(plane(&native).iter().all(|v| *v == 0));
        assert!(native.as_bytes()[BYTES..].iter().all(|v| *v == 0xff));

        let mut content_only = LandscapeFrame::white();
        content_only.set_pixel(crate::status_bar::CONTENT_WIDTH - 1, 0, false);
        assert!(plane(&content_only.project()).iter().all(|v| *v == 0xff));
    }
}
