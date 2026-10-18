# quasiquote0

Flashable, cloud-free frame-client firmware for the MindReset Quote/0 board
(ESP32-C3 driving a UC8251D e-paper panel).

The protocol and durability contract are documented in
[`docs/PROPOSAL.md`](docs/PROPOSAL.md), and the wire, Wi-Fi, and Bluetooth
protocol reference is [`docs/PROTOCOL.md`](docs/PROTOCOL.md). The firmware
owns Wi-Fi pulling, LittleFS persistence, panel refreshes, USB
configuration, and streamed OTA; servers supply only canonical 296x152
monochrome frames.

## Clock bring-up firmware

`quote0-clock` is the current standalone clock application. It replaces the
displayed image with a large landscape `HH:MM` clock in Hong Kong time
(UTC+8), refreshing only when the minute changes. This binary starts USB
control before the panel and does not start Wi-Fi, BLE, or the filesystem.
Saved frame-client configuration and frames remain available for later use.

Build it with `cargo build --locked --bin quote0-clock`. Convert the ELF for
the recovered 4 MiB/DIO/40 MHz setup:

```console
mkdir -p target/clock-flash
.embuild/espressif/python_env/idf5.5_py3.14_env/bin/python -m esptool \
  --chip esp32c3 elf2image --flash_mode dio --flash_freq 40m --flash_size 4MB \
  --elf-sha256-offset 0xb0 -o target/clock-flash/quote0-clock.bin \
  target/riscv32imc-esp-espidf/debug/quote0-clock
```

With the recovered bootloader and partition layout already installed, run
`tools/flash_clock_windows.ps1` in Windows PowerShell. It flashes and verifies
only `ota_0`, synchronizes UTC from the Windows clock, and checks the display.
The Windows tools can read the build over `\\wsl.localhost\Arch\...`; USB
forwarding into WSL is unnecessary.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/clock_usb.ps1 -SyncTime
powershell -NoProfile -ExecutionPolicy Bypass -File tools/clock_usb.ps1
```

RTC time survives deep sleep. A complete loss of power requires another USB
time synchronization; until then the face shows `--:--`. On battery, normal
sleep ends at the next minute boundary or USB insertion. A critically low
cell uses the longer protective sleep interval. USB always keeps the CPU awake.

Host tests for clock boundaries, framebuffer projection, configuration, and
power policy can run independently of ESP-IDF:

```console
cd /tmp
cargo +stable test --manifest-path /path/to/quasiquote0/tests/host/Cargo.toml \
  --target x86_64-unknown-linux-gnu --target-dir /path/to/quasiquote0/target/host-tests
```

## Status

Verified on hardware:

- Yamux control session over USB Serial/JTAG with newline-delimited JSON-RPC
- Streamed, digest-verified frame uploads with operation progress tracking
- Panel refresh including the device-drawn status bar; frames persist in
  LittleFS and are redrawn automatically after reboot or power loss
- USB-visible recovery mode for damaged filesystem state
- Battery management: VBUS-gated wake, deep-sleep timer cycling with one
  conditional pull per wake, critical-battery protection
- BLE BTHome v2 beacon (battery/link telemetry) while awake

Implemented, not yet exercised against real infrastructure: scheduled HTTP
pulling and streamed USB OTA. A connectable GATT push channel is specified
in [`docs/PROTOCOL.md`](docs/PROTOCOL.md) but not implemented yet.

### NFC

The product's tap-to-open feature is a passive NFC tag inside the case,
factory-programmed with the device's deep link. It has no electrical
connection to the ESP32-C3 — stock firmware reverse engineering and an
I2C sweep of every usable GPIO pair found no NFC or I2C device — so the
firmware cannot read or write it. Details and options live in
[`docs/PROTOCOL.md`](docs/PROTOCOL.md#4-nfc-hardware-finding-closed).

## Requirements

- ESP Rust toolchain (`rustup` with `espup`; `rust-toolchain.toml` pins the
  nightly and `rust-src`)
- [`ldproxy`](https://crates.io/crates/ldproxy) (ESP-IDF linker frontend)
- [`espflash`](https://crates.io/crates/espflash) for flashing
- Python 3 with `pyserial`, only for the host client below

## Building and flashing

```console
cargo build
espflash flash --port /dev/ttyACM0 \
  --bootloader "$(ls -t target/riscv32imc-esp-espidf/debug/build/esp-idf-sys-*/out/build/bootloader/bootloader.bin | head -1)" \
  --partition-table "$(ls -t target/riscv32imc-esp-espidf/debug/build/esp-idf-sys-*/out/build/partition_table/partition-table.bin | head -1)" \
  target/riscv32imc-esp-espidf/debug/quasiquote0
```

When migrating from the stock firmware or anything else, erase first with
`espflash erase-flash --port /dev/ttyACM0`; the device formats its LittleFS
volume on first boot. A blank volume boots unconfigured but fully reachable
over USB.

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

USB power always keeps the MCU awake. Both GPIO0 VBUS sensing and USB host
activity guard against sleep, including when old configuration contains
`stay_awake_on_usb: false`. Cold boots and GPIO wakes allow 60 seconds for
recovery; timer wakes allow two seconds for enumeration. Battery sleep is
allowed only after both a timer and GPIO0-high USB-insertion wake source
are successfully armed. Wake-source errors keep the device awake.

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

### Catching a sleeping device on Windows

If the device sleeps before you can open its USB port, run the Windows watcher:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/wake_keep_awake.ps1 -Port COM4
```

It retries every 50 ms for up to 24 hours, immediately sends a minimal
configuration with `power.stay_awake_on_usb: true`, then verifies the saved
setting and checks that the device still responds with USB power detected.
**This replaces the existing configuration and resets Wi-Fi and pull settings.**
It uses Windows' built-in serial API and needs no Python or WSL USB forwarding.
Use `-LogPath <file>` to save progress, `-TimeoutSeconds <seconds>` to change
the watch duration, or `-SelfTest` to check the protocol without opening USB.
The script exits after success; press Ctrl+C to stop a foreground watcher.

## License

MIT — see [LICENSE](LICENSE).

## AI-assisted development

This firmware is developed with the help of an LLM coding agent; commits
tagged `(AI)` were authored by such an agent under human direction and
review. Hardware-facing claims (panel polarity, partition layout, boot
behavior) are validated against a real Quote/0 board before landing.
