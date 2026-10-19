# quasiquote0

Local-first Rust firmware for the **MindReset Quote/0** e-paper display.
Upload a picture from your browser, or have the device fetch frames from your
own server. No vendor account or cloud service is required.

This is experimental firmware for the **standard ESP32-C3 / UC8251D board**.
The Early Bird Edition uses a different display controller and is unsupported;
check the [official hardware guide](https://github.com/MindReset/dot_open_platform/tree/main/devices/quote_0)
before flashing.

- **Browser image uploads:** PNG, JPEG, WebP, or an existing monochrome frame.
  Preview, fit or crop, adjust the threshold, and optionally dither before uploading.
- **A minute status clock:** battery, Wi-Fi, USB, and Bluetooth indicators beside
  your image. Time synchronizes through NTP and survives deep sleep.
- **Battery operation:** wake for status updates and scheduled content pulls;
  keep Wi-Fi and the browser UI available while USB-powered.
- **BTHome v2 telemetry:** broadcast battery and connection state while awake.
- **Browser firmware updates:** upload an application image to the alternate
  OTA slot. Pictures and settings survive normal upgrades.
- **Backup and restore:** download settings, saved credentials, and the original
  image together, then import them from the browser.

## Use it

These examples assume quasiquote0 is already installed. For a stock device,
start with [installation from source](CONTRIBUTING.md#building-and-installing-firmware).
Connect a USB data cable and use the setup tool from this checkout.

### Windows

Copy a saved Wi-Fi profile to the device and open its browser UI:

```powershell
.\tools\setup_usb.ps1 -Port COM4 -WifiProfile "My Wi-Fi" -WaitForNetworkSeconds 30 -OpenBrowser
```

Replace `COM4` and `My Wi-Fi` with your port and saved profile name. The key is
copied locally; the temporary Windows profile export is deleted after use.
The serial interface remains on Windows, so WSL USB forwarding is unnecessary.

For a device that already has network settings:

```powershell
.\tools\setup_usb.ps1 -Port COM4 -WaitForNetworkSeconds 30 -OpenBrowser
```

### Linux and macOS

With Python 3 and `pyserial` installed, provision Wi-Fi and open the UI:

```sh
python3 tools/quote0_setup.py --port /dev/ttyACM0 --wait 30 --open-browser wifi "My Wi-Fi"
```

The tool prompts for the Wi-Fi password. Replace `/dev/ttyACM0` with your serial
port; on macOS it commonly has a `/dev/cu.usbmodem…` name. For an already
configured device, use `status` in place of `wifi "My Wi-Fi"`.

### Display an image

In the browser, choose a file under **Display an image**, adjust its preview,
and select **Upload to display**. Conversion happens entirely in your browser.
You can also download the resulting `.bin` for a frame server.

The display is 296 × 152 pixels in landscape orientation. With the status bar
on, images use 280 × 152 pixels and the device owns the rightmost 16 columns.
Save a status-bar setting change before preparing an image for that layout.
Existing frame `.bin` uploads preserve their bytes.

To automate content, enable **Fetch frames from a server** in the settings and
supply a frame URL and interval. The default interval is 15 minutes. Servers
return exactly 5,624 monochrome bytes; see the [frame and HTTP contracts](docs/PROTOCOL.md#2-wi-fi-pull-protocol-implemented).

### Back up or restore

Under **Backup and restore**, select **Export backup** to download a JSON file
containing settings, Wi-Fi passwords, pull tokens, and the retained source image.
Store it privately. To restore, choose that file and select **Import backup**.
The device validates it, applies the restore on restart, and keeps its existing
access code. If the restored Wi-Fi differs, use USB setup to find its address.

## Install or update

Build and install the firmware using the [build and flashing guide](CONTRIBUTING.md#building-and-installing-firmware).
It covers the required toolchain, first installation, application `.bin`
generation, and Windows JTAG recovery.

Once installed, use **Firmware update** in the browser for later application
updates. Keep USB connected until the device restarts. First installation needs
the project's bootloader and partition layout; an application update alone does
not migrate stock firmware. Back up stock flash before replacing it.

Unconfigured devices start a password-protected `Quote0-…` setup hotspot.
Read its password and browser access code through USB with
`tools/setup_usb.ps1 -ShowAccess` or `tools/quote0_setup.py --port <port> --show-access status`.
Join that hotspot and open `http://192.168.71.1/`, then enter your home Wi-Fi
settings. USB setup also remains available if saved network credentials stop working.

## Limits

- Management uses local HTTP with a device access code. Use it on a trusted
  network. The browser UI is available only while USB-powered.
- The clock currently uses **UTC+8**. After complete power loss it shows `--:--`
  until NTP succeeds. A battery-only clock wake does not start Wi-Fi to sync time.
- Battery percentage is an estimate. USB presence is reported as charging in
  BTHome; it is not a measurement of charge current. Battery life is unmeasured.
- Keep a backup of your images and settings. This firmware is under development.
- Full refresh is the default. Optional [status-strip partial refresh](docs/PARTIAL_REFRESH.md)
  is experimental, has no measured speed benefit, and uses full refresh after
  each deep-sleep boot.
- BLE provides non-connectable BTHome advertisements. There is no GATT control
  service. The firmware does not program the case's passive NFC tag.

## Documentation and contributions

- [Protocol reference](docs/PROTOCOL.md): USB setup, HTTP API, frame format,
  scheduled pulls, and BTHome payloads.
- [Architecture](docs/ARCHITECTURE.md): module ownership, persistence, and power rules.
- [Contributing](CONTRIBUTING.md): build, test, web UI development, and recovery.

Bug reports should include the board edition, firmware build, power source,
and reproduction steps. Remove access codes, Wi-Fi passwords, and bearer tokens
from status output before sharing it.

## License

[MIT](LICENSE).
