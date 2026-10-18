#include "driver/gpio.h"
#include "driver/usb_serial_jtag.h"
#include "esp_ota_ops.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

void app_main(void)
{
    // GPIO20 is panel power, not a UART or LED. Leave the panel off.
    gpio_set_level(GPIO_NUM_20, 0);
    gpio_set_direction(GPIO_NUM_20, GPIO_MODE_OUTPUT);
    const usb_serial_jtag_driver_config_t usb = {
        .tx_buffer_size = 256,
        .rx_buffer_size = 256,
    };
    ESP_ERROR_CHECK(usb_serial_jtag_driver_install(&usb));
    // Remain bootable if the existing OTA bootloader selected this image.
    esp_ota_mark_app_valid_cancel_rollback();
    for (;;) {
        usb_serial_jtag_write_bytes("alive\n", 6, pdMS_TO_TICKS(100));
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
}
