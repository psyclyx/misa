#!/usr/bin/env python3
"""PTY oracle: input/projection proceed while a native grammar load is blocked.

A FIFO named python.so holds the dynamic loader. Opening its writer confirms
that the native worker reached the load; no bytes are sent until a subsequent
keyboard transaction has rendered. Invalid ELF bytes then release the loader
and must produce the ordinary empty-capture fallback. Platforms whose dynamic
loader rejects FIFOs without reading them explicitly skip this oracle.
"""
import errno
import fcntl
import json
import os
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from configuration import application
from fixture_environment import fixture_environment


class UnsupportedLoader(Exception):
    pass


def main():
    binary = str(Path(sys.argv[1]).resolve())
    fixture = str(Path(__file__).with_suffix('.fnl').resolve())
    with tempfile.TemporaryDirectory(prefix='misa-syntax-worker-') as directory:
        work = Path(directory)
        gate = work / 'python.so'
        os.mkfifo(gate)
        config = work / 'config.fnl'
        config.write_text(application({'extensions': [fixture]}))
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        original = termios.tcgetattr(slave)
        env = fixture_environment(work, TERM='xterm-256color', MISA_TREE_SITTER_DIR=directory,
                   MISA_AUTH_FILE=str(work / 'auth'), MISA_STATE_FILE=str(work / 'state'))
        process = subprocess.Popen([binary, '--config', str(config)],
                                   stdin=slave, stdout=slave, stderr=slave,
                                   env=env, close_fds=True)
        output = bytearray()
        deadline = time.monotonic() + 10
        writer = None

        def until(predicate, description):
            while time.monotonic() < deadline:
                if predicate():
                    return
                if process.poll() is not None:
                    raise AssertionError(f'app exited {process.returncode} while {description}: {output[-2000:]!r}')
                if select.select([master], [], [], .01)[0]:
                    output.extend(os.read(master, 65536))
            raise AssertionError(f'timed out {description}: {output[-2000:]!r}')

        def reader_waiting():
            nonlocal writer
            if b'done=1' in output:
                raise UnsupportedLoader('dynamic loader rejected the FIFO without blocking')
            try:
                writer = os.open(gate, os.O_WRONLY | os.O_NONBLOCK)
                return True
            except OSError as error:
                if error.errno != errno.ENXIO:
                    raise
                return False

        def input_processed():
            if b'done=1' in output:
                raise UnsupportedLoader('dynamic loader completed before FIFO data was released')
            return b'done=0 healthy=1' in output

        try:
            until(reader_waiting, 'waiting for grammar loader FIFO reader')
            assert os.write(master, b'\t') == 1
            until(input_processed, 'processing input while grammar loading remains blocked')
            os.write(writer, b'invalid ELF fixture\n')
            os.close(writer)
            writer = None
            until(lambda: b'done=1 healthy=1' in output, 'receiving empty captures after grammar release')
            assert os.write(master, b'\x04') == 1
            process.wait(timeout=max(.01, deadline - time.monotonic()))
            assert process.returncode == 0
            assert termios.tcgetattr(slave) == original, 'terminal modes not restored'
            print('syntax worker PTY passed: input rendered during blocked grammar load, '
                  'empty-capture completion, clean shutdown')
        except UnsupportedLoader as error:
            print(f'syntax worker PTY skipped: {error}')
        finally:
            if writer is not None:
                os.close(writer)
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(slave)


if __name__ == '__main__':
    main()
