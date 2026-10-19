# Status-strip partial-window experiment

The normal firmware uses the hardware-verified full-refresh path. Build
`quasiquote0` with `--features experimental-partial` to test a status-only
window after a successful full baseline in the same boot.

This is an area-limited stock-waveform refresh, not a faster differential
waveform. It retains the panel's proven voltage, timing, LUT, and old-plane
settings. MindReset's [public display driver](https://github.com/MindReset/dot_open_platform/blob/main/devices/quote_0/firmware-resources/display/README.md)
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
unchanged content hash with the status bar enabled is eligible. The first
update, content changes, and every fifth update use full refresh. A
partial timeout powers off the controller before a cold full-refresh
fallback, so partial addressing cannot leak into the full transfer.

The partial budget and baseline are currently per boot: battery deep-sleep
wakes use full refresh. RTC retention of a partial baseline is deferred
until the physical window behavior is verified. USB always remains awake.

`GET /api/status` under `display` includes `refresh_mode` (`full`, `partial-window`, or
`full-fallback`), `refresh_ms` (complete panel transaction, including setup
and sleep), and `partial_error`. A completed transaction alone does not
prove that only the selected pixels refreshed; visual confirmation is
required.

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

Further work is needed for a separately validated fast differential
waveform and for retaining the partial baseline/budget across battery
deep-sleep boots. Neither is enabled by this experiment.
