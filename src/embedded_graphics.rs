//! `embedded-graphics` adapter for the landscape Quote/0 framebuffer.

use core::convert::Infallible;
use embedded_graphics::draw_target::DrawTarget;
use embedded_graphics::geometry::{OriginDimensions, Size};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::Pixel;

/// Draw in the user-visible landscape coordinates before controller projection.
pub struct LandscapeFramebuffer<'a>(pub &'a mut crate::frame::Frame);

impl DrawTarget for LandscapeFramebuffer<'_> {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x >= 0 && point.y >= 0 {
                self.0
                    .set_pixel(point.x as usize, point.y as usize, color == BinaryColor::On);
            }
        }
        Ok(())
    }
}

impl OriginDimensions for LandscapeFramebuffer<'_> {
    fn size(&self) -> Size {
        Size::new(crate::frame::WIDTH as u32, crate::frame::HEIGHT as u32)
    }
}
