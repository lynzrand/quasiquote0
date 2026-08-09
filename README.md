# quasiquote0

Reusable hardware-facing building blocks for the MindReset Quote/0 board.
There is intentionally no firmware application or binary target yet.

| Module | Scope |
| --- | --- |
| `wifi` | ESP-IDF Wi-Fi station construction, configuration, and link control |
| `bluetooth` | BTHome v2 advertising payload encoding; application owns NimBLE lifecycle |
| `usb_serial` | exclusive USB Serial/JTAG transport on GPIO18/19 |
| `display` | native 152x296, 1bpp UC8251D framebuffer and Quote/0 panel GPIO ownership |
| `embedded_graphics` | `embedded-graphics` `DrawTarget` over the native framebuffer |

## Board invariant

GPIO20 powers the EPD panel and is also UART0 TX on the ESP32-C3. The project
therefore configures ESP-IDF with no UART console. USB Serial/JTAG is the
supported host transport.

## Deliberate next boundary

`display::Epd` currently establishes the panel's GPIO, power/reset and BUSY
contracts, while `Frame` and the drawing adapter are fully usable on their
own. A future panel-transport change can add UC8251D SPI commands and the
verified stock LUTs without coupling them to the application protocol.
