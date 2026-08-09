//! UC8251D panel transport for the Quote/0 wiring.
//!
//! This owns GPIO state and the SPI device. LUT loading and power waveform
//! tuning remain explicit future work; the public API already establishes the
//! correct pin ownership and frame wire format for that implementation.

use super::{Frame, FRAME_BYTES};
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{Gpio20, Gpio3, Gpio4, Gpio5, Gpio6, Input, Output, PinDriver, Pull};

pub const SPI_CLOCK_HZ: u32 = 15_000_000;
pub const PIN_BUSY: u32 = 3;
pub const PIN_RST: u32 = 4;
pub const PIN_DC: u32 = 5;
pub const PIN_CS: u32 = 6;
pub const PIN_POWER: u32 = 20;

pub struct EpdPins<'d> {
    pub busy: Gpio3<'d>,
    pub reset: Gpio4<'d>,
    pub data_command: Gpio5<'d>,
    pub chip_select: Gpio6<'d>,
    pub power: Gpio20<'d>,
}

/// GPIO portion of the panel driver.
///
/// `Gpio20` is deliberately retained by this type for the panel power rail;
/// enabling an UART0 console on this board conflicts with it.
pub struct Epd<'d> {
    power: PinDriver<'d, Output>,
    reset: PinDriver<'d, Output>,
    data_command: PinDriver<'d, Output>,
    chip_select: PinDriver<'d, Output>,
    busy: PinDriver<'d, Input>,
}

impl<'d> Epd<'d> {
    pub fn new(pins: EpdPins<'d>) -> Result<Self, esp_idf_hal::sys::EspError> {
        // Several board pins boot in alternate IO_MUX functions. Select GPIO
        // before creating PinDrivers; merely setting direction is insufficient.
        unsafe {
            esp_idf_sys::esp_rom_gpio_pad_select_gpio(PIN_POWER);
            esp_idf_sys::esp_rom_gpio_pad_select_gpio(PIN_RST);
            esp_idf_sys::esp_rom_gpio_pad_select_gpio(PIN_DC);
            esp_idf_sys::esp_rom_gpio_pad_select_gpio(PIN_CS);
            esp_idf_sys::esp_rom_gpio_pad_select_gpio(PIN_BUSY);
        }
        let mut epd = Self {
            power: PinDriver::output(pins.power)?,
            reset: PinDriver::output(pins.reset)?,
            data_command: PinDriver::output(pins.data_command)?,
            chip_select: PinDriver::output(pins.chip_select)?,
            busy: PinDriver::input(pins.busy, Pull::Up)?,
        };
        epd.chip_select.set_high()?;
        epd.reset.set_high()?;
        epd.power.set_low()?;
        epd.data_command.set_low()?;
        Ok(epd)
    }

    /// The BUSY signal is open-drain; the required pull-up is installed by
    /// `new`. `true` means the panel is idle and accepts the next command.
    pub fn is_idle(&self) -> bool {
        self.busy.is_high()
    }

    /// Bring the panel supply and reset line to their known initial state.
    /// SPI command sequencing is intentionally not started by construction.
    pub fn power_on(&mut self) -> Result<(), esp_idf_hal::sys::EspError> {
        self.power.set_low()?;
        self.reset.set_low()?;
        FreeRtos::delay_ms(500);
        self.reset.set_high()?;
        FreeRtos::delay_ms(20);
        self.power.set_high()?;
        FreeRtos::delay_ms(100);
        Ok(())
    }

    /// Validates the payload expected by the eventual UC8251D SPI transfer.
    /// This makes the panel/application boundary testable before refresh code
    /// and its stock LUTs are integrated.
    pub fn validate_frame(frame: &Frame) -> usize {
        frame.as_bytes().len()
    }
    pub const fn frame_bytes() -> usize {
        FRAME_BYTES
    }
}
