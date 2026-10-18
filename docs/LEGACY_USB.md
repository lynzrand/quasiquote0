# Legacy USB diagnostic protocol

The normal browser firmware no longer exposes this interface. It is retained
behind `legacy-usb` for the standalone `quote0-clock` diagnostic and old tools.
The former full USB management application is available in revision `107bb500`.

## Former Yamux/JSON-RPC interface

The standalone `quote0-clock` bring-up binary uses the same Yamux stream-0
JSON-RPC framing and exposes `system.ping`, `device.hello`, `device.status`,
`device.reboot`, `clock.get`, and `clock.set`. `clock.set` accepts
`{"unix_seconds":1792326341}` (UTC, between 2020 and 2100). The face uses a
fixed UTC+8 offset. `clock.get` reports time, acknowledged display time and
refresh count, VBUS/host presence, wake-source setup, and uptime. Frame-client
and management methods below belong to the `quasiquote0` binary. Both binaries
support `clock.get` and `clock.set`; the frame client also reports pull interval
and attempt count. Display refresh/attempt counters are per boot. The clock
shows `--:--` before synchronization; deep sleep retains time, full power loss
does not. The frame client's `device.status.display` includes
`clock_unix_seconds`, `refreshes`, and `error` for acknowledged updates.

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
| `clock.get`            | RTC time, acknowledged display time, power and pull counters |
| `clock.set`            | Synchronize UTC using integer `unix_seconds` |
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
