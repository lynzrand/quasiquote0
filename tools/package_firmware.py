#!/usr/bin/env python3
"""Create a size-checked OTA image using the active ESP-IDF Python environment."""
import argparse
import csv
import hashlib
import subprocess
import sys
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--name', default='quasiquote0')
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    output = root / 'target/firmware' / args.name
    output.mkdir(parents=True, exist_ok=True)
    image = output / f'{args.name}.bin'
    subprocess.run([
        sys.executable, '-m', 'esptool', '--chip', 'esp32c3', 'elf2image',
        '--flash_mode', 'dio', '--flash_freq', '40m', '--flash_size', '4MB',
        '--elf-sha256-offset', '0xb0', '-o', str(image),
        str(root / 'target/riscv32imc-esp-espidf/debug/quasiquote0'),
    ], check=True)
    with (root / 'partitions.csv').open() as table:
        slots = [
            int(row[4].strip(), 0)
            for row in csv.reader(line for line in table if not line.lstrip().startswith('#'))
            if len(row) >= 5 and row[1].strip() == 'app' and row[2].strip() in ('ota_0', 'ota_1')
        ]
    limit = min(slots)
    size = image.stat().st_size
    if not 0 < size <= limit:
        raise SystemExit(f'Application is {size} bytes; OTA slot limit is {limit} bytes')
    digest = hashlib.sha256(image.read_bytes()).hexdigest()
    (output / 'SHA256SUMS').write_text(f'{digest}  {image.name}\n')
    print(f'{image.name}: {size:,} bytes; {limit - size:,} bytes free in OTA slot')


if __name__ == '__main__':
    main()
