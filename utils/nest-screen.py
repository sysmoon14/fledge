#!/usr/bin/env python3
"""See and drive a unit's screen over SSH, for testing the UNFR0ZEN screen
without anyone at the thermostat:

    utils/nest-screen.py [--host root@HOST] shot OUT.png   the display, as a PNG (waking it)
    utils/nest-screen.py [--host root@HOST] wake            lights the display if it is dark
    utils/nest-screen.py [--host root@HOST] press           a press of the ring
    utils/nest-screen.py [--host root@HOST] turn N          N ring steps (negative: back)

Commands chain: `wake shot a.png press shot b.png` runs in order. The display
goes dark after a few idle seconds and its framebuffer with it; a press on a
dark display only lights it. On the home screen a turn changes the target
temperature, so `turn` runs only within TURN_WINDOW seconds of a press or
turn earlier in the same chain (a page this chain opened is still open), on
a lit display.

A gen 3 unit (diamond3): the ring is the ADC's relative dial (REL_DIAL on the
input device named "2280000.adc"), the press gpio-keys' KEY_ENTER, and the
display /dev/fb0 (480x640 visible of 32-bit BGRX). Events are written to the
evdev nodes, which deliver them to every reader, nlclient included.

Turning the ring on the home screen changes the target temperature: look
first. The unit must be awake (utils/push-over-ssh.sh --wake, or MQTT
set/stay_awake) and its SSH must take a key (no password prompt per call).
"""
import argparse
import os
import struct
import subprocess
import sys
import tempfile
import time
import zlib

EV_SYN, EV_KEY, EV_REL = 0, 1, 2
KEY_ENTER, REL_DIAL = 28, 7
# nlclient moves one item per REL_DIAL event of at least ~250 (raw ADC
# counts); smaller events do not add up.
STEP = 300
RING_NAME, BUTTON_NAME = "2280000.adc", "gpio-keys"


class Unit:
    def __init__(self, host):
        self.host = host
        control = tempfile.mkdtemp(prefix="nest-screen-", dir="/tmp")
        self.opts = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "ControlMaster=auto",
                     "-o", f"ControlPath={control}/c", "-o", "ControlPersist=60"]

    def run(self, command, data=None):
        out = subprocess.run(["ssh", *self.opts, self.host, command], input=data, capture_output=True)
        if out.returncode != 0:
            sys.exit(f"{self.host}: {command}: {out.stderr.decode(errors='replace').strip()}")
        return out.stdout

    def device(self, name):
        """/dev/input/eventN of the input device called `name`."""
        text = self.run("cat /proc/bus/input/devices").decode()
        for block in text.split("\n\n"):
            if f'Name="{name}"' in block:
                for word in block.replace("=", " ").split():
                    if word.startswith("event"):
                        return f"/dev/input/{word}"
        sys.exit(f"{self.host}: no input device {name}")

    def send(self, device, events):
        """Writes input_events (32-bit ARM: timeval, type, code, value)."""
        now = time.time()
        data = b"".join(struct.pack("<iiHHi", int(now), int(now % 1 * 1e6), t, c, v) for t, c, v in events)
        self.run(f"cat > {device}", data)


BACKLIGHT = "/sys/class/backlight/*/brightness"
TURN_WINDOW = 5.0


def lit(unit):
    return any(int(v) > 0 for v in unit.run(f"cat {BACKLIGHT}").split())


def wake(unit):
    if not lit(unit):
        press(unit)
        for _ in range(20):
            time.sleep(0.25)
            if lit(unit):
                break
        time.sleep(0.6)  # the first frame


def press(unit):
    device = unit.device(BUTTON_NAME)
    unit.send(device, [(EV_KEY, KEY_ENTER, 1), (EV_SYN, 0, 0)])
    time.sleep(0.15)
    unit.send(device, [(EV_KEY, KEY_ENTER, 0), (EV_SYN, 0, 0)])


def turn(unit, steps):
    if not lit(unit):
        sys.exit("the display is dark: a turn could change the target temperature; wake it and look first")
    device = unit.device(RING_NAME)
    step = STEP if steps > 0 else -STEP
    for _ in range(abs(steps)):
        unit.send(device, [(EV_REL, REL_DIAL, step), (EV_SYN, 0, 0)])
        time.sleep(0.25)


def shot(unit, out, size=480):
    """The round display: the top size x size of the framebuffer, which the
    panel shows turned a quarter clockwise, turned back upright."""
    info = unit.run("cat /sys/class/graphics/fb0/stride /sys/class/graphics/fb0/pan").decode().split()
    stride = int(info[0])
    x, y = (int(v) for v in info[1].split(","))
    raw = unit.run(f"dd if=/dev/fb0 bs={stride} skip={y} count={size} 2>/dev/null")
    width = height = size
    rows = []
    for row in range(size):
        # Upright row `row` is framebuffer column size-1-row, read top down.
        column = (x + size - 1 - row) * 4
        pixels = bytearray(size * 3)
        for i in range(size):
            at = i * stride + column
            pixels[i * 3:i * 3 + 3] = bytes((raw[at + 2], raw[at + 1], raw[at]))
        rows.append(b"\0" + bytes(pixels))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(b"".join(rows), 6)) + chunk(b"IEND", b""))
    with open(out, "wb") as f:
        f.write(png)
    print(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--host", default=os.environ.get("UNFR0ZEN_HOST", "root@10.22.4.165"))
    parser.add_argument("--pause", type=float, default=0.8, help="seconds after each press or turn")
    parser.add_argument("commands", nargs="+", help="wake | shot OUT.png | press | turn N")
    args = parser.parse_args()
    unit = Unit(args.host)
    words = list(args.commands)
    acted = None  # when this chain last pressed or turned
    while words:
        command = words.pop(0)
        if command == "wake":
            wake(unit)
        elif command == "shot" and words:
            wake(unit)
            shot(unit, words.pop(0))
        elif command == "press":
            dark = not lit(unit)
            press(unit)
            acted = None if dark else time.monotonic()
            time.sleep(args.pause)
        elif command == "turn" and words:
            steps = int(words.pop(0))
            if acted is None or time.monotonic() - acted > TURN_WINDOW:
                sys.exit("turn: no page this chain opened is known to be showing (the home screen "
                         "would change the target temperature); press into a menu first, in the same chain")
            turn(unit, steps)
            acted = time.monotonic()
            time.sleep(args.pause)
        else:
            sys.exit(f"unexpected {command!r}: wake | shot OUT.png | press | turn N")


if __name__ == "__main__":
    main()
