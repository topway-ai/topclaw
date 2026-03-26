#!/usr/bin/env python3
"""TopClaw UNO Q bridge server."""

import os
import socketserver
import time

import serial


BRIDGE_HOST = os.environ.get("TOPCLAW_BRIDGE_HOST", "127.0.0.1")
BRIDGE_PORT = int(os.environ.get("TOPCLAW_BRIDGE_PORT", "9999"))
SERIAL_DEVICE = os.environ.get("TOPCLAW_BRIDGE_SERIAL", "/dev/ttyACM0")
SERIAL_BAUD = int(os.environ.get("TOPCLAW_BRIDGE_BAUD", "115200"))
SERIAL_TIMEOUT = float(os.environ.get("TOPCLAW_BRIDGE_TIMEOUT", "2.0"))

_SERIAL = None


def open_serial():
    global _SERIAL
    if _SERIAL is None or not _SERIAL.is_open:
        _SERIAL = serial.Serial(SERIAL_DEVICE, SERIAL_BAUD, timeout=SERIAL_TIMEOUT)
        time.sleep(2.0)
        _SERIAL.reset_input_buffer()
    return _SERIAL


def bridge_command(line):
    port = open_serial()
    port.reset_input_buffer()
    port.write((line.strip() + "\n").encode("utf-8"))
    port.flush()
    response = port.readline().decode("utf-8", errors="replace").strip()
    return response or "error: empty response"


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        line = self.rfile.readline().decode("utf-8", errors="replace").strip()
        if not line:
            return

        try:
            response = bridge_command(line)
        except Exception as exc:  # pragma: no cover - runtime safety path
            response = "error: {}".format(exc)

        self.wfile.write((response + "\n").encode("utf-8"))


class ThreadedTCPServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True


def main():
    with ThreadedTCPServer((BRIDGE_HOST, BRIDGE_PORT), Handler) as server:
        print(
            "TopClaw bridge listening on {}:{} via {}".format(
                BRIDGE_HOST, BRIDGE_PORT, SERIAL_DEVICE
            )
        )
        server.serve_forever()


if __name__ == "__main__":
    main()
