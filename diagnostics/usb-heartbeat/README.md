# USB heartbeat diagnostic

Minimal ESP-IDF C application: emits `alive` over USB Serial/JTAG once a
second and never enters sleep. Panel power stays off. There is no Wi-Fi,
Bluetooth, filesystem, Yamux, or application configuration. UART remains
disabled because GPIO20 powers the display.

Build using this workspace's existing ESP-IDF C tools (no Rust needed):

```sh
bash diagnostics/usb-heartbeat/build.sh
```

Then run on Windows, with the previous config watcher stopped:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/flash_heartbeat_on_wake.ps1 -Port COM4 -LogPath target/usb-heartbeat-flash.log
```

The watcher waits up to 24 hours, resets the chip into ROM download mode,
flashes and verifies the bootloader, existing partition layout, and diagnostic
in `ota_0`, then checks for three heartbeat lines. It resets OTA selection
metadata, keeping NVS, the other application slot, and LittleFS intact.
The display retains its previous image. The diagnostic replaces the frame
client until that firmware is flashed again.

For a startup window too short for the serial flasher, use native Windows
JTAG instead (stop the serial watcher first):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/jtag_flash_heartbeat.ps1 -LogPath target/usb-heartbeat-jtag.log
```

This requires Espressif's Windows OpenOCD distribution extracted under
`target/windows-openocd/openocd-esp32`. The session used the Windows x64
`v0.12.0-esp32-20251215` release listed in ESP-IDF's `tools/tools.json`, with
its SHA-256 verified against that manifest. The watcher stages the debugger
and firmware in Windows temporary storage, repeatedly attempts JTAG attach
and CPU halt, then programs and verifies only the four regions listed in
the generated `flasher_args.json`. It checks three USB heartbeats after reset.
Retries have no deliberate delay by default; process startup and USB probing
determine the actual rate, which is reported in the log. Use
`-RetryDelayMilliseconds <milliseconds>` to throttle retries, or
`-ReportIntervalSeconds 10` for more frequent progress reports.
The log records COM-port appearance/disappearance and retains JTAG failures
before a successful halt, including cases where COM is present but the JTAG
adapter cannot be opened. The most recent such transcript is also saved as
`last-interesting-attempt.log` in the Windows staging directory.

If COM4 appears but JTAG reports `LIBUSB_ERROR_NOT_FOUND`, inspect the JTAG
driver binding. A generic WinUSB driver can omit `DeviceInterfaceGUIDs`;
Espressif's JTAG-specific package registers it for `VID_303A&PID_1001&MI_02`.
The official package is available from
[Espressif's driver releases](https://github.com/espressif/esp-win-usb-drivers/releases/tag/ESP32-USB-JTAG_v6.1.7600.16386).
After extraction, validate or install it using:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/install_windows_jtag_driver.ps1 -DriverDirectory target/windows-jtag-driver -ValidateOnly
# Run the installation from an administrator PowerShell:
powershell -NoProfile -ExecutionPolicy Bypass -File tools/install_windows_jtag_driver.ps1 -DriverDirectory target/windows-jtag-driver
```

The installer checks the expected hardware ID, interface GUID, and Espressif
catalog signature before invoking Windows PnPUtil. Its package matches only
the JTAG interface, leaving the USB serial interface outside its scope.
