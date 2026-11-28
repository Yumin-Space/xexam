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
                       first one, with the password change and the display name), an administrator who creates lee (administrator) and updates
                       kim (disabled), and a made-up username without an account that is guessed until it locks. Then it
                       waits until ai-service has reported to the restarted backend (AI+ available again, at most 40 s).
                       Returns the administrator's session cookie for the browser.
categorize(admin)      (v3.1) creates the two made-up bank categories `Science` (holding General knowledge) and `Everyday life`
                       (holding Kitchen basics) and moves the two demo banks into them; idempotent. prepare() calls it right after
                       the two imports, so alex (granted General knowledge only) sees only `Science`; any other bank stays
                       uncategorised (the walk's 51-question bank).
seed_exams(base, cookie)  three earlier exams of the administrator, so the records pages look used.

ai_chat(base, cookie, text, lang)  one AI chat through the API, answered in `lang` (en, zh or ja: the Settings language
                       the AI Chat page would send); the answer is read to its end; returns the chat id.
delete_ai_chats(base, cookie) deletes every AI chat of the administrator again.

Since v3.1 every Session sends a stable device cookie (xexam_did) derived from the login and a plain desktop User-Agent, so the
many sign-ins of one made-up user land on ONE device row ("Signed-in devices" stays readable); sign_in(..., device='tag') picks
another device of the same user (a second browser).

Every password, key, address and model here is a made-up demo value for an instance that is deleted at the end of the
run; the screenshots show only these.
"""
import base64
import hashlib
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
# v2.6: every user signs in with an e-mail and has a display name ("username" in the UI). The tools keep the short role names
# (sam, alex, ...) as handles in code; login_of() turns a handle into the e-mail login, name_of() into the display name.
# example.com is the reserved sample domain; 'admin' is the login of a fresh install and stays as it is.
LOGINS = {'sam': 'sam@example.com', 'alex': 'alex@example.com', 'kim': 'kim@example.com', 'lee': 'lee@example.com',
          'riley': 'riley@example.com', 'jordan': 'jordan@example.com'}
NAMES = {'admin': 'Admin', 'sam': 'Sam', 'alex': 'Alex', 'kim': 'Kim', 'lee': 'Lee', 'riley': 'Riley', 'jordan': 'Jordan'}
NAME_MAX = 20                               # the server rule: 1..20 characters, no @, no control characters
DID_COOKIE = 'xexam_did'                    # the device cookie of v3.1 (43 base64url characters)
# A plain, made-up desktop browser: the device list names it by user agent (never the Python default)
SEED_UA = ('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36')
# v3.1: the two made-up bank categories and the demo banks they hold
CATEGORY_OF = demo_banks.CATEGORY_OF
MEMORIES = ['I am preparing for a kitchen safety course.', 'Prefers short answers with one example.']


def login_of(handle):
    """The login (e-mail) of a handle such as 'sam'; anything else (admin, a full e-mail, a made-up name) is unchanged."""
    return LOGINS.get(handle, handle)


def name_of(handle):
    """The display name a tool gives a user at the first sign-in: the table above, else the part before the @ (20 chars)."""
    if handle in NAMES:
        return NAMES[handle]
    for short, login in LOGINS.items():
        if handle == login:
            return NAMES[short]
    return handle.split('@', 1)[0][:NAME_MAX]


def device_cookie(login, tag='seed'):
    """A stable, made-up xexam_did value for (login, tag): 32 bytes of a SHA-256 as 43 base64url characters (no padding)."""
    raw = hashlib.sha256(('xexam-demo-device|%s|%s' % (login, tag)).encode('utf-8')).digest()
    return base64.urlsafe_b64encode(raw).decode('ascii').rstrip('=')


class Session:
    """One signed-in browser, as far as the API is concerned (cookies and the Origin header of the page)."""

    def __init__(self, base):
        self.base = base.rstrip('/')
        self.cookie = {}

    def call(self, path, body=None, raw=None, headers=None):
        h = {'Origin': self.base, 'User-Agent': SEED_UA}
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

    def sign_in(self, username, password, device=None):
        """Signs in as a handle or a login and passes the first-sign-in gate: a new password (with the display name when
        the user has none) or just the display name (an existing user that still lacks one). The device cookie is the stable
        one of this login, or the one of `device` (a tag: another browser of the same user)."""
        if device is not None:
            self.cookie[DID_COOKIE] = device_cookie(login_of(username), device)
        else:
            self.cookie.setdefault(DID_COOKIE, device_cookie(login_of(username)))
        state = self.call('/api/auth/login', {'username': login_of(username), 'password': password})
        user = state['user']
        if user.get('mustChangePassword'):
            body = {'password': DEMO_PASSWORD}
            if user.get('needsName'):
                body['displayName'] = name_of(username)
            self.call('/api/auth/password', body)
        elif user.get('needsName'):
            self.call('/api/auth/name', {'displayName': name_of(username)})
        return self

    def create_user(self, handle, **fields):
        """POST /api/users/create for a handle (its e-mail login, the initial password, role user unless given)."""
        body = {'username': login_of(handle), 'password': INITIAL, 'role': 'user'}
        body.update(fields)
        return self.call('/api/users/create', body)

    def import_new(self, path):
        with open(path, 'rb') as fh:
            data = fh.read()
        name = os.path.basename(path)
        pv = self.call('/api/upload', raw=data, headers={'Content-Type': 'application/octet-stream',
                                                         'X-Filename': urllib.parse.quote(name), 'X-Bank-Target': 'new'})
        self.call('/api/import/confirm', {'token': pv['token']})

    def user_id(self, username):
        login = login_of(username)
        return next(u['id'] for u in self.call('/api/users')['users'] if u['username'] == login)


def categorize(admin):
    """Creates the categories `Science` and `Everyday life` (when missing) and puts General knowledge in the first and
    Kitchen basics in the second; every other bank is left as it is. Idempotent. `admin` is a signed-in Session."""
    answer = admin.call('/api/banks')
    cats = {c['name']: c['id'] for c in answer.get('categories', [])}
    for name in dict.fromkeys(CATEGORY_OF.values()):
        if name not in cats:
            answer = admin.call('/api/banks/categories', {'op': 'create', 'name': name})
            cats = {c['name']: c['id'] for c in answer.get('categories', [])}
    banks = admin.call('/api/banks')['banks']
    for bank_name, cat_name in CATEGORY_OF.items():
        for b in banks:
            if b['name'].startswith(bank_name) and b.get('categoryId') != cats[cat_name]:
                admin.call('/api/banks/category', {'ids': [b['id']], 'categoryId': cats[cat_name]})


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

    admin.create_user('sam', banks='all', canManageBanks=True)
    Session(base).sign_in('sam', INITIAL).import_new(banks[demo_banks.KITCHEN])
    for name in ('alex', 'kim'):
        admin.create_user(name, banks=[general])
    categorize(admin)      # v3.1: Science holds General knowledge, Everyday life holds Kitchen basics

    # Two practice records of the administrator (the exams come from seed_exams and from the exam scene itself)
    bank = admin.call('/api/bank')['questions'][:6]
    # v2.6: five different questions are wrong at least once, so the most-missed board of the Home shows all five places
    for wrong, seconds in (({0, 1, 2, 3, 4}, 240), ({0, 1, 2}, 205)):
        admin.call('/api/attempt', {'results': _results(bank, wrong), 'sheets': sorted({q['sheet'] for q in bank}),
                                    'mode': 'all', 'seconds': seconds, 'kind': 'practice'})
    setup_ai(admin, stub_base)
    # The AI+ feature of a user can be switched only while AI+ is available (ai-service connected, the AI+ feature on;
    # the enable call above answers after ai-service has reported that): sam and alex get it, then alex's AI+ feature
    # is paused in AI settings
    for name in ('sam', 'alex'):
        admin.call('/api/users/update', {'id': admin.user_id(name), 'ai': True})
    # One made-up question of sam, so that the AI+ users list shows a calls line and the usage table a Today column with
    # more than the administrator in it
    try:
        sam = Session(base).sign_in('sam', DEMO_PASSWORD)     # the first sign-in above chose it
        ai_chat(base, sam.cookie[COOKIE], 'What is a good way to plan a week of study?', 'en')
    except Exception as e:  # cosmetic only: the images do not depend on it
        print("  warning: the AI question of sam was not asked: %s" % e)
    row = admin.call('/api/ai/admin/suspend', {'userId': admin.user_id('alex')})['user']
    print('  AI+ feature: sam, alex (AI+ feature of %s: %s)'
          % (row['username'], 'paused' if row['suspended'] else 'NOT PAUSED'))


def setup_ai(admin, stub_base):
    """AI settings as an administrator would fill them in (v1.3): one connection with the made-up provider and key (the
    save lists the models and tests one), a second connection that reuses the first one's key, the default model picked,
    the AI+ feature switched on; then two memories of the administrator."""
    saved = admin.call('/api/ai/admin/connections/save', {
        'name': 'Demo provider', 'provider': 'openai', 'baseUrl': stub_base, 'format': 'openai', 'gatewayAuth': 'bearer',
        'apiKey': AI_KEY})
    if not saved.get('ok'):
        raise RuntimeError('the made-up AI provider was refused: %r' % saved)
    conn = saved['connectionId']
    listing = admin.call('/api/ai/admin/models?refresh=1')
    ids = [m['id'] for c in listing.get('connections', []) if c['id'] == conn for m in c.get('models', [])]
    if AI_MODEL not in ids:
        raise RuntimeError('the made-up AI provider did not list %s: %r' % (AI_MODEL, listing))
    host = stub_base.split('//', 1)[1].split('/', 1)[0]
    second = admin.call('/api/ai/admin/connections/save', {
        'name': 'Demo backup', 'provider': 'openai', 'baseUrl': stub_base, 'format': 'openai', 'gatewayAuth': 'bearer',
        'reuseKeysFrom': conn, 'confirmHost': host})
    if not second.get('ok'):
        print('  warning: the second made-up connection was not saved: %r' % second)
    en = admin.call('/api/ai/admin/enable', {'enabled': True, 'default': {'connectionId': conn, 'model': AI_MODEL}})
    if not en.get('ok', True):
        raise RuntimeError('AI+ could not be switched on: %r' % en)
    for text in MEMORIES:
        admin.call('/api/ai/memories/add', {'content': text, 'source': 'manual'})
    print('  AI: enabled with %s (connection %s), %d memories' % (AI_MODEL, conn, len(MEMORIES)))


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


def wait_for_ai_room(base, cookie, need, most=75):
    """Waits (at most `most` seconds) until the signed-in user may make `need` more AI calls within the per-minute cap
    (GET /api/ai/status, field calls), so a scene never shows the cap callout because the scenes before it used the
    minute's calls. Returns the seconds waited."""
    s = Session(base)
    s.cookie[COOKIE] = cookie
    t0 = time.time()
    while True:
        calls = s.call('/api/ai/status').get('calls') or {}
        used, caps = calls.get('used') or {}, calls.get('caps') or {}
        room = min(caps.get(k, 10 ** 6) - used.get(k, 0) for k in ('minute', 'hour', 'day')) if caps else need
        if room >= need or time.time() - t0 >= most:
            if room < need:
                print('  warning: only %d AI calls left within the caps' % room)
            return round(time.time() - t0)
        time.sleep(3)


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
    admin.create_user('lee', role='admin')
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
