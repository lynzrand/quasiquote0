#!/usr/bin/env python3
"""On-board browser API checks. --frame/--firmware explicitly upload those files."""
import argparse
import hashlib
import json
import socket
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from quote0_setup import SetupClient


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--port', required=True)
    parser.add_argument('--frame', type=Path)
    parser.add_argument('--firmware', type=Path)
    args = parser.parse_args()
    client = SetupClient(args.port)
    try:
        status = client.status()
        base = status['network']['url']
        assert base, 'No network address; provision Wi-Fi first'
        token = status['access_token']
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

        def request(path, method='GET', data=None, auth=True, headers=None):
            fields = {'X-Device-Token': token} if auth else {}
            fields.update(headers or {})
            req = urllib.request.Request(base + path, data=data, headers=fields, method=method)
            try:
                with opener.open(req, timeout=125) as reply:
                    return reply.status, reply.read()
            except urllib.error.HTTPError as error:
                return error.code, error.read()

        code, page = request('', auth=False)
        assert code == 200 and b'Device settings' in page
        assert request('api/status', auth=False)[0] == 401
        code, body = request('api/status')
        web_status = json.loads(body)
        assert code == 200 and 'access_token' not in web_status and 'setup' not in web_status
        print('PASS: browser page, required API access code, no web credential disclosure')
        code, body = request('api/config')
        config = json.loads(body)['config']
        assert code == 200 and config['wifi']['password'] in ('', '***')
        assert config['pull']['bearer_token'] in ('', '***')
        code, _ = request('api/config', 'POST', json.dumps(config).encode(),
                          headers={'Content-Type': 'application/json'})
        assert code == 200
        time.sleep(1)
        assert client.status()['network']['connected']
        print('PASS: redacted configuration round trip keeps Wi-Fi connected')
        before = client.status()['display']['desired_sha256']
        assert request('api/frame', 'PUT', b'x', headers={'X-Content-SHA256': '0'*64})[0] == 400
        assert request('api/frame', 'PUT', b'\xff'*5624, headers={'X-Content-SHA256': '0'*64})[0] == 400
        assert client.status()['display']['desired_sha256'] == before
        print('PASS: wrong frame length/digest rejected without changing desired content')
        # Interrupt a well-framed body. USB remains responsive; the lease is released.
        parsed = urllib.parse.urlsplit(base)
        with socket.create_connection((parsed.hostname, parsed.port or 80), timeout=5) as wire:
            wire.sendall((f'PUT /api/frame HTTP/1.1\r\nHost: {parsed.hostname}\r\n'
                          f'X-Device-Token: {token}\r\nContent-Length: 5624\r\n'
                          f'X-Content-SHA256: {"0"*64}\r\nConnection: close\r\n\r\n').encode() + b'\xff'*128)
            time.sleep(0.3)
            assert client.status()['operation_active']
        deadline = time.monotonic() + 8
        while client.status()['operation_active'] and time.monotonic() < deadline:
            time.sleep(0.2)
        assert not client.status()['operation_active']
        assert client.status()['display']['desired_sha256'] == before
        assert request('api/status')[0] == 200
        print('PASS: interrupted upload preserves content and releases operation lease')
        if args.frame:
            frame = args.frame.read_bytes()
            assert len(frame) == 5624
            digest = hashlib.sha256(frame).hexdigest()
            code, body = request('api/frame', 'PUT', frame, headers={'X-Content-SHA256': digest})
            assert code == 200, body
            deadline = time.monotonic() + 25
            while time.monotonic() < deadline:
                state = client.status()['display']
                if state['displayed_content_sha256'] == digest:
                    break
                time.sleep(0.5)
            assert state['desired_sha256'] == state['displayed_content_sha256'] == digest
            assert state['error'] is None, state
            print('PASS: verified frame upload persisted and acknowledged by the panel')
        if args.firmware:
            firmware = args.firmware.read_bytes()
            original_slot = client.status()['ota']['running_partition']
            code, body = request('api/firmware', 'PUT', firmware, headers={'X-Content-SHA256': '0'*64})
            assert code == 400, body
            assert client.status()['ota']['running_partition'] == original_slot
            print('PASS: OTA digest mismatch leaves running slot unchanged')
            code, body = request('api/firmware', 'PUT', firmware,
                                 headers={'X-Content-SHA256': hashlib.sha256(firmware).hexdigest()})
            assert code == 200, body
            client.close()
            client = None
            deadline = time.monotonic() + 45
            while time.monotonic() < deadline:
                time.sleep(1)
                try:
                    client = SetupClient(args.port)
                    after = client.status()
                    if after['ota']['running_partition'] != original_slot and not after['ota']['pending']:
                        assert after['access_token'] == token
                        print('PASS: OTA rebooted into the other slot, confirmed image, retained access code')
                        break
                except (OSError, TimeoutError, ValueError):
                    pass
                if client:
                    client.close()
                    client = None
            else:
                raise AssertionError('OTA did not boot and confirm in the other slot')
        final = client.status()
        print(json.dumps({key: final.get(key) for key in ('network', 'clock', 'memory', 'display', 'ota')}, indent=2))
    finally:
        if client:
            client.close()


if __name__ == '__main__':
    main()
