#!/usr/bin/env python3
"""Fatal signals must restore terminal modes and leave the alternate screen."""
import fcntl
import json
import os
import pty
import resource
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path
from fixture_environment import fixture_environment


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    binary = str(Path(sys.argv[1]).resolve())
    fixture = Path(__file__).with_name('threaded-terminal.fnl').resolve()
    with tempfile.TemporaryDirectory(prefix='misa-terminal-failure-') as directory:
        work = Path(directory)
        config = work / 'config.json'
        config.write_text(json.dumps({'extensions': [str(fixture)]}))
        for sig in (signal.SIGABRT, signal.SIGTERM):
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
            original = termios.tcgetattr(slave)
            process = subprocess.Popen([binary, '--config', str(config)],
                                       stdin=slave, stdout=slave, stderr=slave,
                                       env=fixture_environment(work, TERM='xterm-256color'))
            output = bytearray()
            try:
                deadline = time.monotonic() + 5
                while b'BOOT count=0' not in output:
                    assert process.poll() is None, f'premature exit: {output!r}'
                    assert time.monotonic() < deadline, f'startup timeout: {output!r}'
                    if select.select([master], [], [], .05)[0]:
                        output.extend(os.read(master, 65536))
                assert b'\x1b[?1049h' in output
                os.kill(process.pid, sig)
                process.wait(timeout=5)
                while select.select([master], [], [], .05)[0]:
                    output.extend(os.read(master, 65536))
                assert process.returncode == -sig, process.returncode
                assert b'\x1b[?1049l' in output, 'fatal exit left the alternate screen active'
                assert termios.tcgetattr(slave) == original, 'fatal exit left raw mode active'
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                os.close(master)
                os.close(slave)
    print('fatal terminal cleanup passed: SIGABRT and SIGTERM restore screen and termios')


if __name__ == '__main__':
    main()
