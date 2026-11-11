# -*- coding: utf-8 -*-
"""Nginx behavior check (independent of the backend implementation): starts the nginx image with Docker + an "echo" fake backend, and verifies:
  Host passed through unchanged, X-Forwarded-* cleared, 413 returns JSON, security headers, static files, hidden files 404, unknown methods rejected,
  the rate limits of /api/upload and /api/, JSON 502 when the backend is unreachable, /healthz.

Usage: python -I tests/integration/nginx_check.py <image, default xexam-frontend:test> [port, default 18080]
Uses only Docker and the standard library; created containers / networks all carry the xexam-nginxcheck prefix and are cleaned up at the end."""
import concurrent.futures
import http.client
import json
import subprocess
import sys
import time

IMAGE = sys.argv[1] if len(sys.argv) > 1 else 'xexam-frontend:test'
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 18080
NET = 'xexam-nginxcheck-net'
BACK = 'xexam-nginxcheck-backend'
WEB = 'xexam-nginxcheck-web'

ECHO = r'''
import http.server, json
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def _do(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n) if n else b""
        out = json.dumps({"method": self.command, "path": self.path, "headers": {k.lower(): v for k, v in self.headers.items()}, "body_len": len(body)}).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(out))); self.end_headers(); self.wfile.write(out)
    do_GET = do_POST = do_HEAD = _do
    def log_message(self, *a): pass
http.server.ThreadingHTTPServer(("0.0.0.0", 8765), H).serve_forever()
'''

results = []


def ok(cond, msg):
    results.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(results))
        cleanup()
        sys.exit(1)


def sh(*a, check=True):
    r = subprocess.run(a, capture_output=True, text=True, encoding='utf-8')
    if check and r.returncode != 0:
        raise SystemExit('Command failed: %s\n%s' % (' '.join(a), r.stderr))
    return r


def cleanup():
    for c in (WEB, BACK):
        sh('docker', 'rm', '-f', c, check=False)
    sh('docker', 'network', 'rm', NET, check=False)


def req(method, path, headers=None, body=None, host=None):
    c = http.client.HTTPConnection('127.0.0.1', PORT, timeout=15)
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


cleanup()
sh('docker', 'network', 'create', NET)
try:
    # Fake backend (internal network only)
    sh('docker', 'run', '-d', '--name', BACK, '--network', NET, '--network-alias', 'backend', 'python:3.12-slim', 'python', '-c', ECHO)
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
    import re
    asset = re.search(rb'/assets/[\w.\-]+\.js', b)
    ok(bool(asset), 'Page references a hashed JS file')
    st, h2, b2 = req('GET', asset.group(0).decode(), host='localhost:%d' % PORT)
    ok(st == 200 and 'immutable' in h2.get('cache-control', ''), 'Hashed assets are cacheable long-term. Actual %r' % h2.get('cache-control'))
    ok('javascript' in h2.get('content-type', ''), 'JS Content-Type is correct. Actual %r' % h2.get('content-type'))
    for p, ct in (('/icon.svg', 'svg'), ('/apple-touch-icon.png', 'png')):
        st, h3, _ = req('GET', p, host='localhost:%d' % PORT)
        ok(st == 200 and ct in h3.get('content-type', ''), '%s is reachable' % p)

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
    st, _, b = req('GET', '/api/info', host='xexam.example.com', headers={
        'X-Forwarded-For': '6.6.6.6', 'X-Forwarded-Host': 'evil.example.com', 'X-Forwarded-Proto': 'https',
        'Forwarded': 'for=1.2.3.4', 'X-Real-IP': '7.7.7.7'})
    ok(st == 200, 'Proxied to the backend. Actual %d' % st)
    seen = json.loads(b)['headers']
    ok(seen.get('host') == 'xexam.example.com', 'Host seen by the backend = Host sent by the browser. Actual %r' % seen.get('host'))
    for k in ('x-forwarded-for', 'x-forwarded-host', 'x-forwarded-proto', 'forwarded', 'x-real-ip'):
        ok(k not in seen, 'Backend does not receive %s (Nginx cleared it)' % k)
    st, _, b = req('GET', '/api/info', host='192.168.1.20:8765')
    ok(json.loads(b)['headers'].get('host') == '192.168.1.20:8765', 'Host with port is also passed through unchanged')
    st, _, b = req('POST', '/api/attempt', host='localhost', headers={'Content-Type': 'application/json', 'Origin': 'https://xexam.example.com'}, body=b'{"a":1}')
    seen = json.loads(b)
    ok(seen['headers'].get('origin') == 'https://xexam.example.com' and seen['body_len'] == 7, 'Origin and request body reach the backend unchanged')

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

    # -- Backend down --
    sh('docker', 'stop', BACK)
    time.sleep(1)
    st, hh, b = req('GET', '/api/info', host='localhost')
    ok(st == 502 and 'json' in hh.get('content-type', ''), 'JSON 502 when the backend is unreachable. Actual %d %r' % (st, hh.get('content-type')))
    ok('后端暂时连不上' in json.loads(b)['error'], '502 carries a readable Chinese explanation')

    # -- Runtime identity --
    r = sh('docker', 'exec', WEB, 'id', '-u')
    ok(r.stdout.strip() != '0', 'Nginx does not run as root (uid %s)' % r.stdout.strip())
    r = sh('docker', 'inspect', '-f', '{{.State.Health.Status}}', WEB)
    ok(r.stdout.strip() in ('healthy', 'starting'), 'Container health check status: %s' % r.stdout.strip())
finally:
    cleanup()

print('\n'.join(results))
print('\nAll %d checks passed' % len(results))
