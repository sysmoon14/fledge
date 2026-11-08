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
temperature, so before each step `turn` looks at the display and refuses the
home screen (its tick ring fills the rim, unlike menus and pages), and after
it checks the display changed: a step that did not register stops the chain,
so a later press cannot land on the wrong choice.

A gen 3 unit (diamond3): the ring is the ADC's relative dial (REL_DIAL on the
input device named "2280000.adc"), the press gpio-keys' KEY_ENTER, and the
display /dev/fb0 (480x640 visible of 32-bit BGRX). Events are written to the
evdev nodes, which deliver them to every reader, nlclient included.

Turning the ring on the home screen changes the target temperature: look
first. The unit must be awake (utils/push-over-ssh.sh --wake, or MQTT
set/stay_awake) and its SSH must take a key (no password prompt per call).
"""
import argparse
import math
import os
import struct
import subprocess
import sys
import tempfile
import time
import zlib

EV_SYN, EV_KEY, EV_REL = 0, 1, 2
KEY_ENTER, REL_DIAL = 28, 7
# REL_DIAL events are raw ADC counts. The main menu moves an item for one
# large event (not always the first); lists only follow streams of small
# ones, as a hand makes. A step alternates the two until the display changes.
LARGE, BURST, AMOUNT, TRIES = 1000, 3, 60, 8
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
# The home screen's dial: its ticks (or the heating/cooling colour) fill the
# rim; menus and pages leave it nearly black (about 5%).
HOME_RIM = 0.2


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


def frame(unit, size=480):
    """The visible display, raw: (stride, bytes of its top size rows)."""
    info = unit.run("cat /sys/class/graphics/fb0/stride /sys/class/graphics/fb0/pan").decode().split()
    stride, y = int(info[0]), int(info[1].split(",")[1])
    return stride, unit.run(f"dd if=/dev/fb0 bs={stride} skip={y} count={size} 2>/dev/null")


def home_screen(stride, raw, size=480):
    """Whether the display shows the thermostat's home dial."""
    lit = total = 0
    for angle in range(0, 360, 3):
        for radius in range(205, 232, 4):
            x = int(size / 2 + radius * math.cos(math.radians(angle)))
            y = int(size / 2 + radius * math.sin(math.radians(angle)))
            at = y * stride + x * 4
            total += 1
            lit += max(raw[at:at + 3]) > 60
    return lit / total > HOME_RIM


def turn(unit, steps):
    if not lit(unit):
        sys.exit("the display is dark: a turn could change the target temperature; wake it and look first")
    device = unit.device(RING_NAME)
    sign = 1 if steps > 0 else -1
    large = [(EV_REL, REL_DIAL, sign * LARGE), (EV_SYN, 0, 0)]
    small = [e for _ in range(BURST) for e in ((EV_REL, REL_DIAL, sign * AMOUNT), (EV_SYN, 0, 0))]
    for _ in range(abs(steps)):
        stride, before = frame(unit)
        if home_screen(stride, before):
            sys.exit("turn: the home screen is showing, where a turn changes the target temperature; stopped")
        for attempt in range(TRIES):
            unit.send(device, large if attempt % 2 == 0 else small)
            time.sleep(0.5)
            if frame(unit)[1] != before:
                break
        else:
            sys.exit("turn: the display did not change, so the step did not register; stopped")


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
    while words:
        command = words.pop(0)
        if command == "wake":
            wake(unit)
        elif command == "shot" and words:
            wake(unit)
            shot(unit, words.pop(0))
        elif command == "press":
            press(unit)
            time.sleep(args.pause)
        elif command == "turn" and words:
            turn(unit, int(words.pop(0)))
            time.sleep(args.pause)
        else:
            sys.exit(f"unexpected {command!r}: wake | shot OUT.png | press | turn N")


if __name__ == "__main__":
    main()
