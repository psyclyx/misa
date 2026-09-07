#!/usr/bin/env python3
"""Exercise the default UI through a PTY with Ghostty keyboard/graphics protocols.

Only provider/auth transports are substituted. State observations use a fixture
event, while every tested interaction arrives through the real terminal decoder.
"""
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
import zlib
from pathlib import Path

root = Path(__file__).resolve().parent.parent
binary = str(Path(sys.argv[1]).resolve())
fixture = (root / 'tests/ghostty-input.fnl').read_text()


with tempfile.TemporaryDirectory(prefix='misa-ghostty-') as directory:
    work = Path(directory)
    # Deterministic PNG fixture, generated without a graphics dependency.
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    pixels = b''.join(b'\0' + bytes([40, 150, 120, 255]) * 80 for _ in range(40))
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 80, 40, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(pixels)) + chunk(b'IEND', b'')
    (work / 'image.png').write_bytes(png)
    (work / 'fixture.fnl').write_text(fixture)
    config = json.loads((root / 'config/default.json').read_text())
    config['extensions'] = [str(work/'fixture.fnl')] + [name for name in config['extensions'] if not name.startswith(('provider.', 'protocol.')) and name != 'auth']
    settings = config['config']
    settings['models']['default'] = 'smoke/model'
    settings['images'] = {'clipboard_command': ['cat', str(work/'image.png')]}
    settings['smoke'] = {'directory': directory, 'python': sys.executable}
    for name in ('history', 'themes', 'components', 'preferences'):
        settings.setdefault(name, {})['persist'] = False
    (work / 'config.json').write_text(json.dumps(config))
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 32, 100, 0, 0))
    env = dict(os.environ, TERM='xterm-ghostty', TERM_PROGRAM='ghostty', MISA_EXTENSION_DIR=str(root/'extensions'), MISA_AUTH_FILE=str(work/'auth'), MISA_STATE_FILE=str(work/'state'))
    env.pop('TMUX', None)
    env.pop('STY', None)
    process = subprocess.Popen([binary, '--config', str(work/'config.json')], stdin=slave, stdout=slave, stderr=slave, env=env)
    os.close(slave)
    output = bytearray()
    def until(predicate, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate(): return
            if process.poll() is not None: raise AssertionError(f'app exited {process.returncode}: {output[-3000:]!r}')
            if select.select([master], [], [], .01)[0]: output.extend(os.read(master, 65536))
        raise AssertionError(f'timed out: {output[-3000:]!r}')
    def send(data): os.write(master, data)
    inspection = 0
    def snapshot():
        global inspection
        inspection += 1
        path = work / f'snapshot-{inspection}'
        send(b'\x1bz')
        until(lambda: path.exists() and path.stat().st_size > 0)
        return json.loads(path.read_text())
    def ready(count):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            state = snapshot()
            if len(state['requests']) == count and state['status'] == 'ready': return state
        raise AssertionError(state)
    try:
        until(lambda: b'smoke/model' in output)
        until(lambda: b'\x1b[?2026l' in output)
        assert b'\x1b[?1003h' in output, 'all-motion mouse reporting was not enabled'
        # The empty default UI puts its model button on row 4, column 20.
        # Wait for presentation as well as state: an inspection can complete
        # before the corresponding frame has reached the PTY.
        def move_and_render(column, row):
            start = len(output)
            send(f'\x1b[<35;{column};{row}M'.encode())
            state = snapshot()
            until(lambda: b'\x1b[?2026l' in output[start:])
            return state, bytes(output[start:])
        assert b'48;2;59;82;96' not in output, 'button had a hover background at rest'
        state, frame = move_and_render(20, 4)
        assert state.get('hover_action') == 'models.open', 'mouse motion did not reach the model button'
        assert b'48;2;59;82;96' in frame, 'hover state did not produce a background'
        state, frame = move_and_render(100, 1)
        assert not state.get('hover_action'), 'leaving an action retained hover state'
        assert b'48;2;59;82;96' not in frame, 'leaving an action retained its background'
        send(b'\x16')
        until(lambda: b'\x1b_Ga=t' in output)
        assert snapshot()['attachments'] == 1
        send(b'first\x1b[13;2uline')
        assert snapshot()['text'] == 'first\nline', 'Ghostty Shift-Enter did not insert a newline'
        assert output.count(b'\x1b_Ga=t') == 1, 'draft edits retransmitted image pixels'
        send(b'\r')
        state = snapshot()
        assert len(state['requests']) == 1 and state['requests'][0]['content'][1]['type'] == 'image'
        send(b'second\rthird\r')
        assert snapshot()['pending'] == 'second\nthird'
        send(b'\x1be')
        state = snapshot()
        assert state['text'] == 'second\nthird' and state['pending'] == '', 'default Alt-E binding did not edit pending input'
        send(b'\x1b[13;3u')
        state = ready(2)
        assert state['requests'][1]['content'][0]['text'] == 'second\nthird'
        send(b'again\r')
        ready(3)
        send(b'\x10\r')
        state = ready(4)
        assert state['history'] == ['first\nline', 'second\nthird', 'again'], 'consecutive history was not deduplicated'
        send(b'draft\x10')
        assert snapshot()['text'] == 'again'
        send(b'\x0e')
        assert snapshot()['text'] == 'draft'
        send(b'\x12')
        assert snapshot()['picker'] == {'id': 'input-history', 'count': 3}
        send(b'frst\r')
        assert snapshot()['text'] == 'first\nline', 'fuzzy search did not restore multiline source'
        send(b'\x1bs')
        assert snapshot()['selection'], 'selection could not enter the existing transcript'
        send(b'\x1b[<64;10;10M')
        assert snapshot().get('top') is not None, 'mouse wheel did not scroll the transcript'
        send(b'\x1b')
        until(lambda: not select.select([master], [], [], .04)[0], timeout=1)
        send(b'\x03/clear\r')
        state = snapshot()
        assert state['status'] == 'ready' and state['messages'] == 0, 'conversation clear did not settle'
        send(b'\x03\x04')
        process.wait(timeout=3)
        while select.select([master], [], [], .01)[0]:
            try: output.extend(os.read(master, 65536))
            except OSError: break
        assert process.returncode == 0
        assert re.search(rb'\x1b\[[0-9;]*38;2;', output) and re.search(rb'\x1b\[[0-9;]*48;2;', output), 'truecolor styles were absent'
        assert b'\x1b]8;;https://example.test' in output, 'Markdown links were not clickable'
        assert b'\x1b[?2026h' in output and b'\x1b[<u' in output
        print('Ghostty PTY passed: hover background/leave, images, Shift-Enter, queue editing/steering, history, selection, scrolling, RGB and links')
    finally:
        if process.poll() is None: process.kill(); process.wait()
        os.close(master)
