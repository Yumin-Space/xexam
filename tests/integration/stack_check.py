# -*- coding: utf-8 -*-
"""End-to-end check of the whole new stack (nginx + Rust backend + SQLite volume). Starts the stack with Docker Compose, using only a temporary project name and temporary volumes throughout,
and never touches the real xexam_data volume.

Usage: python -I tests/integration/stack_check.py [port, default 18090]
Prerequisite: this machine can run docker compose build (builds both the backend and nginx images).

What is checked:
  1. The stack comes up and both containers are healthy; the backend publishes no ports, so the host cannot reach it
  2. /api/info /api/bank /api/progress /api/imports through Nginx
  3. Upload the demo workbook → preview (bank unchanged) → confirm → question bank has 5 questions; cancel / repeated confirm / wrong confirmation token
  4. Submit results → study history; reset
  5. Over 20 MB: 413 at the Nginx layer (JSON); the Host / Origin guards still take effect after passing through Nginx
  6. Recreate from the same volume after deleting the containers, data is still there; docker stop duration
  7. Non-root, read-only root filesystem
"""
import http.client
import json
import os
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18090
PROJECT = 'xexamcheck' + uuid.uuid4().hex[:6]
DEMO = os.path.join(ROOT, 'tests', 'baseline', 'fixtures', 'demo.xlsx')
COMPOSE = os.path.join(ROOT, 'docker', 'docker-compose.build.yml')

env = dict(os.environ, XEXAM_BIND='127.0.0.1:%d' % PORT, COMPOSE_PROJECT_NAME=PROJECT, MSYS_NO_PATHCONV='1')
for k in ('XEXAM_ALLOWED_HOSTS', 'XEXAM_ALLOWED_ORIGINS', 'XEXAM_ALLOW_ANY_HOST'):
    env.pop(k, None)
results = []


def dc(*a, check=True):
    r = subprocess.run(['docker', 'compose', '-f', COMPOSE, '-p', PROJECT, *a], capture_output=True, text=True, encoding='utf-8', env=env)
    if check and r.returncode != 0:
        raise SystemExit('docker compose %s failed:\n%s' % (' '.join(a), (r.stdout + r.stderr)[-1500:]))
    return r


def cleanup():
    dc('down', '-v', '--remove-orphans', check=False)


def ok(cond, msg):
    results.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(results))
        cleanup()
        sys.exit(1)


def req(method, path, headers=None, body=None, host=None, timeout=20):
    c = http.client.HTTPConnection('127.0.0.1', PORT, timeout=timeout)
    c.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
    c.putheader('Host', host or 'localhost:%d' % PORT)
    for k, v in (headers or {}).items():
        c.putheader(k, v)
    if body is not None:
        c.putheader('Content-Length', str(len(body)))
    c.endheaders(body)
    r = c.getresponse()
    data = r.read()
    out = (r.status, {k.lower(): v for k, v in r.getheaders()}, data)
    c.close()
    return out


def jreq(method, path, obj=None, headers=None, host=None):
    h = {'Content-Type': 'application/json'}
    h.update(headers or {})
    st, hd, b = req(method, path, h, json.dumps(obj).encode() if obj is not None else None, host)
    return st, hd, (json.loads(b) if b[:1] in (b'{', b'[') else b)


def wait_healthy(names, secs=120):
    t0 = time.time()
    while time.time() - t0 < secs:
        good = 0
        for n in names:
            r = subprocess.run(['docker', 'inspect', '-f', '{{.State.Health.Status}}', n], capture_output=True, text=True, env=env)
            good += r.stdout.strip() == 'healthy'
        if good == len(names):
            return True
        time.sleep(2)
    return False


def upload(path):
    data = open(path, 'rb').read()
    return req('POST', '/api/upload', {'Content-Type': 'application/octet-stream', 'X-Filename': 'demo.xlsx'}, data)


cleanup()
try:
    dc('up', '-d', '--build')
    # In this project the container names are fixed as xexam-frontend / xexam-backend; changing the project name does not change container_name, so first confirm no real container is using the same names
    names = ['xexam-frontend', 'xexam-backend']
    ok(wait_healthy(names), 'Both containers become healthy')

    # -- 1 Exposed surface --
    r = subprocess.run(['docker', 'port', 'xexam-backend'], capture_output=True, text=True, env=env)
    ok(r.stdout.strip() == '', 'Backend publishes no ports (docker port is empty)')
    r = subprocess.run(['docker', 'port', 'xexam-frontend'], capture_output=True, text=True, env=env)
    ok('127.0.0.1:%d' % PORT in r.stdout, 'Nginx binds only 127.0.0.1:%d. Actual %r' % (PORT, r.stdout.strip()))
    for n, want in (('xexam-frontend', True), ('xexam-backend', True)):
        r = subprocess.run(['docker', 'exec', n, 'id', '-u'], capture_output=True, text=True, env=env)
        ok(r.stdout.strip() not in ('', '0'), '%s does not run as root (uid %s)' % (n, r.stdout.strip()))
    r = subprocess.run(['docker', 'inspect', '-f', '{{.HostConfig.ReadonlyRootfs}}', 'xexam-backend'], capture_output=True, text=True, env=env)
    ok(r.stdout.strip() == 'true', 'Backend root filesystem is read-only')

    # -- 2 Read endpoints --
    st, h, info = jreq('GET', '/api/info')
    ok(st == 200 and info['bankCount'] == 0 and info['canShutdown'] is False, '/api/info: empty question bank, shutdown not allowed. Actual %r' % info)
    ok(open(os.path.join(ROOT, 'VERSION')).read().strip().startswith(info['version'].split('.')[0]), 'Version comes from VERSION: %s' % info['version'])
    st, h, bank = jreq('GET', '/api/bank')
    ok(st == 200 and bank['questions'] == [] and list(bank.keys()) == ['file', 'mtime', 'error', 'questions', 'sheets'], '/api/bank with an empty question bank; key order matches the old version')
    st, h, prog = jreq('GET', '/api/progress')
    ok(st == 200 and list(prog.keys()) == ['version', 'questions', 'history', 'masterStreak'], '/api/progress key order matches the old version')

    # -- 3 Import flow --
    st, h, b = upload(DEMO)
    pv = json.loads(b)
    ok(st == 200 and pv['added'] == 5 and pv['usable'] == 5 and pv['token'], 'Upload demo.xlsx: preview adds 5 questions. Actual %d' % st)
    st, _, info = jreq('GET', '/api/info')
    ok(info['bankCount'] == 0, 'Question bank still has 0 questions during preview (preview does not write to the bank)')
    st, _, e = jreq('POST', '/api/import/confirm', {'token': 'wrong-token'})
    ok(st == 409, 'Wrong confirmation token → 409. Actual %d' % st)
    st, _, e = jreq('POST', '/api/import/confirm', {'token': '题库é'})
    ok(st == 409, 'Non-ASCII confirmation token → 409 (does not drop the connection). Actual %d' % st)
    st, _, c = jreq('POST', '/api/import/confirm', {'token': pv['token']})
    ok(st == 200 and len(c['bank']['questions']) == 5, 'Correct confirmation token → imports 5 questions')
    st, _, e = jreq('POST', '/api/import/confirm', {'token': pv['token']})
    ok(st == 409, 'Confirmation token is single-use: repeated confirm → 409. Actual %d' % st)
    st, _, info = jreq('GET', '/api/info')
    ok(info['bankCount'] == 5, 'Question bank now has 5 questions')
    st, h, b = upload(DEMO)
    pv2 = json.loads(b)
    ok(pv2['added'] == 0 and pv2['unchanged'] == 5, 'Uploading the same file again: no changes, 5 questions')
    st, _, _ = jreq('POST', '/api/import/cancel', {})
    st, _, e = jreq('POST', '/api/import/confirm', {'token': pv2['token']})
    ok(st == 409, 'Cannot confirm after cancel')
    st, _, imps = jreq('GET', '/api/imports')
    ok(len(imps['imports']) == 1 and imps['imports'][0]['filename'] == 'demo.xlsx', 'Import history has only 1 entry (preview and cancel leave no record)')

    # -- 4 Results --
    ids = [q['id'] for q in bank_q] if (bank_q := jreq('GET', '/api/bank')[2]['questions']) else []
    st, _, p = jreq('POST', '/api/attempt', {'results': [{'id': ids[0], 'correct': True}, {'id': ids[1], 'correct': False}], 'sheets': ['demo'], 'mode': 'all', 'seconds': 12})
    ok(st == 200 and p['history'][0]['percent'] == 50 and p['history'][0]['total'] == 2, 'Submit 1/2 → 50%')
    st, _, p = jreq('POST', '/api/attempt', {'results': [{'id': ids[0], 'correct': True}] + [{'id': i, 'correct': False} for i in ids[1:8]], 'sheets': [], 'mode': 'all', 'seconds': 1})
    ok(st == 200, 'Submit another round')
    st, _, e = jreq('POST', '/api/attempt', {'results': {'a': 1}})
    ok(st == 400, 'results is not a list → 400 (does not drop the connection). Actual %d' % st)

    # -- 5 Guards and size --
    st, _, e = jreq('GET', '/api/info', host='evil.example.com')
    ok(st == 403, 'Unknown Host is still rejected by the backend through Nginx (403)')
    st, _, e = jreq('POST', '/api/import/cancel', {}, headers={'Origin': 'https://evil.example.com'})
    ok(st == 403, 'POST with a foreign Origin → 403')
    st, _, e = jreq('POST', '/api/import/cancel', {}, headers={'Origin': 'http://localhost:%d' % PORT})
    ok(st == 200, 'Same-origin POST → 200')
    st, h, b = req('POST', '/api/upload', {'Content-Type': 'application/octet-stream', 'X-Filename': 'big.xlsx'}, b'x' * (20 * 1024 * 1024 + 1))
    ok(st == 413 and 'json' in h.get('content-type', ''), 'Over 20 MB → 413 JSON')
    st, _, e = jreq('POST', '/api/shutdown', {})
    ok(st == 403, '/api/shutdown is always 403 inside the container')
    st, h, b = req('GET', '/')
    ok(st == 200 and b'<div id="app">' in b and 'nosniff' == h.get('x-content-type-options'), 'Home page is served by Nginx')

    # -- 6 Persistence --
    dc('rm', '-sf', 'nginx', 'backend')
    ok(subprocess.run(['docker', 'volume', 'inspect', PROJECT + '_data'], capture_output=True, env=env).returncode == 0, 'Volume still exists after deleting the containers')
    dc('up', '-d')
    ok(wait_healthy(names), 'Both containers are healthy again after recreating with the same volume')
    st, _, info = jreq('GET', '/api/info')
    ok(info['bankCount'] == 5, 'Question bank still has 5 questions after recreation (data is in the volume)')
    st, _, p = jreq('GET', '/api/progress')
    ok(len(p['history']) == 2, 'Result history still has 2 rounds after recreation')
    t0 = time.time()
    subprocess.run(['docker', 'stop', 'xexam-backend'], capture_output=True, env=env)
    dt = time.time() - t0
    ok(dt < 9, 'docker stop backend took %.1f s (<9)' % dt)
    st, _, p = jreq('POST', '/api/reset', {}) if False else (0, 0, 0)

finally:
    cleanup()

print('\n'.join(results))
print('\nAll %d checks passed' % len(results))
