# -*- coding: utf-8 -*-
"""End-to-end check of the whole stack (nginx + Rust backend + AI service + their SQLite volumes). Starts the stack with Docker
Compose, using only a temporary project name and temporary volumes throughout, and never touches the real xexam_* volumes.

Usage: python -I tests/integration/stack_check.py [port, default 18090]
Prerequisite: this machine can run docker compose build (builds the backend, AI service and nginx images).
XEXAM_STACK_COMPOSE may name another compose file of the same shape (for example a copy of docker/docker-compose.build.yml
with other image tags and container names, to run next to an installation that uses the usual ones). Containers are
always found through their compose service, never by name.

What is checked:
  1. The stack comes up and the three containers are healthy; the backend and ai-service (the AI service) publish no
     ports, so the host cannot reach them; ai-service runs as uid 10001 with a read-only root filesystem on the internal
     and egress networks, the backend on the internal network only
  2. Sign-in over plain HTTP (v0.6: default administrator, forced password change, no Secure cookie), then
     /api/info /api/bank /api/progress /api/imports through Nginx
  3. Upload a made-up six-column workbook (written by this script) → preview (bank unchanged) → confirm → question bank has 5
     questions; cancel / repeated confirm / wrong confirmation token; a workbook of the old 3-column layout is refused (v0.7)
  4. Submit results → study history; reset
  5. Over 20 MB: 413 at the Nginx layer (JSON); the Host / Origin guards still take effect after passing through Nginx;
     ai-service's heartbeat route of the backend is 404 from outside
  AI (v0.8), with tests/integration/ai_stub.py as a made-up provider on the internal network (alias ai-stub):
  A0. Before AI is set up: ai-service has reported to the backend (connected, AI+ service off), so AI+ is not available:
      /api/users says aiAvailable false, creating a user with the AI+ service and any change of a user's AI+ service
      are refused with 409 ai.serviceOff and change nothing
  A1. AI settings: save the provider, a made-up key (only its hint comes back), enabling before a test is refused, models
      listed, test, enable (at once AI+ is available: /api/auth/state of the administrator shows the connected
      ai-service with its version; a plain user's state has no aiService); a changed address without the key typed
      again is refused and changes nothing
  A2. Users: sam with the AI+ service chats (start, delta, done with tokens), kim without it is refused; the reply
      language follows "lang" (ja: kana, zh-CN: Chinese, missing: English, fr: 400), also for a memory suggestion; free
      mode only for the administrator; user routes are scoped to the caller, the administrator's views are audited in
      the System log; a foreign Origin is refused by the auth step
  A3. Suspend / Resume of sam's AI+ function (403 ai.suspended, read access kept, aiSuspended in /api/users, still listed
      in AI settings, the System log names admin and user; administrators cannot be suspended); suspending during a
      slow answer ends it with ai.suspended within 3 s; switching the AI+ service off for sam during a slow answer ends
      it with ai.notAllowed within 5 s (keep-alive comments arrive while the provider is silent) and removes sam from
      AI settings while sam can still list and delete the chats; a suspension survives switching the service off and
      on; a deleted user's usage stays anonymised
  A4. No response, no System log line and no container log contains the made-up key, the question or the answer
  6. Recreate from the same volumes after deleting the containers: data and AI settings are still there, AI+ becomes
     available again; docker stop duration of each service; when ai-service stops, the backend knows at once (its last
     report says stopping: aiAvailable false, aiService.stopped true, AI+ switches refused) and the app keeps working
     (AI routes answer JSON 502); after docker start AI+ is available again within 15 s
  7. Non-root, read-only root filesystem
  8. SQLite in WAL mode (v0.7): the startup log says so, GET /api/logs shows it through Nginx, reset-password works while the
     server runs, and after docker stop the -wal file in the volume is empty (checkpoint at shutdown)
  9. Volumes (v0.8): the AI volume holds secret.key (mode 600, uid 10001) and xexam-ai.db; the main volume holds no AI
     secret and no AI table, and users has ai_enabled and uid; the link volume holds internal.token (mode 600)
"""
import codecs
import http.client
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import uuid
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18090
PROJECT = 'xexamcheck' + uuid.uuid4().hex[:6]
STUB = PROJECT + '-stubai'
WORK = tempfile.mkdtemp(prefix='xexam_stack_')
DEMO = os.path.join(WORK, 'garden.xlsx')
THREE = os.path.join(WORK, 'three.xlsx')
MSG_OLD_LAYOUT = '只能导入六列格式（A–C 列：英文、中文、日文题目；D–F 列：英文、中文、日文答案）。这个文件是旧的三列或四列格式，请改成六列格式后再导入。'
COMPOSE = os.environ.get('XEXAM_STACK_COMPOSE') or os.path.join(ROOT, 'docker', 'docker-compose.build.yml')
# Made-up values of this check only: the provider key the stub accepts, the question and words of the stub's answer
DEMO_KEY = 'sk-demo-0000-1234'
QUESTION = 'How can I remember the names of garden herbs better?'
ANSWER_WORDS = 'spaced repetition'
# The backend's refusal of an AI+ feature switch while AI+ is not available (backend/src/api/response.rs)
MSG_AI_SERVICE_OFF = '请先在 AI 设置里开启 AI+ 功能。'
# The SSE error text of an answer stopped because an administrator paused the user's AI+ feature
MSG_AI_SUSPENDED = '管理员已暂停你的 AI+ 功能。'
# Key orders of the JSON answers checked here (contracts of v0.8)
USER_ROW_KEYS = ['id', 'username', 'role', 'disabled', 'createdAt', 'lastLoginAt', 'mustChangePassword', 'records',
                 'banks', 'newBanks', 'canManageBanks', 'ai', 'ownedBanks', 'lock', 'aiSuspended']
AI_SERVICE_KEYS = ['connected', 'stopped', 'lastSeen', 'version', 'enabled', 'link']
STATUS_KEYS = ['enabled', 'allowed', 'suspended', 'hasData', 'running', 'retentionDays', 'memoryLimit', 'memoryMaxChars',
               'maxMessageChars', 'messageChars', 'memoryChars', 'keyProblem', 'configured', 'freeMode']
START_KEYS = ['chatId', 'chatTitle', 'userMessageId', 'assistantMessageId', 'model', 'newChat', 'retry', 'free']
CHAT_KEYS = ['id', 'title', 'createdAt', 'updatedAt', 'messageCount', 'running']
ADMIN_ROW_KEYS = ['id', 'username', 'role', 'ai', 'aiAllowed', 'disabled', 'exists', 'suspended', 'chats', 'messages',
                  'memories', 'lastUsedAt', 'tokensIn', 'tokensOut']

env = dict(os.environ, XEXAM_BIND='127.0.0.1:%d' % PORT, COMPOSE_PROJECT_NAME=PROJECT, MSYS_NO_PATHCONV='1')
for k in ('XEXAM_ALLOWED_HOSTS', 'XEXAM_ALLOWED_ORIGINS', 'XEXAM_ALLOW_ANY_HOST', 'XEXAM_INTERNAL_TOKEN', 'XEXAM_AI_SECRET_KEY'):
    env.pop(k, None)
results = []
# The session cookie (v0.6), sent with every request once signed in
COOKIE = None
NEW_PASSWORD = 'stack-check-pass'
# Every response body of the run (A4: none may contain the made-up key)
BODIES = []


def dc(*a, check=True):
    r = subprocess.run(['docker', 'compose', '-f', COMPOSE, '-p', PROJECT, *a], capture_output=True, text=True,
                       encoding='utf-8', errors='replace', env=env)
    if check and r.returncode != 0:
        raise SystemExit('docker compose %s failed:\n%s' % (' '.join(a), (r.stdout + r.stderr)[-1500:]))
    return r


def docker(*a):
    return subprocess.run(['docker', *a], capture_output=True, text=True, encoding='utf-8', errors='replace', env=env)


def cid(service):
    """The container of a compose service (also when it is stopped)."""
    out = dc('ps', '-a', '-q', service, check=False).stdout.split()
    return out[0] if out else ''


def cleanup():
    # The stub first: a container still attached to <project>_internal would keep compose down from removing the network
    docker('rm', '-f', STUB)
    dc('down', '-v', '--remove-orphans', check=False)
    shutil.rmtree(WORK, ignore_errors=True)


def xlsx(path, rows):
    """Writes a minimal one-sheet .xlsx: `rows` are lists of cell texts from column A on (inline strings)."""
    def esc(t):
        return t.replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;')
    cells = []
    for r, row in enumerate(rows, 1):
        cs = ''.join('<c r="%s%d" t="inlineStr"><is><t xml:space="preserve">%s</t></is></c>' % (chr(65 + k), r, esc(t))
                     for k, t in enumerate(row))
        cells.append('<row r="%d">%s</row>' % (r, cs))
    ns = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
    rel = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
    pkg = 'http://schemas.openxmlformats.org/package/2006/relationships'
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as z:
        z.writestr('[Content_Types].xml',
                   '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
                   '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
                   '<Default Extension="xml" ContentType="application/xml"/>'
                   '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
                   '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
                   '</Types>')
        z.writestr('_rels/.rels', '<Relationships xmlns="%s"><Relationship Id="rId1" Type="%s/officeDocument" Target="xl/workbook.xml"/></Relationships>' % (pkg, rel))
        z.writestr('xl/workbook.xml', '<workbook xmlns="%s" xmlns:r="%s"><sheets><sheet name="Garden" sheetId="1" r:id="rId1"/></sheets></workbook>' % (ns, rel))
        z.writestr('xl/_rels/workbook.xml.rels', '<Relationships xmlns="%s"><Relationship Id="rId1" Type="%s/worksheet" Target="worksheets/sheet1.xml"/></Relationships>' % (pkg, rel))
        z.writestr('xl/worksheets/sheet1.xml', '<worksheet xmlns="%s"><sheetData>%s</sheetData></worksheet>' % (ns, ''.join(cells)))


def write_workbooks():
    """The two made-up workbooks of this check, in WORK (removed again by cleanup())."""
    os.makedirs(WORK, exist_ok=True)
    # Five made-up questions of the six-column layout (A-C the question in English, Chinese, Japanese; D-F the answer)
    xlsx(DEMO, [[
        'A gardener waters the herbs every morning (question %d). Which can is used?\n\nA. The small can\n\nB. The big can\n\nC. The hose' % i,
        '【中文翻译】\n园丁每天早上给香草浇水（第 %d 题）。用哪个水壶？\nA. 小水壶\nB. 大水壶\nC. 水管' % i,
        '庭師が毎朝ハーブに水をやります（問題 %d）。どのじょうろを使いますか。\n\nA. 小さいじょうろ\n\nB. 大きいじょうろ\n\nC. ホース' % i,
        '【Answer】A\n\n【Explanation】\nA small can is enough for herbs.',
        '【答案】A\n\n【解析·怎么选】\n浇香草用小水壶就够了。',
        '【正解】A\n\n【解説】\nハーブには小さいじょうろで十分です。',
    ] for i in range(1, 6)])
    # The old three-column layout (A the Japanese question, B the Chinese translation, C the answer): refused since v0.7
    xlsx(THREE, [[
        'これは庭についての十分に長い日本語の問題文です。どれが正しいですか。\n\nA. 甲です\n\nB. 乙です\n\nC. 丙です',
        '【中文翻译】\n这是一个关于花园的足够长的中文题目。哪个正确？\nA. 甲\nB. 乙\nC. 丙',
        '【答案】B\n\n【解析·怎么选】因为乙。',
    ]])


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
    if COOKIE and not any(k.lower() == 'cookie' for k in (headers or {})):
        c.putheader('Cookie', COOKIE)
    for k, v in (headers or {}).items():
        c.putheader(k, v)
    if body is not None:
        c.putheader('Content-Length', str(len(body)))
    c.endheaders(body)
    r = c.getresponse()
    data = r.read()
    hdrs = {k.lower(): v for k, v in r.getheaders()}
    # Several Set-Cookie headers may come back (v0.6 sign-in sends xexam_session and xexam_device): keep them all
    hdrs['set-cookie'] = [v for k, v in r.getheaders() if k.lower() == 'set-cookie']
    out = (r.status, hdrs, data)
    c.close()
    if path.startswith('/api/'):
        BODIES.append(data)
    return out


def jreq(method, path, obj=None, headers=None, host=None):
    h = {'Content-Type': 'application/json'}
    h.update(headers or {})
    st, hd, b = req(method, path, h, json.dumps(obj).encode() if obj is not None else None, host)
    return st, hd, (json.loads(b) if b[:1] in (b'{', b'[') else b)


def wait_healthy(services, secs=180):
    t0 = time.time()
    while time.time() - t0 < secs:
        good = 0
        for s in services:
            r = docker('inspect', '-f', '{{.State.Health.Status}}', cid(s))
            good += r.stdout.strip() == 'healthy'
        if good == len(services):
            return True
        time.sleep(2)
    return False


def bank_size():
    """The number of active questions, from GET /api/bank as the signed-in administrator (the public /api/info no longer has it)"""
    st, _, bank = jreq('GET', '/api/bank')
    return len(bank['questions']) if st == 200 else -1


def upload(path, name='garden.xlsx'):
    data = open(path, 'rb').read()
    return req('POST', '/api/upload', {'Content-Type': 'application/octet-stream', 'X-Filename': name}, data)


def logs_of(service):
    r = docker('logs', cid(service))
    return r.stdout + r.stderr


def session_cookie(h):
    return next((c for c in h.get('set-cookie', []) if c.startswith('xexam_session=')), '').split(';')[0]


def sign_in_new_user(username, initial, password):
    """First sign-in of a user the administrator created: chooses a new password; returns the rotated session cookie."""
    st, h, j = jreq('POST', '/api/auth/login', {'username': username, 'password': initial},
                    headers={'Origin': 'http://localhost:%d' % PORT, 'Cookie': ''})
    ok(st == 200 and j['user']['mustChangePassword'] is True, '%s signs in and must choose a password. Actual %d' % (username, st))
    first = session_cookie(h)
    st, h, j = jreq('POST', '/api/auth/password', {'password': password}, headers={'Origin': 'http://localhost:%d' % PORT, 'Cookie': first})
    ok(st == 200 and session_cookie(h), '%s chose a new password and got a new session cookie. Actual %d' % (username, st))
    return session_cookie(h)


def parse_block(block):
    """One SSE block -> (event name, data JSON) or ('comment', None) for a comment-only block."""
    name, data, comment = 'message', [], False
    for line in block.splitlines():
        if line.startswith(':'):
            comment = True
        elif line.startswith('event:'):
            name = line[6:].strip()
        elif line.startswith('data:'):
            data.append(line[5:][1:] if line[5:6] == ' ' else line[5:])
    if data:
        return name, json.loads('\n'.join(data))
    return ('comment', None) if comment else None


class Stream(threading.Thread):
    """POST of an SSE answer route; records every event (and keep-alive comment) with the time it arrived."""

    def __init__(self, path, obj, cookie, headers=None):
        super().__init__(daemon=True)
        self.path, self.body, self.cookie, self.extra = path, json.dumps(obj).encode(), cookie, headers or {}
        self.status, self.ctype, self.events, self.comments, self.raw = None, '', [], [], b''
        self.started = threading.Event()
        self.error = None

    def run(self):
        try:
            c = http.client.HTTPConnection('127.0.0.1', PORT, timeout=120)
            c.putrequest('POST', self.path, skip_host=True, skip_accept_encoding=True)
            for k, v in dict({'Host': 'localhost:%d' % PORT, 'Cookie': self.cookie, 'Content-Type': 'application/json',
                              'Content-Length': str(len(self.body))}, **self.extra).items():
                c.putheader(k, v)
            c.endheaders(self.body)
            r = c.getresponse()
            self.status, self.ctype = r.status, r.getheader('Content-Type') or ''
            if r.status != 200 or 'text/event-stream' not in self.ctype:
                self.raw = r.read()
                return
            dec, buf = codecs.getincrementaldecoder('utf-8')(), ''
            while True:
                part = r.read1(65536)
                if not part:
                    break
                self.raw += part
                buf += dec.decode(part).replace('\r\n', '\n')
                while '\n\n' in buf:
                    block, buf = buf.split('\n\n', 1)
                    ev = parse_block(block)
                    if ev is None:
                        continue
                    if ev[0] == 'comment':
                        self.comments.append(time.time())
                    else:
                        self.events.append((time.time(), ev[0], ev[1]))
                        if ev[0] == 'start':
                            self.started.set()
            c.close()
        except Exception as e:      # reported by the checks that read the stream
            self.error = repr(e)
        finally:
            BODIES.append(self.raw)
            self.started.set()

    def names(self):
        return [n for _, n, _ in self.events]

    def terminal(self):
        return next(((t, n, d) for t, n, d in reversed(self.events) if n in ('done', 'stopped', 'error')), None)


def kana(text):
    return any(0x3040 <= ord(c) <= 0x30ff for c in text)


def han(text):
    return any(0x4e00 <= ord(c) <= 0x9fff for c in text)


def answer_of(stream):
    return ''.join(d.get('text', '') for _, n, d in stream.events if n == 'delta')


def until(fn, secs, step=0.25):
    """Calls fn() until it returns something true or `secs` have passed; returns the last result."""
    t0 = time.time()
    while True:
        v = fn()
        if v or time.time() - t0 >= secs:
            return v
        time.sleep(step)


def stub_requests():
    r = docker('logs', STUB)
    return [l for l in (r.stdout + r.stderr).splitlines() if re.match(r'^(GET|POST) /', l)]


def volume_report(volume, script):
    """Runs `script` (Python, prints JSON) in a throw-away container with `volume` mounted read-only on /v."""
    r = docker('run', '--rm', '--network', 'none', '-v', volume + ':/v:ro', 'python:3.12-slim', 'python', '-I', '-c', script)
    try:
        return json.loads(r.stdout)
    except ValueError:
        return {'error': (r.stdout + r.stderr)[-400:]}


cleanup()
write_workbooks()
try:
    dc('up', '-d', '--build')
    services = ['frontend', 'backend', 'ai-service']
    ok(wait_healthy(services), 'The three containers become healthy')
    BACKEND, AI, FRONTEND = cid('backend'), cid('ai-service'), cid('frontend')

    # -- 1 Exposed surface --
    r = docker('port', BACKEND)
    ok(r.stdout.strip() == '', 'Backend publishes no ports (docker port is empty)')
    r = docker('port', AI)
    ok(r.stdout.strip() == '', 'ai-service publishes no ports (docker port is empty)')
    r = docker('port', FRONTEND)
    ok('127.0.0.1:%d' % PORT in r.stdout, 'Nginx binds only 127.0.0.1:%d. Actual %r' % (PORT, r.stdout.strip()))
    for n, c in (('frontend', FRONTEND), ('backend', BACKEND), ('ai-service', AI)):
        r = docker('exec', c, 'id', '-u')
        ok(r.stdout.strip() not in ('', '0'), '%s does not run as root (uid %s)' % (n, r.stdout.strip()))
    ok(docker('exec', AI, 'id', '-u').stdout.strip() == '10001', 'ai-service runs as uid 10001')
    for n, c in (('Backend', BACKEND), ('ai-service', AI)):
        r = docker('inspect', '-f', '{{.HostConfig.ReadonlyRootfs}}', c)
        ok(r.stdout.strip() == 'true', '%s root filesystem is read-only' % n)
    nets = lambda c: sorted(json.loads(docker('inspect', '-f', '{{json .NetworkSettings.Networks}}', c).stdout or '{}'))
    ok(nets(AI) == sorted([PROJECT + '_internal', PROJECT + '_egress']), 'ai-service is on the internal and egress networks. Actual %r' % nets(AI))
    ok(nets(BACKEND) == [PROJECT + '_internal'], 'The backend is on the internal network only (no internet). Actual %r' % nets(BACKEND))

    # -- 2 Sign-in (plain HTTP) --
    st, _, e = jreq('GET', '/api/bank')
    ok(st == 401, 'Without a session /api/bank is 401. Actual %d' % st)
    st, _, e = jreq('GET', '/api/ai/status')
    ok(st == 401 and isinstance(e, dict) and e.get('code') == 'auth.signin', 'Without a session /api/ai/status is 401 auth.signin (the auth step). Actual %d %r' % (st, e))
    st, h, j = jreq('POST', '/api/auth/login', {'username': 'admin', 'password': 'admin'}, headers={'Origin': 'http://localhost:%d' % PORT})
    ok(st == 200 and j['user']['mustChangePassword'] is True, 'The default administrator admin / admin signs in and must change the password. Actual %d %r' % (st, j))
    cookies = h.get('set-cookie', [])
    sc = next((c for c in cookies if c.startswith('xexam_session=')), '')
    ok(sc and 'HttpOnly' in sc and 'SameSite=Strict' in sc and 'Secure' not in sc, 'Session cookie over plain HTTP: HttpOnly, SameSite=Strict, no Secure. Actual %r' % cookies)
    dev = next((c for c in cookies if c.startswith('xexam_device=')), '')
    ok(dev and 'HttpOnly' in dev and 'Secure' not in dev, 'Known-browser cookie over plain HTTP: HttpOnly, no Secure. Actual %r' % cookies)
    COOKIE = sc.split(';')[0]
    st, _, e = jreq('GET', '/api/bank')
    ok(st == 403, 'Before the new password every other route is 403. Actual %d' % st)
    st, _, e = jreq('GET', '/api/ai/status')
    ok(st == 403 and isinstance(e, dict) and e.get('code') == 'auth.password', 'Before the new password /api/ai/status is 403 auth.password. Actual %d %r' % (st, e))
    st, h, j = jreq('POST', '/api/auth/password', {'password': NEW_PASSWORD}, headers={'Origin': 'http://localhost:%d' % PORT})
    ok(st == 200 and j['user']['mustChangePassword'] is False, 'New password saved. Actual %d %r' % (st, j))
    # v0.6.5: the password change rotates the session; this client continues with the new cookie, the old one has ended
    old_cookie = COOKIE
    sc = next((c for c in h.get('set-cookie', []) if c.startswith('xexam_session=')), '')
    ok(sc and 'HttpOnly' in sc and 'Secure' not in sc, 'The password change sends a new session cookie. Actual %r' % h.get('set-cookie'))
    COOKIE = sc.split(';')[0]
    ok(COOKIE != old_cookie, 'The new session cookie differs from the old one')
    st, _, _ = jreq('GET', '/api/progress', headers={'Cookie': old_cookie})
    ok(st == 401, 'The session from before the password change has ended. Actual %d' % st)

    # -- 2 Read endpoints --
    st, h, info = jreq('GET', '/api/info')
    ok(st == 200 and info['canShutdown'] is False, '/api/info: shutdown not allowed. Actual %r' % info)
    ok(list(info.keys()) == ['version', 'arch', 'canShutdown'], '/api/info (public) does not say how many questions exist (v0.7.5). Actual %r' % info)
    ok(bank_size() == 0, 'Empty question bank (GET /api/bank, signed in)')
    # VERSION has KEY=VALUE lines; the backend reports CODE_VERSION (the version of the code in the image)
    code_version = re.search(r'(?m)^CODE_VERSION=\s*(\S+)', open(os.path.join(ROOT, 'VERSION'), encoding='utf-8').read())
    ok(code_version is not None and info['version'] == code_version.group(1), 'Version comes from CODE_VERSION in VERSION: %s' % info['version'])
    r = docker('exec', AI, 'wget', '-qO-', 'http://127.0.0.1:8766/healthz')
    health = json.loads(r.stdout or '{}')
    ok(health.get('ok') is True and health.get('version') == code_version.group(1),
       'ai-service reports the same version on its own /healthz (inside its container). Actual %r' % health)
    st, h, bank = jreq('GET', '/api/bank')
    ok(st == 200 and bank['questions'] == [] and list(bank.keys()) == ['file', 'mtime', 'error', 'questions', 'sheets'], '/api/bank with an empty question bank; key order matches the old version')
    st, h, prog = jreq('GET', '/api/progress')
    ok(st == 200 and list(prog.keys()) == ['version', 'questions', 'history', 'masterStreak'], '/api/progress key order matches the old version')

    # -- 3 Import flow --
    st, h, b = upload(DEMO)
    pv = json.loads(b)
    ok(st == 200 and pv['added'] == 5 and pv['usable'] == 5 and pv['token'], 'Upload garden.xlsx (six columns): preview adds 5 questions. Actual %d' % st)
    ok(bank_size() == 0, 'Question bank still has 0 questions during preview (preview does not write to the bank)')
    st, _, e = jreq('POST', '/api/import/confirm', {'token': 'wrong-token'})
    ok(st == 409, 'Wrong confirmation token → 409. Actual %d' % st)
    st, _, e = jreq('POST', '/api/import/confirm', {'token': '题库é'})
    ok(st == 409, 'Non-ASCII confirmation token → 409 (does not drop the connection). Actual %d' % st)
    st, _, c = jreq('POST', '/api/import/confirm', {'token': pv['token']})
    ok(st == 200 and len(c['bank']['questions']) == 5, 'Correct confirmation token → imports 5 questions')
    st, _, e = jreq('POST', '/api/import/confirm', {'token': pv['token']})
    ok(st == 409, 'Confirmation token is single-use: repeated confirm → 409. Actual %d' % st)
    ok(bank_size() == 5, 'Question bank now has 5 questions')
    st, h, b = upload(DEMO)
    pv2 = json.loads(b)
    ok(pv2['added'] == 0 and pv2['unchanged'] == 5, 'Uploading the same file again: no changes, 5 questions')
    st, _, _ = jreq('POST', '/api/import/cancel', {})
    st, _, e = jreq('POST', '/api/import/confirm', {'token': pv2['token']})
    ok(st == 409, 'Cannot confirm after cancel')
    st, _, imps = jreq('GET', '/api/imports')
    ok(len(imps['imports']) == 1 and imps['imports'][0]['filename'] == 'garden.xlsx', 'Import history has only 1 entry (preview and cancel leave no record)')
    st, _, b = upload(THREE, 'three.xlsx')
    ok(st == 400 and json.loads(b).get('error') == MSG_OLD_LAYOUT, 'A workbook of the old 3-column layout is refused (400, six-column message). Actual %d' % st)
    ok(bank_size() == 5, 'The refused workbook changed nothing (still 5 questions)')

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
    st, _, e = jreq('GET', '/api/ai/status', host='evil.example.com')
    ok(st == 403 and isinstance(e, dict) and e.get('code') == 'auth.guard', 'Unknown Host on an AI route → 403 auth.guard (the auth step runs the same guard). Actual %d %r' % (st, e))
    st, _, e = jreq('POST', '/internal/ai/heartbeat', {'version': '0.8', 'enabled': True, 'suspended': [], 'stopping': False})
    ok(st == 404, "POST /internal/ai/heartbeat (ai-service's report to the backend) from outside → 404. Actual %d" % st)

    # -- A0 Before AI is set up: ai-service is connected, but the AI+ service is off, so AI+ is not available --
    def admin_state():
        st, _, j = jreq('GET', '/api/auth/state')
        return j if st == 200 and isinstance(j, dict) else {}

    def users_list():
        st, _, j = jreq('GET', '/api/users')
        return j if st == 200 and isinstance(j, dict) else {}

    def user_rows():
        return {u['username']: u for u in users_list().get('users', [])}

    def svc_when(cond):
        """The administrator's /api/auth/state when cond(aiService) holds, else None."""
        j = admin_state()
        return j if cond(j.get('aiService') or {}) else None

    state = until(lambda: svc_when(lambda a: a.get('connected')), 30) or admin_state()
    svc = state.get('aiService') or {}
    ok(list(state.keys()) == ['user', 'library', 'aiAvailable', 'aiService'] and list(svc.keys()) == AI_SERVICE_KEYS,
       'GET /api/auth/state (administrator): keys user, library, aiAvailable, aiService; aiService keys in order. Actual %r' % state)
    ok(svc.get('connected') is True and svc.get('stopped') is False and svc.get('enabled') is False and svc.get('link') is True
       and svc.get('version') == code_version.group(1) and isinstance(svc.get('lastSeen'), int) and state['aiAvailable'] is False,
       'ai-service has reported to the backend (connected, link on, version %s, AI+ service off): aiAvailable false. Actual %r'
       % (code_version.group(1), svc))
    ul = users_list()
    ok(list(ul.keys()) == ['users', 'aiAvailable'] and ul['aiAvailable'] is False,
       'GET /api/users: top-level keys users, aiAvailable; aiAvailable false before AI is set up. Actual %r' % list(ul.keys()))
    ok(all(list(u.keys()) == USER_ROW_KEYS for u in ul['users']) and all(u['aiSuspended'] is False for u in ul['users']),
       'GET /api/users: row keys in order, aiSuspended appended after lock. Actual %r' % [list(u.keys()) for u in ul['users']][:1])
    src = open(os.path.join(ROOT, 'backend', 'src', 'api', 'response.rs'), encoding='utf-8').read()
    m_off = re.search(r'pub const MSG_AI_SERVICE_OFF: &str = "([^"]*)";', src)
    ok(m_off is not None and m_off.group(1) == MSG_AI_SERVICE_OFF, 'backend/src/api/response.rs has MSG_AI_SERVICE_OFF = %r' % MSG_AI_SERVICE_OFF)
    REFUSED = {'error': MSG_AI_SERVICE_OFF, 'code': 'ai.serviceOff'}
    st, _, e = jreq('POST', '/api/users/create', {'username': 'sam', 'password': 'sam-initial-2026', 'role': 'user', 'banks': 'all', 'ai': True})
    ok(st == 409 and e == REFUSED, 'Creating sam with the AI+ service before AI is set up → 409 ai.serviceOff (exact body). Actual %d %r' % (st, e))
    ok('sam' not in user_rows(), 'The refused create added no user')
    st, _, us = jreq('POST', '/api/users/create', {'username': 'kim', 'password': 'kim-initial-2026', 'role': 'user', 'banks': 'all'})
    ok(st == 200, 'The administrator creates kim without the AI+ service (allowed while AI+ is not available). Actual %d' % st)
    KIM_ID, ADMIN_ID = user_rows()['kim']['id'], user_rows()['admin']['id']
    for body, what in (({'id': KIM_ID, 'ai': True}, 'on'), ({'id': KIM_ID, 'ai': False}, 'off (the stored value)'),
                       ({'id': KIM_ID, 'ai': True, 'disabled': True}, 'on together with disabled')):
        st, _, e = jreq('POST', '/api/users/update', body)
        ok(st == 409 and e == REFUSED, "Switching kim's AI+ service %s before AI is set up → 409 ai.serviceOff. Actual %d %r" % (what, st, e))
    kim = user_rows()['kim']
    ok(kim['ai'] is False and kim['disabled'] is False, 'The refused updates changed nothing (kim: AI+ service off, not disabled). Actual %r' % kim)

    # -- A1 AI settings, with the made-up provider on the internal network --
    vol = os.path.join(ROOT, 'tests', 'integration', 'ai_stub.py') + ':/stub.py:ro'
    r = docker('run', '-d', '--name', STUB, '--network', PROJECT + '_internal', '--network-alias', 'ai-stub', '-v', vol,
               'python:3.12-slim', 'python', '-I', '/stub.py', '--host', '0.0.0.0', '--port', '9000')
    ok(r.returncode == 0, 'The made-up AI provider (ai_stub.py) starts on the internal network as ai-stub. %s' % r.stderr.strip()[-200:])
    for _ in range(60):
        if 'listening on' in (docker('logs', STUB).stdout or ''):
            break
        time.sleep(0.5)
    st, _, s0 = jreq('GET', '/api/ai/status')
    ok(st == 200 and s0['enabled'] is False and s0['allowed'] is False and s0['suspended'] is False and s0['hasData'] is False
       and s0['configured'] is False,
       'GET /api/ai/status (administrator, nothing set up): enabled, allowed, suspended, hasData, configured all false. Actual %d %r' % (st, s0))
    ok(list(s0.keys()) == STATUS_KEYS and s0['freeMode'] is False,
       'GET /api/ai/status: keys in order (suspended after allowed, freeMode last and false). Actual %r' % s0)
    st, _, cfg = jreq('POST', '/api/ai/admin/settings', {'provider': 'openai', 'baseUrl': 'http://ai-stub:9000/v1', 'apiKey': DEMO_KEY})
    ok(st == 200 and cfg['keys']['apiKey'] == {'set': True, 'hint': '\u2026' + DEMO_KEY[-4:], 'unreadable': False},
       'Saving the provider and a key: only {set, hint, unreadable} comes back. Actual %d %r' % (st, cfg.get('keys') if isinstance(cfg, dict) else cfg))
    ok(cfg['provider'] == 'openai' and cfg['baseUrl'] == 'http://ai-stub:9000/v1' and cfg['enabled'] is False, 'The settings are saved; AI is still off')
    st, _, e = jreq('POST', '/api/ai/admin/enable', {'enabled': True})
    ok(st == 409 and e.get('code') == 'ai.notVerified', 'Enabling AI before a successful test → 409 ai.notVerified. Actual %d %r' % (st, e))
    st, _, m = jreq('POST', '/api/ai/admin/models', {})
    ok(st == 200 and m['listed'] is True and 'demo-chat-small' in m['models'], 'The models are listed from the provider. Actual %d %r' % (st, m))
    st, _, cfg = jreq('POST', '/api/ai/admin/settings', {'model': 'demo-chat-small'})
    ok(st == 200 and cfg['model'] == 'demo-chat-small', 'The model is saved. Actual %d' % st)
    st, _, v = jreq('POST', '/api/ai/admin/verify', {})
    ok(st == 200 and v['ok'] is True and v['model'] == 'demo-chat-small', 'The test call succeeds. Actual %d %r' % (st, v))
    st, _, cfg = jreq('POST', '/api/ai/admin/enable', {'enabled': True})
    ok(st == 200 and cfg['enabled'] is True and cfg['verified'] is True, 'AI is enabled after the test. Actual %d' % st)
    state = admin_state()
    svc = state.get('aiService') or {}
    ok(state.get('aiAvailable') is True and svc.get('connected') is True and svc.get('enabled') is True
       and svc.get('version') == code_version.group(1) and svc.get('stopped') is False,
       'Right after enabling (ai-service reports at once): /api/auth/state says aiAvailable true, aiService connected and enabled, '
       'version %s. Actual %r' % (code_version.group(1), state))
    ok(users_list().get('aiAvailable') is True, 'GET /api/users: aiAvailable true once the AI+ service is on')
    st, _, s1 = jreq('GET', '/api/ai/status')
    ok(st == 200 and s1['enabled'] is True and s1['allowed'] is True and s1['suspended'] is False and s1['configured'] is True,
       'GET /api/ai/status: enabled and allowed for the administrator. Actual %r' % s1)
    seen_by_stub = len(stub_requests())
    st, _, e = jreq('POST', '/api/ai/admin/settings', {'baseUrl': 'http://ai-stub:9001/v1'})
    ok(st == 400 and e.get('code') == 'ai.keyReenter' and e.get('field') == 'apiKey',
       'A new address without the key typed again → 400 ai.keyReenter, field apiKey. Actual %d %r' % (st, e))
    st, _, cfg = jreq('GET', '/api/ai/admin/settings')
    ok(st == 200 and cfg['baseUrl'] == 'http://ai-stub:9000/v1' and cfg['enabled'] is True and cfg['keys']['apiKey']['set'] is True,
       'The refused update changed nothing (same address, still enabled, key kept)')
    time.sleep(1)
    ok(len(stub_requests()) == seen_by_stub, 'The provider received no request because of the refused update')

    # -- A2 Users with and without the AI+ service --
    st, _, us = jreq('POST', '/api/users/create', {'username': 'sam', 'password': 'sam-initial-2026', 'role': 'user', 'banks': 'all', 'ai': True})
    ok(st == 200, 'With AI+ available the administrator creates sam with the AI+ service. Actual %d %r' % (st, us))
    rows = user_rows()
    ok(rows['sam']['ai'] is True and rows['kim']['ai'] is False and rows['sam']['aiSuspended'] is False,
       'GET /api/users shows the AI+ service of each user (sam on, kim off; nobody suspended)')
    SAM_ID = rows['sam']['id']
    SAM = sign_in_new_user('sam', 'sam-initial-2026', 'sam-stack-2026')
    KIM = sign_in_new_user('kim', 'kim-initial-2026', 'kim-stack-2026')
    st, _, j = jreq('GET', '/api/auth/state', headers={'Cookie': SAM})
    ok(st == 200 and j['user']['aiAllowed'] is True and j.get('aiAvailable') is True and list(j.keys()) == ['user', 'library', 'aiAvailable'],
       'sam: aiAllowed and aiAvailable in the sign-in state, no aiService (administrators only). Actual %d %r' % (st, j))

    s = Stream('/api/ai/send', {'chatId': None, 'text': QUESTION}, SAM)
    s.run()
    names = s.names()
    term = s.terminal()
    ok(s.status == 200 and 'text/event-stream' in s.ctype, 'sam: POST /api/ai/send streams. Actual %r %r %r' % (s.status, s.ctype, s.raw[:200]))
    ok(names[:1] == ['start'] and names.count('delta') >= 1 and term is not None and term[1] == 'done',
       'sam: events start, delta…, done. Actual %r %r' % (names, s.error))
    ok(isinstance(term[2].get('tokensIn'), int) and isinstance(term[2].get('tokensOut'), int), 'done carries the token counts. Actual %r' % term[2])
    start = s.events[0][2]
    ok(list(start.keys()) == START_KEYS and start['free'] is False and start['newChat'] is True,
       'start carries free (false for a normal chat) as its last key. Actual %r' % start)
    ok(ANSWER_WORDS in answer_of(s), "Without lang the answer is in English (the stub's made-up English answer)")
    chat_id = start['chatId']
    st, _, chats = jreq('GET', '/api/ai/chats', headers={'Cookie': SAM})
    ok(st == 200 and [c['id'] for c in chats['chats']] == [chat_id], 'sam: GET /api/ai/chats lists the one chat. Actual %r' % chats)
    ok(list(chats['chats'][0].keys()) == CHAT_KEYS,
       'The chat summary carries no mode (free mode is a state of the administrator, not of a chat). Actual %r' % chats['chats'][0])
    st, _, ch = jreq('GET', '/api/ai/chat?id=%d' % chat_id, headers={'Cookie': SAM})
    ok(st == 200 and [m['role'] for m in ch['messages']] == ['user', 'assistant'] and ch['messages'][1]['status'] == 'done',
       'sam: the chat holds the question and the finished answer')

    # The reply language is the one of "lang" (the Settings language), not the language of the question
    asked = {}
    for lang, check, what in (('ja', kana, 'Japanese (kana)'), ('zh-CN', lambda t: han(t) and not kana(t), 'Chinese'),
                              ('en', lambda t: ANSWER_WORDS in t, 'English')):
        sl = Stream('/api/ai/send', {'chatId': chat_id, 'text': QUESTION, 'lang': lang}, SAM)
        sl.run()
        term = sl.terminal()
        ok(sl.status == 200 and term is not None and term[1] == 'done' and check(answer_of(sl)),
           'sam: the English question with lang %r is answered in %s. Actual %r %r' % (lang, what, sl.names(), answer_of(sl)[:40]))
        asked[lang] = sl.events[0][2]['userMessageId']
    count = len(jreq('GET', '/api/ai/chat?id=%d' % chat_id, headers={'Cookie': SAM})[2]['messages'])
    for bad in ('fr', 5):
        st, _, e = jreq('POST', '/api/ai/send', {'chatId': chat_id, 'text': QUESTION, 'lang': bad}, headers={'Cookie': SAM})
        ok(st == 400 and isinstance(e, dict) and e.get('code') == 'ai.badRequest', 'sam: lang %r → 400 ai.badRequest. Actual %d %r' % (bad, st, e))
    ok(len(jreq('GET', '/api/ai/chat?id=%d' % chat_id, headers={'Cookie': SAM})[2]['messages']) == count,
       'The refused sends saved no message (still %d messages)' % count)
    st, _, sg = jreq('POST', '/api/ai/memories/suggest', {'messageId': asked['ja'], 'lang': 'ja'}, headers={'Cookie': SAM})
    ok(st == 200 and isinstance(sg, dict) and kana(sg.get('suggestion', '')),
       'sam: a memory suggestion with lang ja is written in Japanese. Actual %d %r' % (st, sg))
    st, _, e = jreq('POST', '/api/ai/memories/suggest', {'messageId': asked['ja'], 'lang': 'fr'}, headers={'Cookie': SAM})
    ok(st == 400 and isinstance(e, dict) and e.get('code') == 'ai.badRequest', 'sam: a memory suggestion with lang fr → 400 ai.badRequest. Actual %d %r' % (st, e))

    st, _, e = jreq('POST', '/api/ai/send', {'chatId': None, 'text': QUESTION}, headers={'Cookie': KIM})
    ok(st == 403 and e.get('code') == 'ai.notAllowed', 'kim (no AI+ service): POST /api/ai/send → 403 ai.notAllowed. Actual %d %r' % (st, e))
    st, _, e = jreq('GET', '/api/ai/chats', headers={'Cookie': KIM})
    ok(st == 200 and e == {'chats': []}, 'kim: GET /api/ai/chats → 200 with no chats. Actual %d %r' % (st, e))
    st, _, e = jreq('GET', '/api/ai/admin/users', headers={'Cookie': KIM})
    ok(st == 403 and e.get('code') == 'ai.adminOnly', 'kim: an administrator route → 403 ai.adminOnly. Actual %d %r' % (st, e))

    def log_messages():
        st, _, lg = jreq('GET', '/api/logs')
        return [l['message'] for l in lg['lines']] if st == 200 else []

    st, _, e = jreq('GET', '/api/ai/chat?id=%d' % chat_id)
    ok(st == 404 and e.get('code') == 'ai.notFound', "admin: sam's chat through the user route → 404 (user routes are scoped). Actual %d" % st)
    st, _, au = jreq('GET', '/api/ai/admin/users')
    listed = {u['username']: u for u in au['users']} if st == 200 else {}
    ok('sam' in listed and listed['sam']['ai'] is True and listed['sam']['chats'] == 1 and 'kim' not in listed,
       'admin: the AI users list has sam (AI+ service, 1 chat) and not kim. Actual %r' % sorted(listed))
    ok(all(list(u.keys()) == ADMIN_ROW_KEYS for u in au['users']) and listed['sam']['suspended'] is False,
       'admin: the AI users rows carry suspended (false for sam). Actual %r' % listed.get('sam'))
    st, _, au = jreq('GET', '/api/ai/admin/user?id=%d' % SAM_ID)
    ok(st == 200 and [c['id'] for c in au['chats']] == [chat_id], "admin: sam's chats in the read-only view. Actual %d" % st)
    views = sum('viewed an AI chat of user' in l for l in log_messages())
    st, _, e = jreq('GET', '/api/ai/admin/chat?user=%d&id=%d' % (KIM_ID, chat_id))
    ok(st == 404, "admin: sam's chat named as kim's → 404. Actual %d" % st)
    st, _, e = jreq('GET', '/api/ai/admin/chat?user=%d&id=%d' % (SAM_ID, chat_id))
    ok(st == 200 and len(e['messages']) == count, "admin: sam's chat with sam's id → 200 (%d messages). Actual %d" % (count, st))
    lines = log_messages()
    ok(any('viewed the AI chats and memories of user "sam"' in l for l in lines), 'The System log has the audit line of the view of sam')
    ok(sum('viewed an AI chat of user' in l for l in lines) == views + 1, 'The refused pair wrote no audit line; the allowed view wrote one')

    st, _, e = jreq('POST', '/api/ai/send', {'chatId': chat_id, 'text': 'One more question'}, headers={'Cookie': SAM, 'Origin': 'https://evil.example.com'})
    ok(st == 403 and e.get('code') == 'auth.guard', 'sam: POST /api/ai/send with a foreign Origin → 403 auth.guard. Actual %d %r' % (st, e))

    # Free mode (administrators only, A12 / A13): a state of the administrator (POST /api/ai/free, freeMode in the status),
    # not of a chat. While it is on every send of that administrator, in any chat, is free; a `free` in a send body is ignored.
    def free(on, cookie=None):
        return jreq('POST', '/api/ai/free', {'free': on}, headers={'Cookie': cookie} if cookie else None)

    st, _, e = free(True)
    ok(st == 200 and e == {'free': True}, 'admin: POST /api/ai/free {"free": true} → 200 {"free": true}. Actual %d %r' % (st, e))
    st, _, ss = jreq('GET', '/api/ai/status')
    ok(st == 200 and ss.get('freeMode') is True, 'admin: GET /api/ai/status says freeMode true. Actual %r' % ss)
    fs = Stream('/api/ai/send', {'chatId': None, 'text': QUESTION, 'lang': 'en'}, COOKIE)
    fs.run()
    term = fs.terminal()
    ok(fs.status == 200 and fs.names()[:1] == ['start'] and fs.events[0][2].get('free') is True and term is not None and term[1] == 'done',
       'admin (free mode on): a new chat is answered in free mode (start.free true) and finishes. Actual %r %r' % (fs.names(), fs.raw[:200]))
    free_id = fs.events[0][2]['chatId']
    st, _, ac = jreq('GET', '/api/ai/chats')
    ok(st == 200 and [list(c.keys()) for c in ac['chats'] if c['id'] == free_id] == [CHAT_KEYS],
       'admin: GET /api/ai/chats lists that chat without a mode. Actual %r' % ac)
    st, _, e = free(False)
    ok(st == 200 and e == {'free': False}, 'admin: POST /api/ai/free {"free": false} → 200. Actual %d %r' % (st, e))
    fs = Stream('/api/ai/send', {'chatId': free_id, 'text': 'And one more?', 'lang': 'en', 'free': True}, COOKIE)
    fs.run()
    term = fs.terminal()
    ok(fs.status == 200 and fs.events and fs.events[0][2].get('free') is False and term is not None and term[1] == 'done',
       'admin (free mode off): the same chat is answered normally; a free in the body is ignored (start.free false). Actual %r' % fs.names())
    for bad in ({}, {'free': 'true'}, {'free': True, 'id': free_id}):
        st, _, e = jreq('POST', '/api/ai/free', bad)
        ok(st == 400 and isinstance(e, dict) and e.get('code') == 'ai.badRequest', 'admin: POST /api/ai/free %r → 400 ai.badRequest. Actual %d %r' % (bad, st, e))
    st, _, e = jreq('POST', '/api/ai/chats/free', {'id': free_id, 'free': True})
    ok(st == 404, 'The per-chat route POST /api/ai/chats/free is gone (404). Actual %d %r' % (st, e))
    st, _, e = free(True, SAM)
    ok(st == 403 and isinstance(e, dict) and e.get('code') == 'ai.adminOnly', 'sam (not an administrator): POST /api/ai/free → 403 ai.adminOnly. Actual %d %r' % (st, e))
    fs = Stream('/api/ai/send', {'chatId': chat_id, 'text': QUESTION, 'lang': 'en', 'free': True}, SAM)
    fs.run()
    term = fs.terminal()
    ok(fs.status == 200 and fs.events and fs.events[0][2].get('free') is False and term is not None and term[1] == 'done',
       'sam: a free in the send body is ignored (start.free false, a normal answer). Actual %r %r' % (fs.names(), fs.raw[:200]))
    st, _, ss = jreq('GET', '/api/ai/status', headers={'Cookie': SAM})
    ok(st == 200 and ss.get('freeMode') is False, 'sam: GET /api/ai/status says freeMode false. Actual %r' % ss)
    st, _, chats = jreq('GET', '/api/ai/chats', headers={'Cookie': SAM})
    ok(st == 200 and [c['id'] for c in chats['chats']] == [chat_id], 'sam still has the one chat. Actual %r' % chats)

    # -- A3 Suspend / Resume of sam's AI+ function (AI settings), and the AI+ service switched off --
    OFF_LINE = 'AI function turned off for user "sam" by admin "admin"'
    ON_LINE = 'AI function turned on for user "sam" by admin "admin"'

    def log_count(text):
        return sum(text in l for l in log_messages())

    st, _, e = jreq('POST', '/api/ai/admin/suspend', {'userId': SAM_ID}, headers={'Cookie': SAM})
    ok(st == 403 and e.get('code') == 'ai.adminOnly', 'sam: POST /api/ai/admin/suspend → 403 ai.adminOnly. Actual %d %r' % (st, e))
    st, _, e = jreq('POST', '/api/ai/admin/suspend', {'userId': ADMIN_ID})
    ok(st == 409 and e.get('code') == 'ai.adminAlways', 'Suspending an administrator → 409 ai.adminAlways. Actual %d %r' % (st, e))
    st, _, e = jreq('POST', '/api/ai/admin/suspend', {'userId': SAM_ID, 'reason': 'x'})
    ok(st == 400 and e.get('code') == 'ai.badRequest', 'A suspend body with another key → 400 ai.badRequest. Actual %d %r' % (st, e))
    st, _, r = jreq('POST', '/api/ai/admin/suspend', {'userId': SAM_ID})
    ok(st == 200 and isinstance(r, dict) and list(r.get('user', {}).keys()) == ADMIN_ROW_KEYS and r['user']['suspended'] is True
       and r['user']['username'] == 'sam', 'admin suspends sam: 200 {"user": AdminUserRow} with suspended true. Actual %d %r' % (st, r))
    st, _, e = jreq('POST', '/api/ai/send', {'chatId': chat_id, 'text': QUESTION, 'lang': 'en'}, headers={'Cookie': SAM})
    ok(st == 403 and e.get('code') == 'ai.suspended', 'sam (suspended): POST /api/ai/send → 403 ai.suspended. Actual %d %r' % (st, e))
    st, _, e = jreq('POST', '/api/ai/memories/suggest', {'messageId': asked['ja'], 'lang': 'ja'}, headers={'Cookie': SAM})
    ok(st == 403 and e.get('code') == 'ai.suspended', 'sam (suspended): a memory suggestion → 403 ai.suspended. Actual %d %r' % (st, e))
    st, _, chats = jreq('GET', '/api/ai/chats', headers={'Cookie': SAM})
    ok(st == 200 and [c['id'] for c in chats['chats']] == [chat_id], 'sam (suspended) still lists the chat. Actual %d %r' % (st, chats))
    st, _, ss = jreq('GET', '/api/ai/status', headers={'Cookie': SAM})
    ok(st == 200 and ss['suspended'] is True and ss['allowed'] is False and ss['enabled'] is True,
       'sam (suspended): /api/ai/status says suspended true, allowed false, enabled true. Actual %r' % ss)
    ok(user_rows()['sam']['aiSuspended'] is True, 'GET /api/users: sam aiSuspended true (the report after the suspend reached the backend at once)')
    st, _, au = jreq('GET', '/api/ai/admin/users')
    listed = {u['username']: u for u in au['users']} if st == 200 else {}
    ok(listed.get('sam', {}).get('suspended') is True and listed['sam']['ai'] is True,
       'AI settings still lists sam, with suspended true. Actual %r' % listed.get('sam'))
    ok(until(lambda: log_count(OFF_LINE) == 1, 5), 'The System log has the line %s' % OFF_LINE)
    st, _, r = jreq('POST', '/api/ai/admin/suspend', {'userId': SAM_ID})
    ok(st == 200 and r['user']['suspended'] is True, 'Suspending sam again → 200, unchanged. Actual %d' % st)
    time.sleep(1)
    ok(log_count(OFF_LINE) == 1, 'The unchanged suspend wrote no second line')
    st, _, r = jreq('POST', '/api/ai/admin/resume', {'userId': SAM_ID})
    ok(st == 200 and r['user']['suspended'] is False, 'admin resumes sam: 200 with suspended false. Actual %d %r' % (st, r))
    ok(until(lambda: log_count(ON_LINE) == 1, 5), 'The System log has the line %s' % ON_LINE)
    ok(user_rows()['sam']['aiSuspended'] is False, 'GET /api/users: sam aiSuspended false again')
    st, _, ss = jreq('GET', '/api/ai/status', headers={'Cookie': SAM})
    ok(st == 200 and ss['suspended'] is False and ss['allowed'] is True, 'sam (resumed): allowed again. Actual %r' % ss)

    # Suspending during a slow answer stops it at once
    slow = Stream('/api/ai/send', {'chatId': None, 'text': 'Please answer slowly [slow]', 'lang': 'en'}, SAM)
    slow.start()
    ok(slow.started.wait(30) and slow.names()[:1] == ['start'], 'sam: a slow answer has started. Actual %r %r' % (slow.names(), slow.raw[:200]))
    time.sleep(1)
    t_off = time.time()
    st, _, _ = jreq('POST', '/api/ai/admin/suspend', {'userId': SAM_ID})
    ok(st == 200, 'admin suspends sam during the answer. Actual %d' % st)
    slow.join(30)
    term = slow.terminal()
    ok(term is not None and term[1] == 'error' and term[2].get('code') == 'ai.suspended' and term[2].get('error') == MSG_AI_SUSPENDED
       and term[0] - t_off < 3,
       "sam's answer ends with error ai.suspended (%s) within 3 s. Actual %r after %.1f s"
       % (MSG_AI_SUSPENDED, term and term[1:], (term[0] - t_off) if term else -1))
    st, _, _ = jreq('POST', '/api/ai/admin/resume', {'userId': SAM_ID})
    ok(st == 200, 'admin resumes sam. Actual %d' % st)

    # The AI+ service switched off during a slow answer
    slow = Stream('/api/ai/send', {'chatId': None, 'text': 'Please answer slowly [slow]', 'lang': 'en'}, SAM)
    slow.start()
    ok(slow.started.wait(30) and slow.names()[:1] == ['start'], 'sam: a slow answer has started. Actual %r %r' % (slow.names(), slow.raw[:200]))
    t0 = time.time()
    while not slow.comments and time.time() - t0 < 19:
        time.sleep(0.2)
    ok(slow.comments, 'While the provider is silent, a keep-alive comment arrives through Nginx (after %.1f s)' % (time.time() - t0))
    st, _, _ = jreq('POST', '/api/users/update', {'id': SAM_ID, 'ai': False})
    t_off = time.time()
    ok(st == 200, 'admin switches the AI+ service off for sam during the answer. Actual %d' % st)
    slow.join(30)
    term = slow.terminal()
    ok(term is not None and term[1] == 'error' and term[2].get('code') == 'ai.notAllowed' and term[0] - t_off < 5,
       "sam's answer ends with error ai.notAllowed within 5 s. Actual %r after %.1f s" % (term and term[1:], (term[0] - t_off) if term else -1))
    st, _, e = jreq('POST', '/api/ai/send', {'chatId': None, 'text': QUESTION, 'lang': 'en'}, headers={'Cookie': SAM})
    ok(st == 403 and e.get('code') == 'ai.notAllowed', 'sam (AI+ service off): POST /api/ai/send → 403 ai.notAllowed. Actual %d %r' % (st, e))
    st, _, au = jreq('GET', '/api/ai/admin/users')
    ok(st == 200 and 'sam' not in [u['username'] for u in au['users']],
       'AI settings no longer lists sam once the AI+ service is off (the data stays). Actual %r' % [u['username'] for u in au.get('users', [])])
    st, _, chats = jreq('GET', '/api/ai/chats', headers={'Cookie': SAM})
    ok(st == 200 and len(chats['chats']) == 3, 'sam (AI+ service off) still lists all three chats. Actual %r' % chats)

    # A suspension survives switching the AI+ service off and on; only Resume ends it
    st, _, r = jreq('POST', '/api/ai/admin/suspend', {'userId': SAM_ID})
    ok(st == 200 and r['user']['suspended'] is True, 'admin suspends sam while the AI+ service is off (any plain user). Actual %d %r' % (st, r))
    st, _, _ = jreq('POST', '/api/users/update', {'id': SAM_ID, 'ai': True})
    ok(st == 200, 'admin switches the AI+ service on again for sam. Actual %d' % st)
    row = user_rows()['sam']
    ok(row['ai'] is True and row['aiSuspended'] is True, 'sam is still suspended after the AI+ service is on again. Actual %r' % row)
    st, _, au = jreq('GET', '/api/ai/admin/users')
    ok(st == 200 and [u['suspended'] for u in au['users'] if u['username'] == 'sam'] == [True],
       'AI settings lists sam again, still suspended. Actual %r' % au)
    st, _, e = jreq('POST', '/api/ai/send', {'chatId': None, 'text': QUESTION, 'lang': 'en'}, headers={'Cookie': SAM})
    ok(st == 403 and e.get('code') == 'ai.suspended', 'sam (service on, still suspended): POST /api/ai/send → 403 ai.suspended. Actual %d %r' % (st, e))
    st, _, _ = jreq('POST', '/api/ai/admin/resume', {'userId': SAM_ID})
    ok(st == 200, 'admin resumes sam. Actual %d' % st)
    st, _, _ = jreq('POST', '/api/users/update', {'id': SAM_ID, 'ai': False})
    ok(st == 200, 'admin switches the AI+ service off for sam again. Actual %d' % st)

    st, _, e = jreq('POST', '/api/ai/chats/delete', {'all': True}, headers={'Cookie': SAM})
    ok(st == 200 and e.get('deleted') == 3, 'sam (AI+ service off) deletes all own chats → deleted 3. Actual %d %r' % (st, e))
    st, _, _ = jreq('POST', '/api/users/delete', {'id': SAM_ID})
    ok(st == 200, 'admin deletes sam. Actual %d' % st)
    st, _, au = jreq('GET', '/api/ai/admin/users')
    ok(st == 200 and 'sam' not in [u['username'] for u in au['users']], 'The AI users list does not have sam')
    st, _, usage = jreq('GET', '/api/ai/admin/usage?days=30')
    ok(st == 200 and any(u['userId'] is None for u in usage['byUser']),
       "The usage keeps sam's calls anonymised (a byUser row with userId null). Actual %r" % (usage.get('byUser') if isinstance(usage, dict) else usage))

    # -- A4 Nothing leaks the key, the question or the answer --
    lines = log_messages()
    ok(any(l.startswith('AI: ') for l in lines), 'AI service lines appear in the System log with the "AI: " prefix')
    for what, text in (('the made-up key', DEMO_KEY), ('the key prefix', 'sk-demo'), ('the question', QUESTION), ('the answer', ANSWER_WORDS)):
        ok(not any(text in l for l in lines), 'The System log does not contain %s' % what)
        for svc_name in ('ai-service', 'backend', 'frontend'):
            ok(text not in logs_of(svc_name), 'The %s container log does not contain %s' % (svc_name, what))
    ok(not any(b'sk-demo' in b for b in BODIES), 'No response of the run contained the made-up key (%d responses)' % len(BODIES))

    # -- 6 Persistence --
    dc('rm', '-sf', 'frontend', 'backend', 'ai-service')
    ok(docker('volume', 'inspect', PROJECT + '_data').returncode == 0 and docker('volume', 'inspect', PROJECT + '_ai_data').returncode == 0,
       'The volumes still exist after deleting the containers')
    dc('up', '-d')
    ok(wait_healthy(services), 'The three containers are healthy again after recreating with the same volumes')
    BACKEND, AI, FRONTEND = cid('backend'), cid('ai-service'), cid('frontend')
    ok(bank_size() == 5, 'Question bank still has 5 questions after recreation (data is in the volume)')
    st, _, p = jreq('GET', '/api/progress')
    ok(len(p['history']) == 2, 'Result history still has 2 rounds after recreation (the session is in the volume too)')
    st, _, cfg = jreq('GET', '/api/ai/admin/settings')
    ok(st == 200 and cfg['enabled'] is True and cfg['keyProblem'] is None and cfg['keys']['apiKey']['unreadable'] is False,
       'The AI settings survive too: still enabled, the saved key still readable (key file in the AI volume). Actual %d' % st)
    ok(until(lambda: users_list().get('aiAvailable') is True, 40),
       'After the recreation ai-service reports to the new backend: AI+ is available again (GET /api/users aiAvailable true)')

    # The app without ai-service
    t0 = time.time()
    docker('stop', AI)
    dt = time.time() - t0
    ok(dt < 9, 'docker stop ai-service took %.1f s (<9)' % dt)
    t_stop = time.time()
    state = until(lambda: svc_when(lambda a: a.get('stopped')), 3) or admin_state()
    svc = state.get('aiService') or {}
    ok(state.get('aiAvailable') is False and svc.get('stopped') is True and svc.get('connected') is False and svc.get('enabled') is False,
       'Within 3 s of docker stop the backend knows (the stopping report): aiAvailable false, aiService stopped, not connected. '
       'Actual %r after %.1f s' % (svc, time.time() - t_stop))
    ok(users_list().get('aiAvailable') is False, 'GET /api/users: aiAvailable false while ai-service is stopped')
    st, _, e = jreq('POST', '/api/users/update', {'id': KIM_ID, 'ai': False})
    ok(st == 409 and e == REFUSED, "ai-service stopped: switching kim's AI+ service → 409 ai.serviceOff. Actual %d %r" % (st, e))
    st, _, info = jreq('GET', '/api/info')
    ok(st == 200 and list(info.keys()) == ['version', 'arch', 'canShutdown'], 'ai-service stopped: /api/info still 200 with the same keys')
    st, _, e = jreq('GET', '/api/ai/status')
    ok(st == 502 and isinstance(e, dict) and e.get('code') == 'ai.unavailable', 'ai-service stopped: /api/ai/status → 502 ai.unavailable. Actual %d %r' % (st, e))
    docker('start', AI)
    t_start = time.time()
    ok(until(lambda: users_list().get('aiAvailable') is True, 15),
       'After docker start AI+ is available again within 15 s (%.1f s)' % (time.time() - t_start))
    ok(wait_healthy(['ai-service']), 'ai-service is healthy again after docker start')
    st, _, s2 = jreq('GET', '/api/ai/status')
    ok(st == 200 and s2['enabled'] is True, 'After the restart AI is still enabled. Actual %d' % st)
    # -- 8 SQLite WAL and the system log (v0.7) --
    def backend_log():
        return logs_of('backend')
    ok('SQLite journal mode: wal' in backend_log(), 'The backend runs SQLite in WAL mode (startup log)')
    st, _, lg = jreq('GET', '/api/logs')
    ok(st == 200 and list(lg.keys()) == ['lines', 'last', 'dropped', 'boot'] and any('SQLite journal mode: wal' in l['message'] for l in lg['lines']),
       'GET /api/logs through Nginx: the administrator sees the startup lines. Actual %d' % st)
    st, _, lg2 = jreq('GET', '/api/logs?after=%d' % lg['last'])
    ok(st == 200 and all(l['seq'] > lg['last'] for l in lg2['lines']), 'GET /api/logs?after=<last> returns only newer lines')
    r = docker('exec', BACKEND, 'xexam-backend', 'reset-password', 'admin')
    m = re.search(r"New password for 'admin': (\S+)", r.stdout)
    ok(r.returncode == 0 and m is not None, 'reset-password runs in the container while the server is up (exit %d)' % r.returncode)
    st, _, _ = jreq('GET', '/api/progress')
    ok(st == 401, 'reset-password signed the administrator out everywhere. Actual %d' % st)
    COOKIE = None
    st, h, j = jreq('POST', '/api/auth/login', {'username': 'admin', 'password': m.group(1)}, headers={'Origin': 'http://localhost:%d' % PORT})
    ok(st == 200 and j['user']['mustChangePassword'] is True, 'The new password from reset-password signs in and must be changed. Actual %d' % st)
    COOKIE = session_cookie(h)
    st, h, j = jreq('POST', '/api/auth/password', {'password': NEW_PASSWORD + '-2'}, headers={'Origin': 'http://localhost:%d' % PORT})
    ok(st == 200, 'New password saved after the reset. Actual %d' % st)
    COOKIE = session_cookie(h)
    st, _, p = jreq('GET', '/api/progress')
    ok(st == 200 and len(p['history']) == 2, 'The study records are unchanged after reset-password')

    t0 = time.time()
    docker('stop', BACKEND)
    dt = time.time() - t0
    ok(dt < 9, 'docker stop backend took %.1f s (<9)' % dt)
    ok('WAL checkpoint at shutdown' in backend_log(), 'docker stop runs the WAL checkpoint (shutdown log)')
    image = docker('inspect', '-f', '{{.Image}}', BACKEND).stdout.strip()
    r = docker('run', '--rm', '--network', 'none', '-v', PROJECT + '_data:/data', '--entrypoint', 'sh', image, '-c',
               'test -s /data/xexam.db && { test -s /data/xexam.db-wal && echo WAL_HAS_CONTENT || echo WAL_EMPTY; }')
    ok(r.stdout.strip() == 'WAL_EMPTY', 'After docker stop the volume holds xexam.db and no -wal content. Actual %r' % (r.stdout.strip() or r.stderr.strip()[-200:]))

    # -- 9 Volumes: each database stays in its own volume; the link volume holds only the token --
    ai_vol = volume_report(PROJECT + '_ai_data', (
        "import json, os, stat\n"
        "k = os.stat('/v/secret.key') if os.path.exists('/v/secret.key') else None\n"
        "print(json.dumps({'files': sorted(os.listdir('/v')), 'key_mode': k and oct(stat.S_IMODE(k.st_mode)), 'key_uid': k and k.st_uid}))"))
    ok('xexam-ai.db' in ai_vol.get('files', []) and ai_vol.get('key_mode') == '0o600' and ai_vol.get('key_uid') == 10001,
       'The AI volume holds xexam-ai.db and secret.key (mode 600, uid 10001). Actual %r' % ai_vol)
    main_vol = volume_report(PROJECT + '_data', (
        "import json, os, sqlite3\n"
        "c = sqlite3.connect('file:/v/xexam.db?immutable=1', uri=True)\n"
        "tables = [r[0] for r in c.execute(\"SELECT name FROM sqlite_master WHERE type = 'table'\")]\n"
        "cols = [r[1] for r in c.execute('PRAGMA table_info(users)')]\n"
        "print(json.dumps({'files': sorted(os.listdir('/v')), 'tables': tables, 'user_columns': cols}))"))
    ok('secret.key' not in main_vol.get('files', ['secret.key']) and not any(n.startswith('xexam-ai.db') for n in main_vol.get('files', [])),
       'The main volume holds no AI key file and no AI database. Actual %r' % main_vol.get('files', main_vol))
    ok('chats' not in main_vol.get('tables', ['chats']) and 'ai_enabled' in main_vol.get('user_columns', []) and 'uid' in main_vol.get('user_columns', []),
       'The main database has no AI table; users has ai_enabled and uid. Actual %r' % main_vol)
    link_vol = volume_report(PROJECT + '_link', (
        "import json, os, stat\n"
        "t = os.stat('/v/internal.token') if os.path.exists('/v/internal.token') else None\n"
        "print(json.dumps({'files': sorted(os.listdir('/v')), 'mode': t and oct(stat.S_IMODE(t.st_mode))}))"))
    ok(link_vol.get('files') == ['internal.token'] and link_vol.get('mode') == '0o600',
       'The link volume holds only internal.token (mode 600). Actual %r' % link_vol)

finally:
    cleanup()

print('\n'.join(results))
print('\nAll %d checks passed' % len(results))
