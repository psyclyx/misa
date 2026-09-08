#!/usr/bin/env python3
"""PTY regression: synchronous dispatch chains are one visible update.

Run against a built executable. The terminal byte stream is the oracle; no
provider/network dependency or terminal-emulator snapshot approximation.
"""
import errno
import fcntl
import json
import os
import pty
import re
import select
import statistics
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path
from fixture_environment import fixture_environment

binary = str(Path(sys.argv[1]).resolve())
fixture = (Path(__file__).resolve().parent / 'settled-frames.fnl').read_text()


with tempfile.TemporaryDirectory(prefix='misa-settled-frames-') as directory:
    work = Path(directory)
    (work / 'fixture.fnl').write_text(fixture)
    (work / 'config.json').write_text(json.dumps({'extensions':[str(work/'fixture.fnl')], 'config':{'python':sys.executable,'producer':str(work/'producer.py')}}))
    latencies=[]
    for trial in range(3):
        # Include an unthrottled producer: continuously pending native records
        # must still leave presentation and keyboard service opportunities.
        count, delay = (20000, 0) if trial == 2 else (100, 0.005)
        (work / 'producer.py').write_text(f"import time\nfor i in range({count}):\n print('{{}}',flush=True)\n" + (f" time.sleep({delay})\n" if delay else ""))
        master,slave=pty.openpty()
        fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',24,100,0,0))
        env=fixture_environment(work,TERM='xterm-256color',MISA_AUTH_FILE=str(work/'auth.json'),MISA_STATE_FILE=str(work/'state.json'))
        process=subprocess.Popen([binary,'--config',str(work/'config.json')],stdin=slave,stdout=slave,stderr=slave,env=env,close_fds=True)
        os.close(slave)
        data=bytearray()
        def read_until(predicate, timeout=3):
            deadline=time.monotonic()+timeout
            while time.monotonic()<deadline:
                if predicate(bytes(data)): return
                if process.poll() is not None: raise AssertionError(f'app exited {process.returncode}: {data!r}')
                if select.select([master],[],[],0.01)[0]:
                    try: data.extend(os.read(master,65536))
                    except OSError as error:
                        if error.errno!=errno.EIO: raise
            raise AssertionError(f'timed out: {data[-2000:]!r}')
        try:
            read_until(lambda b: b'SETTLED' in b)
            read_until(lambda b: re.search(rb'count=[1-9][0-9]*', b) is not None)
            sent=time.monotonic()
            os.write(master,b'hello')
            read_until(lambda b: b'SETTLED hello' in b)
            latencies.append((time.monotonic()-sent)*1000)
            read_until(lambda b: f'count={count} DONE'.encode() in b)
            os.write(master,b'\x04')
            process.wait(timeout=2)
            assert process.returncode==0
            assert b'INTERMEDIATE' not in data, 'an intermediate dispatch state reached terminal output'
            assert data.count(b'\x1b[?2026h')>=3, 'continuous operation output starved frame presentation'
            assert latencies[-1]<1000, 'continuous operation output starved keyboard input'
        finally:
            if process.poll() is None: process.kill(); process.wait()
            os.close(master)
    print(f'settled frame regression passed: 3 trials, no intermediate frame; busy-input latency median={statistics.median(latencies):.1f}ms max={max(latencies):.1f}ms')
