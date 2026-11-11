# -*- coding: utf-8 -*-
"""Fills a fresh, throw-away Xexam instance with the demo data of the screenshots, through the public API only.

prepare(base, banks, stub_base)
                       before the backend restart: the default administrator chooses a demo password and imports
                       "General knowledge"; sam (user, bank manager) imports "Kitchen basics" as its owner; alex and kim
                       (users who see only "General knowledge") are added; the administrator gets two practice records;
                       AI is set up with the made-up provider at `stub_base` (key, model, test, the AI+ feature on) and
                       the administrator saves two memories. Only then (the AI+ feature of a user can be switched only
                       while AI+ is available) sam and alex get the AI+ feature and alex's AI+ feature is paused, so
                       Users and AI settings show a bright and a dark badge, an Active and a Paused row. No AI chat is
                       seeded (the AI Chat scene makes its own).
activity(base)         after the restart, so the System log page shows everyday lines with usernames: sign-ins (alex's
                       first one, with the password change), an administrator who creates lee (administrator) and updates
                       kim (disabled), and a made-up username without an account that is guessed until it locks. Then it
                       waits until ai-service has reported to the restarted backend (AI+ available again, at most 40 s).
                       Returns the administrator's session cookie for the browser.
seed_exams(base, cookie)  three earlier exams of the administrator, so the records pages look used.

ai_chat(base, cookie, text, lang)  one AI chat through the API, answered in `lang` (en, zh or ja: the Settings language
                       the AI Chat page would send); the answer is read to its end; returns the chat id.
delete_ai_chats(base, cookie) deletes every AI chat of the administrator again.

Every password, key, address and model here is a made-up demo value for an instance that is deleted at the end of the
run; the screenshots show only these.
"""
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request

import demo_banks

ADMIN = 'admin'
DEFAULT_PASSWORD = 'admin'                  # what a fresh data directory starts with (it must be changed at once)
DEMO_PASSWORD = 'demo-password-2026'
INITIAL = 'demo-initial-2026'               # the initial password an administrator gives a new user
COOKIE = 'xexam_session'
# A made-up key that only the stub provider (tests/integration/ai_stub.py) accepts; AI settings shows it as "...9c21"
AI_KEY = 'sk-demo-shots-7f3a9c21'
AI_MODEL = 'demo-chat-small'
MEMORIES = ['I am preparing for a kitchen safety course.', 'Prefers short answers with one example.']


class Session:
    """One signed-in browser, as far as the API is concerned (cookies and the Origin header of the page)."""

    def __init__(self, base):
        self.base = base.rstrip('/')
        self.cookie = {}

    def call(self, path, body=None, raw=None, headers=None):
        h = {'Origin': self.base}
        if self.cookie:
            h['Cookie'] = '; '.join('%s=%s' % kv for kv in self.cookie.items())
        if raw is not None:
            h.update(headers or {})
            req = urllib.request.Request(self.base + path, data=raw, method='POST', headers=h)
        elif body is not None:
            h['Content-Type'] = 'application/json'
            req = urllib.request.Request(self.base + path, data=json.dumps(body).encode(), method='POST', headers=h)
        else:
            req = urllib.request.Request(self.base + path, headers=h)
        with urllib.request.urlopen(req, timeout=60) as r:
            for v in r.headers.get_all('Set-Cookie') or []:
                name, _, rest = v.partition('=')
                self.cookie[name.strip()] = rest.split(';', 1)[0]
            return json.load(r)

    def sign_in(self, username, password):
        state = self.call('/api/auth/login', {'username': username, 'password': password})
        if state['user'].get('mustChangePassword'):
            self.call('/api/auth/password', {'password': DEMO_PASSWORD})
        return self

    def import_new(self, path):
        with open(path, 'rb') as fh:
            data = fh.read()
        name = os.path.basename(path)
        pv = self.call('/api/upload', raw=data, headers={'Content-Type': 'application/octet-stream',
                                                         'X-Filename': urllib.parse.quote(name), 'X-Bank-Target': 'new'})
        self.call('/api/import/confirm', {'token': pv['token']})

    def user_id(self, username):
        return next(u['id'] for u in self.call('/api/users')['users'] if u['username'] == username)


def _results(questions, wrong):
    """Per-question answers for POST /api/attempt: right ones, and a wrong pick for the indexes in `wrong`."""
    out = []
    for i, q in enumerate(questions):
        ok = i not in wrong
        if q['kind'] == 'hotspot':
            ans = list(q['answer']) if ok else [(q['answer'][0] + 1) % len(q['items'][0]['choices'])] + list(q['answer'][1:])
        else:
            ans = list(q['answer']) if ok else [next(o['key'] for o in q['options'] if o['key'] not in q['answer'])]
        out.append({'id': q['id'], 'correct': ok, 'answer': ans})
    return out


def prepare(base, banks, stub_base):
    """`banks`: {file name: path} from demo_banks.build(); `stub_base`: the base URL of the made-up AI provider as the AI
    service reaches it."""
    admin = Session(base).sign_in(ADMIN, DEFAULT_PASSWORD)
    admin.import_new(banks[demo_banks.GENERAL])
    general = next(b['id'] for b in admin.call('/api/banks')['banks'] if b['name'].startswith('General'))

    admin.call('/api/users/create', {'username': 'sam', 'password': INITIAL, 'role': 'user', 'banks': 'all',
                                     'canManageBanks': True})
    Session(base).sign_in('sam', INITIAL).import_new(banks[demo_banks.KITCHEN])
    for name in ('alex', 'kim'):
        admin.call('/api/users/create', {'username': name, 'password': INITIAL, 'role': 'user', 'banks': [general]})

    # Two practice records of the administrator (the exams come from seed_exams and from the exam scene itself)
    bank = admin.call('/api/bank')['questions'][:6]
    for wrong, seconds in (({0, 2}, 240), ({2}, 205)):
        admin.call('/api/attempt', {'results': _results(bank, wrong), 'sheets': sorted({q['sheet'] for q in bank}),
                                    'mode': 'all', 'seconds': seconds, 'kind': 'practice'})
    setup_ai(admin, stub_base)
    # The AI+ feature of a user can be switched only while AI+ is available (ai-service connected, the AI+ feature on;
    # the enable call above answers after ai-service has reported that): sam and alex get it, then alex's AI+ feature
    # is paused in AI settings
    for name in ('sam', 'alex'):
        admin.call('/api/users/update', {'id': admin.user_id(name), 'ai': True})
    row = admin.call('/api/ai/admin/suspend', {'userId': admin.user_id('alex')})['user']
    print('  AI+ feature: sam, alex (AI+ feature of %s: %s)'
          % (row['username'], 'paused' if row['suspended'] else 'NOT PAUSED'))


def setup_ai(admin, stub_base):
    """AI settings as an administrator would fill them in: provider and key, the models listed, the model saved, the test,
    the AI+ feature switched on; then two memories of the administrator."""
    admin.call('/api/ai/admin/settings', {'provider': 'openai', 'baseUrl': stub_base, 'apiKey': AI_KEY})
    models = admin.call('/api/ai/admin/models', {})
    if AI_MODEL not in models.get('models', []):
        raise RuntimeError('the made-up AI provider did not list %s: %r' % (AI_MODEL, models))
    admin.call('/api/ai/admin/settings', {'model': AI_MODEL})
    test = admin.call('/api/ai/admin/verify', {})
    if not test.get('ok'):
        raise RuntimeError('the AI test call failed: %r' % test)
    admin.call('/api/ai/admin/enable', {'enabled': True})
    for text in MEMORIES:
        admin.call('/api/ai/memories/add', {'content': text, 'source': 'manual'})
    print('  AI: enabled with %s (test %s ms), %d memories' % (AI_MODEL, test.get('latencyMs'), len(MEMORIES)))


def ai_chat(base, cookie, text, lang):
    """Starts a new AI chat with `text` through the API, answered in `lang` (en, zh or ja), and reads the streamed
    answer to its end; returns the chat id."""
    s = Session(base)
    s.cookie[COOKIE] = cookie
    payload = {'chatId': None, 'text': text, 'lang': lang}
    req = urllib.request.Request(s.base + '/api/ai/send', data=json.dumps(payload).encode(),
                                 method='POST', headers={'Origin': s.base, 'Content-Type': 'application/json',
                                                         'Cookie': '%s=%s' % (COOKIE, cookie)})
    with urllib.request.urlopen(req, timeout=120) as r:
        body = r.read().decode('utf-8')
    events = []
    for block in body.replace('\r\n', '\n').split('\n\n'):
        name = next((l[6:].strip() for l in block.split('\n') if l.startswith('event:')), 'message')
        data = '\n'.join(l[5:].lstrip() for l in block.split('\n') if l.startswith('data:'))
        if data:
            events.append((name, json.loads(data)))
    if not events or events[0][0] != 'start' or events[-1][0] != 'done':
        raise RuntimeError('the AI answer did not finish: %r' % [n for n, _ in events])
    return events[0][1]['chatId']


def set_free(base, cookie, free):
    """Turns the signed-in administrator's AI free mode on or off (POST /api/ai/free); returns the mode now stored."""
    s = Session(base)
    s.cookie[COOKIE] = cookie
    return s.call('/api/ai/free', {'free': bool(free)}).get('free')


def delete_ai_chats(base, cookie):
    s = Session(base)
    s.cookie[COOKIE] = cookie
    return s.call('/api/ai/chats/delete', {'all': True}).get('deleted', 0)


def activity(base, pause=1.0):
    """Run right after the backend restarted (its in-memory log is empty then). Returns the administrator's cookie."""
    admin = Session(base).sign_in(ADMIN, DEMO_PASSWORD)
    time.sleep(pause)
    Session(base).sign_in('sam', DEMO_PASSWORD)
    time.sleep(pause)
    Session(base).sign_in('alex', INITIAL)          # alex's first sign-in: chooses the demo password
    time.sleep(pause)
    admin.call('/api/users/create', {'username': 'lee', 'password': INITIAL, 'role': 'admin'})
    time.sleep(pause)
    admin.call('/api/users/update', {'id': admin.user_id('kim'), 'disabled': True})
    time.sleep(pause)
    codes = []
    for i in range(6):                              # a made-up username without an account: guessed until it locks
        try:
            Session(base).sign_in('guest', 'guess-%d' % i)
            codes.append(200)
        except urllib.error.HTTPError as e:
            codes.append(e.code)
        time.sleep(0.4)
    print('  made-up username "guest": answers %s' % ' '.join(map(str, codes)))
    waited = wait_ai_available(admin)
    print('  AI+ available again %.0f s after the activity' % waited)
    return admin.cookie[COOKIE]


def wait_ai_available(admin, seconds=40):
    """Waits until GET /api/users says aiAvailable: after a restart the backend knows nothing about ai-service until its
    next report (every 25 s; 5 s after a failed one). Returns the seconds waited."""
    started = time.time()
    while True:
        if admin.call('/api/users').get('aiAvailable') is True:
            return time.time() - started
        if time.time() - started >= seconds:
            raise RuntimeError('AI+ was not available again %d s after the activity that follows the backend restart '
                               '(GET /api/users aiAvailable stayed false)' % seconds)
        time.sleep(0.5)


def seed_exams(base, cookie):
    """Three earlier exams of the administrator over every question it sees (skipped if it already has an exam)."""
    admin = Session(base)
    admin.cookie[COOKIE] = cookie
    if any(r.get('kind', 'exam') == 'exam' for r in admin.call('/api/records')['records']):
        return
    bank = admin.call('/api/bank')['questions']
    for wrong, seconds in (({1, 5, 8}, 512), ({1, 8}, 455), ({8}, 401)):
        admin.call('/api/attempt', {'results': _results(bank, wrong), 'sheets': sorted({q['sheet'] for q in bank}),
                                    'mode': 'all', 'seconds': seconds, 'kind': 'exam'})


def record_ids(base, cookie):
    s = Session(base)
    s.cookie[COOKIE] = cookie
    return {r['id'] for r in s.call('/api/records')['records']}


def delete_records(base, cookie, ids):
    """Deletes the records a language run added, so every language starts from the same records."""
    s = Session(base)
    s.cookie[COOKIE] = cookie
    for rid in sorted(ids):
        s.call('/api/records/delete', {'id': rid})
