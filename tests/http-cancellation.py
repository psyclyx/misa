#!/usr/bin/env python3
"""Exercise the real HTTP transport while a chunked body read is canceled."""
import json
import gzip
import resource
import socketserver
import subprocess
import sys
import tempfile
import threading
from pathlib import Path
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from configuration import application
from fixture_environment import fixture_environment


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


class Response(socketserver.StreamRequestHandler):
    def handle(self):
        request = self.rfile.readline()
        headers = {}
        while line := self.rfile.readline().strip():
            key, value = line.split(b':', 1)
            headers[key.lower()] = value.strip()
        if b'/gzip ' in request:
            payload = self.rfile.read(int(headers.get(b'content-length', b'0')))
            assert json.loads(payload) == {'hello': 'world'}, payload
            body = gzip.compress(b'{"answer":42}')
            self.wfile.write(b'HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\n'
                             b'Content-Length: ' + str(len(body)).encode() + b'\r\n\r\n' + body)
            self.wfile.flush()
            return
        self.wfile.write(b'HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n'
                         b'Content-Type: text/event-stream\r\n\r\n'
                         b'A\r\ndata: {}\n\n\r\n')
        self.wfile.flush()
        self.server.release.wait(10)


def main():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    binary = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix='misa-http-cancel-') as directory:
        work = Path(directory)
        for mode in ('cancel', 'idle', 'buffered', 'gzip'):
            with Server(('127.0.0.1', 0), Response) as server:
                server.release = threading.Event()
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start()
                fixture = work / 'fixture.lua'
                fixture.write_text('''
return function(ctx)
  return {events={
    start={event='app/start', handler=function()
      return {fx={{type='http/request', id='request', completion='test/http',
        url=ctx.config.url, method=ctx.config.mode == 'gzip' and 'POST' or 'GET',
        json=ctx.config.mode == 'gzip' and {hello='world'} or nil,
        response_format=ctx.config.format,
        timeouts={first_byte_ms=2000, idle_ms=100, overall_ms=5000}}}}
    end},
    http={event='test/http', handler=function(db,event)
      if event.phase == 'start' then return nil end
      if ctx.config.mode == 'gzip' then
        assert(event.ok and event.status == 200 and event.data.answer == 42)
        return {fx={{type='view/commit',lines={{spans={{text='gzip'}}}}},
                    {type='app/quit'}}}
      end
      if event.phase == 'data' then
        if ctx.config.mode == 'cancel' then
          return {fx={{type='operation/cancel',id='request'}}}
        end
        return nil
      end
      assert(event.ok == false, 'unexpected result: phase=' .. tostring(event.phase) .. ' ok=' .. tostring(event.ok) .. ' message=' .. tostring(event.message))
      assert(event.message == ctx.config.expected, event.message)
      return {fx={{type='view/commit',lines={{spans={{text=event.message}}}}},
                  {type='app/quit'}}}
    end}
  }}
end
''')
                expected = 'gzip' if mode == 'gzip' else 'Canceled' if mode == 'cancel' else 'IdleTimeout'
                config = work / 'config.fnl'
                config.write_text(application({'extensions': [str(fixture)], 'config': {
                    'url': f'http://127.0.0.1:{server.server_address[1]}/{mode}',
                    'mode': mode, 'expected': expected,
                    'format': 'json' if mode == 'gzip' else 'text' if mode == 'buffered' else 'sse_json_stream'}}))
                try:
                    result = subprocess.run([binary, '--config', str(config)],
                                            input=b'', capture_output=True, timeout=8,
                                            env=fixture_environment(work, TERM='dumb'))
                    assert result.returncode == 0, (mode, result.returncode, result.stderr.decode())
                    assert result.stdout == (expected + '\n').encode(), result.stdout
                finally:
                    server.release.set()
                    server.shutdown()
                    thread.join()
    print('HTTP transport passed: SSE cancel, SSE/buffered idle timeout, POST JSON with gzip response')


if __name__ == '__main__':
    main()
