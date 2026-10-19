# Firmware architecture

`src/main.rs` claims the board peripherals, starts USB recovery, constructs the
panel driver, and hands ownership to `Appliance`. The main task remains the
single owner of the panel, Wi-Fi policy, pull schedule, and BLE advertising.
There are no per-upload workers or general-purpose remote procedure calls.

## Code map

| Location | Responsibility |
| --- | --- |
| `appliance.rs` | Owner loop, browser lifecycle, pull scheduling, boot confirmation, sleep |
| `appliance/render.rs` | Frame composition, refresh budget, panel transfer, acknowledgement |
| `app/` | Shared state, configuration mutations, management status snapshots |
| `management/` | Authenticated HTTP routing, bounded body reads, frame and OTA uploads |
| `bootstrap.rs`, `bootstrap_line.rs`, `usb_serial.rs` | Physical USB setup/status and bounded parsing |
| `device.rs`, `store.rs` | Durable configuration, frame publication, display acknowledgement |
| `network.rs`, `network_policy.rs`, `wifi.rs` | USB station/setup mode, battery pull windows, NTP, radio mechanics |
| `pull.rs`, `pull_schedule.rs`, `rtc_schedule.rs` | HTTP frame fetching, retry policy, retained deadlines |
| `bluetooth.rs`, `bthome.rs` | NimBLE host and driver-independent BTHome v2 encoding |
| `frame.rs`, `display/` | Canonical landscape bytes, fixed board projection, UC8251D transport |
| `status_bar.rs`, `status_icons.rs`, `clock_face.rs`, `embedded_graphics.rs` | Status composition and drawing |
| `power.rs`, `power_policy.rs`, `battery.rs` | USB recovery guarantees, wake sources, battery sensing |
| `activity.rs` | Exclusive mutation/publication lease |
| `ota.rs` | Stream verification, inactive-slot writes, boot selection and confirmation |
| `web/` | Browser settings, image conversion, SHA-256, embedded page |

## Ownership and durability

The USB task and HTTP server share `App` with the owner loop. `Device` serializes
its configuration and persistent state with one mutex. Slow network reads and
panel transfers run outside that mutex. An operation lease spans each upload
or pull through verification, publication, and cleanup, preventing concurrent
publishers and battery sleep during mutation.

A frame writer streams to a temporary file, checks the exact 5,624-byte length
and SHA-256, and publishes an immutable content-addressed file. The state file
is atomically replaced before RAM points to the new frame. Unreferenced frames
are collected after publication and at startup. Interrupted uploads leave the
previous desired frame available.

The retained source frame excludes device overlays. Each refresh composes the
status strip from that source. Content, layout, and firmware transitions persist
display acknowledgement; minute ticks update acknowledgement in RAM without
rewriting flash. A partial-window baseline and budget are valid only within the
current boot. Full refresh remains the default.

OTA streams directly to the inactive partition. After length, digest, and SDK
image validation, the pending marker is persisted and the partition is selected.
The new boot confirms only after storage recovery checks, panel refresh, and ten
seconds of owner-loop progress. Startup failures can trigger bootloader rollback.

## Power and resource constraints

USB keeps the MCU awake unconditionally and enables the browser service over
saved Wi-Fi, or a setup hotspot when unconfigured. On battery, minute wakes
refresh the status bar; Wi-Fi starts only when a content pull is due. Timer and
GPIO0-high wake sources must both be armed before deep sleep. USB and operation
checks are repeated immediately before sleep. Critical battery suspends display
and network work.

GPIO20 is panel power, so the UART console must remain disabled. USB Serial/JTAG
is the physical setup and recovery transport. Keep the verified 64 KiB main
stack: full-frame composition and projection have substantial stack peaks.
Wi-Fi/TLS buffers and NimBLE broadcaster-only roles are intentionally bounded
in `sdkconfig.defaults` to leave room for the panel and browser uploads.

## Saved-format compatibility

The v1 configuration still accepts the retired `power` object when loading old
settings or receiving an old client's save request. It is ignored and omitted
from new configuration output. Unknown active configuration fields are rejected.
Saved frame provenance and unused pull diagnostics are ignored when reading old
state; retained frame digests, HTTP validators, credentials, and OTA markers keep
their existing format. Cleanup requires no erase or reprovisioning.

## Verification

`tests/host` compiles the production policy, frame, encoding, and persistence
modules for the host. Durability tests cover failed state/OTA writes, interrupted
or corrupt frames, old saved documents, and flash-free minute acknowledgements.
See the README for build, lint, and browser test commands. The real-browser test
uses mocked device APIs and an isolated browser context; on-board checks in
`tools/check_browser.py` exercise authentication, redacted settings, malformed
and interrupted uploads, and optional explicit frame/firmware files.
