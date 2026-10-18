# Browser management implementation and hardware acceptance

Implemented on 2026-10-18, following [ABLATION.md](ABLATION.md).
The current wire and lifecycle contracts are in [PROTOCOL.md](PROTOCOL.md).

## Delivered behavior

- USB power keeps the CPU awake and the configured Wi-Fi station connected,
  regardless of whether frame pulling is enabled. The local browser UI manages
  Wi-Fi/pull settings, status, frames, firmware, and restart.
- USB power without configured credentials starts a unique WPA2 setup hotspot.
  A small physical USB channel reports credentials/address and sets or clears
  Wi-Fi. The Windows tool copies only the selected saved profile locally;
  exported plaintext keys are temporary and deleted after use.
- API requests use a persistent device access code. Configuration reads redact
  Wi-Fi passwords and bearer tokens; `***` preserves stored secrets on save.
- One operation lease covers publication through persistence and cleanup.
  Frame and firmware uploads are streamed, length/digest checked, and bounded
  by idle and overall deadlines. USB removal aborts uncommitted uploads.
- NTP starts on an active station. Clock-only battery wakes keep networking off;
  due pulls can connect the station and obtain time before HTTPS. RTC pull
  scheduling and minute status heartbeat remain independent.
- BLE retains the working non-connectable BTHome beacon. Unused scanning,
  pairing, and GATT connection pools are excluded; connectable GATT remains
  deferred. Normal firmware does not compile Yamux/JSON-RPC; `legacy-usb`
  retains the standalone clock diagnostic and source for old tools.
- OTA writes the inactive slot and verifies it before durable boot selection.
  Confirmation waits for storage/USB startup, successful panel refresh, and
  ten seconds of owner-loop progress. The installed bootloader supports rollback.

## Hardware checks

The Quote/0 used Windows COM4 and the recovered native JTAG driver; USB did
not need forwarding into WSL. The selected saved network was provisioned
without exposing its key in logs. The retained producer frame remained
`3140d1dfa18cf7dfc6b2b845e75d2298d4239b28060cd85b7125354d4ab79bdf`.

Verified on the board:

- Wi-Fi joins and remains connected with pulling disabled; the browser page
  and authenticated status/configuration routes respond over the LAN.
- Missing access code returns 401; web status excludes access/setup secrets.
  A redacted configuration round trip preserves the Wi-Fi password.
- Wrong frame lengths/hashes and a disconnected partial body preserve the
  desired frame. USB status remains independent of a pending browser upload;
  the operation lease releases after disconnect.
- An invalid OTA digest leaves the running slot unchanged. A valid HTTP upload
  boots the other slot, confirms it, and retains configuration, frame, and
  access code. Subsequent HTTP updates exercised both OTA slots.
- Windows joined the actual WPA2 setup hotspot, received DHCP, loaded the UI,
  read authenticated configuration, and provisioned the saved station through
  the HTTP configuration route. The temporary Windows hotspot profile was
  removed afterwards. A one-second radio switch delay lets the save reply finish.
- NTP established wall time from an initially unknown RTC. Full and partial
  status refreshes continue with Wi-Fi and the BLE beacon active.
- A certificate-validated HTTPS request to `https://example.com/` reached frame
  validation, correctly rejected its short HTML body, and preserved the image.
  The final build completed this check without a reset while BLE advertised.

The final firmware passed the battery/USB reconnect regression: the user
observed two minute changes while unplugged, and the watcher recorded reset
reason 8 (deep sleep), wake cause 7 (GPIO), retained synchronized RTC time and
the same producer digest, both wake sources armed, USB awake protection, and
automatic return to station mode after reconnect. BLE was advertising again.
The temporary HTTPS endpoint was removed and pulling restored to disabled
with its normal 900-second interval.

A new valid frame was not pushed during this migration, preserving the user's
existing picture. Actual storage success/error paths are covered by the host
filesystem tests; the HTTP upload path was exercised through complete invalid
bodies and interruption. Forced rollback and power cuts during OTA have not
been tested. No battery-life or RF-throughput claim is made.

## Memory and image budget

Build: `cargo build --locked --offline --bin quasiquote0 --features experimental-partial`.
Image SHA-256: `fab900cccb8f6b19756d4b26baada4c4fcb2c82f266c7e21123ad1fd669bba66`.

Image: 1,826,640 bytes (1,783.83 KiB), leaving 136.17 KiB in each 1,920 KiB OTA
slot. This is 85.13 KiB smaller than the controlled 1,913,808-byte baseline in
ABLATION.md. The smaller HTTP/NTP probe was not a complete implementation.

Hardware integration exposed two existing limits: HTTP must start after network
initialization, and the panel's SPI error path must return rather than panic.
Wi-Fi reserves four static RX buffers, eight dynamic RX/TX buffers, and a
four-packet RX aggregation window. Wi-Fi speed-only IRAM optimizations are
disabled to reclaim heap. TLS uses dynamic record buffers while retaining the
normal 16 KiB incoming record limit and certificate validation. BLE enables
only the implemented broadcaster role and one controller activity.

The final build reported about 44 KiB of idle free heap; an HTTPS failure-path
check with USB status queries reached an 8,136-byte minimum. These are observed
samples, not a general heap bound. The 64 KiB main stack had about 8 KiB spare
after full display refresh, so it was retained. Earlier untuned builds could
exhaust heap and reset during TLS; those are not the installed final build.

## Repeatable checks

- Host suite: 34 unit checks plus four real-filesystem durability checks.
- `node tests/web_sha256.cjs`: browser SHA-256 checked against Node crypto,
  including padding boundaries, exact frame size, and maximum OTA size.
- `python tools/check_browser.py --port <port>`: non-destructive live API,
  redaction, invalid-frame, and interrupted-body checks.
- Add `--frame <frame.bin>` or `--firmware <application.bin>` only when those
  uploads are intended; the latter verifies both rejected and successful OTA.
- The standalone clock still builds with `--bin quote0-clock --features legacy-usb`.

Windows setup and flash scripts were parsed with the PowerShell parser.
JTAG flashing now detects the active OTA slot through USB; explicit `-Slot`
is required when recovery cannot query it. It does not change boot selection.
