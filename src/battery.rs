//! Quote/0 battery and USB-VBUS sensing (GPIO1 ADC, GPIO0 VBUS).

const SAMPLES: usize = 20;
const EMPTY_MV: u32 = 2910;
const FULL_MV: u32 = 4200;

#[derive(Clone, Copy, Debug)]
pub struct BatteryStatus {
    pub millivolts: Option<u32>,
    pub percent: Option<u32>,
    pub usb_present: bool,
}

pub fn read() -> BatteryStatus {
    use esp_idf_sys::*;
    let config = adc_oneshot_unit_init_cfg_t {
        unit_id: adc_unit_t_ADC_UNIT_1,
        clk_src: soc_periph_adc_digi_clk_src_t_ADC_DIGI_CLK_SRC_DEFAULT as adc_oneshot_clk_src_t,
        ..Default::default()
    };
    let mut unit = core::ptr::null_mut();
    let millivolts = unsafe {
        if esp!(adc_oneshot_new_unit(&config, &mut unit)).is_err() {
            None
        } else {
            let channel = adc_channel_t_ADC_CHANNEL_1;
            let channel_config = adc_oneshot_chan_cfg_t {
                atten: adc_atten_t_ADC_ATTEN_DB_11,
                bitwidth: adc_bitwidth_t_ADC_BITWIDTH_12,
            };
            let mut samples = [0_i32; SAMPLES];
            let mut count = 0;
            if esp!(adc_oneshot_config_channel(unit, channel, &channel_config)).is_ok() {
                for sample in &mut samples {
                    let mut raw = 0;
                    if esp!(adc_oneshot_read(unit, channel, &mut raw)).is_ok() {
                        *sample = raw;
                        count += 1;
                    }
                }
            }
            let _ = esp!(adc_oneshot_del_unit(unit));
            if count < SAMPLES / 2 {
                None
            } else {
                samples[..count].sort_unstable();
                Some((samples[count / 2] as u32 * 3300 / 4095) * 2 + 29)
            }
        }
    };
    let percent =
        millivolts.map(|mv| ((mv.saturating_sub(EMPTY_MV) * 100) / (FULL_MV - EMPTY_MV)).min(100));
    BatteryStatus {
        millivolts,
        percent,
        usb_present: vbus_present() || unsafe { esp_idf_sys::usb_serial_jtag_is_connected() },
    }
}

pub fn vbus_present() -> bool {
    // Release any pin hold left by deep sleep before reading VBUS again.
    static SETUP: std::sync::Once = std::sync::Once::new();
    SETUP.call_once(|| unsafe {
        esp_idf_sys::gpio_hold_dis(0);
        esp_idf_sys::esp_rom_gpio_pad_select_gpio(0);
        esp_idf_sys::gpio_set_direction(0, esp_idf_sys::gpio_mode_t_GPIO_MODE_INPUT);
    });
    unsafe { esp_idf_sys::gpio_get_level(0) != 0 }
}
