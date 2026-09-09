#!/usr/bin/env python3
"""Recover from quick callbacks forming an endless synchronous dispatch chain.

Already committed changes survive the limit. Later keyboard input and shutdown
must work, while headless use fails explicitly instead of hanging. This does
not test or claim preemption of a callback that never returns.
"""
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


def main():
    binary = str(Path(sys.argv[1]).resolve())
    fixture = str(Path(__file__).with_suffix('.fnl').resolve())
    with tempfile.TemporaryDirectory(prefix='misa-dispatch-limit-') as directory:
        work = Path(directory)
        config = work / 'config.fnl'
        settings = {'extensions': [fixture], 'config': {'runtime': {'max_dispatch_chain': 8}}}
        config.write_text(application(settings))
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        original = termios.tcgetattr(slave)
        env = fixture_environment(work, TERM='xterm-256color',
                   MISA_AUTH_FILE=str(work / 'auth'), MISA_STATE_FILE=str(work / 'state'))
        process = subprocess.Popen([binary, '--config', str(config)],
                                   stdin=slave, stdout=slave, stderr=slave,
                                   env=env, close_fds=True)
        output = bytearray()
        deadline = time.monotonic() + 10

        def until(marker):
            while time.monotonic() < deadline:
                if marker in output:
                    return
                if process.poll() is not None:
                    raise AssertionError(f'app exited {process.returncode}: {output[-2000:]!r}')
                if select.select([master], [], [], .01)[0]:
                    output.extend(os.read(master, 65536))
            raise AssertionError(f'timed out awaiting {marker!r}: {output[-2000:]!r}')

        def send(data):
            assert os.write(master, data) == len(data)

        try:
            until(b'count=0 notices=0 healthy=0')
            for trial in (1, 2):
                send(b'\t')
                # The root key consumes one of eight chain events; seven spin
                # transactions commit before the unprocessed tail is discarded.
                until(f'count={7 * trial} notices={trial} healthy={trial - 1}'.encode())
                send(b'\x1b[B')
                until(f'count={7 * trial} notices={trial} healthy={trial}'.encode())
            send(b'\x04')
            process.wait(timeout=max(.01, deadline - time.monotonic()))
            assert process.returncode == 0
            assert termios.tcgetattr(slave) == original, 'terminal modes not restored'
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(slave)
        settings['config']['headless'] = True
        config.write_text(application(settings))
        result = subprocess.run([binary, '--config', str(config)], input=b'',
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                env=env, timeout=max(.01, deadline - time.monotonic()))
        assert result.returncode != 0, 'headless runaway silently succeeded'
        assert b'DispatchChainLimitExceeded' in result.stderr, result.stderr
        print('dispatch limit PTY passed: committed state retained, two recoveries, '
              'healthy input, clean shutdown, explicit headless failure')


if __name__ == '__main__':
    main()
