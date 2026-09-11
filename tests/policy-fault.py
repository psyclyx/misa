#!/usr/bin/env python3
"""A policy fault is reported and an interactive session keeps running.

An invalid native effect and a raising handler both end a headless run. In an
interactive session they are rolled back, reported as ``runtime/effect-error``
and ``runtime/handler-error``, and the session keeps reading input.
"""
import fcntl
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

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from configuration import application
from fixture_environment import fixture_environment


def read_until(master, process, output, needle, timeout=5):
    deadline = time.monotonic() + timeout
    while needle not in output:
        assert process.poll() is None, f'exited before {needle!r}: {output!r}'
        assert time.monotonic() < deadline, f'timed out waiting for {needle!r}: {output!r}'
        if select.select([master], [], [], .05)[0]:
            output.extend(os.read(master, 65536))
    return output


def main():
    binary = str(Path(sys.argv[1]).resolve())
    fixture = Path(__file__).with_name('policy-fault.fnl').resolve()
    with tempfile.TemporaryDirectory(prefix='misa-policy-fault-') as directory:
        work = Path(directory)
        config = work / 'config.fnl'
        config.write_text(application({'extensions': [str(fixture)]}))
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 200, 0, 0))
        process = subprocess.Popen([binary, '--config', str(config)],
                                   stdin=slave, stdout=slave, stderr=slave,
                                   env=fixture_environment(work, TERM='xterm-256color'))
        output = bytearray()
        try:
            read_until(master, process, output, b'boot')
            # An invalid native effect is contained and named in the notice.
            os.write(master, b'f')
            read_until(master, process, output, b'effect-fault')
            assert process.poll() is None, f'invalid effect ended the session: {output!r}'
            # A handler that raises is contained the same way.
            os.write(master, b'h')
            read_until(master, process, output, b'handler-fault')
            assert process.poll() is None, f'handler fault ended the session: {output!r}'
            # The session still accepts input and exits cleanly on request.
            os.write(master, b'c')
            process.wait(timeout=5)
            assert process.returncode == 0, process.returncode
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(slave)
    print('policy fault containment passed: effect and handler faults are reported')


if __name__ == '__main__':
    main()
