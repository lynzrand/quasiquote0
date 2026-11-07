# Stock UC8251D waveform

`src/display/luts.rs` encodes the stock waveform from named levels and timing
groups at compile time. The panel driver receives static byte slices; there
is no runtime calculation or allocation during refresh. This reconstructs the
existing full-refresh waveform without changing its electrical drive sequence.

The reference is **UC8251d A0.1**, sections 14-20 and the data-transmission
waveform on page 43: [controller datasheet](https://www.buydisplay.com/download/ic/UC8251.pdf).
This revision documents the eight-byte groups used by the Quote/0 extraction.
The six-byte layout in the GDEQ0213Z19 module specification describes a
different format and must not be used to decode these tables.

## Encoding

Each eight-byte group contains:

| Byte | Meaning |
| --- | --- |
| 0 | Number of repetitions of the complete group |
| 1 | Four level selectors, from bits 7:6 through bits 1:0 |
| 2, 3 | Frame counts for the two phases of state 1 |
| 4 | State 1 repetitions |
| 5, 6 | Frame counts for the two phases of state 2 |
| 7 | State 2 repetitions |

The controller runs state 1, then state 2, then repeats the group. Pixel-table
level codes are `00 = 0 V`, `01 = VSH`, `10 = VSL`, and `11 = VDHR`. The stock
profile uses the first three. In the VCOM table, code `00` selects `VCOM_DC`;
that is a reference voltage, not a claim that the common electrode is at 0 V.
Voltage amplitudes are configured separately from the LUT.

The register payloads retain their original lengths:

| Register | Role | Data bytes | Groups |
| --- | --- | ---: | ---: |
| `0x20` | VCOM | 80 | 10 |
| `0x21` | White to white | 56 | 7 |
| `0x22` | Black to white | 80 | 10 |
| `0x23` | White to black | 80 | 10 |
| `0x24` | Black to black | 80 | 10 |

All five tables contain four active groups, followed by zeros. A zero group
repeat terminates the waveform. The controller then outputs its documented
two-frame ending; zero-filled groups do not add refresh time.

## Reconstructed source-drive sequences

In this table, `H` means VSH, `L` means VSL, and `0` means 0 V. These are source
driver commands, not measured optical brightness or voltage traces. Frame
counts and level selectors are listed in phase order 1-1, 1-2, 2-1, 2-2.

| Group | Group repeats | Phase frames | State repeats | White destination levels | Black destination levels | Total frames |
| --- | ---: | --- | --- | --- | --- | ---: |
| 1 | 5 | 1, 1, 1, 1 | 1, 1 | L, H, 0, 0 | L, H, 0, 0 | 20 |
| 2 | 1 | 14, 14, 1, 1 | 1, 1 | H, 0, 0, 0 | 0, L, 0, 0 | 30 |
| 3 | 1 | 24, 24, 14, 14 | 1, 1 | L, H, 0, 0 | 0, H, L, 0 | 76 |
| 4 | 1 | 35, 35, 1, 1 | 1, 1 | L, 0, 0, 0 | 0, H, 0, 0 | 72 |

Every VCOM level selector is `00`, so VCOM stays at VCOM_DC throughout these
programmed groups. All tables share the same timing: 198 programmed frames,
plus two finishing frames. Register `0x30 = 0x1B` selects a nominal 160 Hz,
giving 6.25 ms per frame and **1.25 seconds of nominal waveform time**.
Controller initialization, SPI transfers, busy polling, and power sequencing
are additional parts of the complete transaction measured by `refresh_ms`.

White-to-white and black-to-white have identical drive patterns, as do
white-to-black and black-to-black. Consequently, unchanged pixels still get
the destination's complete drive sequence. The common first group applies
alternating source rails, while later groups include longer pulses of both
polarities. Their optical effect depends on the panel and cannot be calculated
from digital selectors alone.

The existing driver writes a zero-filled old-image plane. A future differential
profile would also need correct previous-image data and separate unchanged-pixel
patterns. Changing only that plane while keeping these identical pattern pairs
would not make unchanged pixels quiet. The [status-window experiment](PARTIAL_REFRESH.md)
limits the affected area while retaining this waveform.

## Byte-for-byte reference

`src/display/luts_data.rs` preserves the original 400-byte extraction from
stock firmware address `0x3c0f37a4`, unchanged. The first 376 bytes are the
transmitted register payloads. Their SHA-256 is:

```text
03bbfbf8ebf77e17b633b5c7515d2b112b7bd203a4ebcb415d7182ba01dec34e
```

The extraction includes 24 trailing bytes outside the five LUT payloads;
those were never transmitted and are not part of the generated profile.
Host tests include the original array and compare each generated register
payload with it, including its length and zero tail. The stock array is
independent of the encoder and must not be regenerated merely to accept an
encoding change. It is retained as source, without including a second copy
in production firmware.
