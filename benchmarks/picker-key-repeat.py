#!/usr/bin/env python3
"""Held-arrow release to completed visible final-row latency through the default UI.

Synthetic presses arrive at 30/60 Hz through a PTY; no key is dropped/coalesced by
the fixture. The actual highlighted model row in synchronized terminal frames is
the oracle. Intermediate rendered rows may be skipped by the presenter. Every
run must reach exactly the row implied by all supplied inputs, in order.
Timing includes terminal decoding, native transactions, policy, presentation and
PTY delivery, but excludes the terminal emulator's painting and physical key-up.
"""
import argparse
import csv
import fcntl
import hashlib
import json
import os
import pty
import re
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'tests'))
from fixture_environment import fixture_environment

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--extension-dir', type=Path, default=ROOT / 'extensions')
parser.add_argument('--models', type=int, default=1000)
parser.add_argument('--rates', type=int, nargs='+', default=[30, 60])
parser.add_argument('--seconds', type=float, default=1.0)
parser.add_argument('--samples', type=int, default=10)
parser.add_argument('--warmups', type=int, default=6)
parser.add_argument('--timeout', type=float, default=30.0)
parser.add_argument('--trace', type=Path, help='write raw send and observed-focus timestamps as JSON')
args = parser.parse_args()
if (args.models < 2 or args.samples < 1 or args.warmups < 0 or args.seconds <= 0 or
        any(rate <= 0 or round(rate * args.seconds) >= args.models for rate in args.rates)):
    parser.error('positive rates/duration/sample count required; each hold must be shorter than catalogue')
BEGIN, END = b'\x1b[?2026h', b'\x1b[?2026l'
CSI = re.compile(rb'\x1b\[[0-?]*[ -/]*[@-~]')
OSC = re.compile(rb'\x1b\].*?(?:\x07|\x1b\\)', re.S)
FOCUS = re.compile(rb'> [^\r\n]*?bench/model-(\d{5})')
writer = csv.writer(sys.stdout, lineterminator='\n')
writer.writerow(['models', 'rate_hz', 'sample', 'direction', 'keys', 'final_row',
                 'release_to_final_ms', 'rows_pending_at_release', 'post_release_frames',
                 'max_send_lateness_ms', 'frame_sha256'])
sys.stdout.flush()
traces = []
with tempfile.TemporaryDirectory(prefix='misa-picker-repeat-') as directory:
    work = Path(directory)
    config = json.loads((ROOT / 'config/default.json').read_text())
    config['extensions'] = [str(ROOT / 'benchmarks/picker-key-repeat.fnl')] + [
        name for name in config['extensions']
        if not name.startswith(('provider.', 'protocol.')) and name != 'auth']
    settings = config['config']
    settings['models']['default'] = 'bench/model-00001'
    settings['benchmark'] = {'models': args.models}
    for name in ('history', 'themes', 'components', 'preferences'):
        settings.setdefault(name, {})['persist'] = False
    path = work / 'config.json'
    path.write_text(json.dumps(config))
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 32, 100, 0, 0))
    original = termios.tcgetattr(slave)
    env = fixture_environment(work, TERM='xterm-256color',
                              MISA_EXTENSION_DIR=str(args.extension_dir.resolve()))
    process = subprocess.Popen([str(args.binary.resolve()), '--config', str(path)],
                               stdin=slave, stdout=slave, stderr=slave, env=env, cwd=ROOT)
    pending = bytearray()
    tail = bytearray()
    focused = None
    focus_frame = None
    observed = []

    def receive(timeout):
        global focused, focus_frame
        if process.poll() is not None:
            raise AssertionError(f'app exited {process.returncode}: {tail[-3000:]!r}')
        if not select.select([master], [], [], max(0, timeout))[0]:
            return
        data = os.read(master, 65536)
        timestamp = time.perf_counter()
        pending.extend(data)
        tail.extend(data)
        del tail[:-6000]
        while END in pending:
            boundary = pending.index(END) + len(END)
            data = bytes(pending[:boundary])
            del pending[:boundary]
            if BEGIN not in data:
                continue
            data = data[data.rindex(BEGIN):]
            plain = CSI.sub(b'', OSC.sub(b'', data))
            matches = FOCUS.findall(plain)
            if not matches:  # clock-animation patches have no full picker row
                continue
            assert len(matches) == 1, f'ambiguous focused model: {plain!r}'
            row = int(matches[0])
            if row != focused:
                focused = row
                focus_frame = data
                observed.append((timestamp, row))

    def wait_row(target):
        deadline = time.perf_counter() + args.timeout
        while focused != target and time.perf_counter() < deadline:
            receive(min(.05, deadline - time.perf_counter()))
        assert focused == target, f'expected row {target}, observed {focused}: {tail[-3000:]!r}'

    def hold(rate, keys, direction):
        initial = focused
        target = initial + direction * keys
        assert 1 <= target <= args.models
        first_observation = len(observed)
        sends = []
        epoch = time.perf_counter()
        for index in range(keys):
            due = epoch + index / rate
            while time.perf_counter() < due:
                receive(due - time.perf_counter())
            now = time.perf_counter()
            key = b'\x1b[B' if direction == 1 else b'\x1b[A'
            assert os.write(master, key) == len(key), 'short input write'
            sends.append((now, (now - due) * 1000))
        release = sends[-1][0]
        at_release = focused
        wait_row(target)
        final_time = observed[-1][0]
        # One input interval plus the 16 ms presenter cadence after the final
        # visible row must not reveal a queued extra movement.
        quiet_until = time.perf_counter() + 1 / rate + .016
        while time.perf_counter() < quiet_until:
            receive(quiet_until - time.perf_counter())
        rows = observed[first_observation:]
        last = initial
        for _, row in rows:
            assert 0 < direction * (row - last) <= direction * (target - last), (
                'focus regressed or overshot the supplied keys', initial, target, rows)
            last = row
        assert last == target == focused, 'supplied navigation inputs were lost'
        traces.append({'models': args.models, 'rate': rate, 'initial': initial, 'target': target,
                       'release': release, 'sends': sends, 'frames': rows})
        return ((final_time - release) * 1000, direction * (target - at_release),
                sum(stamp >= release for stamp, _ in rows), max(late for _, late in sends),
                hashlib.sha256(focus_frame).hexdigest())

    try:
        wait_row(1)
        for rate in args.rates:
            for warmup in range(args.warmups):
                hold(rate, 2, 1 if warmup % 2 == 0 else -1)
            # Warm-ups may leave the row above one if the requested count is odd.
            if focused != 1:
                hold(rate, focused - 1, -1)
            for sample in range(args.samples):
                direction = 1 if sample % 2 == 0 else -1
                keys = round(rate * args.seconds)
                result = hold(rate, keys, direction)
                writer.writerow([args.models, rate, sample + 1, direction, keys, focused,
                                 *(f'{value:.3f}' if isinstance(value, float) else value
                                   for value in result)])
                sys.stdout.flush()
            if focused != 1:
                hold(rate, focused - 1, -1)
        assert os.write(master, b'\x1bq') == 2
        process.wait(timeout=args.timeout)
        assert process.returncode == 0
        assert termios.tcgetattr(slave) == original, 'shutdown did not restore terminal modes'
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
        os.close(slave)
        if args.trace:
            args.trace.write_text(json.dumps(traces, indent=2))
