# Display refresh profiles

Normal builds use the accepted 48-frame short waveform for every normal
refresh: startup, full-frame baselines after sleep, main-image changes, and
status windows. A full destination write drives both old states to each
target color, so it does not require previous-image data. Differential updates
leave unchanged pixels at zero source drive. All paths retain cold reset and
power cycling, and identical composed images skip panel transfers.

The original stock waveform is preserved for error recovery. Build with
`--no-default-features` to use it as the full-refresh control. Add
`--features experimental-partial` to that command for the earlier status-only
window after a successful full baseline in the same boot.

Build with `--no-default-features --features experimental-differential` for the first waveform
modification: W->W and B->B use zero source-drive levels, while B->W, W->B,
VCOM, all frame counts, and voltage settings remain exactly stock. This
includes `experimental-partial` and uses the same window and cleanup budget.
It also supports differential updates of uploaded main images, scanning the
full panel against its previous image. The stock window build remains
available as a comparison control.

The default `experimental-fast` profile uses a direct 48-frame destination pulse and
three neutral frames. All voltage settings stay stock;
unchanged pixels still use zero source drive. It retains the original
differential build as a timing/contrast comparison control. This is a separate
waveform experiment, not a claim that stock contrast or ghosting is preserved.
The combined short-waveform/USB-standby hardware trial produced an all-black
screen and was reverted to the normal build. USB standby has been removed;
the current fast build uses the proven cold reset and power cycle for every
transaction. The user confirmed correct rendering with this cold path at
about 1.35 seconds per main-image update. The failed warm trial's shorter
transaction times are not a validated display-speed improvement.

The `experimental-partial` control is an area-limited stock-waveform refresh,
not a faster differential waveform. It retains the panel's proven voltage,
timing, LUT, and old-plane settings. The [waveform reconstruction](WAVEFORM.md)
describes the generated stock LUT's phase levels, timings, and byte-for-byte
verification.
MindReset's [public display driver](https://github.com/MindReset/dot_open_platform/blob/main/devices/quote_0/firmware-resources/display/README.md)
provides full refresh only; its [waveform notes](https://github.com/MindReset/dot_open_platform/blob/main/devices/quote_0/firmware-resources/display/waveforms/README.md)
provide no separate partial waveform.

During bring-up, the GDEQ0213Z19 datasheet was used to check the
UC8251D command family, including `0x90` (window), `0x91` (partial in), and
`0x92` (partial out). That PDF covers a different 104x212 color panel, not
this 152x296 monochrome Quote/0 panel. It is a register reference rather
than a matching panel waveform specification.

The experiment maps the entire rightmost 16 landscape columns into native
controller rows 0..15, selects columns 0..151, and sets `PT_SCAN=0`. The
old and new planes each transfer 304 bytes instead of 5,624. Only an
unchanged content hash with the status bar enabled is eligible for the strip
window. The stock control uses full refresh for content changes. The
differential build instead compares the entire frame for content/layout
changes. The first update and every fifth content/layout change use a full
destination write: the short waveform in normal builds and stock conditioning
in the control builds. A transfer failure powers off the controller before a
cold, stock full-refresh
fallback, so partial addressing cannot leak into the full transfer.

The partial budget and framebuffer baseline remain per boot: changed pixels
after battery deep-sleep wakes require full refresh. The renderer now retains
only the last composed-image digest in a versioned, checksummed RTC struct.
An identical composed frame skips all panel transfers, both within a boot
and after deep sleep. Cold resets and OTA ignore RTC display records. The
first changed image after a skipped wake still establishes a full baseline.
Clock-only windows and identical-pixel skips neither consume nor reset the
content/layout cleanup budget. Idle content therefore never schedules a
periodic full-screen refresh. USB always
remains awake.

`GET /api/status` under `display` includes `refresh_mode` (`full`, `partial-window`, or
`full-fallback`), `refresh_ms` (complete panel transaction, including setup
and sleep), and `partial_error`. A completed transaction alone does not
prove that only the selected pixels refreshed; visual confirmation is
required.

The differential build reports `partial-differential` for a successful
status-window transaction and `differential` for main-image or layout changes.
The normal build reports `fast-full`, `fast-window`, or `fast-differential`.
`full-fallback` identifies stock recovery after failure. `unchanged` with
`refresh_ms = 0` means metadata was acknowledged without driving the panel;
`refreshes` counts only physical transactions.
Its driver retains the last successfully displayed 5,624-byte native frame
on the heap and uploads the relevant bytes as the old image after every
controller restart. Strip updates copy only 304 bytes back into that baseline;
main-image updates copy the complete frame. A full baseline is required in
this boot; any transfer error invalidates it before the cold full-refresh
fallback. All full writes use the zero old-plane. The short full-write profile
uses destination pulses for W->W and B->B as well as the changing transitions;
recovery explicitly restores every stock LUT. Image uploads initiate differential main-image
refreshes immediately, without waiting for the clock.

In the original differential build, zero source-drive selectors retain the
stock timing fields rather than using an all-zero table that would terminate
early. The waveform still runs
for the same 198 programmed frames plus its ending; this experiment targets
unchanged-pixel flashing, not speed. Zero source drive is not floating and
VCOM/border behavior is unchanged, so optical preservation and ghosting must
be checked on the panel. In particular, the previous all-ones old-plane
experiment failed; using real mixed old-image data with distinct unchanged
LUTs is a separate experiment, not an established hardware result.

Left-to-right staggering would require successive partial windows (landscape
left maps to high native row indices). Keeping the controller powered between
windows would avoid repeated initialization, but every band would still run
the selected waveform. No concurrent-window overlap is implemented here.
The documented controller registers provide a shared waveform sequence and
one partial window, with no programmable per-row phase offset.

## Short waveform

The fast profile uses one group with frame counts `[48, 1, 1, 1]`. During
the first phase, B->W uses VSL and W->B uses VSH, following the polarity of
the stock final-drive pulses. The other phases are neutral. VCOM, W->W, and
B->B use zero selectors with matching timing fields. At the stock 160 Hz,
51 programmed frames and the two-frame ending nominally take 331.25 ms.
The pulse length is experimental; optical density, residual images, and
temperature dependence require separate visual validation.

The current fast build retains the full board-proven reset, initialization,
and power-off sequence. It sends POF (`0x02`), deep sleep (`0x07`), and removes
panel power after each transaction. An earlier warm-start trial kept digital
power after POF, but was removed after an all-black optical result. Timeouts
force external power-off, clear the previous image, and require cold recovery.

The full native framebuffer is still retained only in ordinary heap RAM.
The 48-byte `SavedDisplay` RTC record contains a version tag, composed SHA-256,
and checksum, not framebuffer pixels or credentials. A pending/failed refresh
invalidates it before panel transfer; successful acknowledgement replaces it.
This avoids per-minute flash writes and avoids trusting a partially changed
image after failure. Deep-sleep skip logic is covered by host simulations;
physical battery-wake behavior requires a separate hardware run.

Build and convert the experimental image using the esptool settings in
[CONTRIBUTING.md](../CONTRIBUTING.md#application-images-for-browser-ota).
Flash with `tools/flash_app_windows.ps1 -Image <image.bin>`. To
return to full refresh, rebuild without the feature and flash that image;
no configuration, partition, or saved-frame migration is involved.

## Hardware result, 2026-10-18

The test image was programmed and verified through Windows JTAG. At the
20:54 UTC+8 minute rollover, the transaction completed without a partial
error, the retained content hash remained unchanged, no additional HTTP
pull ran, and the user confirmed that only the status strip refreshed.
The complete partial transaction took 2,249 ms versus 2,265 ms for its full
baseline. This demonstrates the selected area, not a meaningful refresh
speed or battery-life improvement. The regular and experimental builds
both compile and pass the host policy tests.

Further work is needed to validate the differential build optically, shorten
its waveform, and retain the partial baseline/budget across battery deep-sleep
boots. The historical hardware result above describes the stock window build.

## Differential runtime result, 2026-11-07

The differential image was installed through browser OTA and confirmed in
`ota_1`. A generated checkerboard image established a full baseline in
2,297 ms. At the next two clock changes, status reported
`partial-differential` in 2,258 ms and 2,260 ms, with no display or partial
errors and no full-refresh fallback. The retained test-content digest did
not change. Main-task minimum free stack remained 13,340 bytes and free
heap was about 43 KB. This validates completed transactions, not reduced
optical flashing, contrast, or long-term ghosting; visual feedback is pending.
The original source image was restored afterward, and an exported backup
matched the pre-experiment image and complete configuration. The device was
left running the differential build for further visual checks.

The experiment was then extended to retain the full native image on the
heap. After OTA confirmation in `ota_0`, two main-image uploads at 12:41
Hong Kong time showed a center test patch and restored the original image.
Both reported `differential`, taking 2,274 ms and 2,314 ms, without display
errors or fallback. A backup comparison confirmed exact restoration of the
source image and settings. Minimum free main-task stack was 13,864 bytes;
free heap was about 37 KB. The user confirmed that unchanged white pixels
were not updated. Density/contrast and long-term ghosting remain tuning concerns.

## Failed fast/standby trial and unchanged-frame result, 2026-11-07

The combined fast/standby image confirmed OTA in `ota_1`. Two main-image
transactions reported `fast-differential` at 519 ms and 545 ms. Two uploads
that changed only pixels hidden by the overlay were then acknowledged as
`unchanged`, with `refresh_ms = 0` and no increment in physical refresh count;
the original source image and complete settings were restored exactly.

The user subsequently reported no observed image change and an all-black
screen. The high-contrast test upload had actually used the periodic stock
cleanup through the warm path (`full`, 1,448 ms), so the failure cannot be
attributed to the short LUT alone. The normal, cold full-refresh build was
restored and confirmed in `ota_0`; its baseline transaction took 2,280 ms.
The test image is retained for visual recovery confirmation. The user's
pre-test image/settings backup is stored privately outside tracked source.
The user confirmed that the stock cold-path redraw displayed correctly.

USB standby was then removed from the fast build. The revised image confirmed
OTA in `ota_1`, and an inverse main-image pattern completed as
`fast-differential` in 1,351 ms using the full cold reset/power cycle. Its
left half should be white and its right half black, with inverted text and
checkerboard blocks. The user confirmed correct rendering, then reported
transient ghosting and accepted it without further tuning. The accepted
settings retain the 48-frame pulse and stock voltage codes (`0x3F` for both
source rails). Long-term ghosting and physical battery-wake behavior remain
unverified.

## Default short profile, 2026-11-07

The accepted short profile now serves all normal refreshes, including startup
and periodic full-frame baselines. The stock profile is selected only for
transfer-error recovery in normal builds. The new default image confirmed
OTA in `ota_0`. Five successive image changes completed without errors or
fallback: four `fast-differential` updates took 1,358, 1,378, 1,384, and
1,357 ms; the periodic `fast-full` destination write took 1,352 ms. The
latest user image and complete settings were restored and verified against
the pre-test backup. Startup profile selection and stock error recovery
are also covered by transport-level host tests. Physical battery wakes
remain unverified.
