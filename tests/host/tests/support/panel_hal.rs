//! Minimal GPIO/SPI doubles for running the production panel driver on a host.
#![allow(dead_code, non_camel_case_types, non_upper_case_globals)]

use std::cell::RefCell;

#[derive(Default)]
pub struct Panel {
    pub powered: bool,
    pub data: bool,
    pub command: u8,
    pub busy_at: Option<u8>,
    pub spi_error_once: Option<u8>,
    pub commands: Vec<u8>,
    pub payloads: Vec<(u8, Vec<u8>)>,
    pub elapsed_ms: u64,
    pub retained_composed: Option<[u8; 32]>,
    pub deep_sleep_wake: bool,
}
thread_local! {
    pub static PANEL: RefCell<Panel> = RefCell::new(Panel::default());
}

pub mod delay {
    pub struct FreeRtos;
    impl FreeRtos {
        pub fn delay_ms(milliseconds: u32) {
            super::PANEL.with_borrow_mut(|panel| panel.elapsed_ms += u64::from(milliseconds));
        }
    }
}
pub mod sys {
    pub type EspError = i32;
}
pub mod spi {
    pub struct SPI2<'d>(pub core::marker::PhantomData<&'d ()>);
}
pub mod gpio {
    use super::PANEL;
    use core::marker::PhantomData;
    pub struct Input;
    pub struct Output;
    pub enum Pull {
        Up,
    }
    pub trait Pin {
        const NUMBER: u32;
    }
    macro_rules! pins {
        ($($name:ident = $number:literal),*) => {$(
            pub struct $name<'d>(pub PhantomData<&'d ()>);
            impl Pin for $name<'_> { const NUMBER: u32 = $number; }
        )*};
    }
    pins!(
        Gpio3 = 3,
        Gpio4 = 4,
        Gpio5 = 5,
        Gpio6 = 6,
        Gpio7 = 7,
        Gpio10 = 10,
        Gpio20 = 20
    );

    pub struct PinDriver<'d, Mode> {
        number: u32,
        marker: PhantomData<(&'d (), Mode)>,
    }
    impl<'d> PinDriver<'d, Output> {
        pub fn output(pin: impl Pin) -> Result<Self, i32> {
            Ok(Self {
                number: pin_number(&pin),
                marker: PhantomData,
            })
        }
        pub fn set_low(&mut self) -> Result<(), i32> {
            PANEL.with_borrow_mut(|panel| match self.number {
                20 => panel.powered = false,
                5 => panel.data = false,
                _ => {}
            });
            Ok(())
        }
        pub fn set_high(&mut self) -> Result<(), i32> {
            PANEL.with_borrow_mut(|panel| match self.number {
                20 => panel.powered = true,
                5 => panel.data = true,
                _ => {}
            });
            Ok(())
        }
    }
    impl<'d> PinDriver<'d, Input> {
        pub fn input(pin: impl Pin, _: Pull) -> Result<Self, i32> {
            Ok(Self {
                number: pin_number(&pin),
                marker: PhantomData,
            })
        }
        pub fn is_high(&self) -> bool {
            PANEL.with_borrow(|panel| panel.busy_at != Some(panel.command))
        }
    }
    fn pin_number<T: Pin>(_: &T) -> u32 {
        T::NUMBER
    }
}

#[macro_export]
macro_rules! esp {
    ($call:expr) => {{
        let code = $call;
        if code == 0 {
            Ok(())
        } else {
            Err(code)
        }
    }};
}

// Keep the SDK's field names so the real transport can compile unchanged.
pub const ESP_OK: i32 = 0;
pub const spi_host_device_t_SPI2_HOST: i32 = 2;
pub type spi_dma_chan_t = i32;
pub type spi_device_handle_t = *mut core::ffi::c_void;
#[derive(Default)]
pub struct spi_bus_config_t__bindgen_ty_1 {
    pub mosi_io_num: i32,
}
#[derive(Default)]
pub struct spi_bus_config_t__bindgen_ty_2 {
    pub miso_io_num: i32,
}
#[derive(Default)]
pub struct spi_bus_config_t__bindgen_ty_3 {
    pub quadwp_io_num: i32,
}
#[derive(Default)]
pub struct spi_bus_config_t__bindgen_ty_4 {
    pub quadhd_io_num: i32,
}
#[derive(Default)]
pub struct spi_bus_config_t {
    pub __bindgen_anon_1: spi_bus_config_t__bindgen_ty_1,
    pub __bindgen_anon_2: spi_bus_config_t__bindgen_ty_2,
    pub __bindgen_anon_3: spi_bus_config_t__bindgen_ty_3,
    pub __bindgen_anon_4: spi_bus_config_t__bindgen_ty_4,
    pub sclk_io_num: i32,
    pub data4_io_num: i32,
    pub data5_io_num: i32,
    pub data6_io_num: i32,
    pub data7_io_num: i32,
    pub data_io_default_level: bool,
    pub max_transfer_sz: i32,
    pub flags: u32,
    pub intr_flags: i32,
    pub sdk_reserved: i32,
}
#[derive(Default)]
pub struct spi_device_interface_config_t {
    pub mode: u8,
    pub clock_speed_hz: i32,
    pub spics_io_num: i32,
    pub queue_size: i32,
    pub sdk_reserved: i32,
}
#[derive(Default)]
pub struct spi_transaction_t__bindgen_ty_1 {
    pub tx_buffer: *const core::ffi::c_void,
}
#[derive(Default)]
pub struct spi_transaction_t {
    pub length: usize,
    pub __bindgen_anon_1: spi_transaction_t__bindgen_ty_1,
    pub sdk_reserved: i32,
}
/// # Safety
/// This SDK-compatible double does not access hardware or dereference pointers.
pub unsafe fn esp_rom_gpio_pad_select_gpio(_: u32) {}
/// # Safety
/// This SDK-compatible double does not access hardware or dereference pointers.
pub unsafe fn spi_bus_initialize(_: i32, _: *const spi_bus_config_t, _: i32) -> i32 {
    0
}
/// # Safety
/// `handle` must point to writable storage for one device handle.
pub unsafe fn spi_bus_add_device(
    _: i32,
    _: *const spi_device_interface_config_t,
    handle: *mut spi_device_handle_t,
) -> i32 {
    *handle = core::ptr::dangling_mut();
    0
}
/// # Safety
/// `transfer` and its TX buffer must remain readable for the transaction.
pub unsafe fn spi_device_polling_transmit(
    _: spi_device_handle_t,
    transfer: *mut spi_transaction_t,
) -> i32 {
    PANEL.with_borrow_mut(|panel| {
        if !panel.data {
            let command = *((*transfer).__bindgen_anon_1.tx_buffer.cast::<u8>());
            if panel.spi_error_once == Some(command) {
                panel.spi_error_once = None;
                return -1;
            }
            panel.command = command;
            panel.commands.push(command);
            panel.payloads.push((command, Vec::new()));
        } else {
            let bytes = core::slice::from_raw_parts(
                (*transfer).__bindgen_anon_1.tx_buffer.cast::<u8>(),
                (*transfer).length / 8,
            );
            panel
                .payloads
                .last_mut()
                .unwrap()
                .1
                .extend_from_slice(bytes);
        }
        0
    })
}
