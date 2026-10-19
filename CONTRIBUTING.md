# Contributing to quasiquote0

Start with the [README](README.md) for device use, the
[protocol reference](docs/PROTOCOL.md) for external contracts, and the
[architecture guide](docs/ARCHITECTURE.md) for ownership and persistence rules.

## Building and installing firmware

The shell commands below run from the repository root in a POSIX shell.
Firmware builds have been exercised in Linux/WSL; the PowerShell tools operate
on the Windows USB interface directly.

Clone the public repository if you do not already have a checkout:

```sh
git clone https://github.com/lynzrand/quasiquote0.git
cd quasiquote0
```

### Requirements

- Rustup. [`rust-toolchain.toml`](rust-toolchain.toml) selects
  `nightly-2026-07-28` with `rust-src` for the ESP32-C3 RISC-V target.
- Git, Python 3, CMake, Ninja, and libclang for the ESP-IDF build and bindings.
- `ldproxy` for linking and `espflash` for serial flashing.
- Python `pyserial` for `tools/quote0_setup.py` and `tools/check_browser.py`.
  The PowerShell setup tool uses Windows serial APIs instead.
- Node.js and pnpm only when editing or testing the web sources. Rust embeds
  the committed HTML asset, so a firmware-only build does not need Node.

Install the Rust utilities if needed:

```sh
cargo +stable install ldproxy espflash --locked
```

The project pins ESP-IDF v5.5.3 in [`.cargo/config.toml`](.cargo/config.toml).
The first build downloads the SDK and supporting tools into `.embuild/`; it
needs network access. Cached builds can use `--offline`.

```sh
cargo build --locked --bin quasiquote0
```

The application ELF is `target/riscv32imc-esp-espidf/debug/quasiquote0`.
For the optional status-window experiment, add `--features experimental-partial`.
Its hardware results and limitations are in [PARTIAL_REFRESH.md](docs/PARTIAL_REFRESH.md).

### First installation over serial

Read MindReset's [custom firmware guide](https://github.com/MindReset/dot_open_platform/blob/main/devices/quote_0/docs/custom-firmware-guide.md)
and back up stock flash before replacing it. Confirm that the device is the
standard UC8251D edition. This project uses a custom bootloader/partition
combination; stock-firmware migration requires both, not just an application.

After building, select the bootloader from that build and flash with the
project partition table:

```sh
quote0_bootloader=$(ls -t target/riscv32imc-esp-espidf/debug/build/esp-idf-sys-*/out/build/bootloader/bootloader.bin | head -1)
espflash flash --port /dev/ttyACM0 \
  --bootloader "$quote0_bootloader" \
  --partition-table partitions.csv \
  target/riscv32imc-esp-espidf/debug/quasiquote0
```

Replace the port for your machine. This operation writes boot metadata and is
for initial installation, not routine dual-slot upgrades. The filesystem region
must be erased when migrating an incompatible stock layout; erasing it destroys
saved images and settings. It is unnecessary for application-only upgrades.
A fresh filesystem enters USB-accessible setup on boot.

### Application images for browser OTA

The browser accepts an application `.bin`, not the ELF, a merged flash image,
or a whole-device backup. With the ESP-IDF Python environment activated:

```sh
mkdir -p target/firmware
python -m esptool --chip esp32c3 elf2image \
  --flash_mode dio --flash_freq 40m --flash_size 4MB \
  --elf-sha256-offset 0xb0 \
  -o target/firmware/quasiquote0.bin \
  target/riscv32imc-esp-espidf/debug/quasiquote0
```

This command uses the esptool 4.x syntax supplied by the verified ESP-IDF
v5.5.3 environment. Its local Python interpreter lives under
`.embuild/espressif/python_env/`; the versioned directory name depends on the
host Python. For example, the local build environment used
`.embuild/espressif/python_env/idf5.5_py3.14_env/bin/python`.

Keep the application within a single `0x1e0000`-byte OTA slot. Upload it using
the browser's **Firmware update** section while USB stays connected. OTA
streams directly to the inactive partition; it does not buffer the whole image
in RAM or LittleFS. Boot confirmation requires a successful panel refresh and
owner-loop progress; it does not certify complete radio health.

### Windows JTAG recovery

`tools/flash_app_windows.ps1` expects Espressif OpenOCD at
`target/windows-openocd/openocd-esp32/`, including `bin/openocd.exe` and
`share/openocd/scripts/`. This is a local recovery prerequisite, not a vendored
toolchain. With the native USB JTAG driver installed and an application image
built above:

```powershell
.\tools\flash_app_windows.ps1 -Port COM4 -Image .\target\firmware\quasiquote0.bin
```

The script detects the running OTA slot through USB and overwrites that slot;
it preserves the bootloader, partition table, and filesystem. If USB bootstrap
is unavailable, specify `-Slot ota_0` or `-Slot ota_1` after determining which
slot boots. The script does not change boot selection. Prefer browser OTA for
normal updates, which writes the inactive slot instead.

To forget Wi-Fi and return to setup mode without deleting the picture:

```powershell
.\tools\setup_usb.ps1 -Port COM4 -ClearWifi
```

`tools/install_windows_jtag_driver.ps1 -DriverDirectory <driver-package>` installs
only the Espressif-signed JTAG interface package. It checks the device ID and
interface GUID; use `-ValidateOnly` to inspect a package without installing it.
Installation requires an administrator PowerShell. Keep the serial interface
available for setup and recovery.

## Board and flash constraints

GPIO20 powers the panel and is also UART0 TX. Keep the UART console disabled;
USB Serial/JTAG on GPIO18/19 carries bounded setup/status messages, not logs.

USB power must keep the MCU awake regardless of saved settings. Both GPIO0 VBUS
and USB host activity guard against sleep. Cold boots/GPIO wakes allow a
60-second recovery window; timer wakes allow two seconds for enumeration.
Battery sleep requires both timer and GPIO0-high wake sources to be armed.
Wake-source errors leave the device awake. See the [architecture invariants](docs/ARCHITECTURE.md#power-and-resource-constraints)
before changing stack sizes, radio roles, or power behavior.

The 4 MiB layout is defined in [partitions.csv](partitions.csv): 12 KiB NVS,
OTA metadata and PHY data, two 1.875 MiB application slots, and a 192 KiB
LittleFS volume labelled `littlefs`. Cargo copies the partition table into the
generated ESP-IDF project. Configuration and retained pictures live in LittleFS.

## Editing the web UI

Edit readable HTML, CSS, and JavaScript in `src/web/`; esbuild bundles and
inlines them into the committed `src/web/embedded.html`:

```sh
pnpm install --frozen-lockfile
pnpm build
pnpm test
```

Commit the generated page with its source changes. The asset freshness check
fails when it is stale. The page needs no runtime package or CDN. Image
conversion remains browser-side, and existing `.bin` uploads retain their bytes.

## Verification

### Host tests and formatting

Select the host target explicitly because the root Cargo configuration targets
the ESP32-C3. Install stable Rust with Clippy and rustfmt, then run:

```sh
quote0_host=$(rustc +stable -vV | sed -n 's/^host: //p')
cargo +stable test --locked --manifest-path tests/host/Cargo.toml \
  --target "$quote0_host" --target-dir target/host-tests
cargo +stable clippy --locked --manifest-path tests/host/Cargo.toml \
  --target "$quote0_host" --target-dir target/host-tests --all-targets -- -D warnings
cargo +stable fmt --all -- --check
```

These tests compile the production BTHome encoder, frame handling, policy, and
persistence modules without ESP-IDF. They cover bounded USB parsing, publication
exclusion, interrupted and corrupt frames, saved-format compatibility, and
minute acknowledgements without flash writes. Passing them does not establish
full filesystem power-loss recovery or hardware behavior.

### Browser tests

`pnpm test` checks SHA-256, conversion geometry/packing/transparency/dithering,
and generated-asset freshness. For decoding and UI integration, start an
isolated Chromium/Edge profile with `--remote-debugging-port=9222`, then run:

```sh
QUOTE0_CDP_URL=http://127.0.0.1:9222 pnpm test:browser
```

The integration test uses mocked device APIs and a fresh browser context. For
a Windows browser driven from WSL, set `QUOTE0_BROWSER_DOWNLOAD_DIR` to an
absolute Windows temporary directory so downloads use the browser's filesystem.
Without `QUOTE0_CDP_URL`, the browser test is skipped; that is not a passing UI check.

### On-board checks

With USB connected and network provisioning complete:

```sh
python3 tools/check_browser.py --port /dev/ttyACM0
```

This checks browser serving, access-code enforcement, redacted configuration
round trips, and rejected/interrupted uploads. It writes the current redacted
configuration back to verify secret preservation. Add `--frame <file>` or
`--firmware <file>` only when you intend to replace displayed content or install
that image. The firmware option also tests rejection of a mismatched digest.

Hardware sessions have exercised setup hotspot provisioning, NTP/HTTPS, OTA in
both slots, minute updates on battery, RTC retention, and wake on USB insertion.
These are manual checks, not an automated hardware suite or a battery-life
measurement. The optional status-window refresh was visually confirmed on USB.

## AI-assisted development

This firmware is developed with the help of an LLM coding agent; commits
tagged `(AI)` were authored by such an agent under human direction and
review. Hardware-facing claims (panel polarity, partition layout, boot
behavior) are validated against a real Quote/0 board before landing.
