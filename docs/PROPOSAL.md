# Proposal: Cloud-Free Quote/0 Frame Client

Status: implemented in part; hardware acceptance in progress

Target: MindReset Quote/0 / Dot, ESP32-C3, 4 MiB flash

Firmware project: `quasiquote0`

## Current design checkpoint

The main application remains a remote framebuffer. The separate `quote0-clock`
binary is a bring-up diagnostic, not a change to the server-rendered content
contract. Its visible clock, minute rollover, suppression of redundant
same-minute refreshes, and USB wakefulness beyond the startup recovery window
were verified on 2026-10-18. Both timer and GPIO wake sources armed successfully;
battery sleep followed by physical USB-insertion wake still needs verification
with the current recovery guards.

The frame client already implements USB management, durable frames,
HTTP(S) pulling, display composition, battery scheduling, and streamed OTA.
Implementation alone does not satisfy the acceptance criteria below. The next
milestone is end-to-end HTTP(S) pull and OTA validation on the board, including
unchanged/invalid response handling and interrupted upgrades. Connectable BLE
frame push remains a subsequent feature; its draft contract is in
[`PROTOCOL.md`](PROTOCOL.md#32-gatt-frame-push-intended-not-yet-implemented).

## Summary

This project will be replacement firmware for the Quote/0. It will behave as a
small remote e-paper framebuffer rather than as a client for the original
vendor service.

The device will:

- be configured exclusively through USB;
- connect to one configured Wi-Fi network in station mode;
- periodically pull a native monochrome frame from a custom HTTP(S) endpoint;
- retain the last valid frame and configuration in LittleFS;
- accept firmware updates as a streamed upload through USB;
- use the ESP32-C3 dual OTA slots and bootloader rollback;
- keep displaying the last successfully committed frame when the network,
  server, configuration, or next frame is invalid.

The device will not run an HTTP server. It will not use the MindReset cloud,
MQTT, vendor authentication, BLE provisioning, or a proprietary content
format. Rendering remains the responsibility of the custom server or another
host.

The existing hardware drivers, UC8251D framebuffer, USB Serial/JTAG transport,
blocking Yamux streams, and newline-delimited JSON-RPC implementation are the
starting point.

## Goals

1. A flashed device can be provisioned without an account or network-side
   discovery service.
2. A simple custom server can supply the exact pixels to display.
3. Loss of power during configuration, frame persistence, or OTA cannot destroy
   the last known-good state or active firmware.
4. The steady-state battery path wakes, fetches, refreshes only when needed,
   and returns to deep sleep.
5. The USB protocol remains bounded-memory and backpressure-aware. Large
   transfers are streamed directly to their destination.
6. A bad response, failed TLS connection, interrupted upload, or display timeout
   is observable over USB and does not blank the display.

## Non-goals for the first release

- Device-side text, HTML, image, widget, or layout rendering.
- An inbound HTTP or HTTPS API.
- MindReset cloud compatibility.
- MQTT or WebSocket support.
- BLE provisioning. A non-connectable BTHome telemetry beacon is implemented;
  connectable frame push remains outside the initial core acceptance milestone.
- Partial e-paper refresh.
- A user-visible USB mass-storage volume.
- Applying every intermediate frame. The newest complete desired frame wins.
- Resuming a partially uploaded OTA image after reboot or USB disconnect.
- Secure Boot and flash encryption in the initial development build.

## Hardware and flash constraints

The verified target is an ESP32-C3 driving a UC8251D revision `0x0a` panel. The
controller RAM is addressed as 152 x 296 pixels, one bit per pixel, while the
device's native viewing orientation is 296 x 152 landscape. A framebuffer is
therefore exactly 5,624 bytes in either geometry. A set bit is white and a
cleared bit is black.

The important board assignments are:

| Function    |    GPIO |
| ----------- | ------: |
| VBUS sense  |       0 |
| Battery ADC |       1 |
| EPD BUSY    |       3 |
| EPD RESET   |       4 |
| EPD D/C     |       5 |
| EPD CS      |       6 |
| EPD MOSI    |       7 |
| EPD SCLK    |      10 |
| USB D- / D+ | 18 / 19 |
| EPD power   |      20 |

GPIO20 is also UART0 TX. UART0 logging must remain disabled because toggling it
also toggles panel power. USB Serial/JTAG owns the host transport.

The current 4 MiB partition table is retained:

| Partition  |      Size | Purpose                                |
| ---------- | --------: | -------------------------------------- |
| `nvs`      |    12 KiB | Minimum ESP-IDF system NVS             |
| `otadata`  |     8 KiB | OTA selection and rollback metadata    |
| `phy_init` |     4 KiB | Radio PHY initialization data          |
| `ota_0`    | 1,920 KiB | Firmware slot A                        |
| `ota_1`    | 1,920 KiB | Firmware slot B                        |
| `littlefs` |   192 KiB | All application-owned persistent state |

Application configuration and content will not be stored in NVS. ESP-IDF may
still use the minimum NVS partition for its own Wi-Fi/system data. An encrypted
credential store would require a later flash-encryption design and potentially
a partition-table change; LittleFS alone does not make secrets confidential to
someone with physical flash access.

## System architecture

The firmware has five logical owners:

```text
USB Serial/JTAG
      |
      v
Yamux driver ---- stream 0 ----> JSON-RPC control
      |                              |
      +---- binary streams ----------+---- config / frame / OTA operations
                                     |
                                     v
                              command scheduler
                               /      |      \
                              v       v       v
                        HTTP client  LittleFS  display owner
                              |                  |
                              v                  v
                         Wi-Fi STA          UC8251D panel
```

Only the display owner may power, reset, or transfer data to the panel. Only
the LittleFS owner may replace durable state. Network and USB operations submit
validated desired frames through the same commit path.

FreeRTOS tasks may block. The implementation does not need an async executor:

- the USB driver task moves Yamux frames between USB and bounded queues;
- a JSON-RPC task blocks on control stream 0;
- independently owned Yamux streams implement `Read` and `Write`, and park
  their calling tasks while input, output capacity, or flow-control credit is
  unavailable;
- a scheduler/network task performs one blocking pull at a time;
- display access is serialized through one owner or mutex-protected worker.

The HTTP client and USB upload paths must not hold the display lock while
waiting for input.

## Operating modes

### USB-powered mode

Either VBUS on GPIO0 or detected USB host activity prevents deep sleep. While
USB is present, the device always stays awake, serves USB, and may run periodic
pulls. The legacy `stay_awake_on_usb` field is accepted but cannot disable this
guard. USB configuration, manual
pulls, diagnostics, frame upload, and OTA are available in this mode.

Cold boots and GPIO wakes provide a 60-second recovery window; timer wakes
provide two seconds for enumeration. Before battery sleep, the firmware must
successfully arm both a timer and GPIO0-high USB-insertion wake, then recheck
USB presence and active transfers. Wake-source errors keep the device awake.

### Battery mode

The normal cycle is:

1. wake from the configured timer;
2. measure battery voltage;
3. load configuration and the current desired frame metadata;
4. enable Wi-Fi and attempt one conditional pull;
5. validate and persist a changed frame;
6. refresh the panel only if the effective frame changed;
7. disable Wi-Fi and enter deep sleep.

At critical battery voltage the firmware skips Wi-Fi and display refresh,
leaves the panel powered off, and sleeps until VBUS or a conservative safety
timer wakes it. Exact thresholds will be hardware-tested before release.

An unconfigured device retains its last frame, stays reachable while USB is
present, and otherwise sleeps after its recovery window. It does not create an
access point.

## Canonical frame format

Every external source supplies a framebuffer in the Quote/0's native viewing
orientation, rather than in the UC8251D controller's portrait RAM geometry:

- width: 296;
- height: 152;
- storage: row-major, 37 bytes per row;
- bit order: most-significant bit is the leftmost pixel in each byte;
- value: `1` is white, `0` is black;
- byte length: exactly 5,624;
- orientation: native landscape as the device is normally viewed;
- digest: SHA-256 over those exact 5,624 bytes.

Typography, layout, dithering, and source-image conversion happen on the host
or server. The firmware performs the one fixed rotation/projection from the
canonical 296 x 152 image into the controller's 152 x 296 RAM order. This keeps
the hardware-specific scan geometry out of every server and host tool while
still leaving all content rendering off-device.

The handedness of that fixed projection will be captured by a hardware-checked
corner-marker fixture before the protocol is frozen. Once frozen, orientation
is part of protocol version 1 and is not selected per frame.

A frame becomes desired state only after its length and digest are verified.
The device may skip superseded frames. It must never expose or display a
partially received frame.

## Device-owned status bar

By default, the rightmost 16 pixels of the landscape canvas are a device-owned
status bar:

| Region     | Geometry  | Ownership                  |
| ---------- | --------- | -------------------------- |
| Content    | 280 x 152 | HTTP or USB frame producer |
| Status bar | 16 x 152  | Firmware                   |

Both regions are byte-aligned. HTTP and USB still carry the complete 296 x 152,
5,624-byte canonical frame, so enabling the bar does not create another frame
format. Producers should leave the status region white, but firmware always
overwrites all 16 columns before projection and display. With the bar disabled
by configuration, the producer owns the complete 296 x 152 canvas.

The bar is a snapshot captured whenever another reason already requires a full
panel refresh. Wi-Fi association, USB insertion/removal, Bluetooth changes, or
small battery changes do not cause a refresh by themselves. This avoids costly
full-screen refreshes and means the indicators describe state at the time of
the last displayed update, not a live dashboard.

The supplied 16 x 16 one-bit artwork is compiled into the firmware for Wi-Fi
(connected, pull failed, disconnected), battery (0/25/50/75/100, critical,
charging), and Bluetooth. It is rendered unchanged in the reserved columns:
Wi-Fi at the top, battery beneath it, Bluetooth lower down. Wi-Fi association
in progress uses the disconnected artwork because no separate progress icon
exists. USB VBUS remains the deliberately short `U` placeholder until its
bitmap is supplied.

Absent or inapplicable indicators are left blank. The Bluetooth icon reflects
whether the implemented BTHome beacon is advertising; it does not imply a
connectable GATT service. Replacing the final USB placeholder does not change the
status model, layout, or frame protocol.

The persisted `content_sha256` identifies the canonical producer-supplied
frame and remains the value used for HTTP caching and content-change decisions.
The `composed_sha256` identifies the exact image last sent to the controller
after the status overlay. A change in a candidate composed hash caused only by
status does not schedule a refresh; it is recorded only when a content,
configuration, recovery, or explicitly forced refresh occurs.

## HTTP(S) pull protocol

The firmware is an outbound client only. It sends a `GET` to the configured
URL with:

```http
Accept: application/vnd.quasiquote0.frame, application/octet-stream
If-None-Match: "previous-etag"
If-Modified-Since: previous-date
Authorization: Bearer <token>        # only when configured
User-Agent: quasiquote0/<version>
```

The server contract is deliberately small:

| Status | Meaning                                        |
| -----: | ---------------------------------------------- |
|  `200` | Body contains one complete native framebuffer  |
|  `204` | No desired content; retain the current display |
|  `304` | Current desired content is unchanged           |

A successful `200` response must have a body of exactly 5,624 bytes. The
device always calculates SHA-256 itself. A server may also send
`X-Frame-SHA256`; when present, it must match. `ETag` and `Last-Modified` are
saved only after the new frame has been validated and committed.

The client rejects oversized bodies as soon as the limit is crossed and
rejects short bodies at EOF. It uses bounded connection and read timeouts,
limits redirects, and never follows an HTTPS-to-HTTP downgrade. HTTP is
disabled by default and requires `allow_insecure_http: true`.

HTTPS uses the ESP-IDF certificate bundle by default. A later configuration
revision may add one custom CA certificate stored in LittleFS. Certificate
verification is never silently disabled.

The default pull interval is 15 minutes. Configuration may select a different
interval, with a conservative minimum while battery-powered. Exponential
backoff limits repeated Wi-Fi or server failures, but a manual `pull.run` is
not blocked by scheduled backoff.

## LittleFS persistence

All application-owned durable data lives below the `littlefs` mount:

```text
/config/device.v1.json       active configuration
/frames/<sha256>.bin         content-addressed native framebuffer
/state/device.v1.json        desired/displayed hash and HTTP validators
/state/diagnostics.bin       bounded recent fault records
/tls/custom-ca.pem           optional future custom CA
```

Configuration is a versioned complete document rather than a collection of
partially updated keys. A representative version 1 document is:

```json
{
  "schema": 1,
  "wifi": {
    "ssid": "example",
    "password": "secret"
  },
  "pull": {
    "enabled": true,
    "url": "https://example.net/devices/quote0/frame",
    "interval_seconds": 900,
    "timeout_seconds": 15,
    "bearer_token": "secret",
    "allow_insecure_http": false
  },
  "display": {
    "status_bar": true
  },
  "power": {
    "stay_awake_on_usb": true
  }
}
```

`config.get` redacts passwords and bearer tokens. Updating a redacted document
does not replace an existing secret with the redaction marker.

Every replacement follows this sequence:

1. write a temporary file;
2. flush and close it;
3. reopen and validate it;
4. atomically rename it over the active file;
5. remove obsolete content only after the active pointer is durable.

Frames are content-addressed so the metadata file can atomically point at a
complete immutable frame. The device retains the active frame until the new
one is safely referenced. At boot it removes abandoned temporary files and
unreferenced frames.

The desired content hash, displayed content hash, and composed display hash are
distinct. Displayed values are updated only after the UC8251D completes its
BUSY cycle successfully. A firmware-version change invalidates the displayed
values once so the first boot after OTA performs a real redraw even if the
desired frame survived.

Routine pull failures are kept primarily in RAM. Only bounded, important fault
records are persisted to avoid unnecessary flash wear.

## USB transport and control protocol

USB Serial/JTAG carries one Yamux session. Stream 0 is the project's explicit
extension for UTF-8 newline-delimited JSON-RPC 2.0. Standard Yamux reserves
stream 0 for the session; consequently a generic Yamux peer is not sufficient
without this small Quote/0 extension.

Large payloads never appear as Base64 JSON fields. JSON-RPC coordinates a
transfer, while a separate Yamux stream carries raw bytes. A newly accepted
stream announces its receive window immediately. Bounded device queues apply
backpressure even though Yamux's protocol-level initial credit is 256 KiB.

Control lines are limited to 16 KiB. Malformed JSON receives a JSON-RPC parse
error and does not reset the session. All methods with side effects return only
after the durable commit succeeds, unless they explicitly return an operation
identifier.

### Initial JSON-RPC methods

| Method                | Purpose                                                        |
| --------------------- | -------------------------------------------------------------- |
| `system.ping`         | Verify framing and liveness                                    |
| `device.hello`        | Protocol, firmware, hardware, and capabilities                 |
| `device.status`       | Power, battery, Wi-Fi, pull, frame, filesystem, and OTA state  |
| `device.reboot`       | Orderly reboot after replying                                  |
| `config.get`          | Return versioned redacted configuration                        |
| `config.set`          | Validate and transactionally replace configuration             |
| `wifi.clear`          | Remove Wi-Fi credentials while retaining the displayed frame   |
| `pull.run`            | Perform one pull immediately                                   |
| `frame.metadata`      | Return desired/displayed generation and hashes                 |
| `frame.begin`         | Open a device-selected binary stream for a direct frame upload |
| `ota.begin`           | Open a device-selected binary stream for an OTA image upload   |
| `ota.status`          | Report active slot, target slot, progress, and rollback state  |
| `diagnostics.get`     | Return bounded recent faults and reset reasons                 |
| `device.factoryReset` | Clear app state after a two-step confirmation                  |

`config.set` replaces the complete document. A successful response reports
whether a reboot or reconnect is required. Factory reset requires a short-lived
confirmation token obtained from `device.status` or a prepare call; it never
erases both firmware slots.

### Binary upload lifecycle

For a direct frame or OTA upload, the host calls `frame.begin` or `ota.begin`
with the exact byte length and SHA-256 digest. The device allocates and opens an
unused locally initiated Yamux stream, immediately announces its receive
window, and returns that stream ID and an operation ID in the JSON-RPC result.
The host writes the raw payload to that stream.

The receiver consumes bytes incrementally and does not wait for EOF. Once it
has received the declared number of bytes, it has all information required to
finish: it stops accepting payload, closes the stream from the device side,
verifies the digest, and commits or rejects the operation. The host must send
exactly the declared length and must not depend on closing its own write side
to make the device proceed.

A short transfer, timeout, reset, hash mismatch, or data received for a closed
operation fails or is discarded and leaves current state unchanged. Completion
is reported by operation status and may also be announced by a JSON-RPC
notification.

Only one frame upload and one OTA operation may be active, and display refresh
is serialized with HTTP pulls. Control traffic remains usable while a binary
producer or consumer is parked.

## Display behavior

The UC8251D driver follows the controller datasheet and the stock firmware's
verified full-refresh sequence and waveform tables. Partial updates remain out
of scope.

For every changed frame the display owner:

1. overlays the current status snapshot when the status bar is enabled;
2. projects the landscape image into the controller's portrait RAM order;
3. powers the panel and waits for stable GPIO state;
4. performs the required reset sequence;
5. initializes the controller and stock full-refresh LUTs;
6. writes a known old-image plane and the new 5,624-byte image plane;
7. triggers one display refresh;
8. waits for BUSY with a bounded timeout;
9. powers the panel down;
10. commits the displayed content and composed hashes only on success.

An unchanged content digest causes no panel power-up solely because a status
marker changed. A failed display operation keeps the desired frame so a later
boot can retry it.

The old and new image planes are constructed consistently on every refresh.
Repeated alternating frames and repeated refresh requests must never produce
the observed every-second-refresh white screen. This becomes a hardware
regression test rather than an assumption in the driver.

## USB OTA and rollback

OTA is delivered through USB, not fetched by the device. JSON-RPC carries the
manifest; a dedicated Yamux stream carries the ESP-IDF application image.

The flow is:

1. Host calls `ota.begin` with image length, SHA-256, and optional version
   metadata.
2. Device selects the inactive OTA partition, calls `esp_ota_begin`, opens a
   device-selected Yamux stream, and returns its stream and operation IDs.
3. The host writes exactly the declared length. The OTA worker reads bounded
   chunks from the Yamux stream and passes them
   directly to `esp_ota_write`; the full image is never held in RAM or
   LittleFS.
4. As soon as the declared length has arrived, the device closes the stream,
   verifies SHA-256, calls `esp_ota_end`, and selects the new boot partition.
5. Device records a small pending-upgrade marker in LittleFS, replies, and
   reboots.
6. The new image performs boot self-tests: LittleFS mount, configuration/state
   read, display-driver initialization without a forced visible clear, USB
   task startup when VBUS is present, and scheduler startup.
7. Only after those checks does it call
   `esp_ota_mark_app_valid_cancel_rollback` and clear the pending marker.

An interrupted or invalid upload never changes the boot partition. A new image
that resets before validation is rolled back by the bootloader. The host can
query the active slot and last rollback result after reconnecting.

SHA-256 detects corruption but does not authenticate a firmware author. For
the development release, physical USB access is the trust boundary. Production
hardening should add signed application verification, Secure Boot, and flash
encryption as one coordinated provisioning design rather than implying that a
hash alone is a signature.

## Boot and recovery behavior

Boot proceeds in the following order:

1. install ESP-IDF patches and capture reset/OTA state;
2. start USB/Yamux transport before filesystem, display, and radio setup;
3. mount LittleFS, recover abandoned transactions, and load validated state;
4. serve normal USB control, or recovery control if storage failed;
5. initialize optional display and radio services without blocking USB control;
6. complete pending OTA self-tests and validation;
7. redraw if no displayed digest is trustworthy;
8. enter the appropriate USB-powered or battery scheduler.

Corrupt configuration is quarantined and the previous/default configuration is
used. A corrupt frame is never sent to the panel. Failure to mount LittleFS
starts a USB-visible recovery mode when VBUS is available; it does not silently
format the volume.

Factory reset deletes application configuration, HTTP validators, frames, and
diagnostics. Formatting LittleFS is a separate recovery operation and requires
explicit confirmation. Neither operation erases the running or fallback OTA
slot.

## Error model and observability

Stable error codes accompany human-readable messages. At minimum the device
distinguishes:

- Wi-Fi association, DHCP, DNS, TCP, TLS, HTTP status, timeout, and body errors;
- frame length, digest, and persistence errors;
- filesystem full, mount, and transaction recovery errors;
- panel initialization and BUSY timeout errors;
- Yamux reset, flow-control, disconnect, and transfer mismatch errors;
- OTA image validation, write, selection, boot validation, and rollback errors.

`device.status` is a snapshot and remains sufficient if notifications are
missed. Optional JSON-RPC notifications may announce completed pulls, display
updates, OTA progress, and imminent reboot, but clients must not depend on them
for correctness.

No ordinary log text may be written into the USB Yamux byte stream. Development
logging uses the independent JTAG/debug path or a bounded in-memory diagnostic
buffer.

## Implementation plan

### Phase 0: Freeze contracts

- Add Rust types for the canonical framebuffer, hashes, configuration, durable
  state, and stable error codes.
- Check the pin map, LUT data, controller sequence, and flash offsets against
  the bundled reverse-engineering material and datasheet.
- Freeze the hardware-verified mapping from the public 296 x 152 landscape
  framebuffer to the controller's 152 x 296 RAM order.
- Add host-side protocol fixtures for Yamux stream 0, JSON-RPC, binary upload,
  and the HTTP response contract.

Exit criterion: protocol examples round-trip in host tests and no firmware
module needs to invent a second framebuffer representation.

### Phase 1: LittleFS durability

- Mount the existing partition and implement versioned config/state codecs.
- Implement temporary-write, validation, rename, boot recovery, space
  accounting, and secret redaction.
- Persist and reload a content-addressed 5,624-byte frame.
- Add power-loss tests at every commit boundary using a filesystem test double
  or fault-injection layer.

Exit criterion: every interrupted update recovers either the complete old state
or complete new state, never a mixture.

### Phase 2: USB management appliance

- Extend stream-0 JSON-RPC beyond `system.ping`.
- Add typed dispatch for hello, status, config, diagnostics, manual frame
  upload, reboot, and reset.
- Add a small host CLI that discovers the USB serial device, opens Yamux, and
  exposes these operations.
- Verify simultaneous control traffic and a parked binary stream.

Exit criterion: a factory-fresh device can be configured and given a persistent
frame using only USB.

### Phase 3: HTTP(S) frame pull

- Add lazy Wi-Fi station ownership and bounded connect/disconnect operations.
- Implement the strict `200`/`204`/`304` client and TLS policy.
- Persist ETag/Last-Modified only with the accepted frame.
- Add scheduled pull, manual pull, backoff, and unchanged-frame suppression.

Exit criterion: a local test server can update the display, while truncated,
oversized, bad-hash, bad-certificate, timeout, redirect-downgrade, and `304`
responses leave the current frame untouched.

### Phase 4: Display integration and power

- Route boot, HTTP, and USB frames through one desired-state commit/display
  path.
- Add the byte-aligned 16-pixel device status overlay with short placeholder
  glyphs and separate content/composed hashes.
- Correct and hardware-test the full UC8251D old/new plane sequence.
- Add VBUS-aware awake and battery wake/fetch/sleep schedulers.
- Add firmware-version redraw invalidation and critical-battery protection.

Exit criterion: alternating black/white/pattern frames over at least 100 full
refreshes never exhibit the every-second-refresh white-screen failure, and a
24-hour battery test shows bounded wake time and no unnecessary refreshes.

### Phase 5: Streamed USB OTA

- Add the binary OTA worker and JSON-RPC orchestration.
- Verify direct chunked writes to the inactive slot with bounded RAM.
- Add boot validation, rollback reporting, and first-boot display redraw.
- Extend the host CLI with `ota` and reconnect/status handling.

Exit criterion: successful upgrades boot the new version; disconnects at
multiple upload offsets keep the old image bootable; deliberate crash-loop and
corrupt-image tests roll back or reject safely.

### Phase 6: Release hardening

- Run stack watermark, heap, LittleFS wear/space, watchdog, and brownout tests.
- Test Wi-Fi and TLS failures, server outages, USB reconnects, malformed Yamux,
  full filesystem, panel BUSY stuck, and repeated abrupt unplugging.
- Document server examples, the host CLI, recovery, flashing, and the security
  model.
- Produce a reproducible release image and partition manifest.

Exit criterion: the complete hardware acceptance matrix passes on a real
Quote/0 and the recovery instructions restore a deliberately broken test unit.

## Acceptance matrix

The first usable release must demonstrate all of the following on hardware:

- USB-only provisioning from factory-reset state.
- Configuration survives reboot and abrupt unplugging.
- Last valid frame survives reboot, network outage, server error, and bad next
  response.
- `304` and identical `200` bodies cause no display refresh.
- `204` retains the current display.
- HTTP works only when explicitly enabled; HTTPS validates certificates.
- USB JSON-RPC remains responsive while a transfer is blocked on input or
  flow-control credit.
- `frame.begin` and `ota.begin` return device-selected stream IDs, and the
  device closes each stream immediately after the declared payload length.
- Frame and OTA uploads use bounded memory and do not aggregate their inputs.
- A landscape corner-marker fixture displays with the documented handedness.
- The enabled status bar always replaces the rightmost 16 columns, placeholder
  glyphs fit without entering the 280 x 152 content area, and transient status
  changes alone cause no panel refresh.
- Repeated full refreshes do not blank every second update.
- Interrupted OTA boots the previous slot; a failed new boot rolls back.
- The first boot after a successful OTA redraws the retained desired frame.
- Battery mode disables the radio outside its fetch window and protects the
  cell at critical voltage.
- No application log bytes corrupt the USB protocol.

## Deferred decisions

These choices are intentionally left until the core path is hardware-verified:

- custom CA installation and replacement policy;
- multiple Wi-Fi profiles;
- signed OTA/Secure Boot/flash-encryption provisioning;
- an optional richer frame container with generation and server timestamp;
- direct USB frame upload as a public feature versus a diagnostic feature;
- partial-refresh waveforms;
- final one-bit status icons and their exact pixel spacing;
- retained diagnostic depth and flash-write policy.

None of them should change the native framebuffer, client-only network model,
LittleFS ownership, or streamed USB OTA architecture described above.

## Reference material

- `target/quote0/re/RE-HANDBOOK.md`: stock firmware behavior, hardware map,
  boot, power, display, and OTA notes.
- `target/quote0/re/RE-EPD.md`: extracted UC8251D initialization, LUTs, and
  display pipeline.
- `target/quote0/re/datasheets/GDEQ0213Z19.pdf`: panel/controller datasheet.
- `target/quote0/docs/PROPOSAL-v0.md`: earlier remote-framebuffer proposal.
- `target/quote0/re/NEW-FIRMWARE-PLAN.md`: earlier cloud-free firmware plan and
  hardware-test lessons.
- Hashicorp Yamux specification and `libp2p/rust-yamux`: framing and
  flow-control references. Stream 0 remains a documented project extension.
