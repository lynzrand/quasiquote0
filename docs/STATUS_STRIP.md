# Status-strip design

The overlay stays within the rightmost 16 columns of the 296 x 152 landscape
frame, preserving the producer's 280 x 152 content area and the 304-byte
native partial window. The compositor clears the strip before each snapshot.

The artwork is authored upright in landscape coordinates. The EPD boundary
projects the complete frame into controller order; individual icons and clock
glyphs do not receive another rotation.

| Region | Landscape rows | Design |
| --- | --- | --- |
| Battery | 4–19 | Upright outline with five fill levels; critical exclamation and charging bolt variants |
| Wi-Fi | 28–43 | Radio arcs, disconnected slash, static connection dots, or failure mark |
| USB power | 52–67 | Power plug, distinct from the battery's charging variant |
| Bluetooth | 76–91 | Upright Bluetooth rune |
| Divider | 103 | Short horizontal rule above the clock |
| Hours | 112–124 | Two upright digits using `FONT_7X13_BOLD` |
| Colon | 126–127, 130–131 | Two static dots; no seconds animation |
| Minutes | 134–146 | Two upright digits in the same font |

Unknown time uses two rows of dashes. Missing optional indicators leave their
slots empty, keeping the clock and remaining icons in stable positions.
The source masks remain 16 x 16 one-bit constants in `src/status_icons.rs`;
no runtime image decoder or extra framebuffer is needed.

Preview the production renderer on the host:

```sh
cargo +stable run --locked --manifest-path tests/host/Cargo.toml \
  --target x86_64-unknown-linux-gnu --target-dir target/host-tests \
  --example status_preview -- target/status-preview
```

The example writes PBM previews and the actual device frame bytes for normal,
low-battery, offline, connecting, and failed snapshots. PBM uses one for black;
the exported `.bin` files retain the device's one-for-white convention.
