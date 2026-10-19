# quasiquote0

Flashable, cloud-free frame-client firmware for the MindReset Quote/0 board
(ESP32-C3 driving a UC8251D e-paper panel).

The current management, frame, and durability contracts are documented in
[`docs/PROTOCOL.md`](docs/PROTOCOL.md). Module ownership and persistence rules
are in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). The firmware
owns Wi-Fi pulling, LittleFS persistence, panel refreshes, USB
configuration, and streamed OTA; servers supply only canonical 296x152
monochrome frames.

## Browser management

With USB connected, Quote/0 stays awake, joins its saved Wi-Fi network, and
serves a small browser UI. An unconfigured device starts a password-protected
setup hotspot. BLE BTHome telemetry remains enabled. On battery, the web UI
and hotspot stop; Wi-Fi runs only for due content pulls.

On Windows, use the native USB setup tool; WSL USB forwarding is unnecessary:

```powershell
# Read status and the current browser address.
.\tools\setup_usb.ps1
# Copy one saved Wi-Fi profile locally, wait for an address, and open the UI.
.\tools\setup_usb.ps1 -WifiProfile "My Wi-Fi" -WaitForNetworkSeconds 30 -OpenBrowser
# Show the device access code and hotspot password, or clear Wi-Fi for setup.
.\tools\setup_usb.ps1 -ShowAccess
.\tools\setup_usb.ps1 -ClearWifi
```

The selected Windows Wi-Fi key is never printed or copied into the repository;
the temporary profile export is deleted after use. Browser API calls require
the persistent device access code. The USB launcher passes it locally in a URL
fragment. The page supports Wi-Fi/pull settings, status, frame upload, and
streamed dual-slot firmware updates. Use the local HTTP UI on a trusted network.

Linux/macOS use `tools/quote0_setup.py` with Python and `pyserial`:

```console
python3 tools/quote0_setup.py --port /dev/ttyACM0 status
python3 tools/quote0_setup.py --port /dev/ttyACM0 --wait 30 --open-browser wifi "My Wi-Fi"
```

USB carries bounded setup/status messages. See
[`docs/PROTOCOL.md`](docs/PROTOCOL.md) for the current APIs.

### Image uploads

Choose a PNG, JPEG, WebP, or existing frame `.bin` in **Display an image**.
The browser resizes using fit (white margins) or centered crop, composites
transparency onto white, and converts to monochrome with an adjustable
threshold and optional Floyd–Steinberg dithering. Preview the exact pixels,
download the 5,624-byte frame, or upload it directly.

Conversion uses the device's saved status-bar setting: 280 × 152 content
pixels plus the reserved strip, or the full 296 × 152 when the bar is off.
Save a status-bar setting change before converting for that layout. Existing
`.bin` uploads preserve their bytes; the shaded strip shows what the firmware
will overlay. All image processing stays in the browser.

### Minute status heartbeat

The complete right-hand status strip updates every minute: battery, Wi-Fi,
USB power, BLE, and a UTC+8 clock. NTP sets time whenever the station is active;
RTC time survives deep sleep. Unknown time displays `--:--`. A clock-only
battery wake does not connect Wi-Fi. Disabling `display.status_bar` disables
minute wakes. Critical battery suspends the heartbeat. Clock ticks do not write
LittleFS state.

The opt-in `experimental-partial` feature confines unchanged-content updates
to the status strip after a full baseline in the same boot. This was visually
verified on USB; it uses the stock waveform with no measured speed gain.
Battery wakes still use full refresh. See [`docs/PARTIAL_REFRESH.md`](docs/PARTIAL_REFRESH.md).

### NFC

The product's tap-to-open feature is a passive NFC tag inside the case,
factory-programmed with the device's deep link. It has no electrical
connection to the ESP32-C3 — stock firmware reverse engineering and an
I2C sweep of every usable GPIO pair found no NFC or I2C device — so the
firmware cannot read or write it. Details and options live in
[`docs/PROTOCOL.md`](docs/PROTOCOL.md#4-nfc-hardware-finding-closed).

## Requirements

- Rustup (`rust-toolchain.toml` pins the nightly and `rust-src` for ESP32-C3)
- [`ldproxy`](https://crates.io/crates/ldproxy) (ESP-IDF linker frontend)
- [`espflash`](https://crates.io/crates/espflash) for flashing
- Python 3 with `pyserial`, for the USB setup and on-board check tools

## Building and flashing

```console
cargo build --locked --bin quasiquote0
espflash flash --port /dev/ttyACM0 \
  --bootloader "$(ls -t target/riscv32imc-esp-espidf/debug/build/esp-idf-sys-*/out/build/bootloader/bootloader.bin | head -1)" \
  --partition-table "$(ls -t target/riscv32imc-esp-espidf/debug/build/esp-idf-sys-*/out/build/partition_table/partition-table.bin | head -1)" \
  target/riscv32imc-esp-espidf/debug/quasiquote0
```

For a first stock-firmware migration, back up flash and install the matching
bootloader and partition table. Erasing flash also destroys retained frames
and configuration; it is unnecessary for application upgrades. The device
formats an erased LittleFS volume on first boot and enters USB-accessible setup.

| Module | Scope |
| --- | --- |
| `wifi` | ESP-IDF Wi-Fi station construction, configuration, and link control |
| `bluetooth`, `bthome` | NimBLE lifecycle and BTHome v2 telemetry encoding |
| `usb_serial` | exclusive USB Serial/JTAG transport on GPIO18/19 |
| `display` | native 152x296, 1bpp UC8251D framebuffer and Quote/0 panel GPIO ownership |
| `embedded_graphics` | `embedded-graphics` `DrawTarget` over the landscape framebuffer |

The firmware starts the USB bootstrap task before optional radio and panel
setup. HTTP starts after the network stack is ready. Browser uploads and
scheduled pulls share one operation lease through verification, durable
publication, and cleanup. Firmware streams directly to the inactive OTA slot.

For the recovered Windows JTAG setup, convert the ELF with the installed
esptool, then run `tools/flash_app_windows.ps1 -Image <application.bin>`. It
preserves the bootloader, partition table, and filesystem and detects the
currently running slot through USB. If bootstrap is unavailable, specify
`-Slot ota_0` or `-Slot ota_1` for recovery; this does not change boot selection.
Use the browser firmware updater for subsequent dual-slot upgrades.

```console
mkdir -p target/browser-flash
.embuild/espressif/python_env/idf5.5_py3.14_env/bin/python -m esptool \
  --chip esp32c3 elf2image --flash_mode dio --flash_freq 40m --flash_size 4MB \
  --elf-sha256-offset 0xb0 -o target/browser-flash/quasiquote0.bin \
  target/riscv32imc-esp-espidf/debug/quasiquote0
```

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

## Editing the web UI

Readable HTML, CSS, and JavaScript live in `src/web/`. A small
[esbuild](https://esbuild.github.io/) build inlines and minifies them into
`src/web/embedded.html`, the only page embedded by Rust:

```console
pnpm install --frozen-lockfile
pnpm build
pnpm test
cargo build --locked --bin quasiquote0 --features experimental-partial
```

The generated page is committed so firmware-only builds remain usable offline
without Node. After editing web sources, run `pnpm build`; `pnpm test` checks
that the committed asset matches the sources. No runtime package or CDN is
needed on the device or in the browser.

`pnpm test` checks SHA-256, conversion geometry/packing/transparency/dithering,
and generated-asset freshness. To run actual browser decoding and UI tests,
start a separate Chromium/Edge profile with `--remote-debugging-port=9222`, then:

```console
QUOTE0_CDP_URL=http://127.0.0.1:9222 pnpm test:browser
```

For a Windows browser driven from WSL, also set `QUOTE0_BROWSER_DOWNLOAD_DIR`
to an absolute Windows temporary directory so test downloads use the browser's
filesystem. The tests use a fresh context and mocked device APIs.

## Verification

Host checks exercise BTHome encoding, clock/refresh/power policies, the bounded
USB parser, exclusive publication, frame integrity, saved-format compatibility,
interrupted writes, and persistence:

```console
cd /tmp
cargo +stable test --manifest-path /path/to/quasiquote0/tests/host/Cargo.toml \
  --target x86_64-unknown-linux-gnu --target-dir /path/to/quasiquote0/target/host-tests
cargo +stable clippy --manifest-path /path/to/quasiquote0/tests/host/Cargo.toml \
  --target x86_64-unknown-linux-gnu --target-dir /path/to/quasiquote0/target/host-tests \
  --all-targets -- -D warnings
```

Hardware checks have covered setup hotspot provisioning, NTP/HTTPS, OTA in both
slots, minute updates on battery, RTC retention, and wake on USB insertion.
The optional status-strip-only refresh was visually confirmed on USB; see
[`docs/PARTIAL_REFRESH.md`](docs/PARTIAL_REFRESH.md) for its limits.

## License

MIT — see [LICENSE](LICENSE).

## AI-assisted development

This firmware is developed with the help of an LLM coding agent; commits
tagged `(AI)` were authored by such an agent under human direction and
review. Hardware-facing claims (panel polarity, partition layout, boot
behavior) are validated against a real Quote/0 board before landing.
