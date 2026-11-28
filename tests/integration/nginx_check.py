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
  the admin's Test (/api/ai/admin/connections/test) has its own location that is checked the same way and waits longer than the
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

v1.3 (N1): the header blocks of the proxied locations live in /etc/nginx/snippets/{api,ai}-proxy.conf and every proxied
  location includes one (checks follow the includes); the real client address (XEXAM_TRUSTED_PROXIES / XEXAM_REAL_IP_HEADER,
  rendered at start by 15-xexam-realip.sh): the default (variable unset, empty, blank or only quotes) trusts 10/8, 172.16/12 and 192.168/16,
  the word none trusts nobody, an explicit narrow value trusts only that; an untrusted peer with a forged CF-Connecting-IP keeps
  its own address, a trusted peer's header is believed; the value parser (standard entries only, v1.4): separators, a bad
  entry skipped with a warning next to a good one, a /0 refused, no valid entry left trusts nobody; X-Real-IP / X-Geo-Country / X-Geo-Region /
  X-Geo-City always overwritten, location headers forwarded only from a trusted peer; the login rate-limit zone; the
  streaming regex with regenerate and edit; the five admin connection / model routes with the 100 s read timeout and no
  /api/ai/admin/connections/test location; gzip on JSON and not on the event stream; ETag / If-None-Match / 304 pass through; coded
  error bodies (net.tooLarge, net.backendDown).

v3.1: /sw.js and /manifest.webmanifest have their own exact locations (200, JavaScript / application/manifest+json, no-cache,
  the security headers repeated, the CSP on the worker), the three new PNGs (/badge-96.png, /icon-maskable-192.png,
  /icon-maskable-512.png) are served as image/png, and look-alike paths (/sw.js/, /sw.js.map, /service-worker.js) stay 404.

v1.9: the upstreams and the DNS resolver are no longer fixed names in nginx.conf: 16-xexam-upstreams.sh renders
  /tmp/xexam-upstreams.conf from XEXAM_BACKEND_UPSTREAM / XEXAM_AI_UPSTREAM / XEXAM_NGINX_RESOLVER (defaults = the one-host
  names and Docker's DNS, no proxy_ssl lines). Checked here: the defaults and the `set` lines (read from nginx -T after every
  1*-xexam-*.sh start-up script ran), routing to two other fakes by name and by IP literal (also the AI auth step), the value
  parser matrix (good and bad values of all three variables, an https upstream turns the certificate check on, nothing is
  left behind after a refusal, the message is cut to 200 characters) and a container with a bad value that exits non-zero.

Usage: python -I tests/integration/nginx_check.py <image, default xexam-frontend:test> [port, default 18080]
Uses only Docker and the standard library; the containers and the network carry the prefix xexam-nginxcheck- plus a random
suffix and are removed at the end."""
import concurrent.futures
import http.client
import json
import ipaddress
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
WEB2 = 'xexam-nginxcheck-webt-' + RUN      # trusts the test client's address (real-IP matrix)
WEB3 = 'xexam-nginxcheck-webi-' + RUN      # invalid XEXAM_TRUSTED_PROXIES (fail closed)
BE_OTHER = 'xexam-nginxcheck-beo-' + RUN   # v1.9: a second fake backend (alias be-other, port 9765)
AI_OTHER = 'xexam-nginxcheck-aio-' + RUN   # v1.9: a second fake AI service (alias ai-other, port 9766)

# One fake for both upstreams. ECHO_NAME / ECHO_PORT choose the role; every answer carries X-Echo-Name, every request is
# printed (method and path) to stdout.
#  backend: GET /internal/ai/auth answers by the session cookie (good... -> 204 + an identity built from the X-Original-*
#           headers; pw / guard -> 403 + X-Xexam-Deny; down -> 502; anything else -> 401 + X-Xexam-Deny: signin)
#           GET /api/etag answers 200 with ETag "v1" and 304 when If-None-Match is "v1"
#  ai-service: POST /api/ai/send, retry, regenerate, edit, study/ask, summary/ask and assess/ask stream text/event-stream (an "echo" event with what arrived, then one
#           "delta" event per second for 4 s); POST /api/ai/admin/connections/test?delay=N waits N seconds before its
#           answer; a POST whose query holds capped=1 answers 429 JSON with Retry-After: 17 (v1.0 per-user caps);
#           everything else answers a JSON echo of the request
ECHO = r'''
import http.server, json, os, sys, time, urllib.parse
NAME = os.environ.get("ECHO_NAME", "backend")
PORT = int(os.environ.get("ECHO_PORT", "8765"))
TAG = os.environ.get("ECHO_TAG", "")   # v1.9: tells two fakes of the same role apart (X-Echo-Tag)
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def _send(self, status, body=b"", headers=()):
        self.send_response(status)
        self.send_header("X-Echo-Name", NAME)
        if TAG:
            self.send_header("X-Echo-Tag", TAG)
        for k, v in headers:
            self.send_header(k, v)
        if status not in (204, 304):
            self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body and status not in (204, 304):
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
            if s in ("pw", "guard", "name"):
                deny = {"pw": "password", "guard": "guard", "name": "name"}[s]
                return self._send(403, b'{"error": "x"}', [("X-Xexam-Deny", deny), ("Content-Type", "application/json")])
            if s == "down":
                return self._send(502, b"down", [("Content-Type", "text/plain")])
            return self._send(401, b'{"error": "x"}', [("X-Xexam-Deny", "signin"), ("Content-Type", "application/json")])
        if NAME == "ai-service" and self.command == "POST" and "capped=1" in self.path:
            cap = json.dumps({"error": "made-up cap text", "code": "ai.callLimit", "limit": 5, "cap": "minute",
                              "retryAt": 1790000000, "retryAfter": 17})
            return self._send(429, cap.encode(), [("Content-Type", "application/json"), ("Retry-After", "17"), ("Cache-Control", "no-store")])
        if NAME == "backend" and self.command == "GET" and self.path.startswith("/api/etag"):
            if self.headers.get("If-None-Match") == '"v1"':
                return self._send(304, headers=[("ETag", '"v1"')])
            return self._send(200, b'{"etag": "v1"}', [("ETag", '"v1"'), ("Content-Type", "application/json")])
        if NAME == "ai-service" and self.command == "POST" and self.path in ("/api/ai/send", "/api/ai/retry", "/api/ai/regenerate", "/api/ai/edit", "/api/ai/study/ask", "/api/ai/summary/ask", "/api/ai/assess/ask"):
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
        if NAME == "ai-service" and self.command == "POST" and self.path.startswith("/api/ai/admin/connections/test?"):
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
    for c in (WEB, WEB2, WEB3, BACK, AI, BE_OTHER, AI_OTHER):
        sh('docker', 'rm', '-f', c, check=False)
    sh('docker', 'network', 'rm', NET, check=False)


def req(method, path, headers=None, body=None, host=None, timeout=15, port=None):
    c = http.client.HTTPConnection('127.0.0.1', port or PORT, timeout=timeout)
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


def generated_file(conf, path):
    """The text of one generated file in an nginx -T dump ('' when it is not there)."""
    m = re.search(r'# configuration file %s:\n(.*?)(?=\n# configuration file |\Z)' % re.escape(path), conf, re.S)
    return m.group(1) if m else ''


def no_comments(text):
    return re.sub(r'#[^\n]*', '', text)


SNIPPETS = {}


def snippet_map(conf):
    """The files under /etc/nginx/snippets/ in an nginx -T dump: {'ai-proxy.conf': [lines]}."""
    out = {}
    for m in re.finditer(r'# configuration file /etc/nginx/snippets/([\w.\-]+):\n(.*?)(?=\n# configuration file |\Z)', conf, re.S):
        out[m.group(1)] = [' '.join(line.split('#', 1)[0].split()) for line in m.group(2).splitlines()]
    return {k: [x for x in v if x] for k, v in out.items()}


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
                lines = [line for line in lines if line]
                out = []
                for line in lines:
                    m = re.fullmatch(r'include snippets/([\w.\-]+);', line)
                    out.extend(SNIPPETS[m.group(1)] if m else [line])
                return sorted(out)
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
       '--read-only', '--tmpfs', '/tmp', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true',
       # none = trust nobody on purpose: the default (unset or empty) trusts the private ranges, which include the test
       # client's address (the Docker gateway), so every "untrusted peer" assertion below needs this
       '-e', 'XEXAM_TRUSTED_PROXIES=none', IMAGE)
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
    # (the generated files /tmp/xexam-realip.conf and /tmp/xexam-upstreams.conf are rendered by the start-up scripts, so run
    # every 1*-xexam-*.sh before nginx -T)
    r = sh('docker', 'run', '--rm', '--tmpfs', '/tmp', '--entrypoint', 'sh', IMAGE, '-c',
           'for f in /docker-entrypoint.d/1*-xexam-*.sh; do "$f" || exit 1; done && nginx -T', check=False)
    SNIPPETS.update(snippet_map(r.stdout))
    ok(set(SNIPPETS) >= {'ai-proxy.conf', 'api-proxy.conf'}, 'nginx -T lists the snippets api-proxy.conf and ai-proxy.conf. Found %r' % sorted(SNIPPETS))
    STREAM_SPEC = '~ ^/api/ai/(send|retry|regenerate|edit|study/ask|summary/ask|assess/ask)$'
    ADMIN_SPEC = '~ ^/api/ai/admin/(connections/save|connections/test|models/pick|models/test|enable)$'
    ai_prefix = location_block(r.stdout, '/api/ai/')
    ai_stream = location_block(r.stdout, STREAM_SPEC)
    ai_verify = location_block(r.stdout, ADMIN_SPEC)
    ok(None not in (ai_prefix, ai_stream, ai_verify),
       'nginx -T shows the locations /api/ai/, %s and %s' % (STREAM_SPEC, ADMIN_SPEC))
    ok('location = /api/ai/admin/@@verify' not in r.stdout, 'The old verify location is gone')
    # every proxied location includes its snippet (the snippet files are the only place of the header block)
    conf_text = r.stdout
    for spec, snip in (('/api/ai/', 'ai-proxy.conf'), ('= /api/ai/import/upload', 'ai-proxy.conf'), (STREAM_SPEC, 'ai-proxy.conf'),
                       (ADMIN_SPEC, 'ai-proxy.conf'), ('/api/', 'api-proxy.conf'), ('= /api/upload', 'api-proxy.conf'),
                       ('= /api/auth/login', 'api-proxy.conf')):
        i = conf_text.index('location %s {' % spec)
        body = conf_text[i:conf_text.index('\n        }', i)]
        ok('include snippets/%s;' % snip in body and 'proxy_set_header X-Real-IP' not in body,
           'Location %s includes snippets/%s and carries no header lines of its own' % (spec, snip))
    ok(all('proxy_set_header X-Real-IP $remote_addr;' in SNIPPETS[k] for k in SNIPPETS)
       and all('proxy_set_header %s $xexam_geo_%s;' % (h, h.split('-')[-1].lower()) in SNIPPETS[k]
               for k in ('api-proxy.conf', 'ai-proxy.conf') for h in ('X-Geo-Country', 'X-Geo-Region', 'X-Geo-City'))
       and not any('X-Geo' in line and '""' in line for k in SNIPPETS for line in SNIPPETS[k]),
       'Snippets: X-Real-IP in both, X-Geo-* set from the trusted-peer maps in both api-proxy.conf and ai-proxy.conf (never a client value)')
    ai_auth = conf_text[conf_text.index('location = /_ai_auth {'):]
    ai_auth = ai_auth[:ai_auth.index(chr(10) + ' ' * 8 + '}')]
    ai_auth_words = [' '.join(line.split()) for line in ai_auth.splitlines()]
    ok('proxy_set_header X-Real-IP $remote_addr;' in ai_auth_words
       and all('proxy_set_header %s $xexam_geo_%s;' % (h, h.split('-')[-1].lower()) in ai_auth_words for h in ('X-Geo-Country', 'X-Geo-Region', 'X-Geo-City')),
       '/_ai_auth overwrites X-Real-IP and X-Geo-* (a client cannot forge them on the auth step)')
    shared = ('auth_request', 'set $ai_upstream', 'proxy_pass', 'proxy_http_version', 'proxy_set_header', 'client_max_body_size',
              'limit_req zone=ai ', 'proxy_connect_timeout', 'proxy_request_buffering', 'proxy_buffering', 'proxy_intercept_errors',
              'error_page')
    same = lambda lines: [line for line in lines if line.startswith(shared)]
    ok(same(ai_prefix) == same(ai_stream) == same(ai_verify) and len(same(ai_prefix)) >= 20,
       'The three AI locations share their auth, proxy_set_header and error_page lines (%d lines)' % len(same(ai_prefix)))
    ok(all('set $ai_upstream $xexam_ai_upstream;' in b for b in (ai_prefix, ai_stream, ai_verify)),
       'The three AI locations proxy to the generated AI upstream (set $ai_upstream $xexam_ai_upstream;)')
    # v1.9: the upstreams and the resolver come from the generated /tmp/xexam-upstreams.conf; the defaults are the one-host names
    up_conf = generated_file(r.stdout, '/tmp/xexam-upstreams.conf')
    up_lines = [' '.join(x.split()) for x in up_conf.splitlines() if x.strip() and not x.strip().startswith('#')]
    ok(up_lines == ['resolver 127.0.0.11 valid=10s ipv6=off;',
                    'map $host $xexam_backend_upstream { default "http://backend:8765"; }',
                    'map $host $xexam_ai_upstream { default "http://ai-service:8766"; }'],
       'Default /tmp/xexam-upstreams.conf: Docker DNS resolver, backend:8765 and ai-service:8766, no proxy_ssl lines. Actual %r' % up_lines)
    own_conf = no_comments(r.stdout.replace(up_conf, ''))
    ok(not re.search(r'^\s*resolver\s', own_conf, re.M), 'nginx.conf itself has no resolver line any more (the generated file has the only one)')
    ok(r.stdout.count('set $backend_upstream $xexam_backend_upstream;') == 3
       and 'set $ai_auth_upstream $xexam_backend_upstream/internal/ai/auth;' in r.stdout
       and 'http://backend:8765' not in own_conf and 'http://ai-service:8766' not in own_conf,
       'The three backend locations and /_ai_auth use $xexam_backend_upstream; no fixed service address is left in nginx.conf')
    ok('connections/enabled' not in ADMIN_SPEC and 'connections/enabled' not in own_conf,
       'The v1.9 route /api/ai/admin/connections/enabled has no location of its own: the /api/ai/ prefix location serves it')
    # v2.6: GET /api/ai/assess/latest (the Home AI card) is a plain read under /api/ai/: it matches no streaming location and has
    # none of its own; only POST /api/ai/assess/ask streams. The v2.5 route /api/ai/assess/saved is gone from the config too
    stream_re = STREAM_SPEC[2:]
    ok(re.fullmatch(stream_re, '/api/ai/assess/ask') and not re.fullmatch(stream_re, '/api/ai/assess/latest')
       and 'assess/latest' not in own_conf and 'assess/saved' not in own_conf and 'assess/saved' not in STREAM_SPEC,
       'v2.6: assess/ask streams, assess/latest is a plain GET served by the /api/ai/ prefix location (no location of its own), assess/saved is gone')
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
    ok(timeouts == [['proxy_read_timeout 90s;'], ['proxy_read_timeout 600s;'], ['proxy_read_timeout 100s;']],
       'Read timeouts: /api/ai/ 90 s, the streaming answers 600 s, the admin connection / model routes 100 s. Actual %r' % timeouts)
    admin_re = ADMIN_SPEC[2:]
    ok(all(re.fullmatch(admin_re, p) for p in ('/api/ai/admin/connections/save', '/api/ai/admin/connections/test',
                                               '/api/ai/admin/models/pick', '/api/ai/admin/models/test', '/api/ai/admin/enable'))
       and not any(re.fullmatch(admin_re, p) for p in ('/api/ai/admin/@@verify', '/api/ai/admin/settings', '/api/ai/admin/models/list')),
       'The five admin routes (save, test, pick, models test, enable) match the long-timeout location; verify and the rest do not')

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
    for p in ('/badge-96.png', '/icon-maskable-192.png', '/icon-maskable-512.png'):
        st, h3, b3 = req('GET', p, host='localhost:%d' % PORT)
        ok(st == 200 and h3.get('content-type', '').split(';')[0].strip() == 'image/png' and b3[1:4] == b'PNG',
           '%s is served as image/png (v3.1). Actual %d %r' % (p, st, h3.get('content-type')))

    # -- v3.1: the service worker and the web app manifest (own locations, headers repeated) --
    st, h3, b3 = req('GET', '/sw.js', host='localhost:%d' % PORT)
    ok(st == 200 and len(b3) > 0, '/sw.js is served. Actual %d' % st)
    ok(h3.get('content-type', '').split(';')[0].strip() in ('application/javascript', 'text/javascript'),
       '/sw.js Content-Type is JavaScript. Actual %r' % h3.get('content-type'))
    ok(h3.get('cache-control') == 'no-cache', '/sw.js: Cache-Control: no-cache. Actual %r' % h3.get('cache-control'))
    for k, want in (('x-content-type-options', 'nosniff'), ('x-frame-options', 'DENY'), ('referrer-policy', 'no-referrer'),
                    ('permissions-policy', 'camera=(), microphone=(), geolocation=()')):
        ok(h3.get(k) == want, '/sw.js security header %s: %s. Actual %r' % (k, want, h3.get(k)))
    csp_sw = h3.get('content-security-policy', '')
    ok("default-src 'self'" in csp_sw and "script-src 'self'" in csp_sw and "frame-ancestors 'none'" in csp_sw,
       '/sw.js carries the strict CSP. Actual %r' % csp_sw[:80])
    st, h3, b3 = req('GET', '/manifest.webmanifest', host='localhost:%d' % PORT)
    ok(st == 200 and len(b3) > 0, '/manifest.webmanifest is served. Actual %d' % st)
    ok(h3.get('content-type', '').split(';')[0].strip() == 'application/manifest+json',
       '/manifest.webmanifest Content-Type is application/manifest+json. Actual %r' % h3.get('content-type'))
    ok(h3.get('cache-control') == 'no-cache', '/manifest.webmanifest: Cache-Control: no-cache. Actual %r' % h3.get('cache-control'))
    for k, want in (('x-content-type-options', 'nosniff'), ('x-frame-options', 'DENY'), ('referrer-policy', 'no-referrer')):
        ok(h3.get(k) == want, '/manifest.webmanifest security header %s: %s. Actual %r' % (k, want, h3.get(k)))
    for p in ('/sw.js/', '/sw.js.map', '/manifest.webmanifest/x', '/service-worker.js'):
        st, _, _ = req('GET', p, host='localhost:%d' % PORT)
        ok(st == 404, '%s is not served (404). Actual %d' % (p, st))
    st, h3, _ = req('GET', '/sw.js', host='localhost:%d' % PORT, headers={'Accept-Encoding': 'gzip'})
    ok(st == 200, '/sw.js with Accept-Encoding: gzip is served. Actual %d' % st)
    st, h3, _ = req('GET', '/manifest.webmanifest', host='localhost:%d' % PORT, headers={'Accept-Encoding': 'gzip'})
    ok(st == 200 and h3.get('content-type', '').split(';')[0].strip() == 'application/manifest+json',
       'The manifest keeps its media type with Accept-Encoding: gzip. Actual %d %r' % (st, h3.get('content-type')))

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
    for k in ('x-forwarded-for', 'x-forwarded-host', 'x-forwarded-proto', 'forwarded'):
        ok(k not in seen, 'Backend does not receive %s (Nginx cleared it)' % k)
    ok(seen.get('x-real-ip') and seen.get('x-real-ip') != '7.7.7.7',
       'Backend receives X-Real-IP = the connecting address, not the client-sent 7.7.7.7. Actual %r' % seen.get('x-real-ip'))
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
              'forwarded', 'x-geo-country', 'x-geo-region', 'x-geo-city'):
        ok(k not in sh_, 'The AI service does not receive %s' % k)
    ok(sh_.get('x-real-ip') and sh_.get('x-real-ip') != '203.0.113.9', 'The AI service receives X-Real-IP = the connecting address. Actual %r' % sh_.get('x-real-ip'))
    ok(sh_.get('x-xexam-identity') == 'id|GET|%s|-' % host,
       'The AI service receives only the identity of the auth step (client value and client X-Original-* ignored). Actual %r' % sh_.get('x-xexam-identity'))
    ok(sh_.get('host') == host and seen['path'] == '/api/ai/status', 'Host and path reach the AI service unchanged. Actual %r %r' % (sh_.get('host'), seen['path']))
    st, hh, b = req('GET', '/api/ai/chat?id=7', host=host, headers={'Cookie': 'xexam_session=good'})
    ok(st == 200 and json.loads(b)['path'] == '/api/ai/chat?id=7', 'The query string reaches the AI service. Actual %d' % st)

    # The admin's Test of the AI settings has its own location (a longer read timeout) with the same auth step and the
    # same header rules
    st, hh, b = req('POST', '/api/ai/admin/connections/test', host=host, headers={
        'Cookie': 'xexam_session=good; CF_Authorization=z', 'Authorization': 'Bearer x', 'Cf-Access-Jwt-Assertion': 'y',
        'X-Xexam-Identity': 'forged', 'X-Forwarded-For': '203.0.113.9', 'Forwarded': 'for=x', 'X-Real-IP': '203.0.113.9',
        'Origin': 'https://app.example.com', 'Content-Type': 'application/json'}, body=b'{}')
    ok(st == 200 and hh.get('x-echo-name') == 'ai-service',
       'POST /api/ai/admin/connections/test with a good session is answered by the AI service. Actual %d %r' % (st, hh.get('x-echo-name')))
    seen = json.loads(b)
    for k in ('cookie', 'authorization', 'cf-access-jwt-assertion', 'x-forwarded-for', 'x-forwarded-host', 'x-forwarded-proto',
              'forwarded'):
        ok(k not in seen['headers'], 'The AI service does not receive %s on the admin Test' % k)
    ok(seen['headers'].get('x-real-ip') not in (None, '203.0.113.9'), 'The admin Test carries X-Real-IP = the connecting address')
    ok(seen['headers'].get('x-xexam-identity') == 'id|POST|%s|https://app.example.com' % host and seen['body_len'] == 2,
       'The admin Test carries the identity of the auth step and its body. Actual %r' % seen['headers'].get('x-xexam-identity'))

    # A Test that answers only after 95 s (longer than the 90 s of /api/ai/; the AI service allows up to 2 min per
    # attempt and one retry) still gets its answer. It runs in the background while the other checks go on and is
    # collected before the AI service is stopped.
    slow = {}

    def slow_verify():
        t = time.time()
        try:
            slow['r'] = req('POST', '/api/ai/admin/connections/test?delay=95', host=host, timeout=200,
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
                                         ('xexam_session=name', 403, 'auth.name', 'name'),
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
        if code == 'auth.name':
            ok(j.get('error') == backend_constant('MSG_NAME_REQUIRED'), 'auth.name text equals MSG_NAME_REQUIRED')
        if code == 'auth.guard':
            ok('XEXAM_ALLOWED_HOSTS' in j.get('error', ''), 'auth.guard text names XEXAM_ALLOWED_HOSTS')
    st, hh, b = req('POST', '/api/ai/send', host=host, headers={'Cookie': 'xexam_session=pw', 'Content-Type': 'application/json'}, body=b'{}')
    ok(st == 403 and (jbody(b) or {}).get('code') == 'auth.password', 'The streaming route refuses the same way (403 auth.password). Actual %d' % st)
    for cookie, want_st, code in ((None, 401, 'auth.signin'), ('xexam_session=pw', 403, 'auth.password'),
                                  ('xexam_session=down', 503, 'ai.backendDown')):
        hdrs = {'Content-Type': 'application/json'}
        if cookie:
            hdrs['Cookie'] = cookie
        st, hh, b = req('POST', '/api/ai/admin/connections/test?probe=verify', host=host, headers=hdrs, body=b'{}')
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
    # The regenerate / edit streams and the study streams (hints / explanations, record summaries) use the streaming location too: no buffering, POST only
    for path in ('/api/ai/regenerate', '/api/ai/edit', '/api/ai/study/ask', '/api/ai/summary/ask', '/api/ai/assess/ask'):
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
    # branch and the chat model pick: the plain /api/ai/ location (answered at once, JSON, not a stream)
    for path in ('/api/ai/branch', '/api/ai/chat/model'):
        st, hh, b = req('POST', path, host=host, headers={'Cookie': 'xexam_session=good', 'Content-Type': 'application/json'}, body=b'{}')
        ok(st == 200 and 'application/json' in hh.get('content-type', '') and (jbody(b) or {}).get('path') == path,
           'POST %s uses the plain /api/ai/ location (JSON echo, no stream). Actual %d %r' % (path, st, hh.get('content-type')))
    st, hh, b = req('GET', '/api/ai/study/saved?mode=practice&key=00000000000000000000000000000000&source=practice&question=q1&kind=hint',
                    host=host, headers={'Cookie': 'xexam_session=good'})
    seen = json.loads(b) if st == 200 else {}
    ok(st == 200 and hh.get('x-echo-name') == 'ai-service' and seen.get('path', '').startswith('/api/ai/study/saved?mode=practice')
       and 'cookie' not in seen.get('headers', {}) and seen.get('headers', {}).get('x-xexam-identity') == 'id|GET|%s|-' % host,
       'GET /api/ai/study/saved reaches the AI service with the identity header and no cookie. Actual %d' % st)
    st, hh, b = req('POST', '/api/ai/memories/add', host=host, headers={'Cookie': 'xexam_session=good', 'Content-Type': 'application/json'},
                    body=b'{"content": "' + b'x' * (200 * 1024) + b'"}')
    ok(st == 200 and json.loads(b)['body_len'] > 200 * 1024, 'A 200 KiB body to another AI route passes. Actual %d' % st)
    st, hh, b = req('DELETE', '/api/ai/admin/connections/test', host=host, headers={'Cookie': 'xexam_session=good'})
    ok(st == 403 and (jbody(b) or {}).get('code') == 'ai.badRequest', 'DELETE /api/ai/admin/connections/test -> 403 JSON ai.badRequest. Actual %d' % st)
    st, hh, b = req('POST', '/api/ai/admin/connections/test', host=host, headers={'Cookie': 'xexam_session=good', 'Content-Type': 'application/json'},
                    body=b'x' * (300 * 1024))
    ok(st == 413 and (jbody(b) or {}).get('code') == 'ai.badRequest',
       '300 KiB to /api/ai/admin/connections/test -> 413 JSON ai.badRequest. Actual %d' % st)

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
    for path in ('/api/ai/send', '/api/ai/regenerate', '/api/ai/study/ask', '/api/ai/summary/ask', '/api/ai/assess/ask', '/api/ai/memories/suggest',
                 '/api/ai/admin/connections/test'):
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

    # -- v1.3: gzip, ETag / 304, real client address, location headers --
    big = '/api/info?pad=' + 'a' * 1500
    st, hh, b = req('GET', big, host='localhost', headers={'Accept-Encoding': 'gzip'})
    ok(st == 200 and hh.get('content-encoding') == 'gzip' and 'accept-encoding' in hh.get('vary', '').lower(),
       'gzip: a JSON answer over 1 KiB is compressed and Vary: Accept-Encoding is set. Actual %d %r %r' % (st, hh.get('content-encoding'), hh.get('vary')))
    st, hh, b = req('GET', '/api/info', host='localhost', headers={'Accept-Encoding': 'gzip'})
    ok(st == 200 and 'content-encoding' not in hh, 'gzip: a JSON answer under 1 KiB stays plain')
    t0 = time.time()
    c = http.client.HTTPConnection('127.0.0.1', PORT, timeout=15)
    c.putrequest('POST', '/api/ai/send', skip_host=True, skip_accept_encoding=True)
    for k, v in (('Host', host), ('Cookie', 'xexam_session=good-gz'), ('Accept-Encoding', 'gzip'), ('Content-Type', 'application/json'), ('Content-Length', '2')):
        c.putheader(k, v)
    c.endheaders(b'{}')
    r = c.getresponse()
    first_chunk = r.read1(65536)
    ok(r.status == 200 and r.getheader('Content-Encoding') is None and b'event: ' in first_chunk and time.time() - t0 < 1.5,
       'gzip: the event stream is never compressed and is not held back. Actual %d %r' % (r.status, r.getheader('Content-Encoding')))
    r.read()
    c.close()
    st, hh, b = req('GET', '/api/etag', host='localhost')
    ok(st == 200 and hh.get('etag') == '"v1"', 'ETag from the backend passes through /api/. Actual %d %r' % (st, hh.get('etag')))
    st, hh, b = req('GET', '/api/etag', host='localhost', headers={'If-None-Match': '"v1"'})
    ok(st == 304 and hh.get('etag') == '"v1"' and b == b'', 'If-None-Match round trip: 304 with the ETag and no body. Actual %d %r' % (st, b[:40]))

    # Real client address. The main container (XEXAM_TRUSTED_PROXIES=none) trusts nobody: a forged CF-Connecting-IP
    # and forged location headers never reach a service, X-Real-IP is the connecting address. More containers: the default
    # (variable unset), one that trusts exactly that connecting address, a narrow value that does not contain it, and
    # invalid settings (fail closed with a warning).
    forged = {'CF-Connecting-IP': '198.51.100.7', 'CF-IPCountry': 'XX', 'CF-Region': 'Made-up Region', 'CF-IPCity': 'Made-up City',
              'X-Geo-Country': 'ZZ', 'X-Geo-City': 'Forged City', 'X-Real-IP': '203.0.113.9', 'X-Forwarded-For': '203.0.113.8'}
    st, hh, b = req('GET', '/api/info', host='localhost', headers=forged)
    seen = json.loads(b)['headers']
    peer = seen.get('x-real-ip')
    ok(peer and peer not in ('198.51.100.7', '203.0.113.9', '203.0.113.8'),
       'Untrusted peer with a forged CF-Connecting-IP / X-Real-IP: the backend sees the connecting address %r' % peer)
    ok(not any(k in seen for k in ('x-geo-country', 'x-geo-region', 'x-geo-city', 'x-forwarded-for')),
       'Untrusted peer: location headers (CF-* and X-Geo-*) are not forwarded as X-Geo-* to the backend')
    st, hh, b = req('GET', '/api/ai/status', host=host, headers=dict(forged, Cookie='xexam_session=good'))
    seen = json.loads(b)['headers']
    ok(seen.get('x-real-ip') == peer and not any(k in seen for k in ('x-geo-country', 'x-geo-region', 'x-geo-city')),
       'Untrusted peer: the AI service sees the connecting address and no location headers. Actual %d %r' % (st, {k: v for k, v in seen.items() if k.startswith(('x-geo', 'x-real'))}))
    st, hh, b = req('GET', '/api/info', host='localhost', headers={'X-Geo-Country': 'ZZ', 'X-Geo-Region': 'R', 'X-Geo-City': 'C'})
    ok(not any(k in json.loads(b)['headers'] for k in ('x-geo-country', 'x-geo-region', 'x-geo-city')), 'Client-sent X-Geo-* headers never reach the backend')

    def start_web(name, port, env):
        args = ['docker', 'run', '-d', '--name', name, '--network', NET, '-p', '127.0.0.1:%d:8080' % port,
                '--read-only', '--tmpfs', '/tmp', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true']
        for k, v in env.items():
            args += ['-e', '%s=%s' % (k, v)]
        sh(*args, IMAGE)
        for _ in range(40):
            try:
                http.client.HTTPConnection('127.0.0.1', port, timeout=3).request('GET', '/healthz')
                break
            except Exception:
                time.sleep(0.5)
        time.sleep(1)

    def req_port(port, path, headers):
        c = http.client.HTTPConnection('127.0.0.1', port, timeout=15)
        c.putrequest('GET', path, skip_host=True, skip_accept_encoding=True)
        for k, v in dict(headers, Host='localhost').items():
            c.putheader(k, v)
        c.endheaders()
        r = c.getresponse()
        data = r.read()
        c.close()
        return r.status, json.loads(data)['headers'] if r.status == 200 else {}

    start_web(WEB2, PORT + 1, {'XEXAM_TRUSTED_PROXIES': peer})
    st, seen = req_port(PORT + 1, '/api/info', forged)
    ok(st == 200 and seen.get('x-real-ip') == '198.51.100.7', 'Trusted peer: X-Real-IP = the CF-Connecting-IP value. Actual %r' % seen.get('x-real-ip'))
    ok(seen.get('x-geo-country') == 'XX' and seen.get('x-geo-region') == 'Made-up Region' and seen.get('x-geo-city') == 'Made-up City',
       'Trusted peer: CF-IPCountry / CF-Region / CF-IPCity arrive as X-Geo-Country / X-Geo-Region / X-Geo-City. Actual %r' % {k: v for k, v in seen.items() if k.startswith('x-geo')})
    ok('x-forwarded-for' not in seen, 'Trusted peer: X-Forwarded-For stays blanked')
    st, seen = req_port(PORT + 1, '/api/info', {'X-Geo-City': 'Forged City', 'X-Geo-Country': 'ZZ'})
    ok(st == 200 and seen.get('x-real-ip') == peer and 'x-geo-city' not in seen and 'x-geo-country' not in seen,
       'Trusted peer without CF headers: a client-sent X-Geo-* is overwritten (empty), X-Real-IP = the peer. Actual %r' % {k: v for k, v in seen.items() if k.startswith('x-')})
    st, seen = req_port(PORT + 1, '/api/ai/status', dict(forged, Cookie='xexam_session=good'))
    ok(st == 200 and seen.get('x-real-ip') == '198.51.100.7' and seen.get('x-geo-country') == 'XX'
       and seen.get('x-geo-region') == 'Made-up Region' and seen.get('x-geo-city') == 'Made-up City',
       'Trusted peer: the AI service gets the real address and the Cloudflare place. Actual %r' % {k: v for k, v in seen.items() if k.startswith(('x-geo', 'x-real'))})
    st, seen = req_port(PORT + 1, '/api/ai/status', {'X-Geo-City': 'Forged City', 'X-Geo-Country': 'ZZ', 'Cookie': 'xexam_session=good'})
    ok(st == 200 and not any(k in seen for k in ('x-geo-country', 'x-geo-region', 'x-geo-city')),
       'Trusted peer without CF headers: a client-sent X-Geo-* never reaches the AI service')
    sh('docker', 'rm', '-f', WEB2)
    # v1.4: the default (variable unset) trusts the private networks. The test client reaches the published port through the
    # Docker gateway, a private address, so it is believed; a narrow explicit value that does not contain it is not.
    ok(any(ipaddress.ip_address(peer) in ipaddress.ip_network(n) for n in ('10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16')),
       'The test client reaches Nginx from a private address (%s), so the default-trust checks below are meaningful' % peer)
    start_web(WEB2, PORT + 1, {})
    st, seen = req_port(PORT + 1, '/api/info', forged)
    ok(st == 200 and seen.get('x-real-ip') == '198.51.100.7' and seen.get('x-geo-country') == 'XX',
       'Default (XEXAM_TRUSTED_PROXIES unset): a peer in a private range is believed (CF-Connecting-IP and location headers). Actual %r' % seen.get('x-real-ip'))
    conf = sh('docker', 'exec', WEB2, 'cat', '/tmp/xexam-realip.conf').stdout
    ok(re.findall(r'set_real_ip_from (\S+);', conf) == ['10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16'],
       'Default: exactly 10.0.0.0/8, 172.16.0.0/12 and 192.168.0.0/16 are trusted (public ranges are not). Actual %r' % re.findall(r'set_real_ip_from (\S+);', conf))
    sh('docker', 'rm', '-f', WEB2)
    # an empty value (a blank deployment-form field) is the default too, not "trust nobody"
    start_web(WEB2, PORT + 1, {'XEXAM_TRUSTED_PROXIES': ''})
    st, seen = req_port(PORT + 1, '/api/info', forged)
    ok(st == 200 and seen.get('x-real-ip') == '198.51.100.7' and seen.get('x-geo-country') == 'XX',
       'Empty XEXAM_TRUSTED_PROXIES: the default applies (a private peer is believed). Actual %r' % seen.get('x-real-ip'))
    sh('docker', 'rm', '-f', WEB2)
    start_web(WEB2, PORT + 1, {'XEXAM_TRUSTED_PROXIES': '203.0.113.1'})
    st, seen = req_port(PORT + 1, '/api/info', forged)
    ok(st == 200 and seen.get('x-real-ip') == peer and 'x-geo-country' not in seen,
       'Explicit narrow value (203.0.113.1): a peer outside it is not believed even though it is private. Actual %r' % seen.get('x-real-ip'))
    sh('docker', 'rm', '-f', WEB2)
    start_web(WEB2, PORT + 1, {'XEXAM_TRUSTED_PROXIES': '10.1.1.*，%s' % peer})
    st, seen = req_port(PORT + 1, '/api/info', forged)
    ok(st == 200 and seen.get('x-real-ip') == '198.51.100.7',
       'A bad entry (10.1.1.*) next to a good one (the peer, after a full-width comma): the good one still works. Actual %r' % seen.get('x-real-ip'))
    sh('docker', 'rm', '-f', WEB2)
    start_web(WEB2, PORT + 1, {'XEXAM_TRUSTED_PROXIES': '0.0.0.0/0,%s' % peer})
    st, seen = req_port(PORT + 1, '/api/info', forged)
    ok(st == 200 and seen.get('x-real-ip') == '198.51.100.7',
       'A refused /0 next to a good entry (the peer): the good one still works. Actual %r' % seen.get('x-real-ip'))
    sh('docker', 'rm', '-f', WEB2)

    # The value parser itself, run in the real image (busybox sed / grep / tr): the rendered file lists the accepted
    # entries; stderr names every skipped one.
    def parse(value):
        env = ['-e', 'XEXAM_TRUSTED_PROXIES=' + value] if value is not None else []
        r = sh('docker', 'run', '--rm', *env, '-e', 'XEXAM_REALIP_OUT=/tmp/o.conf', '--entrypoint', 'sh', IMAGE, '-c',
               '/docker-entrypoint.d/15-xexam-realip.sh; cat /tmp/o.conf')
        return re.findall(r'set_real_ip_from (\S+);', r.stdout), r.stderr

    def parses(label, value, want, skipped=()):
        got, err = parse(value)
        ok(got == want and all(("entry '%s' skipped" % x) in err for x in skipped) and err.count('skipped') == len(skipped),
           'Parser, %s: trusts %r, skips %r with a warning. Actual %r, stderr %r' % (label, want, list(skipped), got, err.strip()))

    dflt = ['10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16']
    parses('unset', None, dflt)
    parses('empty', '', dflt)
    parses('empty ASCII quotes', '""', dflt)
    parses('empty curly quotes', '“”', dflt)
    parses('empty single quotes', "''", dflt)
    parses('only spaces', '  ', dflt)
    parses('none', 'none', [])
    parses('NONE', 'NONE', [])
    parses('quoted none', '"none"', [])
    parses('none with spaces', '  None ', [])
    parses('none mixed with an address: the word is skipped, the address works', 'none, 10.1.1.1', ['10.1.1.1/32'], ['none'])
    parses('quotes around the whole value', '"10.1.1.1"', ['10.1.1.1/32'])
    parses('quotes around entries', '"10.1.1.1","10.2.2.2"', ['10.1.1.1/32', '10.2.2.2/32'])
    got, err = parse('none')
    ok('trusting no proxy' in err and 'skipped' not in err, 'Parser: none trusts nobody with an info line, no warning. stderr %r' % err.strip())
    got, err = parse('')
    ok(err.strip() == '', 'Parser: an empty value gets the default without any message. stderr %r' % err.strip())
    got, err = parse('""')
    ok('quote characters were stripped' in err, 'Parser: stripped quotes give one info line. stderr %r' % err.strip())
    parses('narrow', '172.31.254.2', ['172.31.254.2/32'])
    parses('ASCII comma and spaces', '10.0.0.0/8, 10.1.1.1  192.0.2.5', ['10.0.0.0/8', '10.1.1.1/32', '192.0.2.5/32'])
    parses('full-width comma', '10.0.0.0/8，10.1.1.1', ['10.0.0.0/8', '10.1.1.1/32'])
    parses('ideographic comma', '10.0.0.0/8、10.1.1.1', ['10.0.0.0/8', '10.1.1.1/32'])
    parses('semicolons (ASCII and full-width)', '10.0.0.0/8;10.1.1.1；192.0.2.5', ['10.0.0.0/8', '10.1.1.1/32', '192.0.2.5/32'])
    parses('tabs and newlines', '10.0.0.0/8\t10.1.1.1\n192.0.2.5', ['10.0.0.0/8', '10.1.1.1/32', '192.0.2.5/32'])
    parses('IPv6 address and CIDR', 'fd00::/8,2001:db8::1', ['fd00::/8', '2001:db8::1/128'])
    parses('a bare 10.0.0.0 is that one address', '10.0.0.0', ['10.0.0.0/32'])
    parses('host bits are zeroed', '10.1.1.1/8', ['10.0.0.0/8'])
    parses('wildcard skipped, good kept', '10.1.1.*,10.0.0.0/8', ['10.0.0.0/8'], ['10.1.1.*'])
    parses('short forms skipped', '1 10.1 10.1.1 10.0.0.0/8', ['10.0.0.0/8'], ['1', '10.1', '10.1.1'])
    parses('out of range and text skipped', '300.1.1.1,text,10.0.0.0/33,10.0.0.0/8', ['10.0.0.0/8'], ['300.1.1.1', 'text', '10.0.0.0/33'])
    parses('full-width digits skipped', '１０.0.0.1,10.0.0.0/8', ['10.0.0.0/8'], ['１０.0.0.1'])
    parses('/0, 0.0.0.0 and :: refused', '0.0.0.0/0,::/0,0.0.0.0,::,10.0.0.0/8', ['10.0.0.0/8'], ['0.0.0.0/0', '::/0', '0.0.0.0', '::'])
    parses('all entries bad', '1,10.1.*,0.0.0.0/0', [], ['1', '10.1.*', '0.0.0.0/0'])
    got, err = parse('1,10.1.*')
    ok('trusting no proxy' in err, 'Parser: no valid entry left warns that nobody is trusted. stderr %r' % err.strip())

    start_web(WEB2, PORT + 1, {'XEXAM_TRUSTED_PROXIES': peer, 'XEXAM_REAL_IP_HEADER': 'X-Forwarded-For'})
    st, seen = req_port(PORT + 1, '/api/info', {'X-Forwarded-For': '198.51.100.20, 198.51.100.21'})
    ok(st == 200 and seen.get('x-real-ip') == '198.51.100.21', 'XEXAM_REAL_IP_HEADER=X-Forwarded-For: the last untrusted hop. Actual %r' % seen.get('x-real-ip'))
    sh('docker', 'rm', '-f', WEB2)
    for bad in ('0.0.0.0/0', '::/0', 'not-an-address', '10.1.1.*'):
        sh('docker', 'rm', '-f', WEB3, check=False)
        start_web(WEB3, PORT + 2, {'XEXAM_TRUSTED_PROXIES': bad})
        st, seen = req_port(PORT + 2, '/api/info', forged)
        warn = sh('docker', 'logs', WEB3, check=False)
        ok(st == 200 and seen.get('x-real-ip') == peer and 'x-geo-country' not in seen,
           'Only invalid entries in XEXAM_TRUSTED_PROXIES=%s: nobody is trusted (the forged address is ignored, no location headers). Actual %r' % (bad, seen.get('x-real-ip')))
        ok('xexam-realip' in warn.stderr and 'trusting no proxy' in warn.stderr, 'Only invalid entries in XEXAM_TRUSTED_PROXIES=%s: an English warning on stderr' % bad)
    sh('docker', 'rm', '-f', WEB3, check=False)
    sh('docker', 'run', '--name', WEB3, '--network', NET, '-d', '-e', 'XEXAM_REAL_IP_HEADER=Made-Up', '--read-only', '--tmpfs', '/tmp', IMAGE)
    time.sleep(2)
    ok('XEXAM_REAL_IP_HEADER' in sh('docker', 'logs', WEB3, check=False).stderr, 'An invalid XEXAM_REAL_IP_HEADER also warns and trusts nobody')
    sh('docker', 'rm', '-f', WEB3, check=False)

    # -- v1.9 upstreams: XEXAM_BACKEND_UPSTREAM / XEXAM_AI_UPSTREAM / XEXAM_NGINX_RESOLVER (16-xexam-upstreams.sh) --
    # Two more fakes (other names and ports, tagged with X-Echo-Tag) stand for services on other machines.
    sh('docker', 'run', '-d', '--name', BE_OTHER, '--network', NET, '--network-alias', 'be-other',
       '-e', 'ECHO_NAME=backend', '-e', 'ECHO_PORT=9765', '-e', 'ECHO_TAG=be-other', 'python:3.12-slim', 'python', '-u', '-c', ECHO)
    sh('docker', 'run', '-d', '--name', AI_OTHER, '--network', NET, '--network-alias', 'ai-other',
       '-e', 'ECHO_NAME=ai-service', '-e', 'ECHO_PORT=9766', '-e', 'ECHO_TAG=ai-other', 'python:3.12-slim', 'python', '-u', '-c', ECHO)

    def ip_of(name):
        fmt = '{{(index .NetworkSettings.Networks "%s").IPAddress}}' % NET
        return sh('docker', 'inspect', '-f', fmt, name).stdout.strip()

    def routes_to_other(port, label):
        """GET /api/info and a signed-in GET /api/ai/status through the Nginx on `port`: the answers must come from the
        other fakes, the AI auth step must go to the other backend, and the original fakes must see nothing."""
        orig_before = (echo_log(BACK).count('GET /api/info'), echo_log(AI).count('GET /api/ai/status'), echo_log(BACK).count('GET /internal/ai/auth'))
        auth_before = echo_log(BE_OTHER).count('GET /internal/ai/auth')
        st, hh, b = req('GET', '/api/info', host='localhost', port=port)
        ok(st == 200 and hh.get('x-echo-name') == 'backend' and hh.get('x-echo-tag') == 'be-other',
           '%s: /api/ in the other backend (be-other). Actual %d %r %r' % (label, st, hh.get('x-echo-name'), hh.get('x-echo-tag')))
        st, hh, b = req('GET', '/api/ai/status', host='localhost:%d' % port, headers={'Cookie': 'xexam_session=good'}, port=port)
        ok(st == 200 and hh.get('x-echo-name') == 'ai-service' and hh.get('x-echo-tag') == 'ai-other',
           '%s: /api/ai/ in the other AI service (ai-other). Actual %d %r %r' % (label, st, hh.get('x-echo-name'), hh.get('x-echo-tag')))
        ok(echo_log(BE_OTHER).count('GET /internal/ai/auth') == auth_before + 1,
           '%s: the AI auth step (/_ai_auth) goes to the other backend' % label)
        ok((echo_log(BACK).count('GET /api/info'), echo_log(AI).count('GET /api/ai/status'), echo_log(BACK).count('GET /internal/ai/auth')) == orig_before,
           '%s: the original fake backend and AI service saw none of these requests' % label)

    be_ip, ai_ip = ip_of(BE_OTHER), ip_of(AI_OTHER)
    # by name (a trailing slash is removed), with an explicit resolver of the Docker DNS
    start_web(WEB2, PORT + 1, {'XEXAM_TRUSTED_PROXIES': 'none', 'XEXAM_BACKEND_UPSTREAM': 'http://be-other:9765/',
                               'XEXAM_AI_UPSTREAM': 'http://ai-other:9766', 'XEXAM_NGINX_RESOLVER': '127.0.0.11:53'})
    routes_to_other(PORT + 1, 'Upstreams by name')
    conf = sh('docker', 'exec', WEB2, 'cat', '/tmp/xexam-upstreams.conf').stdout
    ok('resolver 127.0.0.11:53 valid=10s ipv6=off;' in conf and 'default "http://be-other:9765";' in conf and 'proxy_ssl' not in conf,
       'Upstreams by name: the generated file holds the resolver with its port and the two upstreams (no trailing slash, no proxy_ssl). Actual %r' % conf)
    sh('docker', 'rm', '-f', WEB2)
    # by IP literal: no DNS needed at all (the resolver points at a closed port on purpose)
    start_web(WEB3, PORT + 2, {'XEXAM_TRUSTED_PROXIES': 'none', 'XEXAM_BACKEND_UPSTREAM': 'http://%s:9765' % be_ip,
                               'XEXAM_AI_UPSTREAM': 'http://%s:9766' % ai_ip, 'XEXAM_NGINX_RESOLVER': '127.0.0.1:5353'})
    routes_to_other(PORT + 2, 'Upstreams by IP literal')
    sh('docker', 'rm', '-f', WEB3)

    # The value parser, run in the real image (busybox sed / grep / tr). The output file is pre-filled so that "nothing is
    # left behind after a refusal" is visible.
    def upstreams(env):
        args = ['docker', 'run', '--rm', '--tmpfs', '/tmp', '-e', 'XEXAM_UPSTREAMS_OUT=/tmp/o.conf']
        for k, v in env.items():
            args += ['-e', '%s=%s' % (k, v)]
        r = sh(*args, '--entrypoint', 'sh', IMAGE, '-c',
               'echo old > /tmp/o.conf; /docker-entrypoint.d/16-xexam-upstreams.sh; echo "rc=$?" >&2; cat /tmp/o.conf 2>/dev/null; true')
        rc = int(re.search(r'rc=(\d+)', r.stderr).group(1))
        return rc, r.stdout, r.stderr

    def up_good(label, env, backend=None, ai=None, resolver=None, tls=False):
        rc, out, err = upstreams(env)
        want_b = backend or 'http://backend:8765'
        want_a = ai or 'http://ai-service:8766'
        want_r = resolver or '127.0.0.11'
        ok(rc == 0 and 'resolver %s valid=10s ipv6=off;' % want_r in out
           and 'map $host $xexam_backend_upstream { default "%s"; }' % want_b in out
           and 'map $host $xexam_ai_upstream { default "%s"; }' % want_a in out
           and ('proxy_ssl_verify on;' in out) == tls and ('proxy_ssl_server_name on;' in out) == tls
           and ('proxy_ssl_trusted_certificate /etc/ssl/certs/ca-certificates.crt;' in out) == tls,
           'Upstream parser, %s: accepted as backend %s, ai %s, resolver %s%s. Actual rc=%d %r' % (
               label, want_b, want_a, want_r, ', verified TLS' if tls else ', no TLS settings', rc, out))

    def up_bad(label, env, name):
        rc, out, err = upstreams(env)
        ok(rc == 1 and out.strip() == '' and ('xexam-upstreams: %s must be ' % name) in err and "(got '" in err,
           'Upstream parser, %s: refused (exit 1, the old output file removed, a message naming %s). Actual rc=%d out=%r err=%r' % (
               label, name, rc, out, err.strip()))

    B, A, R = 'XEXAM_BACKEND_UPSTREAM', 'XEXAM_AI_UPSTREAM', 'XEXAM_NGINX_RESOLVER'
    up_good('nothing set', {})
    up_good('empty values', {B: '', A: '', R: ''})
    up_good('quotes and spaces only', {B: '""', A: '  ', R: "''"})
    up_good('host name with port', {B: 'http://be.example.test:9765', A: 'http://ai.example.test:9766'}, 'http://be.example.test:9765', 'http://ai.example.test:9766')
    up_good('trailing slash removed, quotes stripped', {B: '"http://10.0.0.2:8765/"', A: "'http://10.0.0.3:8766/'"}, 'http://10.0.0.2:8765', 'http://10.0.0.3:8766')
    up_good('no port', {B: 'http://10.0.0.2', A: 'http://ai.example.test'}, 'http://10.0.0.2', 'http://ai.example.test')
    up_good('IPv6 literal', {B: 'http://[fd00::2]:8765', A: 'http://[fd00::3]'}, 'http://[fd00::2]:8765', 'http://[fd00::3]')
    up_good('https verifies the certificate', {B: 'https://be.example.test:8443'}, backend='https://be.example.test:8443', tls=True)
    up_good('one https is enough for the TLS settings', {A: 'https://ai.example.test'}, ai='https://ai.example.test', tls=True)
    up_good('port 65535', {B: 'http://10.0.0.2:65535'}, 'http://10.0.0.2:65535')
    up_good('resolver IPv4 with port', {R: '10.0.0.53:5353'}, resolver='10.0.0.53:5353')
    up_good('resolver list (comma and space, IPv6 plain and bracketed)', {R: '10.0.0.53, fd00::53 [fd00::54]:53'}, resolver='10.0.0.53 [fd00::53] [fd00::54]:53')
    for label, v in (('a path', 'http://10.0.0.2:8765/api'), ('a query', 'http://10.0.0.2:8765?x=1'), ('a fragment', 'http://10.0.0.2:8765#x'),
                     ('user info', 'http://user@10.0.0.2:8765'), ('a second slash', 'http://10.0.0.2:8765//'),
                     ('no scheme', '10.0.0.2:8765'), ('ftp', 'ftp://10.0.0.2:8765'), ('an empty host', 'http://:8765'),
                     ('a semicolon', 'http://a;b:8765'), ('a closing brace', 'http://a}b'), ('a dollar sign', 'http://a$b'),
                     ('a double quote inside', 'http://a"b'), ('a space inside', 'http://a b'), ('an underscore', 'http://a_b:8765'),
                     ('port 0', 'http://10.0.0.2:0'), ('port 65536', 'http://10.0.0.2:65536'), ('a long port', 'http://10.0.0.2:123456'),
                     ('a text port', 'http://10.0.0.2:http'), ('a leading dot', 'http://.example.test'),
                     ('a trailing dash', 'http://example-.test-'), ('a bare IPv6', 'http://fd00::2:8765'), ('upper-case scheme only text', 'HTTP2://x')):
        up_bad(label, {B: v}, B)
        up_bad(label + ' (AI)', {A: v}, A)
    up_bad('a long host name', {B: 'http://%s.example.test' % ('a' * 260)}, B)
    for label, v in (('a host name', 'ns.example.test'), ('an octet above 255', '10.0.0.256'), ('a path', '10.0.0.53/x'),
                     ('a bracket without an address', '[]'), ('a bad port', '10.0.0.53:99999'), ('a semicolon', '10.0.0.53;10.0.0.54'),
                     ('a brace', '10.0.0.53}'), ('a bad IPv6', 'fd00:::1'), ('a port on a bare IPv6', 'fd00::1:53x')):
        up_bad('resolver ' + label, {R: v}, R)
    rc, out, err = upstreams({B: 'http://10.0.0.2:8765/x' + 'y' * 300})
    got = re.search(r"\(got '([^']*)'\)", err)
    ok(rc == 1 and got and len(got.group(1)) == 200, 'A refused value is cut to 200 characters in the message. Actual %r' % (len(got.group(1)) if got else None))

    # A container with a bad value stops with that message (the entrypoint stops at a failing script) and does not serve
    sh('docker', 'rm', '-f', WEB3, check=False)
    sh('docker', 'run', '-d', '--name', WEB3, '--network', NET, '--read-only', '--tmpfs', '/tmp', '--cap-drop', 'ALL',
       '-e', 'XEXAM_BACKEND_UPSTREAM=http://10.0.0.2:8765/not-a-path', IMAGE)
    code = sh('docker', 'wait', WEB3).stdout.strip()
    logs = sh('docker', 'logs', WEB3, check=False)
    ok(code not in ('', '0') and 'XEXAM_BACKEND_UPSTREAM must be http(s)://host[:port] with no path, for example http://10.0.0.2:8765' in logs.stderr
       and "(got 'http://10.0.0.2:8765/not-a-path')" in logs.stderr,
       'A container with a bad XEXAM_BACKEND_UPSTREAM exits non-zero with the message. Actual exit %r, stderr %r' % (code, logs.stderr[-300:]))
    sh('docker', 'rm', '-f', WEB3, check=False)
    sh('docker', 'rm', '-f', BE_OTHER, AI_OTHER, check=False)

    # -- Upload size --
    st, hh, b = req('POST', '/api/upload', host='localhost', headers={'Content-Type': 'application/octet-stream', 'X-Filename': 'a.xlsx'}, body=b'x' * (5 * 1024 * 1024))
    ok(st == 200 and json.loads(b)['body_len'] == 5 * 1024 * 1024, '5 MB upload passes (and arrives at the backend intact). Actual %d' % st)
    st, hh, b = req('POST', '/api/upload', host='localhost', headers={'Content-Type': 'application/octet-stream', 'X-Filename': 'a.xlsx'}, body=b'x' * (20 * 1024 * 1024 + 1))
    ok(st == 413, 'Over 20 MB is rejected by Nginx (413). Actual %d' % st)
    ok('json' in hh.get('content-type', ''), '413 is JSON, not HTML. Actual %r' % hh.get('content-type'))
    ok('文件太大了' in json.loads(b)['error'], '413 carries the same Chinese message as the backend')
    ok(json.loads(b).get('code') == 'net.tooLarge', '413 carries the code net.tooLarge. Actual %r' % json.loads(b).get('code'))

    # -- Rate limiting --
    codes = [req('POST', '/api/upload', host='localhost', headers={'Content-Type': 'application/octet-stream', 'X-Filename': 'a.xlsx'}, body=b'x')[0] for _ in range(12)]
    ok(429 in codes, 'Upload rate limit: 429 appears after 12 rapid requests. Actual %r' % sorted(set(codes)))
    time.sleep(6)
    # Sign-in: 20 requests a minute per address with a burst of 20 (the bucket empties slowly, nothing else uses it)
    codes = [req('POST', '/api/auth/login', host='localhost', headers={'Content-Type': 'application/json'}, body=b'{}')[0] for _ in range(40)]
    ok(codes[:20].count(200) == 20 and 429 in codes and set(codes) <= {200, 429},
       'Login zone: the burst of 20 passes, then 429. Actual %d x 200, %d x 429' % (codes.count(200), codes.count(429)))
    st, hh, b = req('GET', '/api/info', host='localhost')
    ok(st == 200, 'The login zone does not limit other API routes. Actual %d' % st)
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
       'An admin Test answered after %.0f s (more than the 90 s of /api/ai/, under the 100 s limit) reaches the admin. Actual %d %r' % (slow.get('t', 0), st, b[:120]))

    # -- AI service down: JSON 502, the rest of the app unaffected --
    sh('docker', 'stop', AI)
    time.sleep(1)
    st, hh, b = req('GET', '/api/ai/status', host=host, headers={'Cookie': 'xexam_session=good'})
    ok(st == 502 and (jbody(b) or {}).get('code') == 'ai.unavailable', 'AI service stopped: GET /api/ai/status -> 502 JSON ai.unavailable. Actual %d %r' % (st, b[:120].decode('utf-8', 'replace')))
    st, hh, b = req('POST', '/api/ai/send', host=host, headers={'Cookie': 'xexam_session=good-c', 'Content-Type': 'application/json'}, body=b'{}')
    ok(st == 502 and (jbody(b) or {}).get('code') == 'ai.unavailable', 'AI service stopped: POST /api/ai/send -> 502 JSON ai.unavailable. Actual %d' % st)
    st, hh, b = req('GET', '/api/info', host='localhost')
    ok(st == 200 and hh.get('x-echo-name') == 'backend', 'AI service stopped: GET /api/info still 200 from the backend. Actual %d' % st)
    st, hh, b = req('POST', '/api/ai/admin/connections/test', host=host, headers={'Cookie': 'xexam_session=good', 'Content-Type': 'application/json'},
                    body=b'{}')
    ok(st == 502 and (jbody(b) or {}).get('code') == 'ai.unavailable',
       'AI service stopped: POST /api/ai/admin/connections/test -> 502 JSON ai.unavailable. Actual %d' % st)

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
    for k in ('cookie', 'authorization', 'x-forwarded-for', 'forwarded'):
        ok(k not in (seen.get('headers') or {}), 'The AI service does not receive %s on the upload route' % k)
    ok((seen.get('headers') or {}).get('x-real-ip') not in (None, '203.0.113.9'), 'The upload route carries X-Real-IP = the connecting address')
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
    ok(json.loads(b).get('code') == 'net.backendDown', '502 carries the code net.backendDown. Actual %r' % json.loads(b).get('code'))
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
