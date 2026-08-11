# quasiquote0

Flashable, cloud-free frame-client firmware for the MindReset Quote/0 board.

The protocol and durability contract are documented in
[`docs/PROPOSAL.md`](docs/PROPOSAL.md). The firmware owns Wi-Fi pulling,
LittleFS persistence, panel refreshes, USB configuration, and streamed OTA;
servers supply only canonical 296x152 monochrome frames.

| Module | Scope |
| --- | --- |
| `wifi` | ESP-IDF Wi-Fi station construction, configuration, and link control |
| `bluetooth` | BTHome v2 advertising payload encoding; application owns NimBLE lifecycle |
| `usb_serial` | exclusive USB Serial/JTAG transport on GPIO18/19 |
| `display` | native 152x296, 1bpp UC8251D framebuffer and Quote/0 panel GPIO ownership |
| `embedded_graphics` | `embedded-graphics` `DrawTarget` over the native framebuffer |

The firmware starts a blocking Yamux session over USB Serial/JTAG before it
initializes optional radio and panel services. Quote/0's stream-0 extension
carries newline-delimited JSON-RPC; direct frame and OTA transfers use
device-selected streams.
Each blocking stream has a 4 KiB receive queue by default, and the whole
connection has a 4 KiB outbound queue. Readers, writers, and the USB driver
task park on condition variables at those bounds, so Yamux's protocol-level
256 KiB initial credit never implies a 256 KiB allocation on the device.

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

## USB client

`tools/quasiquote0_usb.py` is a small host client for the stream-0 extension.
It needs `pyserial` and does not use the USB byte stream for logging:

```console
python3 -m pip install pyserial
python3 tools/quasiquote0_usb.py --port /dev/ttyACM0 hello
python3 tools/quasiquote0_usb.py --port /dev/ttyACM0 config-set config.json
python3 tools/quasiquote0_usb.py --port /dev/ttyACM0 frame-put frame.bin
python3 tools/quasiquote0_usb.py --port /dev/ttyACM0 operation-status 1
```

`frame.bin` is exactly 5,624 bytes in the documented native landscape format.
