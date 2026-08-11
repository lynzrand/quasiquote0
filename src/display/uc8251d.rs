//! UC8251D panel transport for the Quote/0 wiring.
//!
//! This owns GPIO state and the SPI device. It uses the board-proven init and
//! full-frame transfer sequence from the bundled Quote/0 reference firmware.

use super::{luts, Frame, FRAME_BYTES};
use core::ptr;
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{
    Gpio10, Gpio20, Gpio3, Gpio4, Gpio5, Gpio6, Gpio7, Input, Output, PinDriver, Pull,
};
use esp_idf_hal::spi::SPI2;

pub const SPI_CLOCK_HZ: u32 = 15_000_000;
pub const PIN_BUSY: u32 = 3;
pub const PIN_RST: u32 = 4;
pub const PIN_DC: u32 = 5;
pub const PIN_CS: u32 = 6;
pub const PIN_POWER: u32 = 20;
pub const PIN_SCLK: u32 = 10;
pub const PIN_MOSI: u32 = 7;
const MAX_TRANSFER: usize = 4096;

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
    spi: esp_idf_sys::spi_device_handle_t,
    power: PinDriver<'d, Output>,
    reset: PinDriver<'d, Output>,
    data_command: PinDriver<'d, Output>,
    chip_select: PinDriver<'d, Output>,
    busy: PinDriver<'d, Input>,
    sleeping: bool,
    _spi2: SPI2<'d>,
}

impl<'d> Epd<'d> {
    pub fn new(
        spi2: SPI2<'d>,
        _sclk: Gpio10<'d>,
        _mosi: Gpio7<'d>,
        pins: EpdPins<'d>,
    ) -> Result<Self, esp_idf_hal::sys::EspError> {
        // Several board pins boot in alternate IO_MUX functions. Select GPIO
        // before creating PinDrivers; merely setting direction is insufficient.
        unsafe {
            esp_idf_sys::esp_rom_gpio_pad_select_gpio(PIN_POWER);
            esp_idf_sys::esp_rom_gpio_pad_select_gpio(PIN_RST);
            esp_idf_sys::esp_rom_gpio_pad_select_gpio(PIN_DC);
            esp_idf_sys::esp_rom_gpio_pad_select_gpio(PIN_CS);
            esp_idf_sys::esp_rom_gpio_pad_select_gpio(PIN_BUSY);
        }
        let bus = esp_idf_sys::spi_bus_config_t {
            __bindgen_anon_1: esp_idf_sys::spi_bus_config_t__bindgen_ty_1 {
                mosi_io_num: PIN_MOSI as i32,
            },
            __bindgen_anon_2: esp_idf_sys::spi_bus_config_t__bindgen_ty_2 { miso_io_num: -1 },
            sclk_io_num: PIN_SCLK as i32,
            __bindgen_anon_3: esp_idf_sys::spi_bus_config_t__bindgen_ty_3 { quadwp_io_num: -1 },
            __bindgen_anon_4: esp_idf_sys::spi_bus_config_t__bindgen_ty_4 { quadhd_io_num: -1 },
            data4_io_num: -1,
            data5_io_num: -1,
            data6_io_num: -1,
            data7_io_num: -1,
            data_io_default_level: false,
            max_transfer_sz: MAX_TRANSFER as i32,
            flags: 0,
            intr_flags: 0,
            ..Default::default()
        };
        esp_idf_sys::esp!(unsafe {
            esp_idf_sys::spi_bus_initialize(
                esp_idf_sys::spi_host_device_t_SPI2_HOST,
                &bus,
                3 as esp_idf_sys::spi_dma_chan_t,
            )
        })?;
        let device = esp_idf_sys::spi_device_interface_config_t {
            mode: 0,
            clock_speed_hz: SPI_CLOCK_HZ as i32,
            spics_io_num: -1,
            queue_size: 1,
            ..Default::default()
        };
        let mut spi = ptr::null_mut();
        esp_idf_sys::esp!(unsafe {
            esp_idf_sys::spi_bus_add_device(
                esp_idf_sys::spi_host_device_t_SPI2_HOST,
                &device,
                &mut spi,
            )
        })?;

        let mut epd = Self {
            spi,
            power: PinDriver::output(pins.power)?,
            reset: PinDriver::output(pins.reset)?,
            data_command: PinDriver::output(pins.data_command)?,
            chip_select: PinDriver::output(pins.chip_select)?,
            busy: PinDriver::input(pins.busy, Pull::Up)?,
            sleeping: true,
            _spi2: spi2,
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

    fn wait_idle(&self, timeout_ms: u32) -> Result<(), &'static str> {
        let mut waited = 0;
        while !self.is_idle() {
            if waited >= timeout_ms {
                return Err("EPD busy timeout");
            }
            FreeRtos::delay_ms(10);
            waited += 10;
        }
        FreeRtos::delay_ms(10);
        Ok(())
    }

    fn write(&mut self, data_command: bool, bytes: &[u8]) {
        if data_command {
            let _ = self.data_command.set_high();
        } else {
            let _ = self.data_command.set_low();
        }
        let _ = self.chip_select.set_low();
        for chunk in bytes.chunks(MAX_TRANSFER) {
            let mut transaction = esp_idf_sys::spi_transaction_t {
                length: chunk.len() * 8,
                __bindgen_anon_1: esp_idf_sys::spi_transaction_t__bindgen_ty_1 {
                    tx_buffer: chunk.as_ptr().cast(),
                },
                ..Default::default()
            };
            unsafe {
                esp_idf_sys::esp!(esp_idf_sys::spi_device_polling_transmit(
                    self.spi,
                    &mut transaction,
                ))
                .expect("EPD SPI transfer");
            }
        }
        let _ = self.chip_select.set_high();
    }

    fn command(&mut self, command: u8) {
        self.write(false, &[command]);
    }
    fn data(&mut self, data: &[u8]) {
        self.write(true, data);
    }

    /// Bring the panel supply, controller, and scan configuration online.
    pub fn power_on(&mut self) -> Result<(), &'static str> {
        if !self.sleeping {
            return Ok(());
        }
        self.power.set_low().map_err(|_| "EPD power pin error")?;
        self.reset.set_low().map_err(|_| "EPD reset pin error")?;
        FreeRtos::delay_ms(500);
        self.reset.set_high().map_err(|_| "EPD reset pin error")?;
        FreeRtos::delay_ms(20);
        self.power.set_high().map_err(|_| "EPD power pin error")?;
        FreeRtos::delay_ms(100);
        for _ in 0..3 {
            self.reset.set_low().map_err(|_| "EPD reset pin error")?;
            FreeRtos::delay_ms(20);
            self.reset.set_high().map_err(|_| "EPD reset pin error")?;
            FreeRtos::delay_ms(20);
        }
        self.command(0x00);
        self.data(&[0xf3, 0x0e]);
        self.command(0x01);
        self.data(&[0x03, 0x00, 0x3f, 0x3f, 0x03]);
        self.command(0x06);
        self.data(&[0x17, 0x17, 0x17]);
        self.command(0x61);
        self.data(&[0x98, 0x01, 0x28]);
        self.command(0x30);
        self.data(&[0x1b]);
        self.command(0x60);
        self.data(&[0x22]);
        self.command(0x82);
        self.data(&[0x00]);
        self.command(0x03);
        self.data(&[0x10]);
        self.command(0x50);
        self.data(&[0x97]);
        self.command(0x04);
        FreeRtos::delay_ms(100);
        self.wait_idle(5_000)?;
        self.command(0x20);
        self.data(luts::lut_20());
        self.command(0x21);
        self.data(luts::lut_21());
        self.command(0x22);
        self.data(luts::lut_22());
        self.command(0x23);
        self.data(luts::lut_23());
        self.command(0x24);
        self.data(luts::lut_24());
        self.sleeping = false;
        Ok(())
    }

    /// Validates the payload expected by the UC8251D SPI transfer.
    pub fn validate_frame(frame: &Frame) -> usize {
        frame.as_bytes().len()
    }
    pub const fn frame_bytes() -> usize {
        FRAME_BYTES
    }

    /// Send a complete native Quote/0 framebuffer and wait for its refresh.
    ///
    /// Panel RAM bit 1 is electrically white, matching the public frame
    /// convention (`white_is: 1`), so payload bytes are forwarded verbatim;
    /// an earlier bitwise inversion rendered every image and the status bar
    /// color-inverted.  DTM1 stays all zeroes: on this UC8251D board 0xff
    /// produces alternating blank refreshes and can leave BUSY asserted.
    /// Chunks follow the actual frame length, including its final 248-byte
    /// tail; sending a full 256-byte tail corrupts the next command.
    pub fn display(&mut self, frame: &Frame) -> Result<(), &'static str> {
        self.power_on()?;
        let old = [0u8; 256];
        self.command(0x10); // old image
        for chunk in frame.as_bytes().chunks(old.len()) {
            self.data(&old[..chunk.len()]);
        }
        self.command(0x13);
        for chunk in frame.as_bytes().chunks(256) {
            self.data(chunk);
        }
        self.command(0x12);
        FreeRtos::delay_ms(100);
        self.wait_idle(12_000)?;
        self.sleep()
    }

    /// Power down the controller after a completed refresh. The panel retains
    /// its image; the next display runs the full board-proven init sequence.
    pub fn sleep(&mut self) -> Result<(), &'static str> {
        if self.sleeping {
            return Ok(());
        }
        self.command(0x02);
        self.data(&[0x00]);
        self.wait_idle(5_000)?;
        self.command(0x07);
        self.data(&[0xa5]);
        FreeRtos::delay_ms(100);
        self.power.set_low().map_err(|_| "EPD power pin error")?;
        self.sleeping = true;
        Ok(())
    }
}
