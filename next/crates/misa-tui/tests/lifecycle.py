#!/usr/bin/env python3
"""Real process/PTY lifecycle gate. Run from next/ after building misa + daemon."""
import os, pathlib, pty, re, select, signal, subprocess, tempfile, time, fcntl, termios, struct
root = pathlib.Path.cwd()
bin = pathlib.Path(os.environ.get('CARGO_TARGET_DIR', 'target')) / 'debug'
work = pathlib.Path(tempfile.mkdtemp(prefix='misa-pty-lifecycle-'))
os.environ.update(XDG_RUNTIME_DIR=str(work/'run'), XDG_STATE_HOME=str(work/'state'), MISA_PREFS=str(work/'prefs.json'))
(work/'run').mkdir(mode=0o700)
os.environ['TERM']='xterm-256color'
children=[]
def spawn(args):
    pid, fd = pty.fork()
    if pid == 0:
        os.execv(str(bin/args[0]), [str(bin/args[0]), *args[1:]])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 120, 0, 0))
    children.append(pid)
    return pid, fd

def drain(fd, seconds=.3):
    end=time.monotonic()+seconds; data=b''
    while time.monotonic()<end:
        if select.select([fd],[],[],max(0,end-time.monotonic()))[0]:
            try: chunk=os.read(fd,65536)
            except OSError: break
            if not chunk: break
            data+=chunk
    return data

def wait(pid, seconds=10):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        found,status=os.waitpid(pid,os.WNOHANG)
        if found:
            children.remove(pid)
            return os.waitstatus_to_exitcode(status)
        time.sleep(.05)
    raise AssertionError(f'process {pid} did not exit')

def alive(pid):
    found,status=os.waitpid(pid,os.WNOHANG)
    if found:
        children.remove(pid)
        raise AssertionError(f'process {pid} exited early: {status}')

try:
    daemon,server=spawn(['misa-daemon','--no-relay','--session','pty'])
    output=b''; deadline=time.monotonic()+20
    while time.monotonic()<deadline and not re.search(rb'misa:[^\s]+:pty',output): output+=drain(server,.2)
    ticket=re.search(rb'misa:[^\s]+:pty',output)
    assert ticket, output
    ticket=ticket[0].decode()
    for forced in [False,True]:
        result=subprocess.run([str(bin/'misa'),*(['--print'] if forced else []),ticket,'terminal EOF test'],input=b'',capture_output=True,timeout=30)
        assert result.returncode==0,(result.stdout,result.stderr)
        assert any(reply in result.stdout for reply in [b'that is all I have.', b'still here.']),result.stdout
        alive(daemon)
    client,terminal=spawn(['misa',ticket]); data=drain(terminal,2)
    assert b'\x1b[' in data,data
    os.write(terminal,b'\x04')
    assert wait(client)==0
    alive(daemon); os.close(terminal)
    client,terminal=spawn(['misa',ticket]); data=drain(terminal,2)
    os.write(terminal,b'/model ');data+=drain(terminal,1)
    assert b'scripted' in data.lower(),data
    os.write(terminal,b'\x1b[200~ pasted\ntext\x1b[201~');data+=drain(terminal,.5)
    os.write(terminal,b'\x04');data+=drain(terminal,.5)
    alive(client) # nonempty editor is not EOF
    assert b'pasted' in data,data
    (work/'interactive.ansi').write_bytes(data)
    os.write(terminal,b'\x11');assert wait(client)==0;os.close(terminal)
    alive(daemon)
    os.write(server,b'\x03');assert wait(daemon)==0
    (work/'daemon.log').write_bytes(output+drain(server,.2));os.close(server)
    print(f'PASS automatic/forced print EOF, daemon survival, empty Ctrl-D, nonempty Ctrl-D, picker/paste, daemon Ctrl-C: {work}')
finally:
    for pid in children:
        try: os.kill(pid,signal.SIGKILL);os.waitpid(pid,0)
        except ProcessLookupError: pass
