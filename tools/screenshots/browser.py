# -*- coding: utf-8 -*-
"""Headless Chrome / Chromium through the DevTools protocol (CDP), with the Python standard library only.

- find_chrome(): the browser to use (an explicit path, XEXAM_CHROME or CHROME_PATH, then the usual places of each OS);
- Chrome: starts one headless browser with a temporary profile (removed again by close()), asks it for a free debugging
  port, and connects to its first tab;
- CDP: a minimal WebSocket client (RFC 6455, client side) that sends commands and waits for their answers;
- strip_png(): removes text, time and EXIF chunks from a PNG, so no screenshot carries metadata;
- kill_tree(): ends a process together with its children (Windows: taskkill /T; elsewhere: its process group).
"""
import base64
import json
import os
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request


def find_chrome(explicit=None):
    """Path of a Chrome / Chromium (or Edge, which speaks the same protocol) executable, or None."""
    for value in (explicit, os.environ.get('XEXAM_CHROME'), os.environ.get('CHROME_PATH')):
        if value:
            found = value if os.path.isfile(value) else shutil.which(value)
            if not found:
                raise SystemExit('Chrome not found at %r (from --chrome, XEXAM_CHROME or CHROME_PATH)' % value)
            return found
    candidates = []
    if sys.platform == 'win32':
        roots = [os.environ.get(k) for k in ('ProgramFiles', 'ProgramFiles(x86)', 'LOCALAPPDATA')]
        for sub in (('Google', 'Chrome', 'Application', 'chrome.exe'), ('Chromium', 'Application', 'chrome.exe'),
                    ('Microsoft', 'Edge', 'Application', 'msedge.exe')):
            candidates += [os.path.join(r, *sub) for r in roots if r]
    elif sys.platform == 'darwin':
        for app in ('Google Chrome.app/Contents/MacOS/Google Chrome', 'Chromium.app/Contents/MacOS/Chromium',
                    'Microsoft Edge.app/Contents/MacOS/Microsoft Edge'):
            candidates += ['/Applications/' + app, os.path.expanduser('~/Applications/' + app)]
    else:
        for name in ('google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser', 'microsoft-edge'):
            path = shutil.which(name)
            if path:
                candidates.append(path)
    return next((c for c in candidates if os.path.isfile(c)), None)


def kill_tree(proc, timeout=10):
    """Ends `proc` and everything it started; waits for it. Safe to call twice."""
    if proc is None or proc.poll() is not None:
        return
    if sys.platform == 'win32':
        subprocess.run(['taskkill', '/PID', str(proc.pid), '/T', '/F'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    else:
        try:
            os.killpg(proc.pid, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            proc.terminate()
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        if sys.platform != 'win32':
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
        proc.kill()
        proc.wait(timeout=timeout)


def popen_group(args, **kw):
    """subprocess.Popen in its own process group, so kill_tree() reaches its children too."""
    if sys.platform == 'win32':
        kw.setdefault('creationflags', subprocess.CREATE_NEW_PROCESS_GROUP)
    else:
        kw.setdefault('start_new_session', True)
    return subprocess.Popen(args, **kw)


def remove_dir(path, tries=10):
    """shutil.rmtree that waits a little for files a process that just exited still holds (Windows)."""
    for i in range(tries):
        if not os.path.exists(path):
            return True
        shutil.rmtree(path, ignore_errors=True)
        if not os.path.exists(path):
            return True
        time.sleep(0.3 * (i + 1))
    return not os.path.exists(path)


# ---------------------------------------------------------------- PNG metadata

_PNG_SIGNATURE = b'\x89PNG\r\n\x1a\n'
# Ancillary chunks that only describe how to show the pixels; everything else that is not critical (tEXt, zTXt, iTXt,
# tIME, eXIf, iCCP with its profile name, private chunks) is dropped
_KEEP = {b'tRNS', b'gAMA', b'cHRM', b'sRGB', b'sBIT', b'pHYs', b'bKGD'}


def strip_png(data):
    if data[:8] != _PNG_SIGNATURE:
        raise ValueError('not a PNG')
    out, pos = [_PNG_SIGNATURE], 8
    while pos < len(data):
        length = struct.unpack('>I', data[pos:pos + 4])[0]
        kind = data[pos + 4:pos + 8]
        chunk = data[pos:pos + 12 + length]
        if kind[:1].isupper() or kind in _KEEP:      # an upper-case first letter marks a critical chunk (IHDR, IDAT, ...)
            out.append(chunk)
        pos += 12 + length
        if kind == b'IEND':
            break
    return b''.join(out)


def png_chunks(data):
    """The chunk types of a PNG, in order (for checks)."""
    kinds, pos = [], 8
    while pos < len(data):
        length = struct.unpack('>I', data[pos:pos + 4])[0]
        kinds.append(data[pos + 4:pos + 8].decode('latin-1'))
        pos += 12 + length
    return kinds


# ---------------------------------------------------------------- WebSocket + CDP

class CDP:
    """One DevTools connection (a tab or the browser). call() sends a command and returns its result; events are skipped."""

    def __init__(self, url, timeout=120):
        u = urllib.parse.urlsplit(url)
        if u.scheme != 'ws':
            raise ValueError('expected a ws:// URL: ' + url)
        self.sock = socket.create_connection((u.hostname, u.port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        path = u.path + ('?' + u.query if u.query else '')
        self.sock.sendall(('GET %s HTTP/1.1\r\nHost: %s:%d\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n'
                           'Sec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n'
                           % (path, u.hostname, u.port, key)).encode())
        buf = b''
        while b'\r\n\r\n' not in buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise ConnectionError('DevTools closed the connection during the handshake')
            buf += chunk
        head, self.buf = buf.split(b'\r\n\r\n', 1)
        if b' 101 ' not in head.split(b'\r\n')[0]:
            raise ConnectionError('DevTools refused the WebSocket: %r' % head[:120])
        self.next_id = 0

    def _read(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(1 << 20)
            if not chunk:
                raise ConnectionError('DevTools closed the connection')
            self.buf += chunk
        data, self.buf = self.buf[:n], self.buf[n:]
        return data

    def _frame(self):
        b0, b1 = self._read(2)
        fin, op, masked, n = b0 & 0x80, b0 & 0x0F, b1 & 0x80, b1 & 0x7F
        if n == 126:
            n = struct.unpack('>H', self._read(2))[0]
        elif n == 127:
            n = struct.unpack('>Q', self._read(8))[0]
        mask = self._read(4) if masked else None
        data = self._read(n)
        if mask:
            data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        return fin, op, data

    def _send(self, op, data):
        head = bytearray([0x80 | op])
        n = len(data)
        if n < 126:
            head.append(0x80 | n)
        elif n < 65536:
            head += bytes([0x80 | 126]) + struct.pack('>H', n)
        else:
            head += bytes([0x80 | 127]) + struct.pack('>Q', n)
        mask = os.urandom(4)
        self.sock.sendall(bytes(head) + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def _message(self):
        parts = []
        while True:
            fin, op, data = self._frame()
            if op == 0x9:                      # ping
                self._send(0xA, data)
                continue
            if op == 0xA:                      # pong
                continue
            if op == 0x8:
                raise ConnectionError('DevTools closed the connection')
            parts.append(data)
            if fin:
                return b''.join(parts)

    def call(self, method, **params):
        self.next_id += 1
        mid = self.next_id
        self._send(0x1, json.dumps({'id': mid, 'method': method, 'params': params}).encode())
        while True:
            msg = json.loads(self._message())
            if msg.get('id') == mid:
                if 'error' in msg:
                    raise RuntimeError('%s: %s' % (method, msg['error']))
                return msg.get('result', {})

    def evaluate(self, js):
        r = self.call('Runtime.evaluate', expression=js, awaitPromise=True, returnByValue=True)
        if 'exceptionDetails' in r:
            d = r['exceptionDetails']
            text = (d.get('exception') or {}).get('description') or d.get('text') or json.dumps(d)
            raise RuntimeError('script error in the page: ' + text[:600])
        return r.get('result', {}).get('value')

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


class Chrome:
    """One headless browser with a temporary profile. Use as a context manager, or call close()."""

    def __init__(self, exe, lang):
        self.profile = tempfile.mkdtemp(prefix='xexam-shots-chrome-')
        args = [exe, '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--hide-scrollbars',
                '--disable-extensions', '--disable-sync', '--disable-background-networking', '--disable-component-update',
                '--disable-default-apps', '--disable-features=Translate,MediaRouter,OptimizationHints', '--mute-audio',
                '--password-store=basic', '--use-mock-keychain', '--lang=' + lang, '--remote-debugging-address=127.0.0.1',
                '--remote-debugging-port=0', '--user-data-dir=' + self.profile, 'about:blank']
        if hasattr(os, 'geteuid') and os.geteuid() == 0:
            args.insert(1, '--no-sandbox')          # Chrome refuses to start as root with the sandbox (containers)
        env = dict(os.environ, TZ='UTC')
        self.page = None
        self.browser_url = None
        self.proc = popen_group(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)
        try:
            port, browser_path = self._devtools_port()
            self.browser_url = 'ws://127.0.0.1:%d%s' % (port, browser_path)
            tabs = []
            for _ in range(100):
                tabs = [t for t in json.load(urllib.request.urlopen('http://127.0.0.1:%d/json/list' % port, timeout=5))
                        if t.get('type') == 'page']
                if tabs:
                    break
                time.sleep(0.1)
            self.page = CDP(tabs[0]['webSocketDebuggerUrl'])
        except BaseException:
            self.close()
            raise

    def _devtools_port(self):
        # With --remote-debugging-port=0 Chrome picks a free port and writes it (and the browser's path) to this file
        path = os.path.join(self.profile, 'DevToolsActivePort')
        deadline = time.time() + 30
        while time.time() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError('the browser exited at once (code %s)' % self.proc.returncode)
            try:
                with open(path, encoding='utf-8') as fh:
                    lines = fh.read().split()
                if len(lines) >= 2:
                    return int(lines[0]), lines[1]
            except (OSError, ValueError):
                pass
            time.sleep(0.1)
        raise RuntimeError('the browser did not open its DevTools port within 30 s')

    def close(self):
        if self.page:
            self.page.close()
            self.page = None
        if self.proc.poll() is None and self.browser_url:
            try:
                b = CDP(self.browser_url, timeout=5)
                try:
                    b.call('Browser.close')
                except (ConnectionError, OSError):
                    pass
                b.close()
                self.proc.wait(timeout=10)
            except Exception:
                pass
        kill_tree(self.proc)
        if not remove_dir(self.profile):
            print('  warning: could not remove the temporary browser profile ' + self.profile)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
