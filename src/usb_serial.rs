//! Exclusive USB Serial/JTAG transport (GPIO18 = D-, GPIO19 = D+).
//!
//! Keep one `UsbSerial` owner in an application. In particular, do not mix
//! logging and a framed application protocol on this stream.

use esp_idf_hal::delay::BLOCK;
use esp_idf_hal::gpio::{Gpio18, Gpio19};
use esp_idf_hal::sys::EspError;
use esp_idf_hal::usb_serial::{UsbSerialConfig, UsbSerialDriver, USB_SERIAL};

pub struct UsbSerial<'d> {
    inner: UsbSerialDriver<'d>,
}

impl<'d> UsbSerial<'d> {
    pub fn new(port: USB_SERIAL<'d>, dm: Gpio18<'d>, dp: Gpio19<'d>) -> Result<Self, EspError> {
        // The hardware receives 64-byte endpoint packets, while ESP-IDF's
        // byte-ring driver requires an RX buffer *larger* than one packet.
        let config = UsbSerialConfig::new()
            .tx_buffer_size(2048)
            .rx_buffer_size(2048);
        Ok(Self {
            inner: UsbSerialDriver::new(port, dm, dp, &config)?,
        })
    }

    pub fn read(&mut self, buffer: &mut [u8], timeout_ms: u32) -> Result<usize, EspError> {
        self.inner.read(buffer, timeout_ms)
    }
    pub fn write_all(&mut self, bytes: &[u8]) -> Result<(), EspError> {
        let mut sent = 0;
        while sent < bytes.len() {
            sent += self.inner.write(&bytes[sent..], BLOCK)?;
        }
        Ok(())
    }
}
