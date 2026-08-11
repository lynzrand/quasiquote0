#!/usr/bin/env python3
"""Small Quote/0 USB management client.

The device is the Yamux server and reserves stream 0 for NDJSON JSON-RPC.
Unlike a generic Yamux client, this program deliberately implements that
Quote/0 extension. It accepts device-selected streams for frame and OTA
uploads, advertises a bounded 4 KiB receive window, and sends raw upload bytes
without aggregating the file in memory.

Requires ``pyserial`` (``python3 -m pip install pyserial``).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

try:
    import serial
except ImportError as error:  # pragma: no cover - exercised by users without dependency
    raise SystemExit("pyserial is required: python3 -m pip install pyserial") from error


VERSION = 0
DATA = 0
WINDOW_UPDATE = 1
PING = 2
GO_AWAY = 3
SYN = 0x1
ACK = 0x2
FIN = 0x4
RST = 0x8
HEADER = struct.Struct(">BBHII")
WINDOW = 4 * 1024
MAX_DATA = 4 * 1024


@dataclass
class Stream:
    send_credit: int = 0
    received: bytearray = field(default_factory=bytearray)
    closed: bool = False


class Yamux:
    def __init__(self, port: str, timeout: float) -> None:
        self.serial = serial.Serial(port, baudrate=115200, timeout=timeout, write_timeout=timeout)
        self.timeout = timeout
        self.streams: dict[int, Stream] = {0: Stream(send_credit=2**32 - 1)}

    def close(self) -> None:
        self.serial.close()

    def _read_exact(self, length: int) -> bytes:
        data = bytearray()
        deadline = time.monotonic() + self.timeout
        while len(data) < length:
            part = self.serial.read(length - len(data))
            if part:
                data.extend(part)
                continue
            if time.monotonic() >= deadline:
                raise TimeoutError(f"timed out waiting for {length} Yamux bytes")
        return bytes(data)

    def _send_header(self, kind: int, flags: int, stream_id: int, length: int) -> None:
        self.serial.write(HEADER.pack(VERSION, kind, flags, stream_id, length))

    def _send_data(self, stream_id: int, data: bytes) -> None:
        self._send_header(DATA, 0, stream_id, len(data))
        self.serial.write(data)

    def pump_one(self) -> None:
        version, kind, flags, stream_id, length = HEADER.unpack(self._read_exact(HEADER.size))
        if version != VERSION:
            raise ValueError(f"unsupported Yamux version {version}")
        payload = self._read_exact(length) if kind == DATA and length else b""
        if kind == DATA:
            stream = self.streams.setdefault(stream_id, Stream())
            stream.received.extend(payload)
            if flags & FIN:
                stream.closed = True
            if stream_id != 0 and payload:
                # We consumed the bytes into this bounded host client buffer;
                # immediately return the exact credit to the device.
                self._send_header(WINDOW_UPDATE, 0, stream_id, len(payload))
            return
        if kind == WINDOW_UPDATE:
            stream = self.streams.setdefault(stream_id, Stream())
            if flags & SYN:
                stream.send_credit = length
                # Acknowledge the server-opened stream with our actual receive
                # capacity; upload streams normally receive no device payload.
                self._send_header(WINDOW_UPDATE, ACK, stream_id, WINDOW)
            elif not flags & RST:
                stream.send_credit += length
            if flags & (FIN | RST):
                stream.closed = True
            return
        if kind == PING and stream_id == 0 and not flags & ACK:
            self._send_header(PING, ACK, 0, length)
            return
        if kind == GO_AWAY and stream_id == 0:
            raise ConnectionError("device sent Yamux GO_AWAY")
        raise ValueError(f"invalid Yamux header kind={kind} flags={flags:#x} stream={stream_id}")

    def _wait_for(self, predicate: Any) -> None:
        deadline = time.monotonic() + self.timeout
        while not predicate():
            if time.monotonic() >= deadline:
                raise TimeoutError("timed out waiting for Yamux peer")
            self.pump_one()

    def rpc(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        request_id = int(time.monotonic_ns() & 0x7FFF_FFFF)
        request = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            request["params"] = params
        self._send_data(0, (json.dumps(request, separators=(",", ":")) + "\n").encode())
        stream = self.streams[0]
        while True:
            newline = stream.received.find(b"\n")
            if newline >= 0:
                line = bytes(stream.received[:newline])
                del stream.received[: newline + 1]
                response = json.loads(line)
                if response.get("id") == request_id:
                    if "error" in response:
                        raise RuntimeError(json.dumps(response["error"], separators=(",", ":")))
                    return response["result"]
            self.pump_one()

    def send_file(self, stream_id: int, path: Path) -> None:
        stream = self.streams.setdefault(stream_id, Stream())
        with path.open("rb") as file:
            while True:
                if stream.send_credit == 0:
                    self._wait_for(lambda: stream.send_credit > 0 or stream.closed)
                if stream.closed:
                    raise ConnectionError("device closed upload stream before file completed")
                chunk = file.read(min(stream.send_credit, MAX_DATA))
                if not chunk:
                    break
                self._send_data(stream_id, chunk)
                stream.send_credit -= len(chunk)
        # Declared length is the completion marker on the device. FIN just
        # releases this host-side half after all bytes have been written.
        self._send_header(WINDOW_UPDATE, FIN, stream_id, 0)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", default="/dev/ttyACM0", help="USB Serial/JTAG tty")
    parser.add_argument("--timeout", type=float, default=10.0, help="I/O timeout in seconds")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("hello")
    commands.add_parser("status")
    commands.add_parser("diagnostics")
    operation = commands.add_parser("operation-status")
    operation.add_argument("operation", type=int)
    commands.add_parser("config-get")
    config_set = commands.add_parser("config-set")
    config_set.add_argument("file", type=Path)
    commands.add_parser("wifi-clear")
    commands.add_parser("pull-run")
    frame = commands.add_parser("frame-put")
    frame.add_argument("file", type=Path)
    frame.add_argument("--generation", default="")
    ota = commands.add_parser("ota")
    ota.add_argument("file", type=Path)
    ota.add_argument("--version")
    args = parser.parse_args()

    client = Yamux(args.port, args.timeout)
    try:
        if args.command == "hello":
            result = client.rpc("device.hello")
        elif args.command == "status":
            result = client.rpc("device.status")
        elif args.command == "diagnostics":
            result = client.rpc("diagnostics.get")
        elif args.command == "operation-status":
            result = client.rpc("operation.status", {"operation": args.operation})
        elif args.command == "config-get":
            result = client.rpc("config.get")
        elif args.command == "config-set":
            result = client.rpc("config.set", {"config": json.loads(args.file.read_text())})
        elif args.command == "wifi-clear":
            result = client.rpc("wifi.clear")
        elif args.command == "pull-run":
            result = client.rpc("pull.run")
        elif args.command == "frame-put":
            if args.file.stat().st_size != 5_624:
                raise SystemExit("frame must be exactly 5,624 bytes")
            result = client.rpc(
                "frame.begin",
                {"sha256": sha256(args.file), "generation": args.generation},
            )
            client.send_file(int(result["stream"]), args.file)
        else:
            result = client.rpc(
                "ota.begin",
                {
                    "bytes": args.file.stat().st_size,
                    "sha256": sha256(args.file),
                    "version": args.version,
                },
            )
            client.send_file(int(result["stream"]), args.file)
        print(json.dumps(result, indent=2, sort_keys=True))
    finally:
        client.close()


if __name__ == "__main__":
    main()
