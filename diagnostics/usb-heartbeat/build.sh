#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
export IDF_PATH="$repo_root/.embuild/espressif/esp-idf/v5.5.3"
export IDF_TOOLS_PATH="$repo_root/.embuild/espressif"
export PATH="$IDF_TOOLS_PATH/tools/riscv32-esp-elf/esp-14.2.0_20251107/riscv32-esp-elf/bin:$PATH"
cmake -S "$repo_root/diagnostics/usb-heartbeat" -B "$repo_root/target/usb-heartbeat" \
    -G Ninja -DIDF_TARGET=esp32c3 \
    -DPYTHON="$IDF_TOOLS_PATH/python_env/idf5.5_py3.14_env/bin/python"
cmake --build "$repo_root/target/usb-heartbeat" -j 6
