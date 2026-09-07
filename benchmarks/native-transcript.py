#!/usr/bin/env python3
"""Keyboard-to-completed-frame wall latency through the native default UI.

No model/network calls. Includes dispatch, projections, viewport, native decoding,
terminal output and PTY scheduling; excludes the terminal emulator's painting.
Six warm-up frames, then ten measured frames per workload. Each frame must carry
its unique marker inside a complete synchronized update. The fixture also checks
stream contents and unrelated block identities on every input.
"""
import argparse
import fcntl
import json
import os
import pty
import re
import select
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
args = parser.parse_args()
if not 0 <= args.rest_ms <= 1000:
    parser.error('--rest-ms must be between 0 and 1000')
root = Path(__file__).resolve().parent.parent
begin, end = b'\x1b[?2026h', b'\x1b[?2026l'
print('blocks,workload,rest_ms,samples,median_ms,best_ms,max_ms,first_frame_ms', flush=True)
for count in (1, 16, 300):
    for mode in ('redraw', 'stream'):
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
                       MISA_STATE_FILE=str(work / 'state'), MISA_EXTENSION_DIR=str(root / 'extensions'))
            for name in ('TMUX', 'STY'):
                env.pop(name, None)
            launched = time.perf_counter()
            process = subprocess.Popen([str(args.binary.resolve()), '--config', str(path)],
                                       stdin=slave, stdout=slave, stderr=slave, env=env, cwd=root)
            os.close(slave)
            pending = bytearray()
            observed = bytearray()

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
                # The final untimed frame validates the last measured stream delta.
                for step in range(1, 18):
                    if args.rest_ms:
                        time.sleep(args.rest_ms / 1000)
                    started = time.perf_counter()
                    os.write(master, b'\x1bs' if mode == 'stream' else b'\x1br')
                    rendered = frame(f'FRAME:{step:04d}'.encode())
                    elapsed = (time.perf_counter() - started) * 1000
                    if mode == 'redraw':
                        canonical = re.sub(rb'FRAME:[0-9]{4}', b'FRAME:XXXX', rendered)
                        if redraw_oracle is None:
                            redraw_oracle = canonical
                        assert canonical == redraw_oracle, 'unchanged transcript produced different frame bytes'
                    else:
                        assert b'x' * step in rendered, 'completed frame omitted the current stream text'
                    assert b'\x1b]8;;https://example.test' in rendered, 'layout dropped the visible Markdown link'
                    if 7 <= step <= 16:
                        samples.append(elapsed)
                os.write(master, b'\x1bq')
                process.wait(timeout=3)
                assert process.returncode == 0
                print(f'{count},{mode},{args.rest_ms:g},{len(samples)},{statistics.median(samples):.3f},'
                      f'{min(samples):.3f},{max(samples):.3f},{first:.3f}', flush=True)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                os.close(master)
