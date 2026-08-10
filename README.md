# quasiquote0

Reusable hardware-facing building blocks and a minimal status firmware for the
MindReset Quote/0 board.

| Module | Scope |
| --- | --- |
| `wifi` | ESP-IDF Wi-Fi station construction, configuration, and link control |
| `bluetooth` | BTHome v2 advertising payload encoding; application owns NimBLE lifecycle |
| `usb_serial` | exclusive USB Serial/JTAG transport on GPIO18/19 |
| `display` | native 152x296, 1bpp UC8251D framebuffer and Quote/0 panel GPIO ownership |
| `embedded_graphics` | `embedded-graphics` `DrawTarget` over the native framebuffer |

The default firmware renders the status screen once at boot, then creates a
blocking Yamux session over USB Serial/JTAG. Quote/0's stream-0 extension
carries newline-delimited JSON-RPC; `system.ping` is currently implemented.

The default binary refreshes the EPD every ten seconds with Wi-Fi link, NimBLE
host, USB-power, and battery ADC state. It does not configure Wi-Fi credentials
or provide an application protocol.

## Board invariant

GPIO20 powers the EPD panel and is also UART0 TX on the ESP32-C3. The project
therefore configures ESP-IDF with no UART console. USB Serial/JTAG is the
supported host transport.

## Flash layout

The 4 MiB device uses the minimum 12 KiB NVS partition, OTA metadata, `ota_0`
and `ota_1` (1.875 MiB each), then a 192 KiB LittleFS volume labelled
`littlefs`.
`partitions.csv` is copied into ESP-IDF's generated project through the Cargo
environment configuration.

## Deliberate next boundary

`display::Epd` uses the reference board's full-frame UC8251D transfer sequence.
The panel's stock waveform tables can be added later without coupling them to
the status application.
