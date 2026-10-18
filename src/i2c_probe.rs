//! Runtime I2C bus probing for hardware bring-up.
//!
//! The Quote/0 pin map leaves a handful of ESP32-C3 GPIOs unallocated, and no
//! stock-firmware driver documents an external I2C bus. This module lets a
//! host sweep arbitrary SDA/SCL pairs over the control RPC and report which
//! 7-bit addresses acknowledge, so peripheral chips (for example a passive
//! NFC tag IC) can be located without a logic analyzer.

use esp_idf_sys::*;

/// Probe every 7-bit address on one pin pair and return those that ACK a
/// one-byte read. The driver is uninstalled afterwards; the pins are left
/// floating rather than restored to any previous function.
pub fn scan(sda: i32, scl: i32) -> Result<Vec<u8>, String> {
    unsafe {
        let mut config: i2c_config_t = std::mem::zeroed();
        config.mode = i2c_mode_t_I2C_MODE_MASTER;
        config.sda_io_num = sda;
        config.scl_io_num = scl;
        config.sda_pullup_en = true;
        config.scl_pullup_en = true;
        config.__bindgen_anon_1.master.clk_speed = 100_000;
        esp!(i2c_param_config(0, &config)).map_err(|error| error.to_string())?;
        esp!(i2c_driver_install(
            0,
            i2c_mode_t_I2C_MODE_MASTER,
            0,
            0,
            0
        ))
        .map_err(|error| error.to_string())?;

        let mut devices = Vec::new();
        for address in 1u8..=126 {
            let mut byte = [0u8];
            // A short timeout keeps floating-pin sweeps fast: unconnected
            // buses NACK immediately, and only a line held low blocks.
            let status = i2c_master_read_from_device(0, address, byte.as_mut_ptr(), 1, 2);
            if status == ESP_OK as i32 {
                devices.push(address);
            }
        }
        i2c_driver_delete(0);
        Ok(devices)
    }
}
