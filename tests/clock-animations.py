#!/usr/bin/env python3
"""PTY oracle: native text/style animation advances while Fennel is blocked.

A FIFO holds the session transaction while several frame patches arrive. The
patch stream must preserve cursor/link/style without repainting unrelated rows.
After release, the database must report zero animation tick events. Removing
motion must silence the terminal; headless output uses the static fallback.
"""
import errno
import fcntl
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
from fixture_environment import fixture_environment


SYNC_END = b'\x1b[?2026l'


def main():
    binary = str(Path(sys.argv[1]).resolve())
    fixture = str(Path(__file__).with_suffix('.fnl').resolve())
    with tempfile.TemporaryDirectory(prefix='misa-clock-animations-') as directory:
        work = Path(directory)
        gate = work / 'gate.fifo'
        os.mkfifo(gate)
        config = work / 'config.json'
        config.write_text(json.dumps({'extensions': [fixture], 'config': {'gate': str(gate)}}))
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        original = termios.tcgetattr(slave)
        env = fixture_environment(work, TERM='xterm-256color',
                   MISA_AUTH_FILE=str(work / 'auth'), MISA_STATE_FILE=str(work / 'state'))
        env.pop('TMUX', None)
        env.pop('STY', None)
        process = subprocess.Popen([binary, '--config', str(config)],
                                   stdin=slave, stdout=slave, stderr=slave,
                                   env=env, close_fds=True)
        output = bytearray()
        deadline = time.monotonic() + 10
        writer = None

        def read_once(timeout=.01):
            if select.select([master], [], [], timeout)[0]:
                output.extend(os.read(master, 65536))

        def until(predicate, description):
            while time.monotonic() < deadline:
                if predicate():
                    return
                if process.poll() is not None:
                    raise AssertionError(f'app exited {process.returncode} while {description}: {output[-2000:]!r}')
                read_once()
            raise AssertionError(f'timed out {description}: {output[-2000:]!r}')

        def reader_waiting():
            nonlocal writer
            try:
                writer = os.open(gate, os.O_WRONLY | os.O_NONBLOCK)
                return True
            except OSError as error:
                if error.errno != errno.ENXIO:
                    raise
                return False

        def send(data):
            assert os.write(master, data) == len(data)

        def observe_quiet(mode):
            start = len(output)
            send(b'\t' if mode == 'static' else b'\x7f')
            marker = f'MODE={mode} ticks=0 gates=1'.encode()
            until(lambda: marker in output[start:] and
                  SYNC_END in output[output.index(marker, start):],
                  f'publishing {mode} frame')
            # Six animation intervals: this checks silence, not response speed.
            quiet_start = len(output)
            quiet_end = time.monotonic() + .25
            while time.monotonic() < quiet_end:
                read_once(max(0, min(.01, quiet_end - time.monotonic())))
            assert len(output) == quiet_start, f'{mode} descriptor kept repainting: {output[quiet_start:]!r}'

        try:
            until(lambda: b'MODE=active ticks=0 gates=0' in output and SYNC_END in output,
                  'waiting for initial frame')
            send(b'\x12')
            until(reader_waiting, 'blocking Fennel on FIFO')
            start = len(output)
            until(lambda: output[start:].count(b'ABC') >= 3 and
                  output[start:].count(b'DEF') >= 3 and
                  output[start:].count(b'GLOW') >= 6,
                  'animating text and style during blocked Fennel')
            patches = bytes(output[start:])
            assert b'MODE=' not in patches, 'animation republished the semantic frame'
            assert not re.search(rb'\x1b\[[0-9;]*[JK]', patches), 'animation cleared unrelated terminal content'
            assert (b'\x1b7' in patches and b'\x1b8' in patches) or (b'\x1b[s' in patches and b'\x1b[u' in patches), 'animation did not preserve the cursor'
            assert b'\x1b]8;;https://example.test/animation\x1b\\' in patches, 'animated link was lost'
            assert re.search(rb'\x1b\[0;1m\x1b\]8;;[^\x1b]+\x1b\\(?:ABC|DEF)', patches), 'animated text lost its base bold style'
            for color in (31, 32):
                assert f'\x1b[0;4;{color}mGLOW'.encode() in patches, 'style animation lost frame color or base underline'
            os.write(writer, b'return true\n')
            os.close(writer)
            writer = None
            until(lambda: b'MODE=active ticks=0 gates=1' in output,
                  'checking unchanged animation policy state')
            observe_quiet('static')
            observe_quiet('removed')
            send(b'\x04')
            process.wait(timeout=max(.01, deadline - time.monotonic()))
            assert process.returncode == 0
            assert termios.tcgetattr(slave) == original, 'terminal modes not restored'
        finally:
            if writer is not None:
                os.close(writer)
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(slave)

        config.write_text(json.dumps({'extensions': [fixture], 'config': {'headless': True}}))
        result = subprocess.run([binary, '--config', str(config)], input=b'',
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                env=env, timeout=3)
        assert result.returncode == 0, result.stderr
        assert result.stdout == b'ABC GLOW MODE=active ticks=0 gates=0\n', result.stdout
        assert result.stderr == b'', result.stderr
        print('clock animation PTY passed: text/style patches during blocked Fennel, '
              'zero policy ticks, static/removal silence, headless fallback')


if __name__ == '__main__':
    main()
