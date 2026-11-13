# -*- coding: utf-8 -*-
"""Nginx behavior check (independent of the backend and AI implementations): starts the nginx image with Docker and two
"echo" fakes, one as the backend (network alias backend, port 8765) and one as the AI service (alias ai-service, port
8766), and verifies:
  Host passed through unchanged, X-Forwarded-* cleared, 413 returns JSON, security headers, static files and icons, hidden
  files 404, unknown methods rejected, the rate limits of /api/upload and /api/, JSON 502 when the backend is unreachable,
  /healthz;
  AI (v0.8): every /api/ai/ request is first checked by the backend (auth_request to /internal/ai/auth with the
  X-Original-* headers); the AI service receives no Cookie, Authorization, Cloudflare Access token or forwarding header,
  only the identity statement the auth step returned (a client value is overwritten); the auth refusals are JSON with a
  stable code and their texts equal the backend's constants; the two answer routes stream without buffering; methods,
  body size and the per-session start limit; JSON 502 when the AI service is down; /internal/ (the auth step, the
  token routes and ai-service's heartbeat) and /_ai_auth are 404 from outside, also through a path with ".." in it;
  the admin's Test (/api/ai/admin/verify) has its own location that is checked the same way and waits longer than the
  90 s of the other AI routes (an answer after 95 s still arrives); the three AI proxy locations carry the same auth,
  header and error lines and the upstream http://ai-service:8766 (read from nginx -T);
  AI (v1.0): a 429 JSON answer of ai-service (the per-user call cap, code ai.callLimit) reaches the browser unchanged on
  every kind of AI location (the two streaming routes, an ordinary AI route, the admin Test): status, JSON body and
  Retry-After header intact, not replaced by an Nginx error page, and Nginx's own 429 (limit_req) stays non-JSON.

  AI import (v1.1, IX-7 / SEC-1): the exact location = /api/ai/import/upload copies every proxy, header-scrub, auth and error
  line of /api/ai/ and differs only in the body limit, the read timeout, the request rate zone and the buffering of the
  request; an unauthenticated 20 MiB POST is answered 401 by the auth step and its body never reaches the AI service; a
  good session reaches the AI service with the identity and without Cookie / Authorization / forwarding headers; a body of
  20 MiB + 1 is 413 JSON with the code ai.import.tooLarge; GET on the route is 403 or 405; other methods are refused.

Usage: python -I tests/integration/nginx_check.py <image, default xexam-frontend:test> [port, default 18080]
Uses only Docker and the standard library; the containers and the network carry the prefix xexam-nginxcheck- plus a random
suffix and are removed at the end."""
import concurrent.futures
import http.client
import json
import os
import re
import socket
import subprocess
import sys
import threading
import time
import uuid

IMAGE = sys.argv[1] if len(sys.argv) > 1 else 'xexam-frontend:test'
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 18080
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
RUN = uuid.uuid4().hex[:6]
NET = 'xexam-nginxcheck-net-' + RUN
BACK = 'xexam-nginxcheck-backend-' + RUN
AI = 'xexam-nginxcheck-ai-' + RUN
WEB = 'xexam-nginxcheck-web-' + RUN

# One fake for both upstreams. ECHO_NAME / ECHO_PORT choose the role; every answer carries X-Echo-Name, every request is
# printed (method and path) to stdout.
#  backend: GET /internal/ai/auth answers by the session cookie (good... -> 204 + an identity built from the X-Original-*
#           headers; pw / guard -> 403 + X-Xexam-Deny; down -> 502; anything else -> 401 + X-Xexam-Deny: signin)
#  ai-service: POST /api/ai/send, /api/ai/retry, /api/ai/study/ask and /api/ai/summary/ask stream text/event-stream (an "echo" event with what arrived, then one
#           "delta" event per second for 4 s); POST /api/ai/admin/verify?delay=N waits N seconds before its
#           answer; a POST whose query holds capped=1 answers 429 JSON with Retry-After: 17 (v1.0 per-user caps);
#           everything else answers a JSON echo of the request
ECHO = r'''
import http.server, json, os, sys, time, urllib.parse
NAME = os.environ.get("ECHO_NAME", "backend")
PORT = int(os.environ.get("ECHO_PORT", "8765"))
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def _send(self, status, body=b"", headers=()):
        self.send_response(status)
        self.send_header("X-Echo-Name", NAME)
        for k, v in headers:
            self.send_header(k, v)
        if status != 204:
            self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body and status != 204:
            self.wfile.write(body)
    def _do(self):
        print(self.command, self.path, flush=True)
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n) if n else b""
        seen = {"name": NAME, "method": self.command, "path": self.path,
                "headers": {k.lower(): v for k, v in self.headers.items()}, "body_len": len(body)}
        if NAME == "backend" and self.path == "/internal/ai/auth" and self.command == "GET":
            cookies = dict(p.strip().split("=", 1) for p in (self.headers.get("Cookie") or "").split(";") if "=" in p)
            s = cookies.get("xexam_session", "")
            if s.startswith("good"):
                ident = "id|%s|%s|%s" % (self.headers.get("X-Original-Method"), self.headers.get("X-Original-Host"),
                                         self.headers.get("X-Original-Origin") or "-")
                return self._send(204, headers=[("X-Xexam-Identity", ident), ("Cache-Control", "no-store")])
            if s in ("pw", "guard"):
                deny = "password" if s == "pw" else "guard"
                return self._send(403, b'{"error": "x"}', [("X-Xexam-Deny", deny), ("Content-Type", "application/json")])
            if s == "down":
                return self._send(502, b"down", [("Content-Type", "text/plain")])
            return self._send(401, b'{"error": "x"}', [("X-Xexam-Deny", "signin"), ("Content-Type", "application/json")])
        if NAME == "ai-service" and self.command == "POST" and "capped=1" in self.path:
            cap = json.dumps({"error": "made-up cap text", "code": "ai.callLimit", "limit": 5, "cap": "minute",
                              "retryAt": 1790000000, "retryAfter": 17})
            return self._send(429, cap.encode(), [("Content-Type", "application/json"), ("Retry-After", "17"), ("Cache-Control", "no-store")])
        if NAME == "ai-service" and self.command == "POST" and self.path in ("/api/ai/send", "/api/ai/retry", "/api/ai/study/ask", "/api/ai/summary/ask"):
            self.send_response(200)
            self.send_header("X-Echo-Name", NAME)
            self.send_header("Content-Type", "text/event-stream; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            def chunk(text):
                data = text.encode()
                self.wfile.write(b"%x\r\n%s\r\n" % (len(data), data))
                self.wfile.flush()
            try:
                chunk("event: echo\ndata: %s\n\n" % json.dumps(seen))
                for i in range(4):
                    chunk('event: delta\ndata: {"text": "part %d"}\n\n' % i)
                    time.sleep(1)
                chunk('event: done\ndata: {"status": "done"}\n\n')
                self.wfile.write(b"0\r\n\r\n")
                self.wfile.flush()
            except OSError:
                self.close_connection = True
            return
        if NAME == "ai-service" and self.command == "POST" and self.path.startswith("/api/ai/admin/verify?"):
            delay = urllib.parse.parse_qs(self.path.split("?", 1)[1]).get("delay", ["0"])[0]
            time.sleep(float(delay))
        self._send(200, json.dumps(seen).encode(), [("Content-Type", "application/json")])
    do_GET = do_POST = do_HEAD = do_PUT = do_DELETE = _do
    def log_message(self, *a): pass
http.server.ThreadingHTTPServer(("0.0.0.0", PORT), H).serve_forever()
'''

results = []


def ok(cond, msg):
    results.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(results))
        cleanup()
        sys.exit(1)


def sh(*a, check=True):
    r = subprocess.run(a, capture_output=True, text=True, encoding='utf-8', errors='replace')
    if check and r.returncode != 0:
        raise SystemExit('Command failed: %s\n%s' % (' '.join(a), r.stderr))
    return r


def cleanup():
    for c in (WEB, BACK, AI):
        sh('docker', 'rm', '-f', c, check=False)
    sh('docker', 'network', 'rm', NET, check=False)


def req(method, path, headers=None, body=None, host=None, timeout=15):
    c = http.client.HTTPConnection('127.0.0.1', PORT, timeout=timeout)
    h = dict(headers or {})
    if host is not None:
        h['Host'] = host
    c.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
    for k, v in h.items():
        c.putheader(k, v)
    if body is not None:
        c.putheader('Content-Length', str(len(body)))
    c.endheaders(body)
    r = c.getresponse()
    data = r.read()
    hdrs = {k.lower(): v for k, v in r.getheaders()}
    c.close()
    return r.status, hdrs, data


def raw(request_line):
    """Sends one request line as typed (no normalisation by the client) and returns the status code."""
    with socket.create_connection(('127.0.0.1', PORT), timeout=10) as s:
        s.sendall(('%s\r\nHost: localhost\r\nConnection: close\r\n\r\n' % request_line).encode('latin-1'))
        data = b''
        while True:
            chunk = s.recv(65536)
            if not chunk:
                break
            data += chunk
    m = re.match(rb'HTTP/1\.[01] (\d{3})', data)
    return int(m.group(1)) if m else 0


def echo_log(name):
    r = sh('docker', 'logs', name, check=False)
    return (r.stdout or '') + (r.stderr or '')


def backend_constant(name):
    src = open(os.path.join(ROOT, 'backend', 'src', 'api', 'response.rs'), encoding='utf-8').read()
    m = re.search(r'pub const %s: &str = "([^"]*)";' % name, src)
    return m.group(1) if m else None


def jbody(b):
    try:
        return json.loads(b)
    except ValueError:
        return None


def location_block(conf, spec):
    """The directives of `location <spec> { ... }` in an nginx -T dump, one whitespace-normalised line each, comments
    removed, sorted; None when the location is missing."""
    i = conf.find('location %s {' % spec)
    if i < 0:
        return None
    start = conf.index('{', i)
    depth = 0
    for k in range(start, len(conf)):
        if conf[k] == '{':
            depth += 1
        elif conf[k] == '}':
            depth -= 1
            if depth == 0:
                lines = (' '.join(line.split('#', 1)[0].split()) for line in conf[start + 1:k].splitlines())
                return sorted(line for line in lines if line)
    return None


cleanup()
sh('docker', 'network', 'create', NET)
try:
    # Fake backend and fake AI service (internal network only)
    sh('docker', 'run', '-d', '--name', BACK, '--network', NET, '--network-alias', 'backend',
       '-e', 'ECHO_NAME=backend', '-e', 'ECHO_PORT=8765', 'python:3.12-slim', 'python', '-u', '-c', ECHO)
    sh('docker', 'run', '-d', '--name', AI, '--network', NET, '--network-alias', 'ai-service',
       '-e', 'ECHO_NAME=ai-service', '-e', 'ECHO_PORT=8766', 'python:3.12-slim', 'python', '-u', '-c', ECHO)
    # nginx: same as in compose - read-only, non-root, all capabilities dropped
    sh('docker', 'run', '-d', '--name', WEB, '--network', NET, '-p', '127.0.0.1:%d:8080' % PORT,
       '--read-only', '--tmpfs', '/tmp', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', IMAGE)
    for _ in range(40):
        try:
            req('GET', '/healthz', host='localhost')
            break
        except Exception:
            time.sleep(0.5)
    time.sleep(1.5)

    # -- The auth_request module is part of the image's nginx --
    r = sh('docker', 'run', '--rm', '--entrypoint', 'nginx', IMAGE, '-V', check=False)
    ok('--with-http_auth_request_module' in (r.stdout + r.stderr), 'nginx -V lists --with-http_auth_request_module')

    # -- The three AI proxy locations: the same auth step, header rules and error pages (from the image's nginx -T) --
    r = sh('docker', 'run', '--rm', '--entrypoint', 'nginx', IMAGE, '-T', check=False)
    ai_prefix = location_block(r.stdout, '/api/ai/')
    ai_stream = location_block(r.stdout, '~ ^/api/ai/(send|retry|study/ask|summary/ask)$')
    ai_verify = location_block(r.stdout, '= /api/ai/admin/verify')
    ok(None not in (ai_prefix, ai_stream, ai_verify),
       'nginx -T shows the locations /api/ai/, ~ ^/api/ai/(send|retry|study/ask|summary/ask)$ and = /api/ai/admin/verify')
    shared = ('auth_request', 'set $ai_upstream', 'proxy_pass', 'proxy_http_version', 'proxy_set_header', 'client_max_body_size',
              'limit_req zone=ai ', 'proxy_connect_timeout', 'proxy_request_buffering', 'proxy_buffering', 'proxy_intercept_errors',
              'error_page')
    same = lambda lines: [line for line in lines if line.startswith(shared)]
    ok(same(ai_prefix) == same(ai_stream) == same(ai_verify) and len(same(ai_prefix)) >= 20,
       'The three AI locations share their auth, proxy_set_header and error_page lines (%d lines)' % len(same(ai_prefix)))
    ok(all('set $ai_upstream http://ai-service:8766;' in b for b in (ai_prefix, ai_stream, ai_verify)),
       'The three AI locations proxy to the compose service ai-service (set $ai_upstream http://ai-service:8766;)')
    # v1.1 (IX-7, SEC-1): the exact upload location copies every proxy / header / auth / error line of /api/ai/
    ai_upload = location_block(r.stdout, '= /api/ai/import/upload')
    ok(ai_upload is not None, 'nginx -T shows the exact location = /api/ai/import/upload')
    keep = lambda lines: [line for line in lines if line.startswith(('auth_request', 'auth_request_set', 'set $ai_upstream', 'proxy_pass',
                                                                     'proxy_http_version', 'proxy_set_header', 'proxy_connect_timeout',
                                                                     'proxy_intercept_errors', 'error_page'))]
    # the one listed difference (IX-7): its 413 goes to @ai_import_too_large instead of @ai_too_large
    not413 = lambda lines: [line for line in keep(lines) if not line.startswith('error_page 413')]
    ok(not413(ai_upload) == not413(ai_prefix) and len(not413(ai_upload)) >= 20
       and 'error_page 413 = @ai_import_too_large;' in ai_upload and 'error_page 413 = @ai_too_large;' in ai_prefix,
       'The upload location carries the same auth, proxy_set_header (every scrub) and error_page lines as /api/ai/, its 413 '
       'going to @ai_import_too_large (%d lines)' % len(keep(ai_upload)))
    ok('client_max_body_size 20m;' in ai_upload or any(l.startswith('client_max_body_size 20') for l in ai_upload),
       'The upload location allows bodies up to 20 MiB. Actual %r' % [l for l in ai_upload if l.startswith('client_max_body_size')])
    no_read = lambda lines: [line for line in lines if not line.startswith('proxy_read_timeout ')]
    ok(no_read(ai_verify) == no_read(ai_prefix), 'The admin Test location equals /api/ai/ except for the read timeout')
    timeouts = [[line for line in b if line.startswith('proxy_read_timeout ')] for b in (ai_prefix, ai_stream, ai_verify)]
    ok(timeouts == [['proxy_read_timeout 90s;'], ['proxy_read_timeout 600s;'], ['proxy_read_timeout 300s;']],
       'Read timeouts: /api/ai/ 90 s, the streaming answers 600 s, the admin Test 300 s. Actual %r' % timeouts)

    st, h, b = req('GET', '/healthz', host='localhost:%d' % PORT)
    ok(st == 200 and b.strip() == b'ok', '/healthz returns 200 ok')
    try:
        st0, _, _ = req('GET', '/healthz', host='')
    except Exception:
        st0 = 444           # 444 = Nginx closes the connection directly; the client sees the connection closed
    ok(st0 in (444, 400), 'Request without a Host is rejected directly by Nginx (never reaches the backend). Actual %r' % st0)

    # -- Static files --
    st, h, b = req('GET', '/', host='localhost:%d' % PORT)
    ok(st == 200 and b'<div id="app">' in b and b'Xexam' in b, 'Home page is served directly by Nginx (contains #app)')
    ok(h.get('cache-control') == 'no-cache', 'index.html: Cache-Control: no-cache. Actual %r' % h.get('cache-control'))
    asset = re.search(rb'/assets/[\w.\-]+\.js', b)
    ok(bool(asset), 'Page references a hashed JS file')
    st, h2, b2 = req('GET', asset.group(0).decode(), host='localhost:%d' % PORT)
    ok(st == 200 and 'immutable' in h2.get('cache-control', ''), 'Hashed assets are cacheable long-term. Actual %r' % h2.get('cache-control'))
    ok('javascript' in h2.get('content-type', ''), 'JS Content-Type is correct. Actual %r' % h2.get('content-type'))
    for p, ct in (('/icon.svg', 'svg'), ('/icon-32.png', 'png'), ('/icon-192.png', 'png'), ('/icon-512.png', 'png'),
                  ('/apple-touch-icon.png', 'png')):
        st, h3, b3 = req('GET', p, host='localhost:%d' % PORT)
        ok(st == 200 and ct in h3.get('content-type', '') and len(b3) > 0, '%s is served (%s). Actual %d %r' % (p, ct, st, h3.get('content-type')))

    # -- Security headers --
    for k, want in (('x-content-type-options', 'nosniff'), ('x-frame-options', 'DENY'), ('referrer-policy', 'no-referrer')):
        ok(h.get(k) == want, 'Home page security header %s: %s. Actual %r' % (k, want, h.get(k)))
    csp = h.get('content-security-policy', '')
    ok("default-src 'self'" in csp and "frame-ancestors 'none'" in csp and "style-src 'self'" in csp, 'Home page has a strict CSP. Actual %r' % csp[:80])
    ok('server' in h and not re.search(r'\d+\.\d+', h['server']), 'Server header carries no version number. Actual %r' % h.get('server'))

    # -- Paths and methods --
    st, _, _ = req('GET', '/nope.txt', host='localhost:%d' % PORT)
    ok(st == 404, 'Unknown path returns 404 (this app has no URL-routed pages, so no SPA fallback is needed). Actual %d' % st)
    st, _, _ = req('GET', '/.git/config', host='localhost:%d' % PORT)
    ok(st == 404, 'Dotfile returns 404. Actual %d' % st)
    st, _, _ = req('GET', '/assets/', host='localhost:%d' % PORT)
    ok(st in (403, 404), 'No directory listing. Actual %d' % st)
    st, _, _ = req('DELETE', '/api/bank', host='localhost:%d' % PORT)
    ok(st == 403, 'DELETE /api/ is rejected by Nginx. Actual %d' % st)
    st, _, _ = req('PUT', '/api/bank', host='localhost:%d' % PORT)
    ok(st == 403, 'PUT /api/ is rejected by Nginx. Actual %d' % st)
    st, _, _ = req('GET', '/api/upload', host='localhost:%d' % PORT)
    ok(st == 403, 'GET /api/upload is rejected (POST only). Actual %d' % st)

    # -- Proxy: Host passed through unchanged, forwarding headers cleared --
    st, hh, b = req('GET', '/api/info', host='xexam.example.com', headers={
        'X-Forwarded-For': '6.6.6.6', 'X-Forwarded-Host': 'evil.example.com', 'X-Forwarded-Proto': 'https',
        'Forwarded': 'for=1.2.3.4', 'X-Real-IP': '7.7.7.7'})
    ok(st == 200 and hh.get('x-echo-name') == 'backend', 'GET /api/info is proxied to the backend. Actual %d %r' % (st, hh.get('x-echo-name')))
    seen = json.loads(b)['headers']
    ok(seen.get('host') == 'xexam.example.com', 'Host seen by the backend = Host sent by the browser. Actual %r' % seen.get('host'))
    for k in ('x-forwarded-for', 'x-forwarded-host', 'x-forwarded-proto', 'forwarded', 'x-real-ip'):
        ok(k not in seen, 'Backend does not receive %s (Nginx cleared it)' % k)
    st, _, b = req('GET', '/api/info', host='192.168.1.20:8765')
    ok(json.loads(b)['headers'].get('host') == '192.168.1.20:8765', 'Host with port is also passed through unchanged')
    st, _, b = req('POST', '/api/attempt', host='localhost', headers={'Content-Type': 'application/json', 'Origin': 'https://xexam.example.com'}, body=b'{"a":1}')
    seen = json.loads(b)
    ok(seen['headers'].get('origin') == 'https://xexam.example.com' and seen['body_len'] == 7, 'Origin and request body reach the backend unchanged')

    # -- AI: the auth step, credentials removed, the identity statement --
    host = 'localhost:%d' % PORT
    st, hh, b = req('GET', '/api/ai/status', host=host, headers={
        'Cookie': 'xexam_session=good; CF_Authorization=z', 'Authorization': 'Bearer x', 'Cf-Access-Jwt-Assertion': 'y',
        'X-Xexam-Identity': 'forged', 'X-Forwarded-For': '203.0.113.9', 'Forwarded': 'for=x', 'X-Real-IP': '203.0.113.9',
        'X-Original-Method': 'POST', 'X-Original-Host': 'evil.example.com', 'X-Original-Origin': 'https://evil.example.com'})
    ok(st == 200 and hh.get('x-echo-name') == 'ai-service', 'GET /api/ai/status with a good session is answered by the AI service. Actual %d %r' % (st, hh.get('x-echo-name')))
    seen = json.loads(b)
    sh_ = seen['headers']
    for k in ('cookie', 'authorization', 'cf-access-jwt-assertion', 'x-forwarded-for', 'x-forwarded-host', 'x-forwarded-proto',
              'forwarded', 'x-real-ip'):
        ok(k not in sh_, 'The AI service does not receive %s' % k)
    ok(sh_.get('x-xexam-identity') == 'id|GET|%s|-' % host,
       'The AI service receives only the identity of the auth step (client value and client X-Original-* ignored). Actual %r' % sh_.get('x-xexam-identity'))
    ok(sh_.get('host') == host and seen['path'] == '/api/ai/status', 'Host and path reach the AI service unchanged. Actual %r %r' % (sh_.get('host'), seen['path']))
    st, hh, b = req('GET', '/api/ai/chat?id=7', host=host, headers={'Cookie': 'xexam_session=good'})
    ok(st == 200 and json.loads(b)['path'] == '/api/ai/chat?id=7', 'The query string reaches the AI service. Actual %d' % st)

    # The admin's Test of the AI settings has its own location (a longer read timeout) with the same auth step and the
    # same header rules
    st, hh, b = req('POST', '/api/ai/admin/verify', host=host, headers={
        'Cookie': 'xexam_session=good; CF_Authorization=z', 'Authorization': 'Bearer x', 'Cf-Access-Jwt-Assertion': 'y',
        'X-Xexam-Identity': 'forged', 'X-Forwarded-For': '203.0.113.9', 'Forwarded': 'for=x', 'X-Real-IP': '203.0.113.9',
        'Origin': 'https://app.example.com', 'Content-Type': 'application/json'}, body=b'{}')
    ok(st == 200 and hh.get('x-echo-name') == 'ai-service',
       'POST /api/ai/admin/verify with a good session is answered by the AI service. Actual %d %r' % (st, hh.get('x-echo-name')))
    seen = json.loads(b)
    for k in ('cookie', 'authorization', 'cf-access-jwt-assertion', 'x-forwarded-for', 'x-forwarded-host', 'x-forwarded-proto',
              'forwarded', 'x-real-ip'):
        ok(k not in seen['headers'], 'The AI service does not receive %s on the admin Test' % k)
    ok(seen['headers'].get('x-xexam-identity') == 'id|POST|%s|https://app.example.com' % host and seen['body_len'] == 2,
       'The admin Test carries the identity of the auth step and its body. Actual %r' % seen['headers'].get('x-xexam-identity'))

    # A Test that answers only after 95 s (longer than the 90 s of /api/ai/; the AI service allows up to 2 min per
    # attempt and one retry) still gets its answer. It runs in the background while the other checks go on and is
    # collected before the AI service is stopped.
    slow = {}

    def slow_verify():
        t = time.time()
        try:
            slow['r'] = req('POST', '/api/ai/admin/verify?delay=95', host=host, timeout=200,
                            headers={'Cookie': 'xexam_session=good', 'Content-Type': 'application/json'}, body=b'{}')
        except Exception as e:
            slow['r'] = (0, {}, repr(e).encode())
        slow['t'] = time.time() - t
    slow_thread = threading.Thread(target=slow_verify, daemon=True)
    slow_thread.start()

    # POST with an Origin: the auth step sees method and Origin; the answer streams without buffering
    t0 = time.time()
    c = http.client.HTTPConnection('127.0.0.1', PORT, timeout=15)
    c.putrequest('POST', '/api/ai/send', skip_host=True, skip_accept_encoding=True)
    payload = b'{"chatId": null, "text": "hello"}'
    for k, v in (('Host', host), ('Cookie', 'xexam_session=good'), ('Origin', 'https://app.example.com'),
                 ('Content-Type', 'application/json'), ('Content-Length', str(len(payload)))):
        c.putheader(k, v)
    c.endheaders(payload)
    r = c.getresponse()
    first, text = None, b''
    while True:
        part = r.read1(65536)
        if not part:
            break
        text += part
        if first is None and b'event: delta' in text:
            first = time.time() - t0
    total = time.time() - t0
    c.close()
    ok(r.status == 200 and 'text/event-stream' in (r.getheader('Content-Type') or ''), 'POST /api/ai/send streams text/event-stream. Actual %d %r' % (r.status, r.getheader('Content-Type')))
    ok(first is not None and first < 1.5 and total >= 3.5,
       'The answer is not buffered: first event after %.2f s (< 1.5), whole body after %.2f s (>= 3.5)' % (first or -1, total))
    echo = json.loads(text.split(b'event: echo\ndata: ', 1)[1].split(b'\n', 1)[0])
    ok(echo['headers'].get('x-xexam-identity') == 'id|POST|%s|https://app.example.com' % host and 'cookie' not in echo['headers'],
       'POST: the identity carries the method and the Origin, no cookie. Actual %r' % echo['headers'].get('x-xexam-identity'))
    ok(echo['body_len'] == len(payload) and text.count(b'event: delta') == 4, 'The request body arrives whole; all 4 events arrive')

    # Refusals of the auth step: JSON with a code; the AI service sees none of them
    msg_signin, msg_password = backend_constant('MSG_SIGN_IN_REQUIRED'), backend_constant('MSG_PASSWORD_CHANGE_REQUIRED')
    ok(msg_signin and msg_password, 'MSG_SIGN_IN_REQUIRED and MSG_PASSWORD_CHANGE_REQUIRED found in backend/src/api/response.rs')
    for cookie, want_st, code, probe in ((None, 401, 'auth.signin', 'none'), ('xexam_session=bad', 401, 'auth.signin', 'bad'),
                                         ('xexam_session=pw', 403, 'auth.password', 'pw'),
                                         ('xexam_session=guard', 403, 'auth.guard', 'guard'),
                                         ('xexam_session=down', 503, 'ai.backendDown', 'down')):
        st, hh, b = req('GET', '/api/ai/chats?probe=' + probe, host=host, headers={'Cookie': cookie} if cookie else {})
        j = jbody(b) or {}
        ok(st == want_st and 'application/json' in hh.get('content-type', '') and j.get('code') == code,
           'Auth step refusal (%s): %d JSON code %s. Actual %d %r %r' % (probe, want_st, code, st, hh.get('content-type'), j))
        ok(hh.get('cache-control') == 'no-store' and hh.get('x-content-type-options') == 'nosniff', 'The refusal (%s) is not cached' % probe)
        if code == 'auth.signin':
            ok(j.get('error') == msg_signin, 'auth.signin text equals MSG_SIGN_IN_REQUIRED')
        if code == 'auth.password':
            ok(j.get('error') == msg_password, 'auth.password text equals MSG_PASSWORD_CHANGE_REQUIRED')
        if code == 'auth.guard':
            ok('XEXAM_ALLOWED_HOSTS' in j.get('error', ''), 'auth.guard text names XEXAM_ALLOWED_HOSTS')
    st, hh, b = req('POST', '/api/ai/send', host=host, headers={'Cookie': 'xexam_session=pw', 'Content-Type': 'application/json'}, body=b'{}')
    ok(st == 403 and (jbody(b) or {}).get('code') == 'auth.password', 'The streaming route refuses the same way (403 auth.password). Actual %d' % st)
    for cookie, want_st, code in ((None, 401, 'auth.signin'), ('xexam_session=pw', 403, 'auth.password'),
                                  ('xexam_session=down', 503, 'ai.backendDown')):
        hdrs = {'Content-Type': 'application/json'}
        if cookie:
            hdrs['Cookie'] = cookie
        st, hh, b = req('POST', '/api/ai/admin/verify?probe=verify', host=host, headers=hdrs, body=b'{}')
        ok(st == want_st and (jbody(b) or {}).get('code') == code,
           'The admin Test refuses the same way (%d %s). Actual %d %r' % (want_st, code, st, b[:120].decode('utf-8', 'replace')))
    time.sleep(0.5)
    ai_log = echo_log(AI)
    ok('probe=' not in ai_log and ai_log.count('POST /api/ai/send') == 1,
       'The AI service received none of the refused requests. Its log: %r' % ai_log[-300:])
    ok('GET /internal/ai/auth' in echo_log(BACK), 'The backend received the auth step as GET /internal/ai/auth')

    # Methods and size
    st, hh, b = req('DELETE', '/api/ai/chats', host=host, headers={'Cookie': 'xexam_session=good'})
    ok(st == 403 and (jbody(b) or {}).get('code') == 'ai.badRequest', 'DELETE /api/ai/chats -> 403 JSON ai.badRequest. Actual %d %r' % (st, b[:120].decode('utf-8', 'replace')))
    st, hh, b = req('PUT', '/api/ai/send', host=host, headers={'Cookie': 'xexam_session=good'}, body=b'{}')
    ok(st == 403, 'PUT /api/ai/send -> 403. Actual %d' % st)
    st, hh, b = req('GET', '/api/ai/send', host=host, headers={'Cookie': 'xexam_session=good'})
    ok(st == 403, 'GET /api/ai/send -> 403 (POST only). Actual %d' % st)
    st, hh, b = req('POST', '/api/ai/send', host=host, headers={'Cookie': 'xexam_session=good', 'Content-Type': 'application/json'},
                    body=b'x' * (300 * 1024))
    ok(st == 413 and (jbody(b) or {}).get('code') == 'ai.badRequest', '300 KiB to /api/ai/send -> 413 JSON ai.badRequest. Actual %d %r' % (st, b[:120].decode('utf-8', 'replace')))
    # The study streams (hints / explanations, record summaries) use the streaming location too: no buffering, POST only
    for path in ('/api/ai/study/ask', '/api/ai/summary/ask'):
        t0 = time.time()
        c = http.client.HTTPConnection('127.0.0.1', PORT, timeout=15)
        c.putrequest('POST', path, skip_host=True, skip_accept_encoding=True)
        payload = b'{"kind": "hint"}'
        for k, v in (('Host', host), ('Cookie', 'xexam_session=good'), ('Origin', 'https://app.example.com'),
                     ('Content-Type', 'application/json'), ('Content-Length', str(len(payload)))):
            c.putheader(k, v)
        c.endheaders(payload)
        r = c.getresponse()
        first, text = None, b''
        while True:
            part = r.read1(65536)
            if not part:
                break
            text += part
            if first is None and b'event: delta' in text:
                first = time.time() - t0
        total = time.time() - t0
        c.close()
        ok(r.status == 200 and 'text/event-stream' in (r.getheader('Content-Type') or ''), 'POST %s streams text/event-stream. Actual %d %r' % (path, r.status, r.getheader('Content-Type')))
        ok(first is not None and first < 1.5 and total >= 3.5,
           'POST %s is not buffered: first event after %.2f s (< 1.5), whole body after %.2f s (>= 3.5)' % (path, first or -1, total))
        echo = json.loads(text.split(b'event: echo\ndata: ', 1)[1].split(b'\n', 1)[0])
        ok(echo['headers'].get('x-xexam-identity') == 'id|POST|%s|https://app.example.com' % host and 'cookie' not in echo['headers'],
           'POST %s: the identity carries the method and the Origin, no cookie' % path)
        st, hh, b = req('GET', path, host=host, headers={'Cookie': 'xexam_session=good'})
        ok(st == 403, 'GET %s -> 403 (POST only). Actual %d' % (path, st))
    st, hh, b = req('GET', '/api/ai/study/saved?mode=practice&key=00000000000000000000000000000000&source=practice&question=q1&kind=hint',
                    host=host, headers={'Cookie': 'xexam_session=good'})
    seen = json.loads(b) if st == 200 else {}
    ok(st == 200 and hh.get('x-echo-name') == 'ai-service' and seen.get('path', '').startswith('/api/ai/study/saved?mode=practice')
       and 'cookie' not in seen.get('headers', {}) and seen.get('headers', {}).get('x-xexam-identity') == 'id|GET|%s|-' % host,
       'GET /api/ai/study/saved reaches the AI service with the identity header and no cookie. Actual %d' % st)
    st, hh, b = req('POST', '/api/ai/memories/add', host=host, headers={'Cookie': 'xexam_session=good', 'Content-Type': 'application/json'},
                    body=b'{"content": "' + b'x' * (200 * 1024) + b'"}')
    ok(st == 200 and json.loads(b)['body_len'] > 200 * 1024, 'A 200 KiB body to another AI route passes. Actual %d' % st)
    st, hh, b = req('DELETE', '/api/ai/admin/verify', host=host, headers={'Cookie': 'xexam_session=good'})
    ok(st == 403 and (jbody(b) or {}).get('code') == 'ai.badRequest', 'DELETE /api/ai/admin/verify -> 403 JSON ai.badRequest. Actual %d' % st)
    st, hh, b = req('POST', '/api/ai/admin/verify', host=host, headers={'Cookie': 'xexam_session=good', 'Content-Type': 'application/json'},
                    body=b'x' * (300 * 1024))
    ok(st == 413 and (jbody(b) or {}).get('code') == 'ai.badRequest',
       '300 KiB to /api/ai/admin/verify -> 413 JSON ai.badRequest. Actual %d' % st)

    # Per-session start limit: one session floods, another one is not affected
    def send(cookie):
        try:
            return req('POST', '/api/ai/send', host=host, headers={'Cookie': cookie, 'Content-Type': 'application/json'}, body=b'{}')
        except Exception as e:
            return 0, {}, repr(e).encode()
    with concurrent.futures.ThreadPoolExecutor(max_workers=30) as pool:
        flood = list(pool.map(lambda _: send('xexam_session=good-a'), range(30)))
    codes = [st for st, _, _ in flood]
    ok(429 in codes and 200 in codes and set(codes) <= {200, 429},
       'Per-session send limit: 30 quick sends of one session -> %d x 200, %d x 429' % (codes.count(200), codes.count(429)))
    limited = next(hh for st, hh, _ in flood if st == 429)
    ok('json' not in limited.get('content-type', ''), "Nginx's own 429 on /api/ai/send is not JSON. Actual %r" % limited.get('content-type'))
    t0 = time.time()
    st, _, _ = send('xexam_session=good-b')
    ok(st == 200, 'Another session can send at once (separate bucket). Actual %d after %.1f s' % (st, time.time() - t0))

    # v1.0 per-user call caps: ai-service answers 429 JSON with Retry-After; Nginx hands it on unchanged on every kind of AI
    # location (no error_page swap, no buffering of the refusal into HTML)
    for path in ('/api/ai/send', '/api/ai/study/ask', '/api/ai/summary/ask', '/api/ai/memories/suggest',
                 '/api/ai/admin/verify'):
        st, hh, b = req('POST', path + '?capped=1', host=host,
                        headers={'Cookie': 'xexam_session=good-cap', 'Content-Type': 'application/json'}, body=b'{}')
        j = jbody(b) or {}
        ok(st == 429 and hh.get('x-echo-name') == 'ai-service' and 'application/json' in hh.get('content-type', ''),
           'POST %s: the AI service 429 reaches the browser as JSON from the AI service. Actual %d %r %r' % (path, st, hh.get('content-type'), b[:120]))
        ok(hh.get('retry-after') == '17', 'POST %s: the Retry-After header of the 429 is intact. Actual %r' % (path, hh.get('retry-after')))
        ok(j == {'error': 'made-up cap text', 'code': 'ai.callLimit', 'limit': 5, 'cap': 'minute', 'retryAt': 1790000000, 'retryAfter': 17},
           'POST %s: the 429 body (code, cap, limit, retryAt, retryAfter) arrives unchanged. Actual %r' % (path, j))

    # /internal/ and the auth location are not reachable from outside, also not through crafted paths
    internal_calls = lambda: len(re.findall(r'(?m)^[A-Z]+ /internal/', echo_log(BACK)))
    before = internal_calls()
    for method, path in (('GET', '/internal/ai/auth'), ('POST', '/internal/ai/log'), ('POST', '/internal/ai/introspect'),
                         ('POST', '/internal/ai/users'), ('POST', '/internal/ai/heartbeat'), ('GET', '/_ai_auth'),
                         ('GET', '/api/internal/ai/auth'), ('POST', '/api/internal/ai/heartbeat')):
        st, hh, b = req(method, path, host=host, headers={'Cookie': 'xexam_session=good', 'Content-Type': 'application/json'},
                        body=b'{}' if method == 'POST' else None)
        ok(st == 404 or (path.startswith('/api/') and hh.get('x-echo-name') == 'backend'),
           '%s %s from outside -> 404 (or the backend\'s ordinary /api/ dispatch). Actual %d' % (method, path, st))
    for line in ('GET /api/../internal/ai/log HTTP/1.1', 'GET /api/ai/../../internal/ai/auth HTTP/1.1',
                 'POST /api/ai/../../internal/ai/heartbeat HTTP/1.1', 'POST /api/%2e%2e/internal/ai/heartbeat HTTP/1.1',
                 'GET /api/%2e%2e/internal/ai/auth HTTP/1.1', 'GET /internal/ai/auth/../../../api/info HTTP/1.1',
                 'GET /api/ai/%2e%2e/%2e%2e/internal/ai/auth HTTP/1.1', 'GET /api/ai/x%2f..%2f..%2f..%2finternal HTTP/1.1'):
        st = raw(line)
        ok(st == 404, 'Raw request "%s" -> 404. Actual %d' % (line, st))
    time.sleep(0.5)
    ok(internal_calls() == before, "None of these reached the backend's /internal/ routes (the auth step was not run either)")

    # -- Upload size --
    st, hh, b = req('POST', '/api/upload', host='localhost', headers={'Content-Type': 'application/octet-stream', 'X-Filename': 'a.xlsx'}, body=b'x' * (5 * 1024 * 1024))
    ok(st == 200 and json.loads(b)['body_len'] == 5 * 1024 * 1024, '5 MB upload passes (and arrives at the backend intact). Actual %d' % st)
    st, hh, b = req('POST', '/api/upload', host='localhost', headers={'Content-Type': 'application/octet-stream', 'X-Filename': 'a.xlsx'}, body=b'x' * (20 * 1024 * 1024 + 1))
    ok(st == 413, 'Over 20 MB is rejected by Nginx (413). Actual %d' % st)
    ok('json' in hh.get('content-type', ''), '413 is JSON, not HTML. Actual %r' % hh.get('content-type'))
    ok('文件太大了' in json.loads(b)['error'], '413 carries the same Chinese message as the backend')

    # -- Rate limiting --
    codes = [req('POST', '/api/upload', host='localhost', headers={'Content-Type': 'application/octet-stream', 'X-Filename': 'a.xlsx'}, body=b'x')[0] for _ in range(12)]
    ok(429 in codes, 'Upload rate limit: 429 appears after 12 rapid requests. Actual %r' % sorted(set(codes)))
    time.sleep(6)
    # /api/ allows 30 requests per second with a burst of 120 (users behind a tunnel share one address). A burst of normal
    # size - ten quick page loads of about ten requests each - passes in full; a flood far beyond the burst gets 429.
    codes = [req('GET', '/api/info', host='localhost')[0] for _ in range(100)]
    ok(codes.count(200) == 100, 'API: a burst of 100 quick requests all pass. Actual %r' % sorted(set(codes)))
    time.sleep(5)
    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
        flood = list(pool.map(lambda _: req('GET', '/api/info', host='localhost'), range(600)))
    codes = [st for st, _, _ in flood]
    ok(codes.count(200) >= 120 and 429 in codes,
       'API: a flood of 600 requests gets 429 after the burst. Actual %d x 200, %d x 429' % (codes.count(200), codes.count(429)))
    limited = next(hh for st, hh, _ in flood if st == 429)
    ok('json' not in limited.get('content-type', ''),
       "API: Nginx's own 429 is not JSON (the frontend tells it apart from the sign-in lock by that). Actual %r" % limited.get('content-type'))
    time.sleep(6)

    # -- The slow admin Test started above --
    slow_thread.join(timeout=200)
    st, hh, b = slow.get('r', (0, {}, b'still running'))
    ok(st == 200 and hh.get('x-echo-name') == 'ai-service' and slow.get('t', 0) >= 95,
       'An admin Test answered after %.0f s (more than the 90 s of /api/ai/) reaches the admin. Actual %d %r' % (slow.get('t', 0), st, b[:120]))

    # -- AI service down: JSON 502, the rest of the app unaffected --
    sh('docker', 'stop', AI)
    time.sleep(1)
    st, hh, b = req('GET', '/api/ai/status', host=host, headers={'Cookie': 'xexam_session=good'})
    ok(st == 502 and (jbody(b) or {}).get('code') == 'ai.unavailable', 'AI service stopped: GET /api/ai/status -> 502 JSON ai.unavailable. Actual %d %r' % (st, b[:120].decode('utf-8', 'replace')))
    st, hh, b = req('POST', '/api/ai/send', host=host, headers={'Cookie': 'xexam_session=good-c', 'Content-Type': 'application/json'}, body=b'{}')
    ok(st == 502 and (jbody(b) or {}).get('code') == 'ai.unavailable', 'AI service stopped: POST /api/ai/send -> 502 JSON ai.unavailable. Actual %d' % st)
    st, hh, b = req('GET', '/api/info', host='localhost')
    ok(st == 200 and hh.get('x-echo-name') == 'backend', 'AI service stopped: GET /api/info still 200 from the backend. Actual %d' % st)
    st, hh, b = req('POST', '/api/ai/admin/verify', host=host, headers={'Cookie': 'xexam_session=good', 'Content-Type': 'application/json'},
                    body=b'{}')
    ok(st == 502 and (jbody(b) or {}).get('code') == 'ai.unavailable',
       'AI service stopped: POST /api/ai/admin/verify -> 502 JSON ai.unavailable. Actual %d' % st)

    # -- AI import upload (v1.1, IX-7 / SEC-1); the fake AI service was stopped by the section above: start it again --
    sh('docker', 'start', AI)
    for _ in range(40):
        if req('GET', '/api/ai/status', host=host, headers={'Cookie': 'xexam_session=good'})[0] == 200:
            break
        time.sleep(0.5)
    up_headers = {'Content-Type': 'application/octet-stream', 'X-Filename': 'Kitchen%20notes.pdf'}
    before = echo_log(AI).count('POST /api/ai/import/upload')
    st, hh, b = req('POST', '/api/ai/import/upload', host=host, headers=up_headers, body=b'x' * (20 * 1024 * 1024))
    ok(st in (401, 403) and (jbody(b) or {}).get('code'), 'An unauthenticated 20 MiB upload is refused by the auth step with a JSON code. Actual %d' % st)
    ok(echo_log(AI).count('POST /api/ai/import/upload') == before, 'The refused upload never reaches the AI service')
    st, hh, b = req('POST', '/api/ai/import/upload', host=host, headers=dict(up_headers, Cookie='xexam_session=good; CF_Authorization=z',
                    Authorization='Bearer x', **{'X-Xexam-Identity': 'forged', 'X-Forwarded-For': '203.0.113.9'}), body=b'0123456789')
    seen = jbody(b) or {}
    ok(st == 200 and hh.get('x-echo-name') == 'ai-service' and seen.get('path') == '/api/ai/import/upload',
       'A good session reaches the AI service on the upload route. Actual %d %r' % (st, hh.get('x-echo-name')))
    for k in ('cookie', 'authorization', 'x-forwarded-for', 'forwarded', 'x-real-ip'):
        ok(k not in (seen.get('headers') or {}), 'The AI service does not receive %s on the upload route' % k)
    ok((seen.get('headers') or {}).get('x-xexam-identity', 'forged') != 'forged' and seen.get('body_len') == 10,
       'The upload carries the identity of the auth step (a client value is overwritten) and its body')
    ok((seen.get('headers') or {}).get('x-filename') == 'Kitchen%20notes.pdf', 'X-Filename reaches the AI service unchanged')
    st, hh, b = req('POST', '/api/ai/import/upload', host=host, headers=dict(up_headers, Cookie='xexam_session=good'), body=b'x' * (20 * 1024 * 1024 + 1))
    ok(st == 413 and (jbody(b) or {}).get('code') == 'ai.import.tooLarge' and 'json' in hh.get('content-type', ''),
       '20 MiB + 1 is 413 JSON with the code ai.import.tooLarge. Actual %d %r' % (st, b[:100]))
    st, hh, b = req('GET', '/api/ai/import/upload', host=host, headers={'Cookie': 'xexam_session=good'})
    ok(st in (403, 405), 'GET on the upload route is refused (403 or 405). Actual %d' % st)
    st, hh, b = req('PUT', '/api/ai/import/upload', host=host, headers=dict(up_headers, Cookie='xexam_session=good'), body=b'x')
    ok(st in (403, 405, 444), 'PUT on the upload route is refused. Actual %d' % st)

    # -- Backend down --
    sh('docker', 'stop', BACK)
    time.sleep(1)
    st, hh, b = req('GET', '/api/info', host='localhost')
    ok(st == 502 and 'json' in hh.get('content-type', ''), 'JSON 502 when the backend is unreachable. Actual %d %r' % (st, hh.get('content-type')))
    ok('后端暂时连不上' in json.loads(b)['error'], '502 carries a readable Chinese explanation')
    st, hh, b = req('GET', '/api/ai/status', host=host, headers={'Cookie': 'xexam_session=good'})
    ok(st == 503 and (jbody(b) or {}).get('code') == 'ai.backendDown', 'Backend down: the AI auth step fails -> 503 JSON ai.backendDown. Actual %d %r' % (st, b[:120].decode('utf-8', 'replace')))

    # -- Runtime identity --
    r = sh('docker', 'exec', WEB, 'id', '-u')
    ok(r.stdout.strip() != '0', 'Nginx does not run as root (uid %s)' % r.stdout.strip())
    r = sh('docker', 'inspect', '-f', '{{.State.Health.Status}}', WEB)
    ok(r.stdout.strip() in ('healthy', 'starting'), 'Container health check status: %s' % r.stdout.strip())
    r = sh('docker', 'logs', WEB, check=False)
    ok('[emerg]' not in r.stderr and 'uninitialized variable' not in r.stderr, 'The Nginx error log has no [emerg] and no uninitialized-variable warning')
    # The access log holds the path only: a query string (session keys, record times) never reaches the access log (stdout)
    req('GET', '/api/ai/study/saved?key=QUERYLOGPROBE&mode=practice', host=host, headers={'Cookie': 'xexam_session=good'})
    r = sh('docker', 'logs', WEB, check=False)
    ok('QUERYLOGPROBE' not in r.stdout and '/api/ai/study/saved HTTP' in r.stdout,
       'The Nginx access log records the path of a request but not its query string')
finally:
    cleanup()

print('\n'.join(results))
print('\nAll %d checks passed' % len(results))
