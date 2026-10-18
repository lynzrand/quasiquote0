# Firmware design ablation, 2026-10-18

Reviewed source revision `107bb500`. These are isolated compile/link experiments,
not battery benchmarks or replacements flashed to the device.

The strongest simplification is to replace the USB management appliance with
a browser UI and a small USB bootstrap channel, while retaining BLE. JSON data
itself is not the main problem: multiplexed streams, generic dispatch, parallel
upload workers, and operation tracking account for much of the extra machinery.

## Direction agreed during the review

- With USB power present, join configured Wi-Fi and keep the connection up.
  Serve the browser UI in this mode.
- When unconfigured and USB-powered, expose a setup hotspot. Keep a small USB
  initial-configuration/recovery path available as well.
- On battery, leave the web server and setup hotspot off. Connect Wi-Fi for
  scheduled pulls; minute status-bar wakes remain independent.
- Synchronize time with NTP when Wi-Fi is available. Preserve RTC time through
  sleep; do not start Wi-Fi on every clock wake. Obtain usable time before an
  HTTPS pull after a cold boot. If offline, show unknown time rather than
  inventing a clock value.
- Keep BLE as a capability. Dropping it is a measured alternative, not the
  selected direction. A GATT management API has no demonstrated need yet.

The implementation now follows this direction. The original measurements
below remain compile-only probes at the reviewed revision, not measurements
of the complete replacement. See [WEB_MANAGEMENT.md](WEB_MANAGEMENT.md) for
the implemented build and hardware checks.

## Measured flash costs

All variants retain the same SDK configuration, compiler settings, OTA partition
layout, and experimental status-window feature. Each OTA slot is 1,920 KiB.
KiB below means 1,024 bytes.

| Variant | Image, KiB | Saved, KiB | Slot headroom, KiB |
| --- | ---: | ---: | ---: |
| Controlled current-design baseline | 1,868.95 | 0 | 51.05 |
| Remove BLE startup/advertising references | 1,684.34 | 184.61 | 235.66 |
| Remove USB application management | 1,679.66 | 189.30 | 240.34 |
| Remove both | 1,495.03 | 373.92 | 424.97 |
| Keep BLE; replace USB management with minimal HTTP + NTP core | 1,726.30 | 142.66 | 193.70 |
| Keep current behavior; remove unused async integrations | 1,861.91 | 7.05 | 58.09 |

The USB removal variant removes normal/recovery RPC dispatch and its upload
workers; it retains USB Serial/JTAG initialization, networking/pulls, storage,
display, clock, and sleep guards. It is not an equally functional replacement.
Its saving includes handlers and paths that a real web interface must replace,
not just protocol headers.

The HTTP/NTP variant adds an ESP-IDF HTTP server, one tiny static page, and the
SDK SNTP service. It keeps the real BLE implementation. It does **not** contain
the real UI, configuration/upload handlers, bootstrap parser, setup-hotspot
policy, admin access control, or the new Wi-Fi lifecycle. Its 142.66 KiB saving
is a budget probe, not a finished-product size prediction.

The BLE cut stubs application entry points; SDK Bluetooth options remain enabled.
It measures code reachable from the present application rather than a fully
retuned Bluetooth-free SDK. Savings are not generally additive.

The async cut removes the unused direct `embassy-time` dependency and the
`embassy-time-driver` / `embassy-sync` service features. The application uses
blocking tasks and has no async call sites. This is a straightforward cleanup
candidate with a smaller, measured benefit.

## RAM and energy

The baseline ELF has about 104.85 KiB of IRAM code and 34.46 KiB of static DRAM.
The HTTP/NTP probe has about 102.50 KiB and 34.48 KiB respectively. These figures
exclude task stacks and runtime allocations; they do not demonstrate equal
runtime memory use.

The USB transport reserves a 12 KiB Yamux task stack and an 8 KiB control task
stack. Frame and OTA uploads each create another 8 KiB worker. Receive queues
are limited to 4 KiB per stream and the outbound payload queue to 4 KiB. A
serialized HTTP upload path can eliminate stream queues and upload workers,
but adds the HTTP service task, sockets, request buffers, and page storage.
The SDK's plain HTTP server defaults to a 6 KiB stack. Net heap savings need
runtime measurement.

The main task is configured with a 64 KiB stack to accommodate frame buffers
and temporaries. Profile its stack watermark before reducing it; its entire
reservation is not evidence of actual use.

The minute heartbeat schedules up to 1,440 display updates per day. At the
default 15-minute pull interval there are 96 regular pull deadlines per day.
The user wants the heartbeat, so retain it, but it is a more significant energy
decision than trimming a glyph or metadata field. The tested partial window
took 2,249 ms versus 2,265 ms for a full transaction, including setup and sleep;
that result does not establish an energy saving. Battery wakes still use full
refresh. No current or battery-life measurement was made in this review.

## Remove work before removing useful capabilities

Three source findings strengthen the case for simplification:

1. **BLE's unchanged-payload cache is never initialized.**
   `src/bluetooth.rs` calls `LAST_PAYLOAD.get()` in both cache helpers, with no
   initializer. The comparison therefore uses a zero payload and unchanged
   advertisements can be stopped/restarted every scheduler iteration. Fixing
   that preserves BLE while removing repeated work. No radio power saving has
   been measured.
2. **Yamux stream state is not reclaimed.**
   `src/yamux.rs` inserts streams in a connection map, but neither local close
   nor received FIN/RST removes them. There is no stream-count cap. Per-stream
   buffering limits therefore do not bound the lifetime connection state.
   Repeated uploads are another maintenance case that a single-upload API
   does not need.
3. **Frame publication should have one owner.**
   HTTP pulling and USB uploads publish and commit through separate steps.
   Between `FrameWriter::finish()` and the desired-state commit, another commit
   can run orphan cleanup and remove the newly published, not-yet-referenced
   frame. This is a possible interleaving found in code, not a hardware failure
   observed here. Serialize publication through commit and cleanup when moving
   uploads to HTTP.

Also defer the proposed GATT control mirror, remove the bring-up `i2c.scan`
operation from the normal product surface, and keep standalone clock/recovery
tools as diagnostics. Deleting unused source is not necessarily a flash win:
the old `status_screen` renderer has no linked symbols in the measured image.

## Recommended smaller product

Keep the 296x152 frame contract, retained content, atomic persistence, HTTPS
pulls, byte-count/hash checks, dual-slot updates/rollback, battery protection,
USB wake guards, minute status clock, and BLE capability.

Replace Yamux, JSON-RPC, operation IDs, and independent USB upload workers with:

- a small browser UI and ordinary HTTP configuration/status/frame/update routes;
- one active mutating transfer, streamed to the existing store or inactive OTA
  slot, with a bounded idle timeout and explicit success/failure;
- a bounded USB setup channel for Wi-Fi configuration and connection status,
  without frame/firmware streaming or multiplexing;
- NTP time synchronization rather than a required USB clock-setting workflow.

Configuration can still use JSON. Removing JSON-RPC does not require inventing
a binary configuration schema. Web writes need device-local authorization;
that is part of the replacement's implementation, beyond the size probe.

When configured Wi-Fi is unreachable, retain bounded connection retries and
USB bootstrap access. Clearing Wi-Fi settings returns the device to the
unconfigured hotspot path. Do not add a second permanent AP or battery web
server to work around provisioning.

Before enabling the replacement on hardware, verify browser provisioning,
offline startup, NTP/HTTPS startup ordering, streamed frame/update failure
handling, and USB removal during an operation. Keep the working USB management
firmware available through that migration. A tiny UI, narrow routes, and a
single owner provide the simplification; an additional generic RPC service
over HTTP would retain much of the current complexity.

## Reproduction artifacts

The review generated `target/ablation/run.py`, isolated source copies under
`target/ablation/workspace`, six `.elf` / `.bin` pairs, individual build logs,
section-size reports, and `target/ablation/results.json`. These are ignored
local artifacts. The runner shares the existing SDK cache/build target but
edits only its source copy; it never flashes a device. A normal project build
is restored afterward. The source and SDK settings reviewed are recorded by
revision above, so later architectural changes require a fresh comparison.
