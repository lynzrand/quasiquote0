# Quote/0 protocol reference

Normative contracts for everything that crosses the device boundary:
the USB wire protocol, the Wi-Fi pull protocol, and the Bluetooth
protocol. Design rationale lives in [`PROPOSAL.md`](PROPOSAL.md); this
document describes the contracts as implemented or intended, and flags
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
should treat that region as reserved.

## 1. USB wire protocol (implemented)

USB Serial/JTAG (GPIO18/19) carries exactly one **Yamux** session
(version 0 framing). No log text ever appears on this wire; the
bootloader and application consoles are disabled.

- **Stream 0** is the Quote/0 extension channel: UTF-8
  newline-delimited JSON-RPC 2.0. A generic Yamux client is not
  sufficient — it must know to speak NDJSON on stream 0.
- **Binary transfers** (frame upload, OTA) use device-initiated
  streams opened by `frame.begin`/`ota.begin`. The response carries
  the stream ID and an operation ID. The host writes exactly the
  announced byte count; receipt of the declared length completes the
  transfer (the device does not wait for EOF).
- Bounded 4 KiB receive windows per stream and a 4 KiB connection-wide
  outbound queue provide backpressure; Yamux's 256 KiB protocol credit
  never becomes a device allocation.
- Control lines are limited to 16 KiB. Malformed JSON yields a
  JSON-RPC parse error without resetting the session.

### JSON-RPC methods (stream 0)

| Method                 | Purpose                                             |
| ---------------------- | --------------------------------------------------- |
| `system.ping`          | Liveness / framing check                            |
| `device.hello`         | Protocol version, firmware, hardware, capabilities  |
| `device.status`        | Battery, filesystem, display, pull, reset token     |
| `device.reboot`        | Orderly reboot after replying                       |
| `config.get`           | Versioned, secret-redacted configuration            |
| `config.set`           | Validate and transactionally replace configuration  |
| `wifi.clear`           | Drop Wi-Fi credentials, keep the displayed frame    |
| `pull.run`             | Request one immediate pull (USB-attached mode only) |
| `frame.metadata`       | Desired/displayed hashes and generation metadata    |
| `frame.begin`          | Open a binary stream for a direct frame upload      |
| `ota.begin`            | Open a binary stream for an OTA image upload        |
| `ota.status`           | Active slot, pending upgrade, in-progress flag      |
| `operation.status`     | Progress/result of a frame or OTA operation         |
| `diagnostics.get`      | Bounded fault records                               |
| `device.factoryReset`  | Clear app state behind a confirmation token         |
| `i2c.scan`             | Bring-up helper: probe an SDA/SCL pair for devices  |

`device.status` reports a short-lived `factory_reset_token`;
`device.factoryReset` must echo it as `confirmationToken` and the
token rotates after every attempt.

### Binary upload lifecycle

`frame.begin` takes `sha256` (lowercase hex), optional `generation`,
`source`, `accepted_at_ms`. The transfer is verified against the
announced digest and the exact 5,624-byte length before it becomes the
desired frame. `ota.begin` takes `bytes`, `sha256`, optional `version`;
bytes stream straight into `esp_ota_write`. One frame upload and one
OTA operation may be active at a time.

## 2. Wi-Fi pull protocol (implemented)

The device is an outbound-only HTTP(S) client. On a scheduled or
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
- Failures back off exponentially 30 s → 1 h; a manual `pull.run`
  bypasses backoff.
- Wi-Fi is brought up per pull attempt and stopped afterwards.
- While USB-powered (`stay_awake_on_usb: true`, the default) the
  device stays awake and serves the control protocol continuously.
- On battery the device performs one conditional pull per wake, then
  deep-sleeps until the next interval. Backoff state is RAM-only and
  resets across sleeps; interval spacing already bounds retry rate.

## 3. Bluetooth Low Energy

The radio runs the NimBLE host. Two services are defined; the beacon
is implemented, the GATT push channel is specified but **not yet
implemented**.

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

### 3.2 GATT frame push (intended, not yet implemented)

For phone/host pushes without USB, a connectable GATT service mirrors
the wire protocol's frame upload. UUIDs below are fixed identifiers;
values are little-endian where applicable.

| Item                        | UUID                                    |
| --------------------------- | --------------------------------------- |
| Service                     | `78000000-a1b2-4c3d-8e4f-120030405060`  |
| Control characteristic      | `78000001-a1b2-4c3d-8e4f-120030405060`  |
| Frame characteristic        | `78000002-a1b2-4c3d-8e4f-120030405060`  |

- **Control characteristic** (write-with-response, 512-byte MTU
  chunks, NDJSON, one JSON-RPC per line): reuses the exact stream-0
  method set of §1, minus `ota.*`. Responses arrive as notifications
  on the same characteristic.
- **Frame characteristic** (write-without-response, raw bytes):
  opened by a successful `frame.begin`; the host writes the announced
  5,624 bytes in ≤ 244-byte chunks; the device ACKs by notification
  when the operation completes.
- Authentication: a `bluetooth.pairing_token` configuration field
  (set over USB or Wi-Fi) gates `frame.begin`; an empty token refuses
  all GATT writes. No legacy pairing/encryption is performed in the
  first revision — the token is the bearer credential.

The practical consequence: the same host client code path
(`frame.begin` + raw stream) works over USB and BLE.

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

Options if device-written NFC is ever required: hardware mod with an
I2C tag IC (e.g. NT3H2111 or ST25DV64KC on GPIO8/9 — both pins are
free and quiet), or a one-time rewrite of the passive tag with a
phone. The `i2c.scan` JSON-RPC method exists to support such bring-up.
