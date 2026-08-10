//! UC8251D E-paper panel driver and its native framebuffer.

mod frame;
mod luts;
mod uc8251d;

pub use frame::{Frame, FrameError, FRAME_BYTES, HEIGHT, ROW_BYTES, WIDTH};
pub use uc8251d::{Epd, EpdPins, SPI_CLOCK_HZ};
