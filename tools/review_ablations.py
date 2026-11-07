#!/usr/bin/env python3
"""Remove one safeguard at a time in a temporary checkout; require test failures.

These are test-sensitivity experiments, not alternative production settings.
Nothing is flashed and the working tree's production sources are never edited.
Logs include the exact patch and command so a failed experiment is reviewable.
"""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile


# name, source, exact old text, replacement. Each substitution must be unique.
ABLATIONS = [
    ("stock-polarity", "src/display/luts.rs",
     "[Low, Reference, Reference, Reference],", "[High, Reference, Reference, Reference],"),
    ("short-pulse-length", "src/display/luts.rs", "frames: [48, 1, 1, 1]", "frames: [24, 1, 1, 1]"),
    ("unchanged-pixel-drive", "src/display/luts.rs",
     "&lut_unchanged()[..56],\n            &FAST_WHITE", "&FAST_WHITE[..56],\n            &FAST_WHITE"),
    ("cold-initialization", "src/display/uc8251d.rs", "if !self.sleeping {", "if self.sleeping {"),
    ("timeout-power-off", "src/display/uc8251d.rs",
     "if result.is_err() {\n            self.power_off();\n        }\n        #[cfg", "if false {\n            self.power_off();\n        }\n        #[cfg"),
    ("real-old-image", "src/display/uc8251d.rs", "previous[..bytes].chunks(256)", "frame.as_bytes()[..bytes].chunks(256)"),
    ("window-extent", "src/display/status_window.rs", "ROWS: usize = 16", "ROWS: usize = 8"),
    ("content-cleanup-budget", "src/refresh_policy.rs", "self.content_updates < 4", "self.content_updates < 255"),
    ("idle-budget-exemption", "src/refresh_policy.rs",
     "if self.displayed.is_some_and(|previous| {\n            previous.content != key.content || previous.status_bar != key.status_bar\n        })", "if true"),
    ("unchanged-frame-skip", "src/appliance/render.rs", "self.last_composed == Some(composed)", "false"),
    ("rtc-validation", "src/rtc_display.rs",
     "(self.magic_version == MAGIC && self.checksum == Self::checksum(self.composed))", "true"),
    ("strip-clearing", "src/status_bar.rs", "for x in CONTENT_WIDTH..WIDTH {\n                frame.set_pixel", "for x in WIDTH..WIDTH {\n                frame.set_pixel"),
    ("minute-clock", "src/clock_face.rs", "minute: ((local / 60) % 60) as u8", "minute: 0"),
    ("adc-failure-filter", "src/battery/samples.rs", "samples[..count].sort_unstable();", "samples.sort_unstable();"),
    ("retained-validator-check", "src/device.rs", "self.backup_frame().is_ok_and(|frame| frame.is_some())", "true"),
    ("validator-context-reset", "src/device.rs",
     "config.pull.url != self.config.pull.url\n            || config.pull.bearer_token != self.config.pull.bearer_token", "false"),
    ("retained-digest-check", "src/store.rs", "if actual != *digest {", "if false {"),
    ("upload-digest-check", "src/store.rs", "if self.expected.is_some_and(|expected| actual != expected) {", "if false {"),
    ("durable-before-ram", "src/device.rs", "self.store.save_state(&next)?;\n        self.state = next;", "self.state = next;\n        self.store.save_state(&self.state)?;"),
    ("restore-replay", "src/store.rs", "store.recover_restore()?;", "// replay omitted"),
    ("flash-free-heartbeat", "src/device.rs", "if persist {", "if true {"),
    ("backup-digest-check", "src/backup.rs", "if actual != expected {", "if false {"),
    ("usb-line-bound", "src/bootstrap_line.rs", "self.bytes.len() == MAX_LINE", "false"),
    ("usb-awake-guard", "src/power_policy.rs", "self.vbus_present || self.usb_host_connected || self.uptime_ms < recovery_ms", "self.uptime_ms < recovery_ms"),
    ("pull-backoff", "src/pull_schedule.rs", "self.next_due_ms = now_ms.saturating_add(delay);", "self.next_due_ms = now_ms;"),
    ("bthome-object-id", "src/bthome.rs", "0x19,\n            self.wifi_connected", "0x18,\n            self.wifi_connected"),
    ("panel-projection", "src/frame.rs", "(source_y, WIDTH - 1 - source_x)", "(source_y, source_x)"),
    ("ota-digest-check", "src/ota.rs", "if actual != expected_digest {", "if false {"),
    ("ota-abort", "src/ota.rs", "esp_idf_sys::esp_ota_abort(handle)", "esp_idf_sys::esp_ota_end(handle)"),
    ("ota-sdk-validation", "src/ota.rs", "Ok(()) => esp_idf_sys::esp!(unsafe { esp_idf_sys::esp_ota_end(handle) })", "Ok(()) => esp_idf_sys::esp!(unsafe { esp_idf_sys::esp_ota_abort(handle) })"),
    ("ota-early-boot-selection", "src/ota.rs", "let result = receive_inner(stream, expected_bytes, expected_digest, handle);", "let result = receive_inner(stream, expected_bytes, expected_digest, handle);\n    select_staged_partition()?;"),
    ("alpha-compositing", "src/web/frame-converter.js", "gray * alpha + 255 * (1 - alpha)", "gray"),
    ("dithering", "src/web/frame-converter.js", "if (!dither) continue;", "if (true) continue;"),
    ("status-width-reservation", "src/web/frame-converter.js", "STATUS_WIDTH = 16", "STATUS_WIDTH = 0"),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path('target/review-ablations'))
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    results = []
    with tempfile.TemporaryDirectory(prefix='quote0-ablation-') as temporary:
        scratch = Path(temporary)
        for directory in ['src', 'tests/host', 'tests']:
            if directory == 'tests':
                for file in (root / directory).glob('*.mjs'):
                    shutil.copy2(file, scratch / 'tests' / file.name)
            else:
                shutil.copytree(root / directory, scratch / directory)
        (scratch / 'package.json').write_text('{"type":"module"}\n')
        rust = ['cargo', '+stable', 'test', '--offline', '--locked', '--manifest-path',
                str(scratch / 'tests/host/Cargo.toml'), '--target', 'x86_64-unknown-linux-gnu',
                '--target-dir', str(output / 'build')]
        web = ['node', str(scratch / 'tests/frame_converter.test.mjs')]
        for name, command in [('rust-baseline', rust), ('web-baseline', web)]:
            run = subprocess.run(command, cwd=scratch, text=True, capture_output=True)
            (output / f'{name}.log').write_text(run.stdout + run.stderr)
            if run.returncode:
                raise RuntimeError(f'{name} failed; see {output}')
        for name, source, old, new in ABLATIONS:
            path = scratch / source
            original = path.read_text()
            # The restore hook occurs in both platform-specific constructors;
            # the host constructor is the last occurrence.
            if name == 'restore-replay':
                assert original.count(old) == 2
                index = original.rindex(old)
                changed = original[:index] + original[index:].replace(old, new, 1)
            else:
                assert original.count(old) == 1, (name, original.count(old))
                changed = original.replace(old, new, 1)
            command = web if source.endswith('.js') else rust
            try:
                path.write_text(changed)
                run = subprocess.run(command, cwd=scratch, text=True, capture_output=True)
            finally:
                path.write_text(original)
            log = run.stdout + run.stderr
            (output / f'{name}.log').write_text(log)
            tested = bool(re.search(r'running [1-9]\d* tests|tests [1-9]\d*', log))
            caught = run.returncode != 0 and tested and ('test result: FAILED' in log or 'fail 0' not in log and source.endswith('.js'))
            result = dict(name=name, source=source, removed=old, replacement=new,
                          command=command, exit=run.returncode,
                          outcome='caught' if caught else 'survived' if run.returncode == 0 else 'invalid')
            results.append(result)
            print(f'{name}: {result["outcome"]}', flush=True)
            (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
    counts = {outcome: sum(row['outcome'] == outcome for row in results)
              for outcome in ['caught', 'survived', 'invalid']}
    print(json.dumps(counts))
    raise SystemExit(bool(counts['survived'] or counts['invalid']))


if __name__ == '__main__':
    main()
