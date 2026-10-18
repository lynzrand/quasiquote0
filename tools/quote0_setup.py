#!/usr/bin/env python3
"""USB bootstrap client for the browser firmware (requires pyserial)."""
import argparse
import getpass
import json
import time
import webbrowser


class SetupClient:
    def __init__(self, port, timeout=5):
        import serial
        self.port = serial.Serial()
        self.port.port = port
        self.port.baudrate = 115200
        self.port.timeout = timeout
        self.port.write_timeout = timeout
        self.port.dtr = False
        self.port.rts = False
        self.port.open()
        self.port.reset_input_buffer()
        self.port.write(b"\n")

    def close(self):
        self.port.close()

    def command(self, cmd, **fields):
        line = json.dumps({"cmd": cmd, **fields}, ensure_ascii=False).encode()
        if len(line) > 1024:
            raise ValueError("USB command too long")
        self.port.write(line + b"\n")
        deadline = time.monotonic() + 8
        while True:
            reply = self.port.read_until(b"\n", 8192)
            if not reply.endswith(b"\n") or time.monotonic() > deadline:
                raise TimeoutError("USB reply timed out")
            if reply.lstrip().startswith(b"{"):
                break
        result = json.loads(reply)
        if not result["ok"]:
            raise RuntimeError(result["error"])
        return result["result"]

    def status(self):
        return self.command("status")["status"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", required=True)
    parser.add_argument("--show-access", action="store_true")
    parser.add_argument("--open-browser", action="store_true")
    parser.add_argument("--wait", type=float, default=0, help="Wait for network, in seconds")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("status")
    wifi = sub.add_parser("wifi")
    wifi.add_argument("ssid")
    wifi.add_argument("--open-network", action="store_true")
    sub.add_parser("clear-wifi")
    args = parser.parse_args()
    client = SetupClient(args.port)
    try:
        if args.cmd == "wifi":
            password = "" if args.open_network else getpass.getpass("Wi-Fi password: ")
            client.command("wifi", ssid=args.ssid, password=password)
        elif args.cmd == "clear-wifi":
            client.command("clear_wifi")
        deadline = time.monotonic() + args.wait
        while True:
            status = client.status()
            expected = "station" if args.cmd == "wifi" else "setup" if args.cmd == "clear-wifi" else None
            ready = status["network"]["connected"] and (expected is None or status["network"]["mode"] == expected)
            if ready or time.monotonic() >= deadline:
                break
            time.sleep(0.3)
        if args.open_browser and status["network"]["url"]:
            webbrowser.open(status["network"]["url"] + "#token=" + status["access_token"])
        if not args.show_access:
            status.pop("access_token", None)
            status.pop("setup", None)
        print(json.dumps(status, indent=2))
    finally:
        client.close()


if __name__ == "__main__":
    main()
