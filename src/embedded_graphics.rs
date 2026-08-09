//! `embedded-graphics` adapter for the native Quote/0 framebuffer.

use crate::display::{Frame, HEIGHT, WIDTH};
use core::convert::Infallible;
use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::geometry::{OriginDimensions, Size};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::Pixel;

/// Draw directly into a native frame. `BinaryColor::On` maps to panel white.
pub struct Framebuffer<'a> {
    frame: &'a mut Frame,
}

impl<'a> Framebuffer<'a> {
    pub fn new(frame: &'a mut Frame) -> Self {
        Self { frame }
    }
    pub fn into_inner(self) -> &'a mut Frame {
        self.frame
    }
}

impl DrawTarget for Framebuffer<'_> {
    type Color = BinaryColor;
    type Error = Infallible;
    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x >= 0 && point.y >= 0 {
                self.frame
                    .set_pixel(point.x as usize, point.y as usize, color == BinaryColor::On);
            }
        }
        Ok(())
    }
}

impl OriginDimensions for Framebuffer<'_> {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}
