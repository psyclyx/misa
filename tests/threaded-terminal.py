#!/usr/bin/env python3
"""Prove terminal presentation can finish while a Fennel transaction is blocked.

Pause PTY output, submit two distinct input events in one write, then wait for
Fennel to open a FIFO in the second transaction. Resume terminal output and
require the first transaction's frame before releasing that FIFO. This is an
ordering oracle, not a latency benchmark. A serial render/session loop either
blocks before reaching the FIFO or defers the frame until the FIFO is released.
"""
import errno
import fcntl
import json
import os
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path


def main():
    binary = str(Path(sys.argv[1]).resolve())
    fixture = Path(__file__).with_suffix('.fnl').resolve()
    with tempfile.TemporaryDirectory(prefix='misa-threaded-terminal-') as directory:
        work = Path(directory)
        gate = work / 'gate.fifo'
        os.mkfifo(gate)
        config = work / 'config.json'
        config.write_text(json.dumps({'extensions': [str(fixture)],
                                      'config': {'gate': str(gate)}}))
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        original = termios.tcgetattr(slave)
        env = dict(os.environ, TERM='xterm-256color',
                   MISA_AUTH_FILE=str(work / 'auth'), MISA_STATE_FILE=str(work / 'state'))
        env.pop('TMUX', None)
        env.pop('STY', None)
        process = subprocess.Popen([binary, '--config', str(config)],
                                   stdin=slave, stdout=slave, stderr=slave,
                                   env=env, close_fds=True)
        output = bytearray()
        deadline = time.monotonic() + 10
        writer = None
        paused = False

        def until(predicate, description):
            while time.monotonic() < deadline:
                if predicate():
                    return
                if process.poll() is not None:
                    raise AssertionError(f'app exited {process.returncode} while {description}: {output[-2000:]!r}')
                if select.select([master], [], [], .01)[0]:
                    output.extend(os.read(master, 65536))
            raise AssertionError(f'timed out {description}: {output[-2000:]!r}')

        def fifo_reader_waiting():
            nonlocal writer
            try:
                writer = os.open(gate, os.O_WRONLY | os.O_NONBLOCK)
                return True
            except OSError as error:
                if error.errno != errno.ENXIO:
                    raise
                return False

        def send(data):
            assert os.write(master, data) == len(data), 'short PTY input write'

        try:
            until(lambda: b'BOOT count=0' in output and b'\x1b[?2026l' in output,
                  'waiting for startup frame')
            termios.tcflow(slave, termios.TCOOFF)
            paused = True
            # TAB publishes READY; Ctrl-R blocks in the next native transaction.
            send(b'\t\x12')
            until(fifo_reader_waiting, 'waiting for Fennel FIFO reader with output paused')
            assert b'READY count=0' not in output, 'PTY output pause did not hold the frame'
            termios.tcflow(slave, termios.TCOON)
            paused = False
            until(lambda: b'READY count=0' in output,
                  'presenting the previous frame while Fennel remains blocked')
            assert b'RELEASED' not in output, 'Fennel gate released prematurely'

            # Exercise bounded buffering and alternating event order while the
            # session cannot consume input, then resize after the queue drains.
            count = 128
            send(b'\x1b[A\x1b[B' * (count // 2))
            until(lambda: struct.unpack('i', fcntl.ioctl(slave, termios.TIOCINQ,
                                                        struct.pack('i', 0)))[0] == 0,
                  'buffering input while Fennel remains blocked')
            os.write(writer, b'return true\n')
            os.close(writer)
            writer = None
            until(lambda: f'RELEASED count={count} size=100x24'.encode() in output,
                  'draining ordered input after releasing Fennel')
            # SIGWINCH may interrupt libc's FIFO read; observe the completed
            # transaction before testing resize rather than signalling the gate.
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 31, 91, 0, 0))
            os.kill(process.pid, signal.SIGWINCH)
            expected = f'RELEASED count={count} size=91x31'.encode()
            until(lambda: expected in output, 'draining ordered input and observing resize')
            send(b'\x04')
            process.wait(timeout=max(.01, deadline - time.monotonic()))
            assert process.returncode == 0, f'app exit status {process.returncode}'
            assert termios.tcgetattr(slave) == original, 'shutdown did not restore terminal modes'
            print('threaded terminal PTY passed: frame presented during blocked Fennel, '
                  f'{count} ordered keys, resize, clean shutdown')
        finally:
            if paused:
                termios.tcflow(slave, termios.TCOON)
            if writer is not None:
                os.close(writer)
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(slave)


if __name__ == '__main__':
    main()
