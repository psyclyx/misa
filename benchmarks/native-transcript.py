#!/usr/bin/env python3
"""Keyboard-to-completed-frame wall latency through the native default UI.

No model/network calls. Includes dispatch, projections, viewport, native decoding,
terminal output and PTY scheduling; excludes the terminal emulator's painting.
Six warm-up frames, then ten measured frames per workload by default. Each frame must carry
its unique marker inside a complete synchronized update. The fixture also checks
stream contents and unrelated block identities on every input.
"""
import argparse
import fcntl
import hashlib
import json
import os
import pty
import re
import select
import signal
import statistics
import struct
import subprocess
import tempfile
import termios
import time
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--rest-ms', type=float, default=0,
                    help='untimed idle gap before each input; 25 ms clears the 16 ms presenter interval')
parser.add_argument('--blocks', type=int, choices=(1, 16, 300), help='run only this transcript size')
parser.add_argument('--mode', choices=('redraw', 'stream'), help='run only this workload')
parser.add_argument('--samples', type=int, default=10)
parser.add_argument('--extension-dir', type=Path, help='saved extension tree for source-level A/B comparisons')
parser.add_argument('--perf-output', type=Path,
                    help='attach perf after warm-up; requires --blocks and --mode; timings are diagnostic only')
args = parser.parse_args()
if args.samples < 1:
    parser.error('--samples must be positive')
if args.perf_output and (not args.blocks or not args.mode):
    parser.error('--perf-output requires --blocks and --mode')
if not 0 <= args.rest_ms <= 1000:
    parser.error('--rest-ms must be between 0 and 1000')
root = Path(__file__).resolve().parent.parent
begin, end = b'\x1b[?2026h', b'\x1b[?2026l'
print('blocks,workload,rest_ms,profiled,samples,median_ms,best_ms,max_ms,first_frame_ms,frame_sha256,p95_ms,p99_ms', flush=True)
for count in ((args.blocks,) if args.blocks else (1, 16, 300)):
    for mode in ((args.mode,) if args.mode else ('redraw', 'stream')):
        with tempfile.TemporaryDirectory(prefix='misa-native-transcript-') as directory:
            work = Path(directory)
            config = json.loads((root / 'config/default.json').read_text())
            config['extensions'] = [name for name in config['extensions']
                                    if not name.startswith(('provider.', 'protocol.')) and name != 'auth']
            config['extensions'].insert(0, str(root / 'benchmarks/native-transcript.fnl'))
            settings = config['config']
            settings['models']['default'] = 'bench/model'
            settings['benchmark'] = {'blocks': count}
            for name in ('history', 'themes', 'components', 'preferences'):
                settings.setdefault(name, {})['persist'] = False
            path = work / 'config.json'
            path.write_text(json.dumps(config))
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 32, 100, 0, 0))
            env = dict(os.environ, TERM='xterm-256color', MISA_AUTH_FILE=str(work / 'auth'),
                       MISA_STATE_FILE=str(work / 'state'),
                       MISA_EXTENSION_DIR=str((args.extension_dir or root / 'extensions').resolve()))
            for name in ('TMUX', 'STY'):
                env.pop(name, None)
            launched = time.perf_counter()
            process = subprocess.Popen([str(args.binary.resolve()), '--config', str(path)],
                                       stdin=slave, stdout=slave, stderr=slave, env=env, cwd=root)
            os.close(slave)
            pending = bytearray()
            observed = bytearray()
            profiler = None

            def frame(marker):
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    while end in pending:
                        boundary = pending.index(end) + len(end)
                        data = bytes(pending[:boundary])
                        del pending[:boundary]
                        if marker in data:
                            assert begin in data, 'marker appeared outside a synchronized frame'
                            return data[data.rindex(begin):]
                    if process.poll() is not None:
                        raise AssertionError(f'app exited {process.returncode}: {pending[-2000:]!r}')
                    if select.select([master], [], [], .01)[0]:
                        try:
                            chunk = os.read(master, 65536)
                        except OSError as error:
                            raise AssertionError(f'PTY closed: {observed[-4000:]!r}') from error
                        pending.extend(chunk)
                        observed.extend(chunk)
                raise AssertionError(f'frame {marker!r} timed out: {pending[-2000:]!r}')

            try:
                frame(b'FRAME:0000')
                first = (time.perf_counter() - launched) * 1000
                samples = []
                redraw_oracle = None
                frame_hash = hashlib.sha256()
                # The final untimed frame validates the last measured stream delta.
                for step in range(1, args.samples + 8):
                    if step == 7 and args.perf_output:
                        profiler = subprocess.Popen(['perf', 'record', '-F', '999', '-g',
                                                     '-o', str(args.perf_output.resolve()), '-p', str(process.pid)],
                                                    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
                        # Attach/startup is outside frame timings. A failed attach must not
                        # silently produce a supposedly profiled result.
                        time.sleep(.25)
                        if profiler.poll() is not None:
                            raise RuntimeError(profiler.communicate()[1].decode())
                    if args.rest_ms:
                        time.sleep(args.rest_ms / 1000)
                    started = time.perf_counter()
                    os.write(master, b'\x1bs' if mode == 'stream' else b'\x1br')
                    rendered = frame(f'FRAME:{step:04d}'.encode())
                    elapsed = (time.perf_counter() - started) * 1000
                    frame_hash.update(rendered)
                    if mode == 'redraw':
                        canonical = re.sub(rb'FRAME:[0-9]{4}', b'FRAME:XXXX', rendered)
                        if redraw_oracle is None:
                            redraw_oracle = canonical
                        assert canonical == redraw_oracle, 'unchanged transcript produced different frame bytes'
                    else:
                        plain = re.sub(rb'\x1b\][^\x1b\x07]*(?:\x07|\x1b\\)', b'', rendered)
                        plain = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b'', plain)
                        # Count the synthetic tail across wrapped lines. Other fixture
                        # text has no words consisting solely of x characters.
                        assert sum(map(len, re.findall(rb'\bx+\b', plain))) == step, \
                            'completed frame omitted the current stream text'
                    assert b'\x1b]8;;https://example.test' in rendered, 'layout dropped the visible Markdown link'
                    if 7 <= step <= args.samples + 6:
                        samples.append(elapsed)
                if profiler:
                    profiler.send_signal(signal.SIGINT)
                    _, diagnostics = profiler.communicate(timeout=5)
                    assert profiler.returncode in (0, -signal.SIGINT, 128 + signal.SIGINT), diagnostics.decode()
                    profiler = None
                os.write(master, b'\x1bq')
                process.wait(timeout=3)
                assert process.returncode == 0
                # Inclusive empirical quantiles; with one sample every quantile
                # is that observation. Report n alongside these estimates.
                percentiles = statistics.quantiles(samples, n=100, method='inclusive') if len(samples) > 1 else samples * 99
                print(f'{count},{mode},{args.rest_ms:g},{bool(args.perf_output)},{len(samples)},{statistics.median(samples):.3f},'
                      f'{min(samples):.3f},{max(samples):.3f},{first:.3f},{frame_hash.hexdigest()},'
                      f'{percentiles[94]:.3f},{percentiles[98]:.3f}', flush=True)
            finally:
                if profiler and profiler.poll() is None:
                    profiler.send_signal(signal.SIGINT)
                    profiler.communicate(timeout=5)
                if process.poll() is None:
                    process.kill()
                    process.wait()
                os.close(master)
