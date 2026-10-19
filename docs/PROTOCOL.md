# Quote/0 protocol reference

Normative contracts for everything that crosses the device boundary:
the USB wire protocol, the Wi-Fi pull protocol, and the Bluetooth
protocol. This document describes the implemented contracts and flags
anything not yet on hardware.

The canonical frame format is shared by every channel:

| Field   | Value                                        |
| ------- | -------------------------------------------- |
| Width   | 296 px (landscape, viewer orientation)       |
| Height  | 152 px                                       |
| Storage | Row-major, 37 bytes/row, 5,624 bytes total  |
| Bit     | MSB-first; 1 = white, 0 = black              |
| Origin  | Top-left; x rightward, y downward            |

The device overlays a 16-column status strip on the **right edge**
(columns 280–295) whenever `display.status_bar` is enabled; producers
should treat that region as reserved. The whole strip is recomposed each
minute: battery, Wi-Fi, USB power, Bluetooth advertising, and a bottom `HH:MM`
clock in UTC+8. The plug icon indicates USB power, not measured charging.
The current implementation refreshes the full panel, preserving the retained
content pixels. Disabling the strip disables minute-only refreshes and wakes;
critical battery suppresses the heartbeat even when the strip is enabled.

## 1. Management on USB power

With USB connected, configured devices join their saved Wi-Fi network and keep
it connected even when pulling is disabled. Unconfigured devices start a WPA2
setup hotspot `Quote0-<last three MAC bytes>` at `http://192.168.71.1/`.
The setup password and browser access code are available through physical USB.
Clearing Wi-Fi credentials enters setup mode without deleting retained content.
Configuration changes allow one second for the save reply before switching
the radio. A configured but unreachable network gets bounded connection attempts (15 s,
then a 30 s retry delay); USB setup remains available to correct credentials.

The HTTP service runs only on USB power. Battery minute wakes keep networking
off unless a pull is due. BLE telemetry remains available during awake windows. Only its implemented
broadcaster role is enabled; scanning, pairing, and GATT connection pools are
excluded from the normal SDK configuration.

### USB bootstrap

USB Serial/JTAG accepts UTF-8 JSON, one command per newline, up to 1,024 bytes.
Oversized lines are discarded through the next newline. No unsolicited logs,
streams, operation IDs, frame uploads, or firmware uploads use this channel.
Hardware reset can still produce a ROM banner before the application starts.

```json
{"cmd":"status"}
{"cmd":"wifi","ssid":"my network","password":"my password"}
{"cmd":"clear_wifi"}
```

Replies are `{"ok":true,"result":{...}}` or `{"ok":false,"error":"..."}`.
The status reply wraps its fields in `result.status` and includes the current
network URL, a persistent random 32-hex `access_token`, and setup SSID/password.
Only the physical USB status response includes these credentials.

Windows: `tools/setup_usb.ps1 -WifiProfile <saved-profile> -WaitForNetworkSeconds 30
-OpenBrowser`. The selected Windows profile is exported temporarily, consumed
locally, and deleted in `finally`; the Wi-Fi key is never printed or placed in
the repository. `-ShowAccess` explicitly displays the device access credentials.
Linux/macOS: `tools/quote0_setup.py --port <port> --wait 30 --open-browser wifi <ssid>`
prompts for the password. 

### Browser and HTTP API

`GET /` serves the self-contained browser UI. Every `/api/*` route requires
`X-Device-Token: <access_token>`. There is no cross-origin API access. The page
accepts the access code manually or via `/#token=<code>` from the USB launcher;
it removes the fragment from the address bar and keeps the code in session
storage. This is a local HTTP service; use it on a trusted network.

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/api/status` | Power, network, NTP, display, pull, storage and OTA state |
| GET | `/api/config` | Configuration in `config`, with passwords/tokens redacted |
| POST | `/api/config` | Validated transactional replacement, JSON up to 4,096 bytes |
| PUT | `/api/frame` | Stream exactly 5,624 canonical bytes into retained storage |
| PUT | `/api/firmware` | Stream an application image into the inactive OTA slot |
| POST | `/api/pull` | Schedule an immediate pull when pulling is enabled |
| POST | `/api/reboot` | Reply, then restart the device |
| GET | `/api/backup` | Download settings, credentials, and the retained image |
| POST | `/api/backup` | Validate and stage a backup, then restart to apply it |

Frame/firmware requests require `Content-Length` and `X-Content-SHA256` (64
lowercase hex digits). Firmware length is nonzero and no more than 1,966,080
bytes. The browser computes the digest before uploading; the device verifies it
independently. Its image picker converts PNG/JPEG/WebP locally with canvas,
fit or centered crop, a threshold, and optional Floyd–Steinberg dithering.
Transparency becomes white. Conversion respects the saved status-bar setting;
reserved columns remain white in converted files. Preview and `.bin` download
use the same packed bytes as upload. Existing frame `.bin` bytes are preserved.
No image decoder or conversion work runs on the ESP32. Uploads stream in 512-byte chunks. Frame deadlines are 15 s,
firmware deadlines 120 s, with the SDK's 5 s idle receive timeout. USB removal
aborts an upload before publication or OTA boot selection.

One exclusive operation lease covers uploads, configuration writes, scheduled
pull publication, and reboot. Upload replies report final success/failure;
there are no asynchronous operation IDs or per-upload workers. Errors return
JSON (`401` for missing/wrong access code, `409` for a conflicting operation,
`400` for invalid input, `503` without USB power or available storage).
The single HTTP server task serializes browser requests.

Configuration GET returns `***` for stored Wi-Fi passwords and bearer tokens.
POST preserves the stored secret when the corresponding field is `***`; an
empty string clears it. A successful frame upload means the desired frame is
durable; `/api/status.display` separately acknowledges panel refresh.
OTA never buffers the image in RAM or LittleFS. Only a validated complete image
is selected after persisting the pending marker. The next boot confirms the
image after storage and USB task initialization, a successful panel refresh,
and ten seconds of owner-loop progress through radio/server startup; the installed
bootloader must support rollback. This does not establish full radio health.

### Backup and restore

The authenticated, USB-powered backup routes use JSON up to 16 KiB. A backup
contains `schema: 1`, a full unredacted `config`, and `frame`. The frame is
either `null` (no retained image) or an object with `sha256` and `hex`: exactly
5,624 source bytes encoded as 11,248 hex characters. Device overlays are not
included. Serialized settings are limited to 4,096 bytes, keeping a complete
backup within the import limit and the server's memory budget.

Unlike `/api/config`, export includes Wi-Fi passwords and pull bearer tokens.
Store the file privately. Device access codes remain local to each destination
and are not exported or overwritten. Export sets `Cache-Control: no-store` and
an attachment filename; it streams the image without allocating a complete frame.

Import checks the schema, settings, byte count, hex encoding, and image hash
before staging any restore. A null frame clears retained content. A durable
restore journal commits the settings and desired image together; startup replays
it before loading state or collecting unused frames. A reset between applying
the settings and image state repeats the restore. Invalid input leaves the
current setup intact. Pull validators are cleared for the restored source.

Successful import replies `{"accepted":true,"rebooting":true}`. Mutations are
blocked once restart is pending, even if the HTTP client disconnects before
reading its reply. Import waits until pending OTA startup checks are complete.
This format backs up application settings/content, not firmware or raw flash.

### Time

The SDK SNTP service starts only after the station obtains an address. NTP
updates the RTC; time survives deep sleep. Full power loss shows `--:--` until
synchronization. USB clock setting is no longer a management prerequisite.
After a cold boot with unknown time, HTTPS pulls allow 12 s for initial NTP,
then back off rather than attempting certificate validation with invalid time.
A battery minute wake never starts Wi-Fi solely to obtain time. The status
clock remains fixed to UTC+8.

## 2. Wi-Fi pull protocol (implemented)

The frame-pull channel is an outbound HTTP(S) client. On a scheduled or
manual pull it sends:

```http
GET <configured url>
Accept: application/vnd.quasiquote0.frame, application/octet-stream
If-None-Match: "<stored ETag>"
If-Modified-Since: <stored Last-Modified>
Authorization: Bearer <token>
User-Agent: quasiquote0/<version>
```

`Authorization` is sent only when a bearer token is configured.

Server contract:

| Status | Meaning                                         |
| -----: | ----------------------------------------------- |
| `200`  | Body is exactly 5,624 canonical frame bytes     |
| `204`  | No desired content; keep the current display    |
| `304`  | Content unchanged; keep frame and validators    |

For `200`, the device always computes SHA-256 itself; a response
`X-Frame-SHA256` header, when present, must match or the body is
discarded. `ETag`/`Last-Modified` become the new validators only after
the frame validates and commits. Bodies longer than 5,624 bytes are
rejected mid-stream; short bodies are rejected at EOF. Redirects are
not followed (rejected until an explicit redirect policy exists).
HTTPS uses the ESP-IDF certificate bundle; plain HTTP requires
`pull.allow_insecure_http: true` and is refused otherwise.

### Scheduling and power

- Default interval 900 s; minimum 60 s (enforced in configuration
  validation).
- Failures back off exponentially 30 s → 1 h; a manual `/api/pull`
  bypasses backoff.
- On battery, Wi-Fi is brought up per pull attempt and stopped afterwards.
  USB power instead keeps the station connected and the browser service available.
- USB power always keeps the device awake, regardless of the legacy
  `stay_awake_on_usb` setting. GPIO0 VBUS detection and USB host-frame
  detection independently prevent sleep. The legacy field is accepted for
  configuration compatibility and omitted from saved/API configuration.
- A cold boot or GPIO wake provides a 60-second recovery window; timer
  wakes provide two seconds for USB enumeration. Battery sleep requires
  successful timer and GPIO0-high wake configuration, followed by a final
  USB/transfer check immediately before entering deep sleep. A wake-source
  error leaves the MCU awake.
- On battery the enabled status bar wakes at minute boundaries and redraws
  every indicator over the retained frame. Wi-Fi stays off on clock-only
  wakes. Pull deadlines and exponential backoff survive deep sleep in RTC
  memory, use a counter independent of wall time, and reset on a normal
  reboot or full power loss. Sleep ends at the next minute or pull deadline,
  whichever is earlier. Clock-only refreshes do not write to LittleFS.

## 3. Bluetooth Low Energy

The NimBLE host broadcasts BTHome v2 telemetry. No connectable service,
scanning, pairing, or GATT management is included.

### 3.1 BTHome v2 beacon (implemented)

A non-connectable, general-discoverable advertisement broadcast
whenever the device is awake (including each battery wake window);
there is no advertising while in deep sleep. Payload:

| Bytes | Value                                            |
| ----- | ------------------------------------------------ |
| 0–2   | Flags: `02 01 06`                                |
| 3–7   | BTHome v2 service data header: `11 16 D2FC 40`   |
| 8+    | BTHome measurements (below)                      |

Measurements (BTHome v2 object IDs):

| ID   | Measurement        |
| ---- | ------------------ |
| `0x01` | Battery, %       |
| `0x0C` | Battery, mV      |
| `0x15` | Battery low flag |
| `0x16` | Charging flag    |
| `0x19` | Wi-Fi connected  |
| `0x24` | USB present      |

Any BTHome-compatible dashboard (Home Assistant, ESPHome proxy, bthome
app) can track the device's battery and link state without pairing.

## 4. NFC (hardware finding, closed)

The product's tap-to-open-App-Clip feature is a **passive NFC tag
inside the case, factory-programmed with the per-device deep link**
(`https://dot.mindreset.tech/app?id=<mac>`). Reverse engineering of
the stock 2.0.8 firmware (all 4,883 decompiled functions, string
tables, NVS, both OTA slots) found no NFC or I2C driver, no second
SPI device, and no GPIO activity beyond the documented pin map; an
electrical I2C sweep of every usable GPIO pair found no acknowledging
device. The tag is powered by the reader's RF field and has **no
electrical connection to the ESP32-C3** — the firmware cannot read or
write it, stock or custom.

