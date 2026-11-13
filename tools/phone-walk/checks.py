# -*- coding: utf-8 -*-
"""The checks of the phone walk (spec 5.4). walk.py calls `run(walk)` (see README.md, "Contract with checks.py"); the return
value is the exit code: 1 when a P1 check failed, else 0.

Every check has an ID (W-SH-1 ...), a page, a priority (P1 unless marked P2 below), and runs on the dimensions of the
spec's "Run dimensions": the default run is zh / light at the sizes the row names (360x740, 390x844, 844x390 unless the
row names others); the text-fit checks (W-QZ-2, W-QZ-9, W-HM-1, W-CH-2, W-US-1, W-AS-1, W-AS-2, W-AS-3) run zh, en and ja; the contrast check runs
light and dark; `--full` runs every check on the whole matrix x 3 languages x 2 themes; --langs / --widths / --themes
override. The in-page measurements are in checks.js (window.__walk.*). Elements are found only through the test hooks of
spec 2.12 f (.qbar, .pfoot, [data-slot], [data-act], .stem, .aip-panel*, [data-aip-dock], .rv, .home-go, .topbar,
.ai-composer, .ai-conv-head, .ai-scroll, #toast, .nav-pop, dialog .acts, .ibtn) or by their role.

Output: report.json (one entry per check run: id, page, state, user, size, lang, theme, pass, detail, screenshot,
priority) and the screenshots `<id>-<page>-<user>-<lang>-<theme>-<w>x<h>.png` (PNG metadata stripped) in `walk.out`.
Only made-up values exist; nothing prints keys. The administrator's AI request caps are raised for the walk (so the AI
checks do not trip them by accident) and put back to the defaults at the end; W-CAP-* lower them on purpose.
"""
import json
import os
import re
import sys
import time
import traceback
import urllib.request
from collections import OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
SHOTS = os.path.join(os.path.dirname(HERE), 'screenshots')
for _p in (SHOTS, HERE):
    if _p not in sys.path:
        sys.path.insert(0, _p)

import seed  # noqa: E402

LANGS = ['zh', 'en', 'ja']
MATRIX = [(360, 740), (390, 844), (412, 915), (740, 360), (844, 390), (768, 1024), (1280, 800)]
DEFAULT = [(360, 740), (390, 844), (844, 390)]
PHONES_P = [(360, 740), (390, 844), (412, 915)]
DEFAULT_CAPS = {'callsPerMinute': 5, 'callsPerHour': 20, 'callsPerDay': 100}
WALK_CAPS = {'callsPerMinute': 100, 'callsPerHour': 2000, 'callsPerDay': 20000}
CLEAR_SESSION = "Object.keys(localStorage).filter(k => /session\\.v1$/.test(k)).forEach(k => localStorage.removeItem(k)); 1"
# Parts of a check that are P2 (reported, never fail the run): (check id, state)
P2_STATES = {('W-JC-1', 'wide'), ('W-US-1', 'bank-row'), ('W-AS-1', 'usage-cards'), ('W-US-2', 'forbidden'), ('W-SH-10', 'short-page')}
with open(os.path.join(HERE, 'checks.js'), encoding='utf-8') as _fh:
    CHECKS_JS = _fh.read()


def phone(s):
    return min(s) < 700


def portrait_phone(s):
    return phone(s) and s[0] < s[1]


def landscape_phone(s):
    return phone(s) and s[0] > s[1]


def sz(s):
    return '%dx%d' % tuple(s)


# ---------------------------------------------------------------------------------------------------- the run
class Run:
    """Dimensions, devices, fixtures and the report of one walk."""

    def __init__(self, walk):
        self.walk, self.args, self.out = walk, walk.args, walk.out
        self.results = []
        self.devs = OrderedDict()
        # user -> (lang, theme) last saved to the server by one of the devices
        self.saved = {}
        self.cache = {}
        self.t0 = time.time()
        self.notes = []

    # -- selection
    def want(self, cid):
        return self.walk.wants_check(cid)

    def want_any(self, ids, pages):
        return any(self.want(i) for i in ids) and any(self.walk.wants_page(p) for p in pages)

    def langs(self, default):
        return self.args.langs or (LANGS if self.args.full else default)

    def themes(self, default):
        return self.args.themes or (['light', 'dark'] if self.args.full else default)

    def sizes(self, default, ok=None):
        base = self.args.widths or (MATRIX if self.args.full else default)
        return [s for s in base if ok is None or ok(s)]

    # -- devices (one browser per user and language, resized and re-themed on demand)
    def dev(self, user, lang, theme, size):
        key = (user, lang)
        d = self.devs.get(key)
        if d is None:
            while len(self.devs) >= 3:
                self.devs.popitem(last=False)[1].close()
            d = self.walk.device(user, lang=lang, theme=theme, width=size[0], height=size[1])
            self.devs[key] = d
        else:
            self.devs.move_to_end(key)
            if d.kb_open:
                d.keyboard(False)
            if (d.w, d.h) != tuple(size):
                d.set_size(*size)
            if d.theme != theme:
                d.set_theme(theme)
                d.save_settings()
            elif self.saved.get(user) != (lang, theme):
                # the settings are the user's, on the server: a device of the same user in another language saved its
                # own meanwhile, so this one saves its language again before the page is loaded
                d.save_settings()
        self.saved[user] = (lang, theme)
        return d

    def drop_devices(self):
        for d in list(self.devs.values()):
            try:
                d.close()
            except Exception:
                pass
        self.devs.clear()

    def close(self):
        self.drop_devices()

    # -- API helpers
    def session(self, user):
        return self.walk.stack.session(user)

    def set_caps(self, caps):
        try:
            self.session('admin').call('/api/ai/admin/limits', caps)
            return True
        except Exception as e:  # an ai-service without the caps keys: the AI checks still run
            self.notes.append('could not set the AI request caps %r: %s' % (caps, e))
            return False

    def calls_status(self, user):
        try:
            return self.session(user).call('/api/ai/status').get('calls')
        except Exception:
            return None

    def wait_quiet(self, user, seconds=75):
        """Waits until the user's last-minute call count is 0 (the minute cap is a rolling window)."""
        end = time.time() + seconds
        while time.time() < end:
            c = self.calls_status(user)
            if not c or c.get('used', {}).get('minute', 0) == 0:
                return True
            time.sleep(3)
        return False

    def chat_id(self, user='sam'):
        """A made-up AI chat of `user` with a few messages (created once through the API)."""
        key = 'chat-' + user
        if key not in self.cache:
            s = self.session(user)
            chats = s.call('/api/ai/chats').get('chats', [])
            cid = chats[0]['id'] if chats and len(chats) else None
            if cid is None:
                cookie = self.walk.stack.cookie(user)
                for text in ('Give me one tip about kitchen safety.', 'And one more, please.', 'Thank you. Last question: how do I remember it?'):
                    cid = ai_send(self.walk.base, cookie, text, 'zh', cid)
            self.cache[key] = cid
        return self.cache[key]

    def record_id(self, user='lee'):
        if 'rec' not in self.cache:
            self.cache['rec'] = self.session(user).call('/api/records')['records'][0]['id']
        return self.cache['rec']

    def long_user(self):
        """A made-up user with a 40-character name and the AI+ feature (W-AS-1)."""
        if 'longuser' not in self.cache:
            s = self.session('admin')
            name = 'walk-long-user-' + 'x' * 25
            try:
                s.call('/api/users/create', {'username': name, 'password': seed.INITIAL, 'role': 'user'})
            except Exception:
                pass
            try:
                s.call('/api/users/update', {'id': s.user_id(name), 'ai': True})
            except Exception:
                pass
            self.cache['longuser'] = name
        return self.cache['longuser']

    def paging_users(self):
        """Twelve made-up AI+ users, each with one AI call, so that both admin tables run to a second page (W-PT-*)."""
        if 'pgusers' not in self.cache:
            s = self.session('admin')
            names = []
            for i in range(12):
                name = 'walk-pg-%02d' % (i + 1)
                try:
                    s.call('/api/users/create', {'username': name, 'password': seed.INITIAL, 'role': 'user'})
                except Exception:
                    pass
                try:
                    s.call('/api/users/update', {'id': s.user_id(name), 'ai': True})
                except Exception:
                    pass
                try:
                    cookie = seed.Session(self.walk.base).sign_in(name, seed.INITIAL).cookie[seed.COOKIE]
                except Exception:
                    cookie = seed.Session(self.walk.base).sign_in(name, seed.DEMO_PASSWORD).cookie[seed.COOKIE]
                try:
                    ai_send(self.walk.base, cookie, 'One short tip about knives, please. ' + name, 'zh')
                except Exception:
                    pass
                names.append(name)
            self.cache['pgusers'] = names
        return self.cache['pgusers']

    # -- the report
    def rec(self, cid, page, state, user, size, lang, theme, ok, detail='', d=None):
        size_s = sz(size) if not isinstance(size, str) else size
        pri = 2 if (cid, state) in P2_STATES else 1
        shot = None
        if d is not None:
            try:
                name = '%s-%s-%s-%s-%s-%s.png' % (cid, page, user or 'anon', lang, theme, size_s)
                if state:
                    name = name.replace('.png', '-%s.png' % re.sub(r'[^a-z0-9]+', '-', state.lower()).strip('-'))
                shot = os.path.join(self.out, name)
                d.shot(shot)
            except Exception:
                shot = None
        entry = {'id': cid, 'page': page, 'state': state, 'user': user, 'size': size_s, 'lang': lang, 'theme': theme,
                 'pass': ok, 'detail': detail, 'screenshot': shot, 'priority': pri}
        self.results.append(entry)
        word = 'skip' if ok is None else ('PASS' if ok else ('FAIL' if pri == 1 else 'fail(P2)'))
        print('%-8s %-9s %-14s %-6s %-9s %s %s  %s' % (word, cid, page + (':' + state if state else ''), user or '-', size_s, lang,
                                                       theme, detail[:140]), flush=True)
        return ok

    def guard(self, cid, page, fn, *a):
        """Runs one case; an exception is a failed entry (with its message), not the end of the walk."""
        try:
            return fn(*a)
        except Exception as e:
            tb = traceback.extract_tb(e.__traceback__)[-1]
            self.rec(cid, page, 'error', None, '0x0', '-', '-', False, '%s: %s (%s:%d)' % (type(e).__name__, str(e)[:200], tb.filename.split(os.sep)[-1], tb.lineno))
            self.drop_devices()
            return None

    def write_report(self):
        failed = [r for r in self.results if r['pass'] is False and r['priority'] == 1]
        summary = {'total': len(self.results), 'passed': sum(1 for r in self.results if r['pass'] is True),
                   'failed': sum(1 for r in self.results if r['pass'] is False),
                   'p1Failed': len(failed), 'skipped': sum(1 for r in self.results if r['pass'] is None)}
        doc = {'seconds': round(time.time() - self.t0), 'full': bool(self.args.full), 'summary': summary, 'notes': self.notes,
               'results': self.results}
        os.makedirs(self.out, exist_ok=True)
        with open(os.path.join(self.out, 'report.json'), 'w', encoding='utf-8', newline='\n') as fh:
            json.dump(doc, fh, indent=1, ensure_ascii=False)
            fh.write('\n')
        print('\nphone walk: %(total)d entries, %(passed)d passed, %(failed)d failed (%(p1Failed)d P1), %(skipped)d skipped' % summary)
        for r in failed[:30]:
            print('  P1 FAIL %s %s:%s %s %s %s %s: %s' % (r['id'], r['page'], r['state'], r['user'], r['size'], r['lang'], r['theme'], r['detail'][:200]))
        print('report: %s (%d s)' % (os.path.join(self.out, 'report.json'), doc['seconds']), flush=True)
        return 1 if failed else 0


def ai_send(base, cookie, text, lang, chat_id=None):
    """POST /api/ai/send read to its end (made-up text, the stub answers); returns the chat id."""
    payload = {'chatId': chat_id, 'text': text, 'lang': lang}
    req = urllib.request.Request(base + '/api/ai/send', data=json.dumps(payload).encode(), method='POST',
                                 headers={'Origin': base, 'Content-Type': 'application/json', 'Cookie': '%s=%s' % (seed.COOKIE, cookie)})
    with urllib.request.urlopen(req, timeout=120) as r:
        body = r.read().decode('utf-8')
    cid = chat_id
    for block in body.replace('\r\n', '\n').split('\n\n'):
        if any(l.startswith('event: start') or l.startswith('event:start') for l in block.split('\n')):
            data = '\n'.join(l[5:].lstrip() for l in block.split('\n') if l.startswith('data:'))
            cid = json.loads(data)['chatId']
    return cid


# ---------------------------------------------------------------------------------------------------- page helpers
def goto(d, route, settle=1.4):
    d.nav(route, settle)
    d.js(CHECKS_JS)


def W(d, name, *args):
    return d.js('window.__walk.%s(%s)' % (name, ', '.join(json.dumps(a) for a in args)))


def until(d, expr, seconds=10.0, step=0.25):
    end = time.time() + seconds
    while time.time() < end:
        if d.js('!!(%s)' % expr):
            return True
        time.sleep(step)
    return False


def set_count(d, n):
    d.js("""(() => { const i = document.querySelector('.home-count input'); if (!i) return 0;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, '%d');
      i.dispatchEvent(new Event('input', {bubbles: true})); i.dispatchEvent(new Event('change', {bubbles: true})); i.blur(); return 1 })()""" % n)


def start_quiz(d, kind, count=None):
    # v1.1 (TL-6): leaving a quiz page saves its session on pagehide, so the saved session is cleared only after leaving it
    goto(d, '#/', 0.8)
    d.js(CLEAR_SESSION)
    d.reload(1.4)
    d.js(CHECKS_JS)
    if count:
        set_count(d, count)
    d.js("window.__start('%s')" % kind)
    until(d, "location.hash.indexOf('quiz') >= 0 && document.querySelector('.stem')", 10)
    time.sleep(0.6)
    d.js(CHECKS_JS)
    d.js('window.scrollTo(0, 0)')


FINISH_JS = """(async (n) => { const wait = window.__wait; const click = s => { const e = document.querySelector(s); if (e && !e.disabled) { e.click(); return true } return false };
  for (let i = 0; i < n; i++) {
    try { await window.__answer(i %% 3 === 0) } catch (e) {}
    await wait(40);
    if (i < n - 1) { click('[data-act="next"]'); await wait(70) }
  }
  click('[data-act="submit"]') || click('[data-act="finish"]') || click('.rail .btn.primary'); await wait(350);
  click('dialog[open] .acts .btn.primary') || click('dialog[open] .btn.primary'); await wait(1800);
  return location.hash })""" % ()


def make_result(d, n):
    """A whole exam of n questions through the UI (some answers wrong); ends on the result page."""
    start_quiz(d, 'exam', n)
    got = d.js('(%s)(%d)' % (FINISH_JS, n))
    time.sleep(0.8)
    d.js(CHECKS_JS)
    d.js('window.scrollTo(0, 0)')
    return got


def tap_ai(d, slot):
    """A real tap on the AI 提示 / AI 解释 bar button, then waits until the panel text settled."""
    d.tap_sel('[data-slot="%s"]' % slot)
    d.js("window.__settle('#aip-panel-body')")
    time.sleep(0.9)


def panel_state(d):
    return d.js("""(() => { const p = document.querySelector('#aip-panel'); if (!p) return null;
      const h = p.querySelector('.aip-panel-head'); const r = h ? h.getBoundingClientRect() : null;
      return {collapsed: p.classList.contains('collapsed'), head: r ? {y: r.top, b: r.bottom} : null,
              note: (document.querySelector('#aip-panel-note') || {}).className || null} })()""")


# ---------------------------------------------------------------------------------------------------- W-SH-1 / 3 / 5
def v_goto(route):
    return lambda rc, d, s: goto(d, route)


def v_chat(rc, d, s):
    goto(d, '#/ai/%d' % rc.chat_id('sam'), 2.0)


def v_record(rc, d, s):
    goto(d, '#/records/%d' % rc.record_id('lee'), 2.0)


def v_quiz(kind):
    return lambda rc, d, s: start_quiz(d, kind)


def v_panel(rc, d, s):
    start_quiz(d, 'practice')
    tap_ai(d, 'hint')


def v_result(rc, d, s):
    make_result(d, 12)


def v_menu(rc, d, s):
    goto(d, '#/')
    d.tap_sel('#menu-btn')
    time.sleep(0.5)


# (page, state, user, setup)
VISITS = [
    ('home', 'lee', 'lee', v_goto('#/')), ('home', 'riley', 'riley', v_goto('#/')),
    ('quiz', 'practice', 'sam', v_quiz('practice')), ('quiz', 'exam', 'riley', v_quiz('exam')),
    ('quiz', 'panel-open', 'sam', v_panel), ('result', 'exam', 'sam', v_result),
    ('records', 'list', 'lee', v_goto('#/records')), ('records', 'detail', 'lee', v_record),
    ('ai', 'list', 'sam', v_goto('#/ai')), ('ai', 'chat', 'sam', v_chat),
    ('ai-settings', '', 'admin', v_goto('#/ai-settings')), ('users', '', 'admin', v_goto('#/users')),
    ('logs', '', 'admin', v_goto('#/logs')), ('settings', '', 'sam', v_goto('#/settings')),
    ('banks', '', 'admin', v_goto('#/banks')), ('about', '', 'sam', v_goto('#/about')),
    ('signin', '', None, v_goto('#/')), ('forbidden', '', 'riley', v_goto('#/banks')),
    ('menu', 'open', 'admin', v_menu),
]


def g_shared(rc):
    ids = ('W-SH-1', 'W-SH-3', 'W-SH-5')
    if not any(rc.want(i) for i in ids):
        return
    for page, state, user, setup in VISITS:
        if not rc.walk.wants_page(page):
            continue
        for lang in rc.langs(['zh']):
            for theme in rc.themes(['light']):
                for size in rc.sizes(DEFAULT):
                    rc.guard('W-SH-1', page, visit, rc, page, state, user, setup, lang, theme, size)


def visit(rc, page, state, user, setup, lang, theme, size):
    d = rc.dev(user, lang, theme, size)
    setup(rc, d, size)
    time.sleep(0.3)
    touch = phone(size)
    if rc.want('W-SH-1'):
        o = W(d, 'overflow')
        bad = o['doc'] > 1 or bool(o['offenders'])
        rc.rec('W-SH-1', page, state, user, size, lang, theme, not bad,
               'document %+d px wider than the viewport; %s' % (o['doc'], '; '.join(o['offenders']) or 'no offender'), d if bad else None)
    if rc.want('W-SH-3') or rc.want('W-SH-5'):
        a = W(d, 'audit', {'touch': touch, 'within': '.nav-pop' if page == 'menu' else None})
        if rc.want('W-SH-3'):
            if not touch:
                rc.rec('W-SH-3', page, state, user, size, lang, theme, None, 'not a touch size (hit areas are a touch rule, SH-20)')
            else:
                rc.rec('W-SH-3', page, state, user, size, lang, theme, not a['small'],
                       '%d controls; %s' % (a['count'], '; '.join(a['small']) or 'all hit areas >= 44x44'), d if a['small'] else None)
        if rc.want('W-SH-5'):
            rc.rec('W-SH-5', page, state, user, size, lang, theme, not a['covered'],
                   '%d controls; %s' % (a['count'], '; '.join(a['covered']) or 'nothing covers a control'), d if a['covered'] else None)


# ---------------------------------------------------------------------------------------------------- W-SH-2 toast
# Shows a two-line toast the way utils/toast.ts does (the text first, then the lift is measured again: toast() calls the
# page's refresh, a scroll event runs the same code), then measures the toast against the pinned bar and the screen: the
# toast must not touch the bar and must lie fully between the top row (--top-h) and the bottom safe area, inside the width.
TOAST_AT = """(async (bar) => {
  const t = document.getElementById('toast'); if (!t) return null;
  const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  t.classList.remove('show'); t.textContent = 'Walk toast line one. Walk toast line two, a little longer.';
  window.dispatchEvent(new Event('scroll')); await frame();
  t.classList.add('show'); await new Promise((r) => setTimeout(r, 250));
  const p = document.createElement('div');
  p.style.cssText = 'position:fixed;visibility:hidden;pointer-events:none;padding-bottom:env(safe-area-inset-bottom)';
  document.body.append(p); const safe = parseFloat(getComputedStyle(p).paddingBottom) || 0; p.remove();
  const r = t.getBoundingClientRect();
  const b = [...document.querySelectorAll(bar)].find((e) => e.offsetParent || getComputedStyle(e).position === 'fixed');
  const x = b ? b.getBoundingClientRect() : null;
  return {toast: [r.left, r.top, r.right, r.bottom], bar: x ? [x.left, x.top, x.right, x.bottom] : null, scrollY: Math.round(scrollY),
    topH: parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--top-h')) || 0, safe, w: innerWidth, h: innerHeight,
    lift: getComputedStyle(document.documentElement).getPropertyValue('--toast-lift').trim()};
})"""


def g_toast(rc):
    if not rc.want('W-SH-2') or not rc.walk.wants_page('quiz'):
        return
    lang, theme = rc.langs(['zh'])[0], rc.themes(['light'])[0]
    for size in rc.sizes(DEFAULT, phone):
        rc.guard('W-SH-2', 'quiz', toast_case, rc, lang, theme, size)


def toast_verdict(r):
    """The problems of one toast measure (empty when it is fine)."""
    t, b, bad = r['toast'], r['bar'], []
    if b and b[1] < r['h'] and b[3] > 0 and t[0] < b[2] and t[2] > b[0] and t[1] < b[3] and t[3] > b[1]:
        bad.append('touches the bar %s' % [round(v) for v in b])
    if t[1] < r['topH'] - 0.5:
        bad.append('top %d above the top row (--top-h %d)' % (round(t[1]), round(r['topH'])))
    if t[3] > r['h'] - r['safe'] + 0.5:
        bad.append('bottom %d below the safe bottom %d' % (round(t[3]), round(r['h'] - r['safe'])))
    if t[0] < -0.5 or t[2] > r['w'] + 0.5:
        bad.append('outside the width')
    return bad


def toast_case(rc, lang, theme, size):
    for page, user, setup, bar in (('quiz', 'sam', lambda d: start_quiz(d, 'practice'), '.pfoot'),
                                   ('home', 'lee', lambda d: goto(d, '#/'), '.home-go'),
                                   ('home', 'alex', lambda d: goto(d, '#/'), '.home-go'),
                                   ('ai', 'sam', lambda d: goto(d, '#/ai/%d' % rc.chat_id('sam'), 2.0), '.ai-composer')):
        d = rc.dev(user, lang, theme, size)
        setup(d)
        time.sleep(0.5)
        if not d.js('!!document.querySelector(%s)' % json.dumps(bar)):
            rc.rec('W-SH-2', page, 'toast', user, size, lang, theme, None, 'no %s on this page' % bar)
            continue
        # the top, the middle and the end of the page (a bar that scrolled away, or is high up, must not lift the toast
        # off the screen)
        end = max(0, d.js('Math.round(document.documentElement.scrollHeight - innerHeight)') or 0)
        seen = set()
        for state, y in (('toast-top', 0), ('toast-middle', end // 2), ('toast-end', end)):
            if y in seen:                                        # a page that cannot scroll has only the top
                continue
            seen.add(y)
            d.scroll(y)
            time.sleep(0.35)
            r = d.js('(%s)(%s)' % (TOAST_AT, json.dumps(bar)))
            bad = toast_verdict(r)
            rc.rec('W-SH-2', page, state, user, size, lang, theme, not bad,
                   '%s; toast %s, %s %s, scrollY %d, --top-h %d, --toast-lift %s' % (
                       '; '.join(bad) or 'clear of the bar, inside the screen', [round(v) for v in r['toast']], bar,
                       r['bar'] and [round(v) for v in r['bar']], r['scrollY'], round(r['topH']), r['lift']), d if bad else None)
            d.js("document.getElementById('toast').classList.remove('show')")
        d.scroll(0)


# ---------------------------------------------------------------------------------------------------- W-SH-4 pinned bars
def g_pinned(rc):
    if not rc.want('W-SH-4'):
        return
    lang, theme = rc.langs(['zh'])[0], rc.themes(['light'])[0]
    for size in rc.sizes([(390, 844), (844, 390)], phone):
        rc.guard('W-SH-4', 'quiz', pinned_case, rc, lang, theme, size)


def pinned_case(rc, lang, theme, size):
    portrait = size[0] < size[1]
    insets = {'top': 47, 'bottom': 34, 'left': 0, 'right': 0} if portrait else {'top': 0, 'bottom': 21, 'left': 47, 'right': 47}
    for page, user, setup, bar in (('quiz', 'sam', lambda d: start_quiz(d, 'practice'), '.pfoot'),
                                   ('home', 'lee', lambda d: goto(d, '#/'), '.home-go'),
                                   ('ai', 'sam', lambda d: goto(d, '#/ai/%d' % rc.chat_id('sam'), 2.0), '.ai-composer')):
        if not rc.walk.wants_page(page):
            continue
        d = rc.dev(user, lang, theme, size)
        if not d.safe_area_supported:
            rc.rec('W-SH-4', page, 'insets', user, size, lang, theme, None, 'this Chrome has no Emulation.setSafeAreaInsetsOverride')
            continue
        setup(d)
        time.sleep(0.4)
        if page == 'home':
            d.js("document.querySelector('.home-go') && document.querySelector('.home-go').scrollIntoView({block: 'center'})")
            time.sleep(0.3)
        r = W(d, 'pinned', bar, insets)
        if not r.get('found'):
            rc.rec('W-SH-4', page, 'insets', user, size, lang, theme, False, 'no visible %s' % bar, d)
            continue
        rc.rec('W-SH-4', page, 'insets', user, size, lang, theme, not r['problems'], '; '.join(r['problems']) or 'inside the viewport and clear of the insets', d if r['problems'] else None)


# ---------------------------------------------------------------------------------------------------- W-SH-6 contrast
SAMPLES = {
    'home': [('primary button', '.btn.primary'), ('tags', '.badge, .tag'), ('dim text', '.dim, .sub')],
    'quiz': [('disabled primary', '.btn.primary:disabled, .btn.primary[aria-disabled="true"]'), ('greyed AI', '.aip-dock-na, .aip-bar-btn[aria-disabled="true"]'),
             ('kind tag', '.badge, .tag')],
    'logs': [('log levels', '.lg-lv'), ('log lines', '.lg-msg')],
    'ai-settings': [('tags', '.badge, .tag, .ai-tag'), ('muted text', '.dim, .sub, .set-desc')],
}


def g_contrast(rc):
    if not rc.want('W-SH-6'):
        return
    for theme in rc.themes(['light', 'dark']):
        for lang in rc.langs(['zh']):
            for size in rc.sizes([(390, 844), (1280, 800)]):
                rc.guard('W-SH-6', 'home', contrast_case, rc, lang, theme, size)


def contrast_case(rc, lang, theme, size):
    plan = [('home', 'lee', lambda d: goto(d, '#/')), ('quiz', 'alex', lambda d: start_quiz(d, 'practice')),
            ('logs', 'admin', lambda d: goto(d, '#/logs')), ('ai-settings', 'admin', lambda d: goto(d, '#/ai-settings', 2.0)),
            ('quiz-saved', 'sam', saved_setup)]
    for page, user, setup in plan:
        if not rc.walk.wants_page(page.replace('-saved', '')):
            continue
        d = rc.dev(user, lang, theme, size)
        setup(d)
        if page == 'quiz':      # a multiple-choice question has the neutral-disabled Check button
            d.js(GOTO_MULTI)
        time.sleep(0.4)
        res = W(d, 'contrastOf', [{'name': n, 'selector': s} for n, s in SAMPLES[page]])
        bad = [r for r in res if r.get('found') and r['ratio'] < r['need']]
        seen = [r['name'] for r in res if r.get('found')]
        rc.rec('W-SH-6', page, 'contrast', user, size, lang, theme, not bad,
               '; '.join('%s %.2f < %.1f (%s)' % (r['name'], r['ratio'], r['need'], r['el']) for r in bad) or 'sampled: %s' % (', '.join(seen) or 'nothing found'),
               d if bad else None)


# ---------------------------------------------------------------------------------------------------- W-SH-7 dialogs
# (a greyed control, e.g. Reset / Delete on the administrator's own row, only explains itself in a toast: skip it)
OPEN_BY_TEXT = """(re => { const rx = new RegExp(re, 'i'); const b = [...document.querySelectorAll('button, a')].filter(e => (e.offsetParent || e.getClientRects().length) &&
  !e.disabled && e.getAttribute('aria-disabled') !== 'true' &&
  rx.test(e.getAttribute('aria-label') || e.innerText || e.title || '')); if (!b.length) return 0; b[0].scrollIntoView({block: 'center'}); b[0].click(); return 1 })"""
CLOSE_DIALOGS = "document.querySelectorAll('dialog[open]').forEach(x => x.close()); 1"


def g_dialogs(rc):
    if not rc.want('W-SH-7') or not rc.walk.wants_page('quiz'):
        return
    lang, theme = rc.langs(['zh'])[0], rc.themes(['light'])[0]
    for size in rc.sizes(DEFAULT, phone):
        rc.guard('W-SH-7', 'quiz', dialogs_case, rc, lang, theme, size)


def dialogs_case(rc, lang, theme, size):
    plan = [('submit', 'sam', 'quiz', None), ('edit user', 'admin', 'users', '编辑|edit|編集'), ('reset password', 'admin', 'users', '重置|reset|リセット'),
            ('delete user', 'admin', 'users', '删除|delete|削除'), ('delete bank', 'admin', 'banks', '删除|delete|削除')]
    for name, user, page, rx in plan:
        d = rc.dev(user, lang, theme, size)
        if name == 'submit':
            # a short exam, so the walk below reaches the last question (Submit) well within its 60 steps
            start_quiz(d, 'exam', 5)
            d.js("(async () => { const w = window.__wait; for (let i = 0; i < 60; i++) { const f = document.querySelector('[data-act=\"submit\"]'); if (f) { f.click(); break } const n = document.querySelector('[data-act=\"next\"]:not([disabled])'); if (!n) break; n.click(); await w(50) } await w(500) })()")
            opened = d.js("!!document.querySelector('dialog[open]')")
        else:
            goto(d, '#/' + page, 1.8)
            opened = d.js('(%s)(%s)' % (OPEN_BY_TEXT, json.dumps(rx))) and (time.sleep(0.7) or True) and d.js("!!document.querySelector('dialog[open]')")
        if not opened:
            rc.rec('W-SH-7', page, name, user, size, lang, theme, None, 'no control opened a dialog here')
            continue
        r = W(d, 'dialogActs')
        ok = bool(r.get('found')) and (not r.get('acts') or r.get('inside'))
        rc.rec('W-SH-7', page, name, user, size, lang, theme, ok, 'action row %s in a %dx%d viewport' % (
            [round(r['box'][k]) for k in ('x', 'y', 'w', 'h')] if r.get('acts') else 'none', size[0], size[1]), d if not ok else None)
        d.js(CLOSE_DIALOGS)


# ---------------------------------------------------------------------------------------------------- W-MN-1 menu
def g_menu(rc):
    if not rc.want('W-MN-1') or not rc.walk.wants_page('menu'):
        return
    for lang in rc.langs(['zh']):
        for theme in rc.themes(['light']):
            for size in rc.sizes(DEFAULT, phone):
                rc.guard('W-MN-1', 'menu', menu_case, rc, lang, theme, size)


def menu_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/')
    d.tap_sel('#menu-btn')
    time.sleep(0.6)
    r = W(d, 'menuReach')
    if not r.get('found'):
        rc.rec('W-MN-1', 'menu', 'open', 'admin', size, lang, theme, False, 'the menu did not open', d)
        return
    rc.rec('W-MN-1', 'menu', 'open', 'admin', size, lang, theme, not r['unreachable'],
           '%d entries%s; %s' % (r['total'], ', the list scrolls' if r['scrollable'] else '', '; '.join(r['unreachable']) or 'all reachable'),
           d if r['unreachable'] else None)


# ---------------------------------------------------------------------------------------------------- practice / exam
GOTO_MULTI = """(async () => { const wait = window.__wait;
  for (let i = 0; i < 90; i++) {
    const q = await window.__current();
    if (q && q.kind !== 'hotspot' && q.answer && q.answer.length > 1) return i;
    const n = document.querySelector('[data-act="next"]:not([disabled])'); if (!n) return -1;
    n.click(); await wait(80);
  } return -1 })()"""
GOTO_LAST = """(async () => { const wait = window.__wait;
  for (let i = 0; i < 120; i++) {
    const n = document.querySelector('[data-act="next"]:not([disabled])'); if (!n) return i;
    n.click(); await wait(55);
  } return -1 })()"""


def slot_diff(base, other):
    """Differences (> 1 px) of the fixed slots between two states."""
    out = []
    # FB-3: Check answer and Next (Finish / Submit on the last question) are two separate buttons in fixed places too
    for k in ('prev', 'main', 'check', 'next', 'hint', 'explain', 'flag'):
        if k in base and k in other:
            for f in ('l', 'w', 't'):
                if abs(base[k][f] - other[k][f]) > 1.01:
                    out.append('%s.%s %s->%s' % (k, f, base[k][f], other[k][f]))
        elif (k in base) != (k in other):
            out.append('%s appears / disappears' % k)
    return out


def g_bar(rc):
    """W-QZ-2: one row, one-line labels, fixed slots."""
    if not rc.want('W-QZ-2') or not rc.walk.wants_page('quiz'):
        return
    for lang in rc.langs(LANGS):
        for theme in rc.themes(['light']):
            for size in rc.sizes([(360, 740), (390, 844), (412, 915), (768, 1024), (740, 360), (844, 390)]):
                rc.guard('W-QZ-2', 'quiz', bar_case, rc, lang, theme, size)


def bar_case(rc, lang, theme, size):
    for user, kind in (('sam', 'practice'), ('riley', 'exam')) + ((('alex', 'practice'),) if rc.args.full else ()):
        d = rc.dev(user, lang, theme, size)
        start_quiz(d, kind, 51)
        states, rows, tall = OrderedDict(), [], []

        def grab(name):
            time.sleep(0.4)
            states[name] = W(d, 'slots')
            rows.append((name, W(d, 'oneRow', '.pfoot', 'button, a', 4)))
            # the text buttons (Check answer, Next / Finish / Submit) keep their label on one line: a wrapped label makes the
            # button and the bar taller (the centres stay in one row, so oneRow alone does not see it)
            hs = d.js("[...document.querySelectorAll('.pfoot .bar-main > .btn')].filter(b => b.getClientRects().length)"
                      ".map(b => Math.round(b.getBoundingClientRect().height))") or []
            if any(x > 48 for x in hs):
                tall.append('%s: a text button is %d px tall (its label wraps)' % (name, max(hs)))
        grab('first')
        # Revision 4: the page scrolled to its end, and after an AI hint (the panel open under the card)
        d.js('window.scrollTo(0, document.documentElement.scrollHeight)')
        grab('end-of-page')
        d.js('window.scrollTo(0, 0)')
        if kind == 'practice' and user == 'sam' and d.js("!!document.querySelector('.pfoot [data-slot=\"hint\"]:not([aria-disabled=true])')"):
            tap_ai(d, 'hint')
            grab('after-AI')
            d.js('window.scrollTo(0, 0)')
        if kind == 'practice':
            if d.js(GOTO_MULTI) >= 0:
                grab('multi-unchecked')
                d.js("window.__answer(false)")
                time.sleep(0.3)
                if d.js("!!document.querySelector('[data-act=\"check\"]:not([disabled])')"):
                    d.tap_sel('[data-act="check"]')
                    time.sleep(0.5)
                    grab('checked')
        d.js(GOTO_LAST)
        grab('last')
        bad = ['%s: %d controls spread %s px' % (n, r.get('n', 0), r.get('spread')) for n, r in rows if r.get('found') and not r['one']]
        bad += tall
        miss = [n for n, s in states.items() if not s]
        base = next((s for s in states.values() if s), None)
        diffs = []
        for n, s in states.items():
            if s and base and s is not base:
                diffs += ['%s %s' % (n, x) for x in slot_diff(base, s)]
        ok = not bad and not diffs and not miss
        rc.rec('W-QZ-2', 'quiz', kind, user, size, lang, theme, ok,
               '; '.join(bad + diffs + ['no .pfoot in ' + ','.join(miss)] * bool(miss)) or 'one row, slots identical in %s' % ', '.join(states), d if not ok else None)


def g_panel(rc):
    """W-QZ-3 / 4 / 5: the panel, the pinned bar and question changes (sam, practice, zh)."""
    ids = ('W-QZ-3', 'W-QZ-4', 'W-QZ-5')
    if not any(rc.want(i) for i in ids) or not rc.walk.wants_page('quiz'):
        return
    lang, theme = rc.langs(['zh'])[0], rc.themes(['light'])[0]
    for size in rc.sizes([(390, 844), (412, 915), (360, 740)], portrait_phone):
        rc.guard('W-QZ-3', 'quiz', panel_case, rc, lang, theme, size)


def panel_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    start_quiz(d, 'practice')
    d.tap_sel('.opt', 0)            # v1.1 (PC-1): no question type is checked on pick; the pick is checked by the 核对答案 button
    time.sleep(0.4)
    d.tap_sel('.pfoot [data-act="check"]')
    time.sleep(0.6)
    tap_ai(d, 'hint')
    if rc.want('W-QZ-3') and size[0] in (390, 412):
        r = W(d, 'hitAtCentre', '.pfoot [data-act="next"], .pfoot [data-act="finish"], .pfoot [data-act="submit"]')
        rc.rec('W-QZ-3', 'quiz', 'panel-open', 'sam', size, lang, theme, bool(r and r['ok']),
               'the element at the centre of Next is %s' % (r['hit'] if r else 'missing (no Next)'), d if not (r and r['ok']) else None)
    if rc.want('W-QZ-4'):
        ps = panel_state(d)
        top = W(d, 'topH')
        if not ps or not ps['head']:
            rc.rec('W-QZ-4', 'quiz', 'panel-head', 'sam', size, lang, theme, False, 'no AI+ panel after tapping AI 提示', d)
        else:
            y = ps['head']['y']
            ok = top - 1 <= y <= 0.25 * size[1] + 1 and not ps['collapsed']
            # A short page cannot scroll the head that far up (the scroll is computed when the press expands the panel,
            # before a saved answer fills it): then the whole panel (head to footer) must be in the free area between the
            # top row and the pinned bar
            end = d.js("""(() => { const p = document.querySelector('#aip-panel'); const b = document.querySelector('.pfoot');
              const se = document.scrollingElement; return { atEnd: se.scrollTop >= se.scrollHeight - innerHeight - 2,
              bottom: p ? p.getBoundingClientRect().bottom : 0, bar: b ? b.getBoundingClientRect().top : innerHeight } })()""")
            whole = bool(end) and y >= top - 1 and end['bottom'] <= end['bar'] + 1
            ok = ok or (whole and not ps['collapsed'])
            rc.rec('W-QZ-4', 'quiz', 'panel-head', 'sam', size, lang, theme, ok, 'panel head top %d px (free area from %d px to %d px)%s, %s' % (
                round(y), round(top), round(0.25 * size[1]), '; the whole panel is in the free area' if whole else '',
                'collapsed' if ps['collapsed'] else 'expanded'), d if not ok else None)
    if rc.want('W-QZ-5') and size == (390, 844):
        d.net_clear()
        problems = []
        steps = [('Next', lambda: d.tap_sel('.pfoot [data-act="next"]')), ('Previous', lambda: d.tap_sel('.pfoot [data-slot="prev"]'))]

        def tile():
            if d.js("(() => { const t = document.querySelector('[data-focus=\"rail-toggle\"], .rail-toggle'); return t ? t.getAttribute('aria-expanded') : null })()") == 'false':
                d.tap_sel('[data-focus="rail-toggle"], .rail-toggle')
                time.sleep(0.3)
            d.tap_sel('.tile', 2)
        steps.append(('a tile of the answer sheet', tile))
        for name, act in steps:
            act()
            time.sleep(0.8)
            ps = panel_state(d)
            if not ps or ps['collapsed']:
                problems.append('after %s the panel is %s' % (name, 'gone' if not ps else 'collapsed'))
        asks = d.net_requests('/api/ai/study/ask')
        if asks:
            problems.append('%d POST /api/ai/study/ask during question changes' % len(asks))
        # fold the answer sheet again (the app remembers it per user and browser; later checks expect the default)
        if d.js("(() => { const t = document.querySelector('[data-focus=\"rail-toggle\"], .rail-toggle'); return t ? t.getAttribute('aria-expanded') : null })()") == 'true':
            d.tap_sel('[data-focus="rail-toggle"], .rail-toggle')
            time.sleep(0.3)
        rc.rec('W-QZ-5', 'quiz', 'question-change', 'sam', size, lang, theme, not problems, '; '.join(problems) or 'panel stays expanded, 0 ask requests', d if problems else None)


def g_greyed(rc):
    """W-QZ-6: alex (AI+ paused) sees the greyed bar with 无法使用 at 360 and 390."""
    if not rc.want('W-QZ-6') or not rc.walk.wants_page('quiz'):
        return
    for lang in rc.langs(['zh']):
        for size in rc.sizes([(360, 740), (390, 844)], portrait_phone):
            rc.guard('W-QZ-6', 'quiz', greyed_case, rc, lang, rc.themes(['light'])[0], size)


def greyed_case(rc, lang, theme, size):
    d = rc.dev('alex', lang, theme, size)
    start_quiz(d, 'practice')
    r = d.js("""(() => { const e = [...document.querySelectorAll('.aip-dock-na, [data-aip-dock-na]')].find(x => x.offsetParent); if (!e) return null;
      const b = e.getBoundingClientRect(); return {x: b.left, r: b.right, y: b.top, b: b.bottom, text: (e.innerText || '').replace(/\\s+/g, ' ').trim()} })()""")
    ok = bool(r) and r['x'] >= -0.5 and r['r'] <= size[0] + 0.5 and 0 <= r['y'] < size[1] and bool(re.search('无法使用|Unavailable|利用不可', r['text']))
    rc.rec('W-QZ-6', 'quiz', 'greyed', 'alex', size, lang, theme, ok, ('greyed bar "%s" at %d..%d x %d' % (r['text'][:50], r['x'], r['r'], r['y'])) if r else 'no greyed AI+ bar', d if not ok else None)


def g_landscape(rc):
    """W-QZ-7 (space between the top row and the bar) and W-QZ-8 (answer sheet)."""
    if rc.want('W-QZ-7') and rc.walk.wants_page('quiz'):
        for size in rc.sizes([(844, 390), (740, 360)], landscape_phone):
            rc.guard('W-QZ-7', 'quiz', space_case, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], size)
    if rc.want('W-QZ-8') and rc.walk.wants_page('quiz'):
        for size in rc.sizes([(390, 844), (1280, 800)], lambda s: s in ((390, 844), (1280, 800))):
            rc.guard('W-QZ-8', 'quiz', sheet_case, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], size)


def space_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    start_quiz(d, 'practice')
    q, b = W(d, 'rectOf', '.qbar'), W(d, 'rectOf', '.pfoot')
    need = 220 if size[0] >= 844 else 200
    if not q or not b:
        rc.rec('W-QZ-7', 'quiz', 'landscape', 'sam', size, lang, theme, False, 'no .qbar or .pfoot', d)
        return
    gap = b['y'] - q['b']
    rc.rec('W-QZ-7', 'quiz', 'landscape', 'sam', size, lang, theme, gap >= need, '%d px between the top row and the bar (need %d)' % (gap, need), d if gap < need else None)


def sheet_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    start_quiz(d, 'practice', 51)
    if phone(size):
        tog = d.js("""(() => { const t = document.querySelector('[data-focus="rail-toggle"], .rail-toggle'); return t && (t.offsetParent || t.getClientRects().length) ? t.getAttribute('aria-expanded') : null })()""")
        tiles_hidden = d.js("[...document.querySelectorAll('.tile')].filter(e => e.offsetParent || e.getClientRects().length).length") == 0
        problems = []
        if tog != 'false':
            problems.append('no collapsed sheet toggle (aria-expanded=%s)' % tog)
        if not tiles_hidden:
            problems.append('tiles visible while collapsed')
        if tog:
            d.tap_sel('[data-focus="rail-toggle"], .rail-toggle')
            time.sleep(0.4)
        big = d.js("[...document.querySelectorAll('.tile')].map(e => e.getBoundingClientRect()).filter(r => r.width > 44.5 || r.height > 44.5).length")
        if big:
            problems.append('%d tiles larger than 44 px' % big)
        if tog:
            # fold it again: the app remembers the choice per user and browser, and the next size reuses this browser
            d.tap_sel('[data-focus="rail-toggle"], .rail-toggle')
            time.sleep(0.3)
        rc.rec('W-QZ-8', 'quiz', 'sheet-phone', 'sam', size, lang, theme, not problems, '; '.join(problems) or 'collapsed with a summary row, tiles <= 44 px', d if problems else None)
    else:
        r = W(d, 'rectOf', '.rail')
        ok = bool(r) and r['h'] <= size[1] + 1
        rc.rec('W-QZ-8', 'quiz', 'rail-desktop', 'sam', size, lang, theme, ok, 'rail height %s px in a %d px viewport' % (round(r['h']) if r else '?', size[1]), d if not ok else None)


def g_first_line(rc):
    """W-QZ-1: the first stem line is within 120 px of the top at 390x844."""
    if not rc.want('W-QZ-1') or not rc.walk.wants_page('quiz'):
        return
    for lang in rc.langs(LANGS):
        for size in rc.sizes([(390, 844)], lambda s: s == (390, 844)) or [(390, 844)]:
            for user in ('riley', 'sam', 'alex'):
                rc.guard('W-QZ-1', 'quiz', first_line_case, rc, user, lang, rc.themes(['light'])[0], size)


INSET_TOP_JS = """(() => { const e = document.createElement('div');
  e.style.cssText = 'position:fixed;top:0;left:0;width:0;height:0;padding-top:env(safe-area-inset-top)';
  document.body.append(e); const v = parseFloat(getComputedStyle(e).paddingTop) || 0; e.remove(); return v })()"""


def first_line_case(rc, user, lang, theme, size):
    d = rc.dev(user, lang, theme, size)
    start_quiz(d, 'practice')
    r = W(d, 'rectOf', '.stem')
    # Measured from the top of the safe area: the emulated notch inset (47 px in portrait) is screen the page cannot use
    inset = d.js(INSET_TOP_JS) or 0
    y = r['y'] - inset if r else None
    ok = bool(r) and y <= 120
    rc.rec('W-QZ-1', 'quiz', 'practice', user, size, lang, theme, ok, 'the stem starts %s px below the safe-area top (limit 120; inset %d px)' % (
        round(y) if r else '?', round(inset)), d if not ok else None)


# ---------------------------------------------------------------------------------------------------- result, records, home
def g_result(rc):
    if not (rc.want('W-RS-1') or rc.want('W-RS-2')) or not rc.walk.wants_page('result'):
        return
    for size in rc.sizes([(390, 844)], lambda s: s == (390, 844)) or [(390, 844)]:
        rc.guard('W-RS-1', 'result', result_case, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], size)


def result_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    got = make_result(d, 51)
    if 'result' not in str(got):
        rc.rec('W-RS-1', 'result', 'exam-51', 'sam', size, lang, theme, False, 'the exam did not end on the result page (hash %s)' % got, d)
        return
    first = W(d, 'rectOf', '.rv')
    if rc.want('W-RS-1'):
        ok = bool(first) and first['y'] < size[1]
        rc.rec('W-RS-1', 'result', 'exam-51', 'sam', size, lang, theme, ok, 'first review card top %s px (screen %d px)' % (round(first['y']) if first else '?', size[1]), d if not ok else None)
    if rc.want('W-RS-2'):
        problems = []
        n_open = d.js("document.querySelectorAll('.rv')[0] ? document.querySelectorAll('.rv')[0].querySelectorAll('[data-aip-opener], .aip-ask').length : -1")
        if n_open != 1:
            problems.append('card 1 has %s AI openers (expected exactly 1)' % n_open)
        d.js("document.querySelector('.rv').scrollIntoView({block: 'center'})")
        d.net_clear()
        d.tap_sel('.rv .aip-chev')
        time.sleep(1.0)
        if d.net_requests('/api/ai/study/ask'):
            problems.append('the chevron sent an ask request')
        ps = panel_state(d)
        if not ps or ps['collapsed']:
            problems.append('the chevron did not expand the panel')
        else:
            d.tap_sel('.rv .aip-chev')           # collapse again, then ask with the card's own button
            time.sleep(0.4)
        d.net_clear()
        d.tap_sel('.rv .aip-ask')
        d.js("window.__settle('#aip-panel-body')")
        asks = d.net_requests('/api/ai/study/ask')
        if len(asks) != 1:
            problems.append('AI analysis sent %d ask requests (expected 1)' % len(asks))
        d.net_clear()
        d.tap_sel('.rv .aip-ask')
        time.sleep(0.8)
        if d.net_requests('/api/ai/study/ask'):
            problems.append('a second press of AI analysis asked again')
        rc.rec('W-RS-2', 'result', 'ai-row', 'sam', size, lang, theme, not problems, '; '.join(problems) or 'one AI row, chevron without request, one ask', d if problems else None)


def g_records(rc):
    if not rc.want('W-RC-1') or not rc.walk.wants_page('records'):
        return
    for size in rc.sizes([(360, 740)], lambda s: s == (360, 740)) or [(360, 740)]:
        rc.guard('W-RC-1', 'records', records_case, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], size)


def records_case(rc, lang, theme, size):
    d = rc.dev('lee', lang, theme, size)
    goto(d, '#/records', 2.0)
    r = d.js("""(() => { const sels = ['.rec-card', '.rec-item', '.rec-row', '.rec-list > li', '.rec-table tbody tr']; for (const s of sels) {
      const els = [...document.querySelectorAll(s)].filter(e => e.offsetParent); if (els.length) {
        const bs = els.slice(0, 5).map(e => e.getBoundingClientRect()); return {sel: s, top: bs[0].top, maxH: Math.max(...bs.map(b => b.height)), n: els.length} } } return null })()""")
    ok = bool(r) and r['top'] <= 360 and r['maxH'] <= 140
    rc.rec('W-RC-1', 'records', 'list', 'lee', size, lang, theme, ok, ('first card top %d px (limit 360), tallest of the first five %d px (limit 140), %s' % (r['top'], r['maxH'], r['sel'])) if r else 'no record card found', d if not ok else None)


def g_home(rc):
    if not rc.want('W-HM-1') or not rc.walk.wants_page('home'):
        return
    for lang in rc.langs(LANGS):
        for size in rc.sizes(PHONES_P, portrait_phone):
            rc.guard('W-HM-1', 'home', home_case, rc, lang, rc.themes(['light'])[0], size)


def home_case(rc, lang, theme, size):
    d = rc.dev('lee', lang, theme, size)
    goto(d, '#/', 1.8)
    row = W(d, 'oneRow', '.home-latest-acts, .home-latest .acts', 'a, button', 4)
    edge = d.js("""(() => { const g = document.querySelector('.home-go'); if (!g) return null; const p = g.closest('.panel'); if (!p) return null;
      const a = g.getBoundingClientRect(), b = p.getBoundingClientRect(); return {dl: a.left - b.left, dr: a.right - b.right} })()""")
    problems = []
    if not row.get('found'):
        problems.append('no latest-result actions row')
    elif not row['one']:
        problems.append('latest actions are not one row (spread %s px)' % row['spread'])
    if not edge:
        problems.append('no .home-go inside a panel')
    elif abs(edge['dl']) > 1.01 or abs(edge['dr']) > 1.01:
        problems.append('the start bar sticks out of / lies inside its panel by %+.1f / %+.1f px' % (edge['dl'], edge['dr']))
    rc.rec('W-HM-1', 'home', 'latest+start', 'lee', size, lang, theme, not problems, '; '.join(problems) or 'latest actions in one row, start bar edges = panel edges', d if problems else None)


# ---------------------------------------------------------------------------------------------------- AI Chat
def g_chat(rc):
    ids = ('W-CH-1', 'W-CH-2', 'W-CH-3')
    if not any(rc.want(i) for i in ids) or not rc.walk.wants_page('ai'):
        return
    theme = rc.themes(['light'])[0]
    if rc.want('W-CH-1'):
        for size in rc.sizes(DEFAULT + [(740, 360)], phone):
            rc.guard('W-CH-1', 'ai', chat_layout_case, rc, rc.langs(['zh'])[0], theme, size)
    if rc.want('W-CH-2'):
        for lang in rc.langs(LANGS):
            for size in rc.sizes(PHONES_P, portrait_phone):
                rc.guard('W-CH-2', 'ai', chat_title_case, rc, lang, theme, size)
    if rc.want('W-CH-3'):
        for size in rc.sizes([(390, 844), (360, 740)], lambda s: s in ((390, 844), (360, 740))):
            rc.guard('W-CH-3', 'ai', chat_keyboard_case, rc, rc.langs(['zh'])[0], theme, size)
        for size in rc.sizes([(844, 390)], lambda s: s == (844, 390)):
            rc.guard('W-CH-3', 'ai', chat_keyboard_landscape, rc, rc.langs(['zh'])[0], theme, size)


def open_chat(rc, d):
    goto(d, '#/ai/%d' % rc.chat_id('sam'), 2.2)
    d.js(CHECKS_JS)


def chat_layout_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    open_chat(rc, d)
    d.js('window.scrollTo(0, 99999)')
    time.sleep(0.3)
    c = W(d, 'chat')
    problems = []
    if c['docH'] > size[1] + 1 or c['scrollY'] > 0:
        problems.append('the document scrolls (height %s, scrollY %s, viewport %d)' % (c['docH'], c['scrollY'], size[1]))
    if not c['composer']:
        problems.append('no composer')
    elif c['composer']['b'] > size[1] + 1:
        problems.append('composer bottom %d is below the viewport (%d)' % (c['composer']['b'], size[1]))
    # FB-8: at <= 719 px the conversation head is the only header (the title row is hidden)
    if size[0] <= 719:
        shown = d.js("(() => { const e = document.querySelector('.page-ai .ai-head'); return !!e && e.offsetParent !== null })()")
        if shown:
            problems.append('the title row (.ai-head) is still shown inside a conversation')
    rc.rec('W-CH-1', 'ai', 'chat', 'sam', size, lang, theme, not problems, '; '.join(problems) or 'document fixed, composer bottom %d <= %d' % (c['composer']['b'], size[1]), d if problems else None)


def chat_title_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/ai', 2.0)
    r = d.js("""(() => { const h = [...document.querySelectorAll('h1')].find(e => e.offsetParent); if (!h) return null;
      const row = h.parentElement; return {h: row.getBoundingClientRect().height, n: h.getBoundingClientRect().height} })()""")
    ok = bool(r) and r['h'] <= 44.5
    rc.rec('W-CH-2', 'ai', 'title-row', 'sam', size, lang, theme, ok, 'title row %s px high (limit 44)' % (round(r['h'], 1) if r else '?'), d if not ok else None)


def chat_keyboard_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    # (a) Android-like: the viewport shrinks to 55 %
    open_chat(rc, d)
    W(d, 'toEnd')
    time.sleep(0.3)
    before = W(d, 'chat')
    d.tap_sel('.ai-composer textarea, #ai-input')
    time.sleep(0.3)
    d.keyboard(True)
    time.sleep(0.6)
    after = W(d, 'chat')
    rc.rec('W-CH-3', 'ai', 'keyboard-resize', 'sam', size, lang, theme, *keyboard_verdict(before, after, after['vh'], size), d=d)
    # A 6-line draft with the keyboard open: the input stops growing before the conversation drops under ~100 px, and the
    # composer never covers the conversation head
    d.type('\n'.join('Made-up draft line %d' % (i + 1) for i in range(6)))
    time.sleep(0.5)
    rc.rec('W-CH-3', 'ai', 'keyboard-draft', 'sam', size, lang, theme, *keyboard_room(W(d, 'chat'), 90), d=d)
    # Send with the keyboard open: the focus stays in the input (the keyboard would close otherwise)
    d.tap_sel('#ai-send', wait=0.6)
    active = d.js('document.activeElement && document.activeElement.id')
    until(d, "!document.querySelector('#ai-send.stop')", 20)
    rc.rec('W-CH-3', 'ai', 'keyboard-send-focus', 'sam', size, lang, theme, active == 'ai-input',
           'focus after Send: %s' % (active or 'none'), d if active != 'ai-input' else None)
    d.keyboard(False)
    time.sleep(0.5)
    back = W(d, 'chat')
    gap = back['composer'] and abs(back['composer']['b'] - back['vh']) > 1.5
    rc.rec('W-CH-3', 'ai', 'keyboard-closed', 'sam', size, lang, theme, not gap, 'composer bottom %s, viewport %s after the keyboard closed' % (round(back['composer']['b']) if back['composer'] else '?', back['vh']), d if gap else None)
    # (b) iOS-like layout math: the full viewport, --vvh 55 % and --kb 45 % by script
    open_chat(rc, d)
    W(d, 'toEnd')
    time.sleep(0.3)
    before = W(d, 'chat')
    vvh, kb = round(size[1] * 0.55), size[1] - round(size[1] * 0.55)
    d.js("document.documentElement.style.setProperty('--vvh', '%dpx'); document.documentElement.style.setProperty('--kb', '%dpx'); 1" % (vvh, kb))
    time.sleep(0.6)
    after = W(d, 'chat')
    rc.rec('W-CH-3', 'ai', 'keyboard-ios-math', 'sam', size, lang, theme, *keyboard_verdict(before, after, size[1] - kb, size), d=d)
    d.js("document.documentElement.style.removeProperty('--vvh'); document.documentElement.style.removeProperty('--kb'); 1")


def chat_keyboard_landscape(rc, lang, theme, size):
    """W-CH-3 on a phone turned sideways with the keyboard open (55 % of 390 px): the composer stays under the head."""
    d = rc.dev('sam', lang, theme, size)
    open_chat(rc, d)
    W(d, 'toEnd')
    d.tap_sel('.ai-composer textarea, #ai-input')
    d.keyboard(True)
    time.sleep(0.6)
    d.type('\n'.join('Made-up draft line %d' % (i + 1) for i in range(4)))
    time.sleep(0.5)
    c = W(d, 'chat')
    rc.rec('W-CH-3', 'ai', 'keyboard-landscape', 'sam', size, lang, theme, *keyboard_room(c, 24), d=d)
    d.js("(() => { const t = document.getElementById('ai-input'); t.value = ''; t.dispatchEvent(new Event('input')); return 1 })()")


def keyboard_room(c, min_msgs):
    """(pass, detail): the composer's top is not above the conversation head's bottom, at least `min_msgs` px of messages
    show between them, and the newest message is not under the composer."""
    problems = []
    head, comp, sc, last = c['head'], c['composer'], c['scroller'], c['last']
    if not head or not comp or not sc:
        return False, 'no conversation head, composer or message list'
    if comp['y'] < head['y'] + head['h'] - 1:
        problems.append('the composer (top %d) covers the conversation head (bottom %d)' % (comp['y'], head['y'] + head['h']))
    room = min(sc['y'] + sc['h'], comp['y']) - max(sc['y'], head['y'] + head['h'])
    if room < min_msgs:
        problems.append('%d px of messages visible (at least %d)' % (room, min_msgs))
    if last and c['atEnd'] and last['b'] > comp['y'] + 1.5:
        problems.append('the newest message ends under the composer (%d > %d)' % (last['b'], comp['y']))
    return not problems, '; '.join(problems) or '%d px of messages above the composer, head clear' % room


def keyboard_verdict(before, after, bottom, size):
    """(pass, detail) of W-CH-3: nothing slid, the composer sits on the keyboard, the newest message is visible, 16 px text."""
    problems = []
    if after['scrollY'] != 0:
        problems.append('scrollY %s' % after['scrollY'])
    for k in ('topbar', 'head'):
        if before[k] and after[k] and abs(before[k]['y'] - after[k]['y']) > 1.01:
            problems.append('%s moved %d -> %d' % (k, before[k]['y'], after[k]['y']))
    comp = after['composer']
    if not comp:
        problems.append('no composer')
    elif abs(comp['b'] - bottom) > 1.5:
        problems.append('composer bottom %d, expected %d' % (comp['b'], bottom))
    last, sc = after['last'], after['scroller']
    if before['atEnd'] and last and comp:
        if last['b'] > comp['y'] + 1.5 or (sc and last['b'] < sc['y']):
            problems.append('the newest message is not visible (bottom %d, composer top %d)' % (last['b'], comp['y']))
    if after['font'] is not None and after['font'] < 16:
        problems.append('textarea font %spx < 16' % after['font'])
    return (not problems, '; '.join(problems) or 'nothing slid; composer on the keyboard at %d; newest message visible; text %spx' % (bottom, after['font']))


# ---------------------------------------------------------------------------------------------------- users, logs, settings, AI settings
def g_users(rc):
    if not rc.want('W-US-1') or not rc.walk.wants_page('users'):
        return
    for lang in rc.langs(LANGS):
        for size in rc.sizes([(360, 740), (390, 844)], portrait_phone):
            rc.guard('W-US-1', 'users', users_case, rc, lang, rc.themes(['light'])[0], size)


def users_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/users', 2.0)
    r = d.js("""(() => { const q = 'a[href], button, input:not([type=hidden]), select, [role=switch]';
      const els = [...document.querySelectorAll('.main ' + q.split(', ').join(', .main '))].filter(e => (e.offsetParent || e.getClientRects().length) && e.getBoundingClientRect().width > 1);
      const boxes = els.map(e => ({e, b: window.__walk.hitBox(e)}));
      const clash = [];
      for (let i = 0; i < boxes.length; i++) for (let j = i + 1; j < boxes.length; j++) {
        const a = boxes[i], b = boxes[j]; if (a.e.contains(b.e) || b.e.contains(a.e)) continue;
        const w = Math.min(a.b.r, b.b.r) - Math.max(a.b.l, b.b.l), h = Math.min(a.b.b, b.b.b) - Math.max(a.b.t, b.b.t);
        if (w > 1 && h > 1) clash.push((a.e.getAttribute('aria-label') || a.e.innerText || a.e.tagName).trim().slice(0, 20) + ' / ' + (b.e.getAttribute('aria-label') || b.e.innerText || b.e.tagName).trim().slice(0, 20) + ' ' + Math.round(w) + 'x' + Math.round(h));
      }
      const sw = els.find(e => e.matches('[role=switch], .mini-switch'));
      let under = null;
      if (sw) { sw.scrollIntoView({block: 'center'}); const r = sw.getBoundingClientRect(); const h = document.elementFromPoint(r.left + r.width / 2, r.bottom + 7);
        under = h && h !== sw && !sw.contains(h) && h.closest('button, a, input, [role=switch]') ? (h.closest('button, a, input, [role=switch]').getAttribute('aria-label') || h.innerText || h.tagName).trim().slice(0, 30) : null }
      return {n: els.length, clash: clash.slice(0, 6), under, hasSwitch: !!sw} })()""")
    problems = ['overlapping hit areas: ' + '; '.join(r['clash'])] if r['clash'] else []
    if r['under']:
        problems.append('7 px under the AI+ switch a tap hits "%s"' % r['under'])
    rc.rec('W-US-1', 'users', 'hit-areas', 'admin', size, lang, theme, not problems, '; '.join(problems) or '%d controls, no overlap%s' % (r['n'], '' if r['hasSwitch'] else ' (no switch found)'), d if problems else None)
    # The bank-access tools row lives in the add form: open it first
    if d.js("(() => { const b = document.getElementById('us-add-btn'); if (!b || document.getElementById('us-add')) return 0; b.click(); return 1 })()"):
        time.sleep(0.8)
    row = d.js("""(() => { const e = [...document.querySelectorAll('label, button, a, span')].find(x => (x.offsetParent || x.getClientRects().length) && /^(全选|All|全選択|すべて選択)$/.test((x.innerText || '').trim()));
      if (!e) return null; let p = e.parentElement; for (let i = 0; i < 3 && p; i++) { if (/(全部不选|None|全解除|すべて解除)/.test(p.innerText || '')) break; p = p.parentElement }
      const items = [...p.querySelectorAll('label, button, input[type=checkbox]')].filter(x => x.offsetParent || x.getClientRects().length);
      const ys = items.map(x => { const r = x.getBoundingClientRect(); return r.top + r.height / 2 }); return {n: items.length, spread: Math.max(...ys) - Math.min(...ys)} })()""")
    if row is None:
        rc.rec('W-US-1', 'users', 'bank-row', 'admin', size, lang, theme, None, 'the bank-access row is not visible on this page state')
    else:
        rc.rec('W-US-1', 'users', 'bank-row', 'admin', size, lang, theme, row['spread'] <= 4, 'bank-access row of %d controls, spread %s px' % (row['n'], round(row['spread'], 1)), d if row['spread'] > 4 else None)


def g_logs_settings(rc):
    if rc.want('W-LG-1') and rc.walk.wants_page('logs'):
        for size in rc.sizes([(360, 740), (390, 844)], portrait_phone):
            rc.guard('W-LG-1', 'logs', logs_case, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], size)
    if rc.want('W-ST-1') and rc.walk.wants_page('settings'):
        for size in rc.sizes([(360, 740)], lambda s: s == (360, 740)) or [(360, 740)]:
            rc.guard('W-ST-1', 'settings', settings_case, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], size)


def logs_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/logs', 2.0)
    d.js('window.scrollTo(0, 0)')
    tall = d.js('document.documentElement.scrollHeight > innerHeight + 200')
    if not tall:
        rc.rec('W-LG-1', 'logs', 'swipe', 'admin', size, lang, theme, None, 'the page is not long enough to scroll')
        return
    r = W(d, 'rectOf', '.lg-line', 2)
    if not r:
        rc.rec('W-LG-1', 'logs', 'swipe', 'admin', size, lang, theme, False, 'no log line on the page', d)
        return
    y0 = d.js('window.scrollY')
    d.swipe(min(r['x'] + r['w'] / 2, size[0] - 20), min(r['y'] + r['h'] / 2, size[1] - 120), -300)
    y1 = d.js('window.scrollY')
    rc.rec('W-LG-1', 'logs', 'swipe', 'admin', size, lang, theme, y1 > y0 + 20, 'a touch swipe on the log lines moved the page %d -> %d' % (y0, y1), d if y1 <= y0 + 20 else None)


def settings_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/settings', 1.6)
    # The theme control: the segmented row under PHONE, the cards otherwise (only one of them is visible)
    r = W(d, 'rectOf', '.set-themeseg, .set-themecards')
    ok = bool(r) and r['b'] <= size[1]
    rc.rec('W-ST-1', 'settings', 'theme', 'sam', size, lang, theme, ok, 'the theme control ends at %s px (screen %d px)' % (round(r['b']) if r else '?', size[1]), d if not ok else None)


def g_ai_settings(rc):
    if not rc.want('W-AS-1') or not rc.walk.wants_page('ai-settings'):
        return
    rc.long_user()
    for lang in rc.langs(LANGS):
        for size in rc.sizes([(768, 1024), (1024, 768), (1280, 800)], lambda s: not phone(s)):
            rc.guard('W-AS-1', 'ai-settings', ai_settings_case, rc, lang, rc.themes(['light'])[0], size)
        for size in rc.sizes([(360, 740)], lambda s: s == (360, 740)):
            rc.guard('W-AS-1', 'ai-settings', ai_settings_case, rc, lang, rc.themes(['light'])[0], size)


def ai_settings_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    # AS-9: the tables are in the AI+ users and AI usage tabs; a phone checks the usage cards only (P2)
    for tab in (['usage'] if phone(size) else ['users', 'usage']):
        goto(d, '#/ai-settings?tab=' + tab, 2.5)
        tables = W(d, 'tables')
        wide = [t for t in tables if t['wide'] and not t['scrolls']]
        state = 'usage-cards' if phone(size) else 'tables-' + tab
        if not tables:
            rc.rec('W-AS-1', 'ai-settings', state, 'admin', size, lang, theme, None, 'no table in the %s tab (nothing to measure yet)' % tab)
        elif phone(size):
            problems = ['%s %d px in a %d px card' % (t['el'], t['w'], t['host']) for t in tables if t['wide']]
            rc.rec('W-AS-1', 'ai-settings', state, 'admin', size, lang, theme, not problems, '; '.join(problems) or '%d tables, none wider than its card' % len(tables), d if problems else None)
        else:
            rc.rec('W-AS-1', 'ai-settings', state, 'admin', size, lang, theme, not wide, '; '.join('%s %d px in a %d px card' % (t['el'], t['w'], t['host']) for t in wide) or '%d tables, none wider than its card' % len(tables), d if wide else None)


# ---------------------------------------------------------------------------------------------------- caps
CAP_BAD_NOTE = "(() => { const n = document.querySelector('#aip-panel-note'); if (!n || n.hidden) return null; const w = n.querySelector('.callout.warn, .warn'); return {warn: !!w || /warn/.test(n.className), err: !!n.querySelector('.err, .callout.err') || /\\berr\\b/.test(n.className), alert: n.querySelectorAll('[role=alert]').length + (n.getAttribute('role') === 'alert' ? 1 : 0), text: (n.innerText || '').trim().slice(0, 80)} })()"
REGEN = """(() => { const re = /重新生成|Regenerate|再生成/; const b = [...document.querySelectorAll('#aip-panel .aip-panel-foot button, #aip-panel-in button')].find(e => re.test(e.getAttribute('aria-label') || e.innerText || ''));
  return b ? {disabled: b.getAttribute('aria-disabled') === 'true' || b.disabled, aria: b.getAttribute('aria-disabled')} : null })()"""


def g_caps(rc):
    if not any(rc.want(i) for i in ('W-CAP-1', 'W-CAP-2', 'W-CAP-3')):
        return
    if rc.want('W-CAP-1') or rc.want('W-CAP-3'):
        if rc.walk.wants_page('quiz'):
            rc.guard('W-CAP-1', 'quiz', caps_study, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], (390, 844))
    if rc.want('W-CAP-2') and rc.walk.wants_page('ai'):
        rc.guard('W-CAP-2', 'ai', caps_chat, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], (390, 844))
    rc.set_caps(WALK_CAPS)


def caps_study(rc, lang, theme, size):
    rc.set_caps(WALK_CAPS)
    d = rc.dev('lee', lang, theme, size)
    start_quiz(d, 'practice')
    rc.wait_quiet('lee')
    rc.set_caps(dict(WALK_CAPS, callsPerMinute=1))
    tap_ai(d, 'hint')                                            # the one allowed call
    d.tap_sel('.pfoot [data-act="next"]')
    time.sleep(0.6)
    d.net_clear()
    d.tap_sel('[data-slot="hint"]')                              # the second ask: refused
    time.sleep(2.0)
    note = d.js(CAP_BAD_NOTE)
    problems = []
    if not note:
        problems.append('no note after the refused ask')
    else:
        if not note['warn'] or note['err']:
            problems.append('the cap note is %s (expected the orange warn callout)' % ('red error' if note['err'] else 'not a warn callout'))
        if note['alert'] > 1:
            problems.append('role=alert appears %d times (expected once)' % note['alert'])
    d.tap_sel('.pfoot [data-slot="prev"]')                       # back to question 1, which has its answer
    time.sleep(1.0)
    regen = d.js(REGEN)
    if not regen or not regen['disabled']:
        problems.append('Regenerate is not aria-disabled while capped (%s)' % (regen,))
    if rc.want('W-CAP-1'):
        if rc.args.slow:
            blocked = (rc.calls_status('lee') or {}).get('blocked')
            if blocked:
                d.js('window.__noReload = 1')
                time.sleep(max(0, blocked['retryAt'] - time.time()) + 3)
                gone = d.js("(() => { const n = document.querySelector('#aip-panel-note'); return !n || n.hidden || !/warn/.test(n.innerHTML + n.className) })()")
                if not gone or d.js('window.__noReload') != 1:
                    problems.append('the callout did not go away by itself after retryAt (or the page reloaded)')
            else:
                problems.append('the status shows no block to wait for')
            detail_tail = ' (waited for retryAt)'
        else:
            detail_tail = ' (the retryAt wait runs with --slow)'
        rc.rec('W-CAP-1', 'quiz', 'second-ask', 'lee', size, lang, theme, not problems, ('; '.join(problems) or 'warn callout, one alert, Regenerate aria-disabled') + (detail_tail if not problems else ''), d if problems else None)
    if rc.want('W-CAP-3'):
        p3 = []
        d.js(CHECKS_JS)
        if not (regen and regen['disabled']):
            rc.rec('W-CAP-3', 'quiz', 'stale-block', 'lee', size, lang, theme, False, 'no blocked state to start from', d)
            return
        rc.set_caps(WALK_CAPS)                                    # an administrator raises the caps while the callout shows
        time.sleep(6.0)                                           # recheckCallBlock() is throttled to one status read per 5 s
        d.net_clear()
        d.js("""(() => { const re = /重新生成|Regenerate|再生成/; const b = [...document.querySelectorAll('#aip-panel button')].find(e => re.test(e.getAttribute('aria-label') || e.innerText || '')); if (b) b.setAttribute('data-walk-regen', '1'); return !!b })()""")
        d.tap_sel('[data-walk-regen]')
        d.js("window.__settle('#aip-panel-body')")
        if not d.net_requests('/api/ai/study/ask'):
            p3.append('one tap on Regenerate did not ask after the caps were raised')
        after = d.js(REGEN)
        if after and after['disabled']:
            p3.append('Regenerate is still aria-disabled')
        rc.rec('W-CAP-3', 'quiz', 'stale-block', 'lee', size, lang, theme, not p3, '; '.join(p3) or 'the tap re-checked and asked, no reload', d if p3 else None)


def caps_chat(rc, lang, theme, size):
    rc.set_caps(WALK_CAPS)
    d = rc.dev('lee', lang, theme, size)
    goto(d, '#/ai/new', 2.0)
    rc.wait_quiet('lee')
    rc.set_caps(dict(WALK_CAPS, callsPerMinute=1))
    d.tap_sel('.ai-composer textarea, #ai-input')
    d.type('First walk question.')
    d.tap_sel('#ai-send, .ai-composer [data-act="send"]')
    until(d, "document.querySelectorAll('#ai-msgs > li, .ai-msgs > li').length >= 2", 25)
    time.sleep(2.0)
    d.tap_sel('.ai-composer textarea, #ai-input')
    d.type('Second walk question.')
    d.tap_sel('#ai-send, .ai-composer [data-act="send"]')
    time.sleep(2.0)
    r = d.js("""(() => { const c = document.querySelector('.ai-composer'); const cap = c && c.querySelector('.callout.warn, #ai-cap-note');
      const send = document.querySelector('#ai-send, .ai-composer [data-act="send"]'); const ta = document.querySelector('.ai-composer textarea, #ai-input');
      const others = [...document.querySelectorAll('.ai-remember, #ai-retry, .ai-retry')].filter(e => e.offsetParent);
      return {cap: !!cap && cap.getBoundingClientRect().height > 0, send: send ? send.getAttribute('aria-disabled') === 'true' : null, text: ta ? ta.value : null,
              others: others.length, othersOff: others.filter(e => e.getAttribute('aria-disabled') === 'true').length} })()""")
    problems = []
    if not r['cap']:
        problems.append('no warn callout in the composer status slot')
    if not r['send']:
        problems.append('Send is not aria-disabled')
    if r['others'] != r['othersOff']:
        problems.append('%d of %d Retry / Remember controls are not aria-disabled' % (r['others'] - r['othersOff'], r['others']))
    if not r['text'] or 'Second walk' not in r['text']:
        problems.append('the typed text is gone from the textarea (%r)' % (r['text'],))
    rc.rec('W-CAP-2', 'ai', 'capped', 'lee', size, lang, theme, not problems, '; '.join(problems) or 'callout in the composer, Send / Retry / Remember aria-disabled, text kept', d if problems else None)


# ---------------------------------------------------------------------------------------------------- Revision 4 checks
# W-SH-8 (FB-1 / FB-1b), W-SH-9 (FB-2), W-SH-10 (FB-4), W-QZ-9 (FB-3), W-AS-2 (FB-5), W-AS-3 (FB-6), W-CH-4 (FB-5),
# W-US-2 (US-12, P2). The UI texts are read from the catalogs (frontend/src/i18n/catalog/*.ts), never typed here.
CATALOG_DIR = os.path.join(os.path.dirname(os.path.dirname(HERE)), 'frontend', 'src', 'i18n', 'catalog')
_CAT = {}


def cat(lang, key):
    """The catalog text of `key` in `lang` (zh / en / ja), parsed from the TypeScript catalogs ('key': 'text' lines)."""
    if not _CAT:
        for name in os.listdir(CATALOG_DIR):
            if not name.endswith('.ts') or name.endswith('.test.ts'):
                continue
            cur = None
            with open(os.path.join(CATALOG_DIR, name), encoding='utf-8') as fh:
                for line in fh:
                    m = re.match(r'^  (en|zh|ja): \{', line)
                    if m:
                        cur = m.group(1)
                        continue
                    m = re.match(r"^\s+'([\w.]+)': '((?:[^'\\]|\\.)*)',?\s*$", line)
                    if cur and m:
                        _CAT[(cur, m.group(1))] = m.group(2).replace("\\'", "'")
    return _CAT.get((lang, key), _CAT.get(('en', key), ''))


def fill(text, **kw):
    for k, v in kw.items():
        text = text.replace('{%s}' % k, str(v))
    return text


def digits(s):
    return re.sub(r'\D', '', s or '')


OLD_OPEN_CHAT = r'在 ?AI ?对话中打开|Open in AI Chat|AI ?チャットで開く'
VIS = "(e => !!e && (e.offsetParent !== null || e.getClientRects().length > 0) && getComputedStyle(e).visibility !== 'hidden')"


# ---- W-US-2 (P2): riley on #/users stays there and reads the forbidden text
def g_us2(rc):
    if not rc.want('W-US-2') or not rc.walk.wants_page('users'):
        return
    for lang in rc.langs(['zh']):
        rc.guard('W-US-2', 'users', us2_case, rc, lang, rc.themes(['light'])[0], (390, 844))


def us2_case(rc, lang, theme, size):
    d = rc.dev('riley', lang, theme, size)
    goto(d, '#/users', 1.8)
    r = d.js("({hash: location.hash, text: (document.querySelector('.main') || document.body).innerText})")
    want = cat(lang, 'forbidden.users')
    ok = r['hash'] == '#/users' and want and want in r['text']
    rc.rec('W-US-2', 'users', 'forbidden', 'riley', size, lang, theme, ok,
           'hash %s; forbidden text %s' % (r['hash'], 'shown' if want and want in r['text'] else 'missing'), d if not ok else None)


# ---- W-CH-5 (A-OWNER-76): on a phone the history list / conversation only SCROLL their rows; the frame never drags
def g_ch5(rc):
    if not rc.want('W-CH-5') or not rc.walk.wants_page('ai'):
        return
    for size in rc.sizes([(390, 844), (360, 740)], lambda s: s in ((390, 844), (360, 740))):
        rc.guard('W-CH-5', 'ai', ch5_case, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], size)


CH5_STATE = """(sel => { const r = s => { const e = document.querySelector(s); if (!e || !e.offsetParent) return null;
    const b = e.getBoundingClientRect(); return [Math.round(b.top * 10) / 10, Math.round(b.left * 10) / 10, Math.round(b.height * 10) / 10]; };
  const vv = window.visualViewport || {};
  const sc = document.querySelector(sel);
  return { frame: ['.topbar', '.ai-list-head', '.ai-conv-head', '.ai-dock', '.ai-composer'].map(r),
    win: [scrollX, scrollY, document.scrollingElement.scrollTop, vv.offsetTop || 0, vv.offsetLeft || 0, vv.pageTop || 0],
    st: sc ? sc.scrollTop : -1, over: sc ? sc.scrollHeight - sc.clientHeight : -1 } })"""

# a synthetic, cancelable one-finger touchmove: the guard must cancel it (frame) or leave it (a scroller that can follow)
CH5_PROBE = """((sel, dy) => { const e = document.querySelector(sel); if (!e) return null;
  const b = e.getBoundingClientRect(); const x = b.left + b.width / 2, y = b.top + Math.min(b.height / 2, 40);
  const mk = (type, yy) => { const t = new Touch({ identifier: 7, target: e, clientX: x, clientY: yy, pageX: x, pageY: yy });
    return new TouchEvent(type, { touches: type === 'touchend' ? [] : [t], targetTouches: type === 'touchend' ? [] : [t], changedTouches: [t], bubbles: true, cancelable: true }); };
  e.dispatchEvent(mk('touchstart', y)); const ev = mk('touchmove', y + dy); e.dispatchEvent(ev); e.dispatchEvent(mk('touchend', y + dy));
  return ev.defaultPrevented })"""


def ch5_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/ai', 2.0)
    d.js(CHECKS_JS)
    problems = []
    sel = '.ai-list-body'
    if not d.js("(() => { const e = document.querySelector('.ai-list-body'); return !!e && e.offsetParent !== null })()"):
        rc.rec('W-CH-5', 'ai', 'history-swipe', 'sam', size, lang, theme, False, 'no history list on screen', d)
        return
    tmpl = "(n => { const ul = document.querySelector('.ai-chats'); if (!ul || !ul.firstElementChild) return -1; const rows = Array.from(ul.children); rows.slice(n || 1).forEach(r => r.remove()); "            "if (n > 1) for (let i = rows.length; i < n; i++) ul.appendChild(rows[0].cloneNode(true)); return ul.children.length })"
    for label, n in (('short', 1), ('long', 40)):
        goto(d, '#/ai', 1.5)
        d.js(CHECKS_JS)
        d.js_call(tmpl, n)
        if label == 'short':
            d.js("Array.from(document.querySelectorAll('.ai-list-body > :not(.ai-chats)')).forEach(e => { e.style.display = 'none' })")
        time.sleep(0.3)
        for dy in (-180, 180):
            before = d.js_call(CH5_STATE, sel)
            r = d.rect(sel, 0, scroll=False)
            d.swipe(r['cx'], r['cy'], dy)
            after = d.js_call(CH5_STATE, sel)
            tag = '%s list swipe %+d' % (label, dy)
            if before['frame'] != after['frame']:
                problems.append('%s: the frame moved (%s -> %s)' % (tag, before['frame'], after['frame']))
            if before['win'] != after['win']:
                problems.append('%s: document / visual viewport offset changed (%s -> %s)' % (tag, before['win'], after['win']))
            if label == 'long' and dy < 0 and after['over'] > 4 and after['st'] <= before['st']:
                problems.append('the long list did not scroll (overflow %s px)' % after['over'])
            if label == 'short' and after['st'] != before['st']:
                problems.append('the short list scrolled')
        # deterministic: the guard cancels a drag nothing can follow, and leaves a drag a scroller can follow
        st = d.js_call(CH5_STATE, sel)
        prevented_head = d.js_call(CH5_PROBE, '.ai-list-head', 60)
        if prevented_head is False:
            problems.append('%s: a drag on the list header was not cancelled' % label)
        if label == 'short':
            if d.js_call(CH5_PROBE, '.ai-chats', -60) is False:
                problems.append('short list: a drag on rows that cannot scroll was not cancelled')
        elif st['over'] > 4:
            d.js("document.querySelector('.ai-list-body').scrollTop = 20")
            if d.js_call(CH5_PROBE, '.ai-chats', -60) is True:
                problems.append('long list: a drag a scroller can follow was cancelled')
    # the conversation: a drag on its head never moves anything
    goto(d, '#/ai/%d' % rc.chat_id('sam'), 2.2)
    d.js(CHECKS_JS)
    before = d.js_call(CH5_STATE, '.ai-scroll')
    head = d.rect('.ai-conv-head', 0, scroll=False) or d.rect('.topbar', 0, scroll=False)
    d.swipe(head['cx'], head['cy'], 150)
    d.swipe(head['cx'], head['cy'], -150)
    after = d.js_call(CH5_STATE, '.ai-scroll')
    if before['frame'] != after['frame'] or before['win'] != after['win']:
        problems.append('conversation head swipe moved the frame (%s -> %s)' % (before, after))
    rc.rec('W-CH-5', 'ai', 'history-swipe', 'sam', size, lang, theme, not problems,
           '; '.join(problems) or 'short and long list, conversation head: frame and viewport unchanged, long list scrolled', d if problems else None)


# ---- W-CH-4: the memory editor counts tokens (n / 1000, A-OWNER-43b: ASCII 1/4, other characters 3/2 token); over 1000 is refused, 1000 saves
def g_ch4(rc):
    if not rc.want('W-CH-4') or not rc.walk.wants_page('ai'):
        return
    rc.guard('W-CH-4', 'ai', ch4_case, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], (390, 844))


SET_VALUE = """((sel, v) => { const e = document.querySelector(sel); if (!e) return -1; e.focus();
  const proto = e.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(e, v);
  e.dispatchEvent(new Event('input', {bubbles: true})); e.dispatchEvent(new Event('change', {bubbles: true})); return e.value.length })"""


def ch4_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/ai', 2.0)
    d.js("(() => { const b = ['ai-tab', 'ai-ltab', 'ai-mtab'].map(p => document.getElementById(p + '-memory')).find(e => e && e.offsetParent); if (b) b.click(); return !!b })()")
    time.sleep(0.8)
    problems = []
    count = d.js("(document.getElementById('ai-mem-count') || {}).innerText || ''")
    if digits(count) != '01000':
        problems.append('counter reads %r (want 0 / 1000)' % count)
    # 667 CJK characters = 1001 tokens by the estimate: the add is refused with the catalog text (nothing is sent), or the field stops sooner
    d.js("document.getElementById('ai-mem-input').focus()")
    d.js_call(SET_VALUE, '#ai-mem-input', '记' * 667)
    time.sleep(0.3)
    n = d.js("document.getElementById('ai-mem-input').value.length")
    if n > 666:
        d.net_clear()
        d.tap_sel('#ai-mem-add')
        time.sleep(0.8)
        posts = [u for m, u in d.net_requests('/api/ai/memories') if m == 'POST']
        err = fill(cat(lang, 'ai.err.memoryTooLong'), n=1000)
        if posts and err not in d.js('document.body.innerText'):
            problems.append('1001 tokens were sent and no %r shown' % err)
    before = len(rc.session('sam').call('/api/ai/memories').get('memories', []))
    text = ('练习记忆' * 200)[:666]
    d.js_call(SET_VALUE, '#ai-mem-input', text)
    time.sleep(0.3)
    d.tap_sel('#ai-mem-add')
    time.sleep(1.2)
    mems = rc.session('sam').call('/api/ai/memories').get('memories', [])
    saved = [m for m in mems if m.get('content') == text]
    if len(mems) != before + 1 or not saved:
        problems.append('1000 tokens did not save (%d -> %d memories)' % (before, len(mems)))
    for m in saved:     # leave the made-up user as it was
        try:
            rc.session('sam').call('/api/ai/memories/delete', {'id': m['id']})
        except Exception:
            pass
    rc.rec('W-CH-4', 'ai', 'memory-length', 'sam', size, lang, theme, not problems,
           '; '.join(problems) or 'counter 0 / 1000; 1001 tokens refused (field keeps %d); 999 saved' % min(n, 667), d if problems else None)


# ---- W-AS-3: the category tabs under the status chips (v1.1: five, the fifth is AI 题库)
AS3_SIZES = [(320, 640), (360, 740), (390, 844), (768, 1024), (1280, 800)]
AS3_CONTENT = {'feature': '#ai-limits-save', 'users': '.ais-users', 'usage': '.ais-totals', 'safety': '#ai-limits-save'}


def g_as3(rc):
    if not rc.want('W-AS-3') or not rc.walk.wants_page('ai-settings'):
        return
    for lang in rc.langs(LANGS):
        for size in rc.sizes([(320, 640), (360, 740), (390, 844), (1280, 800)], lambda s: s in AS3_SIZES):
            rc.guard('W-AS-3', 'ai-settings', as3_case, rc, lang, rc.themes(['light'])[0], size)


def as3_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings', 2.5)
    d.js(CHECKS_JS)
    problems = []
    r = d.js("""(() => { const t = [...document.querySelectorAll('.ais-tabs button')];
      return {ids: t.map(b => b.id), pressed: t.filter(b => b.getAttribute('aria-pressed') === 'true').map(b => b.id),
              chips: !!document.querySelector('.ais-status')} })()""")
    if r['ids'] != ['ais-tab-feature', 'ais-tab-users', 'ais-tab-usage', 'ais-tab-safety', 'ais-tab-gen']:
        problems.append('tabs %s' % ','.join(r['ids']))
    if r['pressed'] != ['ais-tab-feature']:
        problems.append('#/ai-settings opens %s' % (r['pressed'] or 'no tab'))
    row = W(d, 'oneRow', '.ais-tabs', 'button', 5)
    if not row.get('found') or not row['one']:
        problems.append('the tabs are not one row (spread %s)' % row.get('spread'))
    if phone(size):
        a = W(d, 'audit', {'touch': True, 'within': '.ais-tabs'})
        if a['small']:
            problems.append('small tabs: ' + '; '.join(a['small']))
    for k in ('users', 'usage', 'safety', 'feature'):
        h0 = d.js('history.length')
        d.tap_sel('#ais-tab-' + k, wait=1.6)
        s = d.js("""((k, sels) => ({hash: location.hash, hl: history.length, chips: %s(document.querySelector('.ais-status')),
            pressed: (document.querySelector('.ais-tabs [aria-pressed=true]') || {}).id,
            here: Object.fromEntries(Object.entries(sels).map(([n, s]) => [n, !!document.querySelector(s)]))}))""" % VIS
                    + "(%s, %s)" % (json.dumps(k), json.dumps(AS3_CONTENT)))
        if s['hash'] != '#/ai-settings?tab=' + k:
            problems.append('%s: hash %s' % (k, s['hash']))
        if s['hl'] != h0:
            problems.append('%s: a new history entry' % k)
        if s['pressed'] != 'ais-tab-' + k or not s['chips']:
            problems.append('%s: pressed %s, chips %s' % (k, s['pressed'], s['chips']))
        if k in ('users', 'usage') and not s['here'][k]:
            problems.append('%s: its content is missing' % k)
        if k == 'safety' and not s['here']['safety']:
            problems.append('safety: no limits form')
        if k in ('users', 'usage', 'feature') and s['here']['safety']:
            problems.append('%s: the safety form shows too' % k)
        if W(d, 'overflow')['doc'] > 1:
            problems.append('%s: the page scrolls sideways' % k)
    d.tap_sel('#ais-tab-safety', wait=1.2)
    d.reload(2.5)
    kept = d.js("(document.querySelector('.ais-tabs [aria-pressed=true]') || {}).id")
    if kept != 'ais-tab-safety':
        problems.append('a reload shows %s' % kept)
    rc.rec('W-AS-3', 'ai-settings', 'tabs', 'admin', size, lang, theme, not problems,
           '; '.join(problems) or '5 tabs in one row; each tab replaces the hash, keeps the chips and survives a reload', d if problems else None)


# ---- W-AS-2: the answer and memory length in tokens
def g_as2(rc):
    if not rc.want('W-AS-2') or not rc.walk.wants_page('ai-settings'):
        return
    for lang in rc.langs(LANGS):
        for size in rc.sizes([(390, 844), (1280, 800)], lambda s: s in [(390, 844), (1280, 800)]):
            rc.guard('W-AS-2', 'ai-settings', as2_case, rc, lang, rc.themes(['light'])[0], size)


def as2_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings?tab=safety', 2.5)
    d.js(CHECKS_JS)
    problems = []
    r = d.js("""(() => { const g = [...document.querySelectorAll('.ais-limits .ais-subgroup')].map(e => e.classList.contains('ais-caps-group') ? 'caps' : e.classList.contains('ais-length-group') ? 'length' : 'server');
      const v = id => (document.getElementById(id) || {}).value;
      return {groups: g, reply: v('ai-lim-replyMaxTokens'), mem: v('ai-lim-memoryTokens'), msg: !!document.getElementById('ai-lim-messageChars'),
              hint: (document.getElementById('ai-lim-replyMaxTokens-hint2') || {}).innerText || '', live: (document.getElementById('ai-lim-replyMaxTokens-live') || {}).innerText || '', hint1: (document.getElementById('ai-lim-replyMaxTokens-hint') || {}).innerText || ''} })()""")
    if r['groups'] != ['caps', 'length', 'server']:
        problems.append('groups %s' % ','.join(r['groups']))
    if r['reply'] != '3000' or r['mem'] != '1000' or r['msg']:
        problems.append('fields reply %s, memory %s, per-message field %s' % (r['reply'], r['mem'], r['msg']))
    if r['hint'] or r['live']:
        problems.append('the character hint / live line is still there (lengths are tokens now): %r %r' % (r['hint'], r['live']))
    if '250000' not in digits(r['hint1']) or '3000' not in digits(r['hint1']):
        problems.append('range hint %r' % r['hint1'])
    if W(d, 'overflow')['doc'] > 1:
        problems.append('the page scrolls sideways')
    if lang == 'zh' and size == (390, 844):
        admin = rc.session('admin')
        for bad in ('0', '250001'):
            d.js_call(SET_VALUE, '#ai-lim-replyMaxTokens', bad)
            d.tap_sel('#ai-limits-save', wait=1.2)
            err = d.js("(() => { const f = document.getElementById('ai-lim-replyMaxTokens'); const box = f && f.closest('.ais-length'); const e = box && box.querySelector('.ais-ferr'); return e ? e.innerText : null })()")
            now = admin.call('/api/ai/admin/limits').get('limits', {}).get('replyMaxTokens')
            if not err or now != 3000:
                problems.append('%s: error %r, stored %s' % (bad, err, now))
        d.js_call(SET_VALUE, '#ai-lim-replyMaxTokens', '250000')
        d.tap_sel('#ai-limits-save', wait=1.5)
        now = admin.call('/api/ai/admin/limits').get('limits', {}).get('replyMaxTokens')
        if now != 250000:
            problems.append('250000 did not save (stored %s)' % now)
        admin.call('/api/ai/admin/limits', {'replyMaxTokens': 3000, 'memoryTokens': 1000})
    rc.rec('W-AS-2', 'ai-settings', 'lengths', 'admin', size, lang, theme, not problems,
           '; '.join(problems) or 'caps, length, server; 3000 / 1000 tokens, no character hint%s' % (
               '; 0 and 250001 refused, 250000 saved' if lang == 'zh' and size == (390, 844) else ''), d if problems else None)


# ---- W-QZ-9: Check answer and Next are two separate buttons on every practice question
QZ9_SIZES = [(360, 740), (390, 844), (412, 915), (1280, 800)]
GOTO_KIND = """(async (want) => { const wait = window.__wait;
  for (let i = 0; i < 90; i++) {
    const q = await window.__current();
    if (q && q.kind !== 'hotspot' && q.answer && (want === 'single' ? q.answer.length === 1 : q.answer.length > 1)) return i;
    const n = document.querySelector('[data-act="next"]:not([disabled])'); if (!n) return -1;
    n.click(); await wait(80);
  } return -1 })"""
QZ9_STATE = """(() => { const bar = [...document.querySelectorAll('.pfoot')].find(e => e.getClientRects().length); if (!bar) return null;
  const v = e => !!e && e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden';
  const box = e => { const r = e.getBoundingClientRect(); return [Math.round(r.left), Math.round(r.top - bar.getBoundingClientRect().top), Math.round(r.width), Math.round(r.height)] };
  const slots = [...bar.querySelectorAll('[data-slot]')].filter(v).filter(e => e.dataset.slot !== 'main')
    .sort((a, b) => a.getBoundingClientRect().left - b.getBoundingClientRect().left).map(e => e.dataset.slot);
  const c = bar.querySelector('[data-act="check"]'), n = bar.querySelector('[data-slot="next"]');
  return {slots, check: c ? {vis: v(c), dis: c.disabled, box: box(c)} : null, next: n ? {vis: v(n), dis: n.disabled, act: n.dataset.act, box: box(n)} : null,
          wide: document.documentElement.scrollWidth - innerWidth} })()"""


def g_qz9(rc):
    if not rc.want('W-QZ-9') or not rc.walk.wants_page('quiz'):
        return
    for lang in rc.langs(LANGS):
        for size in rc.sizes([(390, 844), (1280, 800)], lambda s: s in QZ9_SIZES):
            rc.guard('W-QZ-9', 'quiz', qz9_case, rc, lang, rc.themes(['light'])[0], size)


def qz9_case(rc, lang, theme, size):
    for user in ('sam', 'riley'):
        d = rc.dev(user, lang, theme, size)
        start_quiz(d, 'practice', 51)
        states = OrderedDict()

        def grab(name, check_on):
            time.sleep(0.35)
            states[name] = (d.js(QZ9_STATE), check_on)
        if d.js('(%s)("single")' % GOTO_KIND) >= 0:
            grab('single-before', False)
            d.js('window.__answer(false)')
            grab('single-after', True)    # v1.1 (PC-1): nothing is checked on pick; Check answer checks a single too
        if d.js('(%s)("multi")' % GOTO_KIND) >= 0:
            grab('multi-empty', False)
            d.js('window.__answer(false)')
            grab('multi-chosen', True)
            if d.js("!!document.querySelector('[data-act=\"check\"]:not([disabled])')"):
                d.tap_sel('[data-act="check"]')
                grab('multi-checked', False)
        d.js(GOTO_LAST)
        grab('last', None)
        want = ['prev', 'check', 'next'] + (['hint', 'explain'] if user == 'sam' else []) + ['flag']
        problems, base = [], None
        for name, (s, check_on) in states.items():
            if not s or not s['check'] or not s['next'] or not s['check']['vis'] or not s['next']['vis']:
                problems.append('%s: Check answer and Next are not both shown' % name)
                continue
            if s['slots'] != want:
                problems.append('%s: order %s' % (name, ','.join(s['slots'])))
            if check_on is not None and s['check']['dis'] == check_on:
                problems.append('%s: Check answer %s' % (name, 'greyed' if check_on else 'enabled'))
            if s['next']['dis']:
                problems.append('%s: Next is greyed' % name)
            if name == 'last' and s['next']['act'] != 'finish':
                problems.append('last: Next acts as %s' % s['next']['act'])
            if s['wide'] > 1:
                problems.append('%s: sideways overflow' % name)
            base = base or s
            for k in ('check', 'next'):
                if any(abs(a - b) > 1 for a, b in zip(base[k]['box'], s[k]['box'])):
                    problems.append('%s: %s moved %s -> %s' % (name, k, base[k]['box'], s[k]['box']))
        rc.rec('W-QZ-9', 'quiz', 'practice', user, size, lang, theme, not problems,
               '; '.join(problems) or 'two buttons, same boxes in %s' % ', '.join(states), d if problems else None)
    d = rc.dev('riley', lang, theme, size)
    start_quiz(d, 'exam', 12)
    has = d.js("!!document.querySelector('.pfoot [data-act=\"check\"]')")
    rc.rec('W-QZ-9', 'quiz', 'exam', 'riley', size, lang, theme, not has, 'the exam has %s Check answer' % ('a' if has else 'no'), d if has else None)


# ---- W-SH-10: the footer line on every page
FOOTER_JS = """(async (ver) => {
  const wait = ms => new Promise(r => setTimeout(r, ms));
  const v = %s;
  window.scrollTo(0, document.documentElement.scrollHeight);
  const lb = document.querySelector('.ai-list-body'); if (lb && v(lb)) lb.scrollTop = lb.scrollHeight;
  await wait(400);
  const lines = [...document.querySelectorAll('.gh-line')].filter(v);
  const out = {n: lines.length, problems: []};
  if (lines.length !== 1) return out;
  const g = lines[0], gr = g.getBoundingClientRect();
  const links = [...g.querySelectorAll('a.gh-link')];
  const want = ['https://github.com/Yumin-Space/xexam', 'https://github.com/Yumin-Space/xexam/releases'];
  if (links.length !== 2) out.problems.push(links.length + ' links');
  links.forEach((a, i) => {
    if (a.getAttribute('href') !== want[i]) out.problems.push('link ' + (i + 1) + ' href ' + a.getAttribute('href'));
    const rel = (a.getAttribute('rel') || '').split(/\\s+/);
    if (a.target !== '_blank' || !rel.includes('noopener') || !rel.includes('noreferrer')) out.problems.push('link ' + (i + 1) + ' target / rel');
    const r = a.getBoundingClientRect(); const h = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    if (!h || !(h === a || a.contains(h))) out.problems.push('link ' + (i + 1) + ' covered by ' + (h ? h.tagName + '.' + h.className : 'nothing'));
  });
  if (links[1] && links[1].innerText.trim() !== 'Xexam ' + ver) out.problems.push('version link reads ' + links[1].innerText.trim());
  const foot = g.closest('footer') || g.parentElement;
  if (/Xexam v\\d/.test(foot.innerText)) out.problems.push('the old "Xexam v" line is still there');
  const rg = document.createRange(); rg.selectNodeContents(g); const cr = rg.getBoundingClientRect();
  const lh = parseFloat(getComputedStyle(g).lineHeight) || 20;
  if (cr.height > lh * 1.6) out.problems.push('the line wraps (' + Math.round(cr.height) + ' px)');
  const col = g.closest('.ai-list') || document.querySelector('.main') || g.parentElement;
  const cc = col.getBoundingClientRect(); const cs = getComputedStyle(col);
  const mid = (cc.left + parseFloat(cs.paddingLeft) + cc.right - parseFloat(cs.paddingRight)) / 2;
  out.offCentre = Math.round((cr.left + cr.right) / 2 - mid);
  if (Math.abs(out.offCentre) > 2) out.problems.push('off centre by ' + out.offCentre + ' px');
  const shell = g.closest('footer.foot');
  if (shell) {
    const note = shell.querySelector('.foot-note'), btn = shell.querySelector('.btn');
    if (!note || note.getBoundingClientRect().bottom > gr.top + 0.5) out.problems.push('the disclaimer is not above the GitHub line');
    else {
      const prev = btn && v(btn) ? btn.getBoundingClientRect() : note.getBoundingClientRect();
      // A-OWNER-20 (v1.1): one blank text line above the GitHub line and one between the divider and the disclaimer
      const line = parseFloat(getComputedStyle(note).lineHeight) || 18.6;
      const gap = gr.top - prev.bottom, rule = note.getBoundingClientRect().top - shell.getBoundingClientRect().top;
      if (gap < line - 5 || gap > line + 6) out.problems.push('gap above the GitHub line ' + Math.round(gap) + ' px (one line: ' + Math.round(line) + ')');
      if (rule < line - 5 || rule > line + 6) out.problems.push('rule to disclaimer ' + Math.round(rule) + ' px (one line: ' + Math.round(line) + ')');
    }
  }
  if (!document.documentElement.classList.contains('no-doc-scroll')) {
    const p = document.createElement('div'); p.style.cssText = 'position:fixed;padding-bottom:env(safe-area-inset-bottom)'; document.body.append(p);
    const safe = parseFloat(getComputedStyle(p).paddingBottom) || 0; p.remove();
    const below = document.documentElement.scrollHeight - (gr.bottom + scrollY);
    // owner (2026-11-13): on a page shorter than the screen the line sits at the screen bottom, like on a long page
    if (below > 12.5 + safe && !document.querySelector('.pfoot, .home-go')) {
      if (document.documentElement.scrollHeight <= innerHeight + 1) out.problems.push('a short page: ' + Math.round(below) + ' px blank under the line (it must sit at the screen bottom)');
      else out.problems.push('the page goes on ' + Math.round(below) + ' px under the line');
    }
  }
  if (innerWidth <= 900 && document.querySelector('.page-ai') && (!g.closest('.ai-list') || g.closest('.ai-conv, .ai-scroll'))) out.problems.push('AI Chat: the line is not in the chat list pane');
  return out })"""
SH10_VISITS = [('home', 'lee', v_goto('#/')), ('quiz', 'sam', v_quiz('practice')), ('quiz', 'riley', v_quiz('exam')),
               ('result', 'sam', v_result), ('records', 'lee', v_goto('#/records')), ('ai', 'sam', v_goto('#/ai')),
               ('ai-settings', 'admin', v_goto('#/ai-settings')), ('users', 'admin', v_goto('#/users')),
               ('logs', 'admin', v_goto('#/logs')), ('settings', 'sam', v_goto('#/settings')), ('banks', 'admin', v_goto('#/banks')),
               ('about', 'sam', v_goto('#/about')), ('signin', None, v_goto('#/'))]


def g_sh10(rc):
    if not rc.want('W-SH-10'):
        return
    ver = rc.session('admin').call('/api/info').get('version', '')
    for page, user, setup in SH10_VISITS:
        if not rc.walk.wants_page(page) and not (page == 'signin' and rc.args.pages):
            continue
        for lang in rc.langs(['zh']):
            for theme in rc.themes(['light']):
                for size in rc.sizes([(360, 740), (390, 844), (1280, 800)]):
                    rc.guard('W-SH-10', page, sh10_case, rc, page, user, setup, ver, lang, theme, size)


def sh10_case(rc, page, user, setup, ver, lang, theme, size):
    d = rc.dev(user, lang, theme, size)
    setup(rc, d, size)
    time.sleep(0.4)
    d.js(CHECKS_JS)
    r = d.js('(%s)(%s)' % (FOOTER_JS % VIS, json.dumps(ver)))
    problems = list(r['problems']) if r['n'] == 1 else ['%d visible GitHub lines' % r['n']]
    if r['n'] == 1 and phone(size):
        a = W(d, 'audit', {'touch': True, 'within': '.gh-line'})
        problems += ['small link: ' + x for x in a['small']]
    rc.rec('W-SH-10', page, user or 'anon', user, size, lang, theme, not problems,
           '; '.join(problems) or 'one centred GitHub line, two links, version %s, nothing under it' % ver, d if problems else None)
    if r.get('short'):
        rc.rec('W-SH-10', page, 'short-page', user, size, lang, theme, False,
               'a short page: %d px blank under the GitHub line (footer after the content; the owner decides)' % r['short'], d)


# ---- W-SH-8: nothing in the AI+ panel / the summary overlay navigates; the Saved text and its tip
SAVED_JS = """((sel) => { const s = document.querySelector(sel); if (!s) return null; const cs = getComputedStyle(s);
  const id = s.getAttribute('aria-describedby'); const tip = id ? document.getElementById(id) : null;
  return {vis: cs.visibility !== 'hidden' && s.getClientRects().length > 0, tag: s.tagName, href: s.hasAttribute('href'), role: s.getAttribute('role'),
          text: s.innerText.trim(), tip: tip ? {cls: tip.className, role: tip.getAttribute('role'), text: tip.textContent.trim(),
          shown: getComputedStyle(tip).display !== 'none' && getComputedStyle(tip).visibility !== 'hidden'} : null} })"""
NO_NAV_JS = """((root) => { const r = document.querySelector(root); if (!r) return null; const rx = new RegExp(%s);
  return {links: r.querySelectorAll('a[href]').length, old: [...r.querySelectorAll('*')].filter(e => rx.test(e.getAttribute('aria-label') || '') || (e.children.length === 0 && rx.test(e.textContent || ''))).length} })"""
FOOT_XS = """((sel) => [...document.querySelectorAll(sel)].filter(e => e.getClientRects().length).map(e => Math.round(e.getBoundingClientRect().left)))"""


def g_sh8(rc):
    if not rc.want('W-SH-8'):
        return
    for lang in rc.langs(['zh']):
        for size in rc.sizes([(390, 844), (1280, 800)], lambda s: s in [(390, 844), (1280, 800)]):
            if rc.walk.wants_page('quiz'):
                rc.guard('W-SH-8', 'quiz', sh8_panel, rc, lang, rc.themes(['light'])[0], size)
            if rc.walk.wants_page('result'):
                rc.guard('W-SH-8', 'result', sh8_result, rc, lang, rc.themes(['light'])[0], size)
            if rc.walk.wants_page('home'):
                rc.guard('W-SH-8', 'home', sh8_summary, rc, lang, rc.themes(['light'])[0], size)


def wait_shown(d, sel, seconds=15):
    return until(d, "(%s)(document.querySelector(%s))" % (VIS, json.dumps(sel)), seconds)


def saved_problems(rc, d, lang, size, sel, user):
    """The Saved text: a muted non-control text whose tip names the chat as the AI Chat list does."""
    s = d.js('(%s)(%s)' % (SAVED_JS, json.dumps(sel)))
    if not s or not s['vis']:
        return ['no visible Saved text (%s)' % sel]
    out = []
    if s['text'] != cat(lang, 'aiplus.saved'):
        out.append('Saved text %r' % s['text'])
    if s['tag'] in ('A', 'BUTTON') or s['href'] or s['role'] in ('button', 'link'):
        out.append('the Saved text is a control (%s, role %s)' % (s['tag'], s['role']))
    tip = s['tip']
    if not tip or 'tip' not in tip['cls'].split() or tip['role'] != 'tooltip':
        out.append('no tooltip described by the Saved text')
    else:
        pre, _, post = cat(lang, 'aiplus.savedTip').partition('{title}')
        title = tip['text'][len(pre):len(tip['text']) - len(post)] if tip['text'].startswith(pre) and tip['text'].endswith(post) else None
        chats = rc.session(user).call('/api/ai/chats').get('chats', [])
        if not title or not any(digits(c.get('title')) and digits(c.get('title')) == digits(title) for c in chats):
            out.append('the tip %r names no chat of the list' % tip['text'])
        r = d.rect(sel)
        if phone(size):
            d.tap(r['cx'], r['cy'], 0.5)
        else:
            d.cdp.call('Input.dispatchMouseEvent', type='mouseMoved', x=r['cx'], y=r['cy'])
            time.sleep(0.6)
        s2 = d.js('(%s)(%s)' % (SAVED_JS, json.dumps(sel)))
        if not (s2 and s2['tip'] and s2['tip']['shown']):
            out.append('a %s does not show the tip' % ('tap' if phone(size) else 'hover'))
    return out


def tap_all(d, sels, still):
    """Taps / clicks every visible control and text of `sels` (in order); returns the ones after which the page moved."""
    moved = []
    for sel in sels:
        n = d.js('document.querySelectorAll(%s).length' % json.dumps(sel))
        for i in range(n):
            h0 = d.js('location.hash')
            r = d.js("""((s, i) => { const e = document.querySelectorAll(s)[i]; if (!e || !e.getClientRects().length || getComputedStyle(e).visibility === 'hidden' || e.disabled) return null;
              const b = e.getBoundingClientRect(); return {x: b.left + b.width / 2, y: b.top + b.height / 2} })""" + '(%s, %d)' % (json.dumps(sel), i))
            if not r or r['y'] < 0 or r['y'] > d.h:
                continue
            (d.tap if phone((d.w, d.h)) else d.click)(r['x'], r['y'], 0.7)
            d.js("document.querySelectorAll('dialog[open]:not(#aip-sum)').forEach(x => x.close()); 1")
            if d.js('location.hash') != h0 or not d.js('!!document.querySelector(%s)' % json.dumps(still)):
                moved.append('%s[%d]' % (sel, i))
                return moved
    return moved


def sh8_panel(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    start_quiz(d, 'practice')
    problems = []
    d.tap_sel('.aip-panel-head', wait=0.8)       # open the panel without an ask: nothing answered yet
    before = d.js("(() => { const s = document.querySelector('#aip-panel .aip-saved'); return s ? getComputedStyle(s).visibility : 'none' })()")
    xs0 = d.js('(%s)("#aip-panel .aip-panel-foot button")' % FOOT_XS)
    if before not in ('hidden', 'none'):
        problems.append('the Saved slot shows before any answer (%s)' % before)
    tap_ai(d, 'hint')
    if not wait_shown(d, '#aip-panel .aip-saved'):
        problems.append('no Saved text after the hint')
    else:
        xs1 = d.js('(%s)("#aip-panel .aip-panel-foot button")' % FOOT_XS)
        if xs0 and (len(xs0) != len(xs1) or any(abs(a - b) > 1 for a, b in zip(xs0, xs1))):
            problems.append('footer buttons moved %s -> %s' % (xs0, xs1))
        problems += saved_problems(rc, d, lang, size, '#aip-panel .aip-saved', 'sam')
    nn = d.js('(%s)("#aip-panel")' % (NO_NAV_JS % json.dumps(OLD_OPEN_CHAT)))
    if nn and (nn['links'] or nn['old']):
        problems.append('%d links, %d old Open in AI Chat controls in the panel' % (nn['links'], nn['old']))
    moved = tap_all(d, ['#aip-panel .aip-panel-foot button', '#aip-panel .aip-saved', '#aip-panel .aip-panel-title', '#aip-panel .aip-panel-head button'], '.pfoot')
    if moved:
        problems.append('the page moved after tapping ' + ', '.join(moved))
    rc.rec('W-SH-8', 'quiz', 'panel', 'sam', size, lang, theme, not problems, '; '.join(problems) or 'Saved text and tip as specified; nothing navigates', d if problems else None)


def sh8_result(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    make_result(d, 6)
    problems = []
    d.tap_sel('.aip-ask', wait=0.8)
    d.js("window.__settle('#aip-panel-body')")
    if not wait_shown(d, '#aip-panel .aip-saved'):
        problems.append('no Saved text after AI analysis')
    else:
        problems += saved_problems(rc, d, lang, size, '#aip-panel .aip-saved', 'sam')
    nn = d.js('(%s)("#aip-panel")' % (NO_NAV_JS % json.dumps(OLD_OPEN_CHAT)))
    if nn and (nn['links'] or nn['old']):
        problems.append('%d links, %d old Open in AI Chat controls on the card' % (nn['links'], nn['old']))
    moved = tap_all(d, ['#aip-panel .aip-panel-foot button', '#aip-panel .aip-saved', '#aip-panel .aip-panel-title'], '.rv')
    if moved:
        problems.append('the page moved after tapping ' + ', '.join(moved))
    rc.rec('W-SH-8', 'result', 'card', 'sam', size, lang, theme, not problems, '; '.join(problems) or 'Saved text and tip as specified; nothing navigates', d if problems else None)


def sh8_summary(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/', 1.8)
    if not d.js("!!document.querySelector('.home-latest button.aip-sum-btn:not([aria-disabled=true])')"):
        make_result(d, 4)
        goto(d, '#/', 1.8)
    problems = []
    if not d.js("!!document.querySelector('.home-latest button.aip-sum-btn')"):
        rc.rec('W-SH-8', 'home', 'summary', 'sam', size, lang, theme, None, 'no AI summary button on Home')
        return
    d.tap_sel('.home-latest button.aip-sum-btn', wait=1.0)
    if not wait_shown(d, '#aip-sum .aip-sum-saved', 20):
        problems.append('no Saved text in the summary window')
    else:
        problems += saved_problems(rc, d, lang, size, '#aip-sum .aip-sum-saved', 'sam')
    nn = d.js('(%s)("#aip-sum")' % (NO_NAV_JS % json.dumps(OLD_OPEN_CHAT)))
    if nn and (nn['links'] or nn['old']):
        problems.append('%d links, %d old Open in AI Chat controls in the window' % (nn['links'], nn['old']))
    moved = tap_all(d, ['#aip-sum footer button', '#aip-sum .aip-sum-saved'], '.page-home')
    if moved:
        problems.append('the page moved after tapping ' + ', '.join(moved))
    d.js("document.querySelectorAll('dialog[open]').forEach(x => x.close()); 1")
    rc.rec('W-SH-8', 'home', 'summary', 'sam', size, lang, theme, not problems, '; '.join(problems) or 'Saved text and tip as specified; nothing navigates', d if problems else None)


# ---- W-SH-9: titles focused by script show no focus frame; the next real control does
FOCUS_JS = """(() => { const a = document.activeElement; if (!a) return null; const cs = getComputedStyle(a);
  return {id: a.id, tag: a.tagName, outline: cs.outlineStyle, shadow: cs.boxShadow} })()"""


def g_sh9(rc):
    if not rc.want('W-SH-9'):
        return
    for theme in rc.themes(['light', 'dark']):
        for size in rc.sizes([(390, 844), (1280, 800)], lambda s: s in [(390, 844), (1280, 800)]):
            rc.guard('W-SH-9', 'home', sh9_case, rc, rc.langs(['zh'])[0], theme, size)


def frame_problems(f, want_id, what):
    if not f or f['id'] != want_id:
        return ['%s: focus on %s, not the title' % (what, f and (f['id'] or f['tag']))]
    if f['outline'] != 'none' or (f['shadow'] and f['shadow'] != 'none'):
        return ['%s: the title shows a frame (outline %s, shadow %s)' % (what, f['outline'], f['shadow'])]
    return []


def press_enter(d):
    """Enter on the focused control, as a keyboard sends it (keyDown with the text, so a button is activated)."""
    d.cdp.call('Input.dispatchKeyEvent', type='keyDown', key='Enter', code='Enter', windowsVirtualKeyCode=13, text='\r')
    d.cdp.call('Input.dispatchKeyEvent', type='keyUp', key='Enter', code='Enter', windowsVirtualKeyCode=13)


def tab_ring(d, what):
    d.key('Tab', vk=9)
    time.sleep(0.3)
    f = d.js(FOCUS_JS)
    return [] if f and f['outline'] != 'none' else ['%s: Tab to %s shows no ring' % (what, f and (f['id'] or f['tag']))]


def sh9_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/', 1.8)
    if not d.js("!!document.querySelector('.home-latest button.aip-sum-btn:not([aria-disabled=true])')"):
        make_result(d, 4)
        goto(d, '#/', 1.8)
    problems = []
    how = 'tap' if phone(size) else 'click'
    d.tap_sel('.home-latest button.aip-sum-btn', wait=1.2)
    problems += frame_problems(d.js(FOCUS_JS), 'aip-sum-h', 'overlay by ' + how)
    d.key('Escape', vk=27)
    time.sleep(0.5)
    d.js("(() => { const b = document.querySelector('.home-latest button.aip-sum-btn'); b.scrollIntoView({block: 'center'}); b.focus(); })()")
    press_enter(d)
    time.sleep(1.2)
    problems += frame_problems(d.js(FOCUS_JS), 'aip-sum-h', 'overlay by keyboard')
    problems += tab_ring(d, 'overlay')
    d.key('Escape', vk=27)
    time.sleep(0.4)
    start_quiz(d, 'practice')
    d.js("(() => { const b = document.querySelector('.pfoot [data-slot=\"hint\"]'); b.focus(); })()")
    press_enter(d)
    time.sleep(1.2)
    problems += frame_problems(d.js(FOCUS_JS), 'aip-panel-h', 'panel by keyboard')
    problems += tab_ring(d, 'panel')
    rc.rec('W-SH-9', 'home', 'titles', 'sam', size, lang, theme, not problems,
           '; '.join(problems) or 'overlay (%s, keyboard) and panel titles focused without a frame; Tab shows the ring' % how, d if problems else None)


# ---- W-SH-6 additions (Revision 4): the footer's two lines and the Saved text
SAMPLES['home'] += [('footer disclaimer', '.foot-note'), ('GitHub line', '.gh-link')]
SAMPLES['quiz-saved'] = [('Saved text', '.aip-saved')]


def saved_setup(d):
    start_quiz(d, 'practice')
    tap_ai(d, 'hint')
    wait_shown(d, '#aip-panel .aip-saved')


# ---------------------------------------------------------------------------------------------------- v1.1 checks
# W-IM-1 (the AI import card of Bank management), W-GB-1 (Home's AI button), W-BT-1 (every badge has a tip), W-FP-1 (six boxes in two
# groups), W-UC-1 (the usage charts), W-AS-1v (the top-row 交卷 / 完成 button and count, no finish button in the narrow answer sheet), W-FT-1 (footer space), W-SI-1 (sign-in).
# They use only the classes of the v1.1 components (.bk-aicard, .gen-home, .ai-badge-tip, .ais-feats, .uc-chart ...).
NO_SIDE_SCROLL = "(() => document.documentElement.scrollWidth <= window.innerWidth + 1)()"
BADGE_TIPS = """(() => {
  const out = [];
  for (const b of document.querySelectorAll('.ai-badge')) {
    const host = b.closest('.has-tip');
    const tip = host ? host.querySelector(':scope > .tip') : null;
    const text = tip ? (tip.textContent || '').trim() : '';
    out.push({where: (b.closest('[id], .nav-who-tags, .set-role, .us-row, .ais-badge, .aip-panel-head, .aip-na-bar, .home-bankhead, .bk-bankhead') || b.parentElement || {}).className || '?', tip: text});
  }
  return out })()"""
LABEL_OVERLAP = """(() => {
  const res = [];
  for (const svg of document.querySelectorAll('.uc-chart svg')) {
    const bars = !!svg.closest('.uc-bars');
    const rs = [...svg.querySelectorAll('.uc-label')].filter(e => !(bars && e.hasAttribute('transform'))).map(e => e.getBoundingClientRect()).filter(r => r.width > 0);
    let bad = 0;
    for (let i = 0; i < rs.length; i++) for (let j = i + 1; j < rs.length; j++) {
      const a = rs[i], b = rs[j];
      if (a.left < b.right - 0.5 && b.left < a.right - 0.5 && a.top < b.bottom - 0.5 && b.top < a.bottom - 0.5) bad++;
    }
    const box = svg.getBoundingClientRect();
    res.push({label: svg.getAttribute('aria-label'), role: svg.getAttribute('role'), bars, overlaps: bad, w: Math.round(box.width), host: Math.round(svg.parentElement.getBoundingClientRect().width)});
  }
  return res })()"""
FEAT_ROWS = """(() => {
  const rows = [...document.querySelectorAll('.ais-feats')].map(f => {
    // v1.1 (A-OWNER-21): an administrator row also has the free-mode switch (.ais-fg-free), not one of the six boxes
    const boxes = f.querySelectorAll('.ais-fg:not(.ais-fg-free) input[type=checkbox]').length;
    const groups = [...f.querySelectorAll('.ais-fg:not(.ais-fg-free)')].map(g => new Set([...g.querySelectorAll('label')].map(l => Math.round(l.getBoundingClientRect().top))).size);
    const free = f.querySelectorAll('.ais-fg-free input[role=switch]').length;
    const admin = [...f.querySelectorAll('.ais-fg:not(.ais-fg-free) input')].every(i => i.disabled);
    return {boxes, groups, admin, free, freeOk: admin ? free === 1 : free === 0};
  });
  return rows })()"""


def g_v11(rc):
    for gid, fn in (('W-IM-1', im1_case), ('W-IM-2', im2_case), ('W-GB-1', gb1_case), ('W-BT-1', bt1_case), ('W-FP-1', fp1_case), ('W-UC-1', uc1_case), ('W-UC-2', uc2_case),
                    ('W-AS-1v', as1_case), ('W-FT-1', ft1_case), ('W-SI-1', si1_case), ('W-BK-29', bk29_case), ('W-AB-1', ab1_case), ('W-L26', l26_case), ('W-AI-BTN', aibtn_case), ('W-L73', l73_case)):
        if not rc.want(gid):
            continue
        for lang in rc.langs(['zh']):
            sizes = rc.sizes([(360, 740), (390, 844), (412, 915)] if gid in ('W-IM-1', 'W-IM-2', 'W-GB-1', 'W-FP-1', 'W-UC-1')
                             else [(390, 844)], lambda s: True)
            if gid == 'W-L26':
                sizes = [(390, 844)]
            if gid == 'W-L73':
                sizes = [(360, 740), (390, 844), (1280, 800)]
            if gid == 'W-AI-BTN':
                for theme in rc.themes(['light', 'dark']):
                    for size in [(390, 844), (1280, 800)]:
                        rc.guard(gid, 'v11', fn, rc, lang, theme, size)
                continue
            if gid == 'W-UC-2':
                for theme in rc.themes(['light', 'dark']):
                    for size in [(320, 640), (390, 844), (1280, 800)]:
                        rc.guard(gid, 'v11', fn, rc, lang, theme, size)
                continue
            if gid == 'W-IM-1':
                sizes = [(360, 740), (390, 844), (412, 915), (768, 1024), (1280, 800)]
            if gid in ('W-AS-1v', 'W-BK-29'):
                sizes = [(320, 640), (360, 740), (390, 844), (768, 1024), (1280, 800)]
            for size in sizes:
                rc.guard(gid, 'v11', fn, rc, lang, rc.themes(['light'])[0], size)


def l26_case(rc, lang, theme, size):
    """L-26: a generation started from Home, whose job finishes while the user stays on Home, shows its new bank row there
    without a reload"""
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/', 2.0)
    # an AI+ bank in the scope greys the button: untick the AI+ banks first (order-independent)
    d.js("""(() => { for (const row of document.querySelectorAll('.home-setup input[type=checkbox]')) { const host = row.closest('.home-bankhead'); if (row.checked && host && host.querySelector('.ai-badge')) row.click(); } return 1 })()""")
    time.sleep(0.5)
    if not d.js("!!document.querySelector('.home-setup-head .gen-btn:not([aria-disabled=true])')"):
        rc.rec('W-L26', 'home', 'refresh', 'admin', size, lang, theme, None, 'no usable AI button on Home on this stack')
        return
    name = 'L26walk%s%d' % (lang, size[0])
    d.tap_sel('.home-setup-head .gen-btn')
    if not until(d, "document.querySelector('dialog[open] #gen-name')", 6.0):
        rc.rec('W-L26', 'home', 'refresh', 'admin', size, lang, theme, False, 'the generation dialog did not open', d)
        return
    d.js("(() => { const i = document.querySelector('dialog[open] #gen-name'); i.value = %r; i.dispatchEvent(new Event('input', {bubbles: true})); return 1 })()" % name)
    d.js("document.querySelector('dialog[open] .acts .btn.primary').click()")
    ok = until(d, "document.body.innerText.includes(%r)" % name, 60.0)
    rc.rec('W-L26', 'home', 'refresh', 'admin', size, lang, theme, ok,
           'the new bank row appeared on Home without a reload' if ok else 'the finished job did not redraw Home (no row %s after 60 s)' % name,
           d if not ok else None)


# A-OWNER-64.3 / 65 / 65b / 65c: the AI+ action buttons (AI import, Home and Bank-management generation) share the darker orange
# with white text; greyed they are plain grey buttons with no reason line, the reason is in a tip on tap / hover / focus
AIBTN_JS = """((sel) => { const b = document.querySelector(sel); if (!b) return null;
  const cs = getComputedStyle(b), rgb = (c) => (c.match(/[\\d.]+/g) || []).map(Number);
  const lin = (v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); };
  const L = (c) => 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
  const bg = rgb(cs.backgroundColor), fg = rgb(cs.color);
  const hi = Math.max(L(bg), L(fg)) + 0.05, lo = Math.min(L(bg), L(fg)) + 0.05;
  const r = b.getBoundingClientRect(); const wrap = b.closest('.has-tip'); const tip = wrap && wrap.querySelector(':scope > .tip');
  return {bg: cs.backgroundColor, fg: cs.color, ratio: Math.round(hi / lo * 100) / 100, dis: b.getAttribute('aria-disabled') === 'true',
          h: Math.round(r.height), desc: b.getAttribute('aria-describedby'), tipId: tip ? tip.id : null, tipText: tip ? tip.textContent.trim() : '',
          tipTap: !!(wrap && wrap.classList.contains('tip-tap'))} })(%s)"""
# A-OWNER-73b: flat amber #F59E0B with white text (the owner's pick; its text outline carries the contrast, so no 4.5:1 rule here)
ORANGE = 'rgb(245, 158, 11)'
AIBTN_UNTICK = """(() => { for (const row of document.querySelectorAll('.home-setup input[type=checkbox]')) { const host = row.closest('.home-bankhead'); if (row.checked && host && host.querySelector('.ai-badge')) row.click(); } return 1 })()"""
AIBTN_TICK = """(() => { for (const row of document.querySelectorAll('.home-setup input[type=checkbox]')) { const host = row.closest('.home-bankhead'); if (!row.checked && host && host.querySelector('.ai-badge')) { row.click(); return 1 } } return 0 })()"""


def aibtn_look(d, sel, what, problems, want_dis, coarse):
    """Colour / state of one AI button; a greyed one must be grey (not orange), with the reason only in its tip"""
    r = d.js(AIBTN_JS % json.dumps(sel))
    if r is None:
        problems.append('%s: no button' % what)
        return None
    if not want_dis:
        if r['dis']:
            problems.append('%s: greyed, expected usable' % what)
        elif r['bg'] != ORANGE or r['fg'] != 'rgb(255, 255, 255)':
            problems.append('%s: %s on %s, expected white on %s' % (what, r['fg'], r['bg'], ORANGE))
    else:
        if not r['dis']:
            problems.append('%s: usable, expected greyed' % what)
            return r
        if r['bg'] == ORANGE or r['fg'] == 'rgb(255, 255, 255)':
            problems.append('%s: greyed but still orange / white (%s on %s)' % (what, r['fg'], r['bg']))
        if not (r['tipId'] and r['tipText'] and r['desc'] == r['tipId'] and r['tipTap']):
            problems.append('%s: the reason is not a tap tip described by the button (%s)' % (what, r['tipId']))
    if coarse and r['h'] < 40:
        problems.append('%s: %d px high' % (what, r['h']))
    return r


def aibtn_tip(d, sel, what, problems):
    """Tap the greyed button: the tip box shows inside the window, no toast, nothing starts; Escape closes it"""
    d.tap_sel(sel)
    time.sleep(0.3)
    q = json.dumps(sel)
    r = d.js("""((sel) => { const b = document.querySelector(sel), w = b && b.closest('.has-tip'), tip = w && w.querySelector(':scope > .tip');
      const cs = tip ? getComputedStyle(tip) : null, rc = tip ? tip.getBoundingClientRect() : null;
      return {open: !!tip && cs.display !== 'none' && rc.width > 0, inside: !!rc && rc.left >= 0 && rc.right <= innerWidth + 1 && rc.top >= 0,
              toast: !!tip && !!document.querySelector('#toast.show') && document.getElementById('toast').textContent.trim() === tip.textContent.trim(), dialog: !!document.querySelector('dialog[open]')} })(%s)""" % q)
    if not r['open']:
        problems.append('%s: a tap shows no tip box' % what)
    elif not r['inside']:
        problems.append('%s: the tip box is outside the window' % what)
    if r['toast']:
        problems.append('%s: a toast shows besides the tip' % what)
    if r['dialog']:
        problems.append('%s: a tap on the greyed button started something' % what)
    d.key('Escape', 'Escape', 27)
    time.sleep(0.2)
    if d.js("""((sel) => { const w = document.querySelector(sel).closest('.has-tip'), tip = w.querySelector(':scope > .tip'); return getComputedStyle(tip).display !== 'none' })(%s)""" % q):
        problems.append('%s: Escape does not close the tip' % what)


def aibtn_banks(d, who, problems, seen, coarse):
    """The Bank management toolbar generation button and the AI import button of the signed-in device, in whatever state they are"""
    goto(d, '#/banks', 2.0)
    st = d.js(AIBTN_JS % json.dumps('.bk-selbar .gen-sel'))
    if st is not None:
        aibtn_look(d, '.bk-selbar .gen-sel', who + 'Banks toolbar', problems, st['dis'], coarse)
        seen.append(who + 'Banks toolbar ' + ('greyed' if st['dis'] else 'usable'))
        if st['dis']:
            aibtn_tip(d, '.bk-selbar .gen-sel', who + 'Banks toolbar', problems)
    imp = d.js(AIBTN_JS % json.dumps('.bk-aicard .gen-impbtn'))
    if imp is not None:
        aibtn_look(d, '.bk-aicard .gen-impbtn', who + 'AI import', problems, imp['dis'], coarse)
        seen.append(who + 'AI import ' + ('greyed' if imp['dis'] else 'usable'))
        if d.js("!!document.querySelector('.gen-impwhy')"):
            problems.append(who + 'AI import: a reason line is under the button')
        if imp['dis']:
            aibtn_tip(d, '.bk-aicard .gen-impbtn', who + 'AI import', problems)


def aibtn_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    coarse = phone(size)
    problems = []
    seen = []
    # Home: no AI+ bank ticked -> usable orange; one ticked -> grey with the tip only
    goto(d, '#/', 2.0)
    d.js(AIBTN_UNTICK)
    time.sleep(0.5)
    if aibtn_look(d, '.home-setup-head .gen-btn', 'Home usable', problems, False, coarse):
        seen.append('Home usable')
    if d.js(AIBTN_TICK):
        time.sleep(0.5)
        r = aibtn_look(d, '.home-setup-head .gen-btn', 'Home greyed', problems, True, coarse)
        if r:
            seen.append('Home greyed')
            n = d.js("""((t) => { let c = 0; for (const e of document.querySelectorAll('body *')) { if (e.closest('.tip, .hold-tip, [hidden], #toast')) continue; if ([...e.childNodes].some((x) => x.nodeType === 3 && x.textContent.includes(t))) c++; } return c })(%s)""" % json.dumps(r['tipText'][:12]))
            if n:
                problems.append('Home greyed: the reason is also written on the page (%d places)' % n)
            aibtn_tip(d, '.home-setup-head .gen-btn', 'Home greyed', problems)
        d.js(AIBTN_UNTICK)
    # Bank management: the toolbar button and the AI import button, whatever state they are in (admin, then a bank manager)
    aibtn_banks(d, 'admin ', problems, seen, coarse)
    d2 = rc.dev('sam', lang, theme, size)
    aibtn_banks(d2, 'sam ', problems, seen, coarse)
    if not seen:
        rc.rec('W-AI-BTN', 'home', 'ai-buttons', 'admin', size, lang, theme, None, 'no AI button on this stack')
        return
    rc.rec('W-AI-BTN', 'home', 'ai-buttons', 'admin', size, lang, theme, not problems,
           '; '.join(problems) or 'AI+ buttons (%s): white on %s, greyed ones plain grey with the reason only in the tip' % (', '.join(seen), ORANGE),
           d if problems else None)


def im1_case(rc, lang, theme, size):
    """A-OWNER-35 / 37 / 29b: the AI import is its own card (not inside the Excel card) with a heading, the limits note
    (types, MB and the question limit), the button with "or drop it here" and the notice; the button is the Excel button's
    twin and aligned with it"""
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/banks', 2.0)
    r = d.js("""(() => { const card = document.querySelector('.bk-aicard'); if (!card) return null;
      const btn = card.querySelector('.gen-impbtn, button'); const note = card.querySelector('.gen-impnote');
      const lh = note ? parseFloat(getComputedStyle(note).lineHeight) || 20 : 20;
      const x = document.querySelector('.bk-choose'); const rx = x ? x.getBoundingClientRect() : null; const rb = btn ? btn.getBoundingClientRect() : null;
      return {btnH: rb ? Math.round(rb.height) : 0, btnW: rb ? Math.round(rb.width) : 0, cardW: Math.round(card.getBoundingClientRect().width),
              noteLines: note ? Math.round(note.getBoundingClientRect().height / lh) : 0, note: note ? note.textContent : '',
              inExcel: !!document.querySelector('#bk-import .gen-impbtn'), head: !!card.querySelector('h2'),
              drop: !!card.querySelector('.bk-droprow'), over: card.classList.contains('bk-drop'),
              twin: rx && rb ? {dw: Math.round(Math.abs(rx.width - rb.width)), dh: Math.round(Math.abs(rx.height - rb.height)), dl: Math.round(Math.abs(rx.left - rb.left))} : null} })()""")
    if r is None:
        rc.rec('W-IM-1', 'banks', 'ai-card', 'sam', size, lang, theme, None, 'no AI card (AI+ not usable for this account on this stack)')
        return
    problems = []
    if not d.js(NO_SIDE_SCROLL):
        problems.append('sideways scroll')
    if r['inExcel']:
        problems.append('the AI button is inside the Excel card')
    if not (r['head'] and r['drop'] and r['over']):
        problems.append('the AI card lacks its heading / drop zone')
    if not re.search(r'\d+', r['note']) or len(re.findall(r'\d+', r['note'])) < 3:
        problems.append('the note does not name the size and the question limit: %s' % r['note'][:60])
    if phone(size) and r['btnH'] < 44:
        problems.append('button %d px high' % r['btnH'])
    if r['noteLines'] > 5:
        problems.append('note %d lines' % r['noteLines'])
    t = r['twin']
    if t and (t['dw'] > 2 or t['dh'] > 2 or t['dl'] > 2):
        problems.append('the AI button is not the twin of the Excel button (%s)' % t)
    rc.rec('W-IM-1', 'banks', 'ai-card', 'sam', size, lang, theme, not problems,
           '; '.join(problems) or 'AI card: button %d px high (twin of the Excel button), note %d lines, no sideways scroll' % (r['btnH'], r['noteLines']),
           d if problems else None)
    # a user without AI+ sees no card (A-OWNER-15)
    d2 = rc.dev('riley', lang, theme, size)
    goto(d2, '#/banks', 1.6)
    if d2.js("!!document.querySelector('.bk-aicard')"):
        rc.rec('W-IM-1', 'banks', 'no-ai', 'riley', size, lang, theme, False, 'a user without the AI+ feature sees the AI card', d2)


def im2_case(rc, lang, theme, size):
    """A-OWNER-33: a failed import card has a close button in its top-right corner (44 px on a phone) that removes the card.
    The made-up file has no text, so it fails on the server with no AI call and no daily job used."""
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/banks', 2.0)
    if not d.js("!!document.querySelector('#bk-aifile')"):
        rc.rec('W-IM-2', 'banks', 'failed-card', 'sam', size, lang, theme, None, 'no AI card (AI+ not usable for this account on this stack)')
        return
    until(d, "document.querySelector('.bk-aicard .gen-impbtn:not([disabled])')", 10.0)
    # a file with only a space: the upload is accepted, the server finds no text and fails the job with no AI call and no
    # charge (A-OWNER-47 / 58b: a long file is not a local error any more); only the import notice dialog is confirmed
    d.js("""(() => {
      const dt = new DataTransfer(); dt.items.add(new File([' '], 'walk-blank.txt', {type: 'text/plain'}));
      const input = document.querySelector('#bk-aifile'); input.files = dt.files;
      input.dispatchEvent(new Event('change', {bubbles: true})); return 1 })()""")
    notice = "document.querySelector('dialog[open]:has(.gen-file-line) .acts .btn.primary')"
    if until(d, notice, 6.0):
        d.js(notice + '.click()')
    if not until(d, "document.querySelector('.gen-file .gen-x')", 25.0):
        seen = d.js("(() => [...document.querySelectorAll('.gen-card, .gen-error, [role=alert], #toast, dialog[open]')].map((e) => e.className + ': ' + e.textContent.trim().slice(0, 80)).join(' | '))()")
        rc.rec('W-IM-2', 'banks', 'failed-card', 'sam', size, lang, theme, False, 'no failed import card with a close button appeared (page: %s)' % seen, d)
        return
    r = d.js("""(() => { const card = document.querySelector('.gen-file'), x = card.querySelector('.gen-x');
      const rc = card.getBoundingClientRect(), rx = x.getBoundingClientRect(), cs = getComputedStyle(card);
      const pad = parseFloat(cs.paddingTop) || 0;
      return {w: Math.round(rx.width), h: Math.round(rx.height), right: Math.round(rc.right - rx.right), top: Math.round(rx.top - rc.top),
              coarse: matchMedia('(pointer: coarse)').matches, pad: Math.round(pad)} })()""")
    problems = []
    if r['coarse'] and (r['w'] < 44 or r['h'] < 44):
        problems.append('close button %dx%d px (want 44)' % (r['w'], r['h']))
    if r['right'] > 24 or r['top'] > 24 or r['right'] < -12 or r['top'] < -12:
        problems.append('close button is not in the top-right corner (%d px from the right, %d from the top)' % (r['right'], r['top']))
    d.js("document.querySelector('.gen-file .gen-x').click()")
    if not until(d, "!document.querySelector('.gen-file')", 5.0):
        problems.append('tapping the close button did not remove the card')
    rc.rec('W-IM-2', 'banks', 'failed-card', 'sam', size, lang, theme, not problems,
           '; '.join(problems) or 'failed card: close button %dx%d px in the top-right corner, tapping it removes the card' % (r['w'], r['h']),
           d if problems else None)


def bk29_case(rc, lang, theme, size):
    # A-OWNER-29 / 29b / 30: no text element of Bank management under about 4 characters wide, no per-row AI action, and the
    # AI import button is the same box as the Excel button
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/banks', 2.0)
    if not d.js("!!document.querySelector('.bk-bank .ai-badge')"):
        # the check needs a bank row with the AI+ badge: make one (long name) from Home, once per stack
        goto(d, '#/', 2.0)
        if d.js("!!document.querySelector('.home-setup-head .gen-btn:not([aria-disabled=true])')"):
            d.tap_sel('.home-setup-head .gen-btn')
            if until(d, "document.querySelector('dialog[open] #gen-name')", 6.0):
                d.js("(() => { const i = document.querySelector('dialog[open] #gen-name'); i.value = 'Bk29 walk sample bank with a rather long name 2026'; i.dispatchEvent(new Event('input', {bubbles: true})); return 1 })()")
                d.js("document.querySelector('dialog[open] .acts .btn.primary').click()")
                until(d, "document.body.innerText.includes('Bk29 walk sample')", 60.0)
        goto(d, '#/banks', 2.0)
    r = d.js("""(() => { const out = {narrow: [], rowGen: document.querySelectorAll('.bk-bank .gen-btn, .bk-bank .gen-host').length};
      for (const el of document.querySelectorAll('.bk-bank, .bk-bank *')) {
        if (el.closest('svg') || el.closest('[hidden]') || el.closest('.sr-only')) continue;
        const own = [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent.trim()).join('');
        if (own.length < 4) continue;
        const rc = el.getBoundingClientRect(); if (!rc.width || !rc.height) continue;
        const fs = parseFloat(getComputedStyle(el).fontSize) || 14;
        if (rc.width < fs * 3 && rc.height > fs * 2.2) out.narrow.push((el.className || el.tagName) + ' ' + Math.round(rc.width) + 'px: ' + own.slice(0, 20));
      }
      out.badge = [];
      const bar = document.querySelector('.bk-selbar'); out.barShown = !!bar && !bar.hidden && bar.getClientRects().length > 0;
      out.barBtns = bar ? [...bar.querySelectorAll('button')].map((b) => Math.round(b.getBoundingClientRect().height)) : [];
      out.coarse = matchMedia('(pointer: coarse)').matches;
      for (const card of document.querySelectorAll('.bk-bank')) {
        const bd = card.querySelector('.ai-badge'), nm = card.querySelector('.bk-bankname'); if (!bd || !nm) continue;
        const cr = card.getBoundingClientRect(), br = bd.getBoundingClientRect(), nr = nm.getBoundingClientRect(), hr = card.querySelector('.bk-bankhead').getBoundingClientRect();
        if (br.right > cr.right + 0.5) out.badge.push('badge past the card edge');
        if (innerWidth <= 900 && br.top < nr.bottom - 1) out.badge.push('badge beside the name');
        if (nr.width < hr.width * 0.5) out.badge.push('name only ' + Math.round(nr.width) + 'px');
      }
      const x = document.querySelector('.bk-choose'), a = document.querySelector('.gen-impbtn');
      if (x && a) { const rx = x.getBoundingClientRect(), ra = a.getBoundingClientRect();
        out.twin = {dw: Math.round(Math.abs(rx.width - ra.width)), dh: Math.round(Math.abs(rx.height - ra.height)), dl: Math.round(Math.abs(rx.left - ra.left))}; }
      return out; })()""")
    problems = []
    if r['narrow']:
        problems.append('narrow text: ' + '; '.join(r['narrow'][:3]))
    if r['rowGen']:
        problems.append('a per-row AI action is back')
    if not r['barShown']:
        problems.append('the selection toolbar is not always shown (the list jumps on the first tick)')
    elif r['coarse'] and any(h < 44 for h in r['barBtns']):
        problems.append('toolbar buttons under 44 px: %s' % r['barBtns'])
    if r['badge']:
        problems.append('AI row: ' + '; '.join(r['badge'][:2]))
    t = r.get('twin')
    if t and (t['dw'] > 2 or t['dh'] > 2 or t['dl'] > 2):
        problems.append('the AI import button is not the twin of the Excel button (%s)' % t)
    rc.rec('W-BK-29', 'banks', 'rows', 'sam', size, lang, theme, not problems,
           '; '.join(problems) or 'no text under 3 em wide, no row AI action, import button matches the Excel button', d if problems else None)


def gb1_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/', 2.0)
    r = d.js("""(() => { const b = document.querySelector('.home-setup-head .gen-btn'); if (!b) return null;
      const r = b.getBoundingClientRect(); return {h: Math.round(r.height), w: Math.round(r.width), right: Math.round(r.right), vw: innerWidth} })()""")
    if r is None:
        rc.rec('W-GB-1', 'home', 'button', 'sam', size, lang, theme, None, 'no AI button (AI+ not usable for this account on this stack)')
        return
    problems = []
    if not d.js(NO_SIDE_SCROLL):
        problems.append('sideways scroll')
    if r['right'] > r['vw']:
        problems.append('the button leaves the screen')
    rc.rec('W-GB-1', 'home', 'button', 'sam', size, lang, theme, not problems,
           '; '.join(problems) or 'AI button %dx%d px inside the header, no sideways scroll' % (r['w'], r['h']), d if problems else None)


def bt1_case(rc, lang, theme, size):
    problems = []
    d = rc.dev('admin', lang, theme, size)
    for route in ('#/', '#/settings', '#/users', '#/ai-settings?tab=users'):
        goto(d, route, 2.0)
        for b in d.js(BADGE_TIPS) or []:
            if not b['tip']:
                problems.append('%s: a badge (%s) has no tip' % (route, b['where']))
        if d.js("(() => { const b = document.getElementById('acct-btn'); return !!b && b.hasAttribute('title') })()"):
            problems.append('%s: the account button still has a native title' % route)
    d2 = rc.dev('sam', lang, theme, size)
    goto(d2, '#/', 2.0)
    for b in d2.js(BADGE_TIPS) or []:
        if not b['tip']:
            problems.append('sam #/: a badge (%s) has no tip' % b['where'])
    rc.rec('W-BT-1', 'v11', 'badge-tips', 'admin', size, lang, theme, not problems, '; '.join(problems[:4]) or 'every badge has a non-empty tip', d if problems else None)


def fp1_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings?tab=users', 2.5)
    rows = d.js(FEAT_ROWS) or []
    if not rows:
        rc.rec('W-FP-1', 'ai-settings', 'features', 'admin', size, lang, theme, None, 'no AI+ users listed')
        return
    problems = []
    for i, r in enumerate(rows):
        if r['boxes'] != 6:
            problems.append('row %d has %d boxes' % (i, r['boxes']))
        if not r['freeOk']:
            problems.append('row %d (%s) has %d free-mode switches' % (i, 'administrator' if r['admin'] else 'user', r['free']))
        if len(r['groups']) != 2:
            problems.append('row %d has %d groups' % (i, len(r['groups'])))
        elif lang in ('zh', 'ja') and any(g > 1 for g in r['groups']):
            problems.append('row %d: a group wraps at %d px in %s' % (i, size[0], lang))
    if not any(r['admin'] for r in rows):
        problems.append('no administrator row with disabled boxes')
    rc.rec('W-FP-1', 'ai-settings', 'features', 'admin', size, lang, theme, not problems,
           '; '.join(problems[:3]) or '%d rows: six boxes in two groups' % len(rows), d if problems else None)


UC_BARS = """(() => {
  const out = [];
  for (const host of document.querySelectorAll('.uc-bars')) {
    const items = [...host.querySelectorAll('.uc-item')];
    const labels = items.map(g => (g.querySelector('.uc-label') || {}).textContent || '');
    const card = host.closest('.uc-card');
    out.push({n: items.length, labelled: labels.filter(Boolean).length, title: card ? card.querySelector('.uc-title').textContent : '',
      switchBtns: card ? card.querySelectorAll('.uc-sw-btn').length : 0, other: labels.some(l => /其他|Other|その他/.test(l)),
      stacked: host.querySelectorAll('.uc-bar-in').length});
  }
  return out })()"""
UC_TIPS = """(async () => {
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  const res = {};
  const bars = document.querySelector('.uc-bars');
  const g = bars && bars.querySelector('.uc-item');
  if (g) { g.dispatchEvent(new MouseEvent('click', {bubbles: true})); await wait(80);
    const tip = bars.querySelector('.uc-tip'); res.barTip = tip && !tip.hidden ? tip.textContent : ''; }
  const line = document.querySelector('.uc-line svg');
  if (line) {
    res.stroke = parseFloat(getComputedStyle(line.querySelector('.uc-stroke')).strokeWidth);
    const dots = [...line.querySelectorAll('.uc-dot')]; const dot = dots[dots.length - 1];
    if (dot) { const r = dot.getBoundingClientRect();
      // 10 px beside and 10 px above the point: inside the hit area, outside the point
      line.dispatchEvent(new PointerEvent('pointermove', {bubbles: true, pointerType: 'mouse', clientX: r.left + r.width / 2 - 4, clientY: r.top + r.height / 2 - 10}));
      await wait(80); const tip = document.querySelector('.uc-line .uc-tip'); res.lineTip = tip && !tip.hidden ? tip.textContent : '';
      line.dispatchEvent(new PointerEvent('pointermove', {bubbles: true, pointerType: 'mouse', clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 - 90}));
      await wait(80); res.lineFar = (() => { const t2 = document.querySelector('.uc-line .uc-tip'); return t2 && !t2.hidden; })(); }
  }
  return res })()"""


def uc1_case(rc, lang, theme, size):
    """W-UC-1 plus A-OWNER-39 / 40: every bar labelled, a 调用次数 | Token 数 switch on both bar charts (stacked tokens, the
    title follows), a tip on a bar and near the line, the line 3 px"""
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings?tab=usage', 3.0)
    charts = d.js(LABEL_OVERLAP) or []
    if len(charts) < 3:
        rc.rec('W-UC-1', 'ai-settings', 'charts', 'admin', size, lang, theme, None, '%d charts (no usage in the period yet)' % len(charts))
        return
    problems = []
    for c in charts:
        if c['role'] != ('group' if c['bars'] else 'img') or not c['label']:
            problems.append('a chart has no role / label')
        if c['overlaps']:
            problems.append('%s: %d overlapping labels' % (c['label'], c['overlaps']))
        if c['w'] > c['host'] + 1:
            problems.append('%s: %d px in a %d px box' % (c['label'], c['w'], c['host']))
    if not d.js(NO_SIDE_SCROLL):
        problems.append('sideways scroll')
    if d.js(UC2_OUTSIDE):
        problems.append('a label leaves its card')
    bars = d.js(UC_BARS) or []
    if len(bars) != 2:
        problems.append('%d bar charts with the switch (2 expected)' % len(bars))
    for b in bars:
        if b['labelled'] != b['n']:
            problems.append('%d of %d bars have a label' % (b['labelled'], b['n']))
        if b['switchBtns'] != 2:
            problems.append('no calls / tokens switch')
    # the switch: tokens on the first chart (stacked bars when anything was counted), the title follows, then back
    if bars and d.js("(() => { const b = document.querySelectorAll('.uc-sw-btn')[1]; if (!b) return false; b.click(); return true })()"):
        time.sleep(0.4)
        after = d.js(UC_BARS)
        if not re.search(r'Token|トークン', after[0]['title'], re.I):
            problems.append('the title did not follow the switch: %s' % after[0]['title'])
        d.js("document.querySelectorAll('.uc-sw-btn')[0].click()")
        time.sleep(0.3)
    tips = d.js(UC_TIPS) or {}
    if bars and not tips.get('barTip'):
        problems.append('a bar shows no tip on tap')
    if tips.get('stroke') is not None and tips['stroke'] < 2.9:
        problems.append('the line is %s px' % tips['stroke'])
    if 'lineTip' in tips and not tips['lineTip']:
        problems.append('no tip 10 px beside the point')
    if tips.get('lineFar'):
        problems.append('a tip 90 px from the line')
    rc.rec('W-UC-1', 'ai-settings', 'charts', 'admin', size, lang, theme, not problems,
           '; '.join(problems[:3]) or '%d charts, every bar labelled, switch + tips work, labels never overlap, they fit the card' % len(charts),
           d if problems else None)


UC2_JS = r"""(async () => {
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  const line = document.querySelector('.uc-line'); if (!line) return {none: true};
  const card = line.closest('.uc-card');
  const q = (s) => card.querySelector(s);
  const btns = [...card.querySelectorAll('.uc-sw-btn')];
  const sw = q('.uc-switch');
  // computed colours can be color(srgb ...) or color-mix results: draw one pixel to get plain 0-255 channels
  const cv = document.createElement('canvas'); cv.width = cv.height = 1; const cx = cv.getContext('2d', {willReadFrequently: true});
  const px = (c) => { cx.clearRect(0, 0, 1, 1); cx.fillStyle = '#000'; cx.fillStyle = c; cx.fillRect(0, 0, 1, 1); return [...cx.getImageData(0, 0, 1, 1).data]; };
  const lum = (c) => { const m = px(c); const f = (v) => { v /= 255; return v <= .03928 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4; };
    return .2126 * f(m[0]) + .7152 * f(m[1]) + .0722 * f(m[2]); };
  const cr = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + .05) / (Math.min(x, y) + .05); };
  const res = {pref: localStorage.getItem('xexam.uc.period'), title0: q('.uc-title').textContent, n: btns.length,
    labels: btns.map(b => b.textContent), pressed0: btns.map(b => b.getAttribute('aria-pressed')),
    swRole: sw.getAttribute('role'), swLabel: sw.getAttribute('aria-label'), btnH: btns.map(b => Math.round(b.getBoundingClientRect().height)),
    coarse: matchMedia('(pointer: coarse)').matches, legend0: q('.uc-legend').hidden};
  btns[1].click(); await wait(250);
  const line2 = card.querySelector('.uc-line'); const svg = line2.querySelector('svg');
  res.title1 = q('.uc-title').textContent; res.pressed1 = btns.map(b => b.getAttribute('aria-pressed'));
  res.legend1 = !q('.uc-legend').hidden && q('.uc-legend').getClientRects().length > 0;
  res.legendTxt = q('.uc-legend').textContent;
  res.areas = [svg.querySelectorAll('.uc-area-out').length, svg.querySelectorAll('.uc-area-in').length, svg.querySelectorAll('.uc-stroke-in').length];
  const fillOut = getComputedStyle(svg.querySelector('.uc-area-out')).fill, fillIn = getComputedStyle(svg.querySelector('.uc-area-in')).fill;
  const cardBg = getComputedStyle(card.closest('.panel') || document.body).backgroundColor;
  res.fills = [fillOut, fillIn, cardBg];
  try { res.crOutIn = +cr(fillOut, fillIn).toFixed(2); res.crInBg = +cr(fillIn, cardBg).toFixed(2); } catch (e) { res.crErr = String(e); }
  const pb = btns[1]; res.btnCr = +cr(getComputedStyle(pb).color, getComputedStyle(pb).backgroundColor).toFixed(2);
  const sws = [...card.querySelectorAll('.uc-sw')].map(e => getComputedStyle(e)); res.swatch = sws.map(s => s.backgroundColor + '@' + s.opacity);
  res.legendMatch = sws.length === 2 && sws[0].backgroundColor === fillIn && sws[1].backgroundColor === fillOut;
  res.ticks = [...svg.querySelectorAll('text.uc-label:not([transform])')].map(t => t.textContent);
  res.svgW = Math.round(svg.getBoundingClientRect().width); res.hostW = Math.round(line2.getBoundingClientRect().width);
  res.side = document.documentElement.scrollWidth <= innerWidth + 1;
  // keyboard: focus the plot, End = the newest period, ArrowLeft one back
  svg.focus(); svg.dispatchEvent(new KeyboardEvent('keydown', {key: 'End', bubbles: true})); await wait(80);
  const tip = line2.querySelector('.uc-tip'); res.tipEnd = tip && !tip.hidden ? [...tip.children].map(c => c.textContent) : null;
  svg.dispatchEvent(new KeyboardEvent('keydown', {key: 'ArrowLeft', bubbles: true})); await wait(80);
  res.tipLeft = tip && !tip.hidden ? tip.firstChild.textContent : null;
  res.focusable = svg.getAttribute('tabindex');
  // the hit area of the input band: a point on the input line well below the total line
  const pts = [...svg.querySelectorAll('.uc-dot')];
  svg.blur(); await wait(50);
  return res; })()"""

UC2_HIT = r"""(async () => {
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  const line = document.querySelector('.uc-line'); const svg = line.querySelector('svg');
  const inP = svg.querySelector('.uc-stroke-in'); const tot = svg.querySelector('.uc-stroke:not(.uc-stroke-in)');
  if (!inP) return {noIn: true};
  // walk the input path, find an x where the total line is >= 40 px above it
  const L = inP.getTotalLength(); const m = svg.getScreenCTM(); let found = null;
  for (let s = 0; s <= L; s += 3) { const p = inP.getPointAtLength(s);
    const L2 = tot.getTotalLength(); let best = null;
    for (let u = 0; u <= L2; u += 3) { const q = tot.getPointAtLength(u); if (!best || Math.abs(q.x - p.x) < Math.abs(best.x - p.x)) best = q; }
    if (best && p.y - best.y >= 40) { found = {p, gap: p.y - best.y}; break; } }
  if (!found) return {noGap: true};
  const sx = found.p.x * m.a + m.e, sy = found.p.y * m.d + m.f;
  svg.dispatchEvent(new PointerEvent('pointermove', {bubbles: true, pointerType: 'mouse', clientX: sx, clientY: sy})); await wait(80);
  const tip = line.querySelector('.uc-tip');
  return {gap: Math.round(found.gap), tipOnInputLine: !!(tip && !tip.hidden)}; })()"""



UC2_TITLES = {
    'zh': ('按时间（调用次数：次）', '按时间（Token：输入 / 输出）', '调用次数', 'Token 数'),
    'en': ('Over time (calls: times)', 'Over time (tokens: input / output)', 'Calls', 'Tokens'),
    'ja': ('期間別（呼び出し回数）', '期間別（Token：入力 / 出力）', '呼び出し回数', 'Token 数'),
}
UC2_OUTSIDE = """(() => { const bad = []; for (const card of document.querySelectorAll('.uc-card')) { const c = card.getBoundingClientRect();
  for (const l of card.querySelectorAll('.uc-label')) { const r = l.getBoundingClientRect(); if (r.width && (r.left < c.left - 1 || r.right > c.right + 1 || r.bottom > c.bottom + 1 || r.top < c.top - 1)) bad.push(l.textContent.slice(0, 12)); } }
  return bad })()"""
UC2_MOVE = """(async () => { const wait = (ms) => new Promise(r => setTimeout(r, ms)); const cards = [...document.querySelectorAll('.uc-card')]; const res = [];
  for (const card of cards) { card.scrollIntoView({block: 'start'}); await wait(150); const btns = [...card.querySelectorAll('.uc-sw-btn')]; const was = btns.map(b => b.getBoundingClientRect().top);
    btns[1].click(); await wait(200); const now = [...card.querySelectorAll('.uc-sw-btn')].map(b => b.getBoundingClientRect().top);
    res.push(Math.abs(now[1] - was[1]) < 1); card.querySelectorAll('.uc-sw-btn')[0].click(); await wait(150); }
  return res })()"""


def uc2_case(rc, lang, theme, size):
    """W-UC-2 (A-OWNER-63 / 59): the by-time chart has the calls | tokens switch (titles, aria-pressed, 44 px on touch, pressed text
    4.5:1), token mode shows the input / output areas with a legend that matches them, K/M ticks, the exact tip against the API,
    End / ArrowLeft, the choice is remembered; the switches do not move; no label leaves its card"""
    d = rc.dev('admin', lang, theme, size)
    d.js("localStorage.removeItem('xexam.uc.period')")
    goto(d, '#/ai-settings?tab=usage', 3.0)
    r = d.js(UC2_JS)
    if not r or r.get('none'):
        rc.rec('W-UC-2', 'ai-settings', 'by-time', 'admin', size, lang, theme, None, 'no by-time chart (no usage in the period yet)')
        return
    T = UC2_TITLES[lang]
    p = []
    if r['title0'] != T[0] or r['title1'] != T[1]:
        p.append('titles %r / %r' % (r['title0'], r['title1']))
    if r['labels'] != [T[2], T[3]]:
        p.append('switch labels %r' % r['labels'])
    if r['pressed0'] != ['true', 'false'] or r['pressed1'] != ['false', 'true']:
        p.append('aria-pressed %r -> %r' % (r['pressed0'], r['pressed1']))
    if r['swRole'] not in ('group', 'radiogroup') or not r['swLabel']:
        p.append('switch role / label')
    if not r['legend0'] or not r['legend1']:
        p.append('legend wrong (hidden in calls: %s, shown in tokens: %s)' % (r['legend0'], r['legend1']))
    if r['areas'][:2] != [1, 1]:
        p.append('areas %r' % r['areas'])
    if not r.get('legendMatch'):
        p.append('legend swatches differ from the areas %r' % r['swatch'])
    if r.get('crInBg', 9) < 1.4:
        p.append('input area %s:1 against the card' % r.get('crInBg'))
    if min(r['btnH']) < (44 if r['coarse'] else 32):
        p.append('switch %r px (coarse %s)' % (r['btnH'], r['coarse']))
    if r.get('btnCr', 9) < 4.5:
        p.append('pressed button contrast %s' % r['btnCr'])
    bad = [x for x in r['ticks'] if not re.fullmatch(r'\d+(\.\d)?[KM]?', x) or (x.isdigit() and int(x) >= 1000)]
    if bad:
        p.append('y ticks not K/M: %r' % bad[:4])
    if r['svgW'] > r['hostW'] + 1 or not r['side']:
        p.append('width %d in %d / side scroll' % (r['svgW'], r['hostW']))
    usage = d.js("fetch('/api/ai/admin/usage?days=7').then(x => x.json())") or {}
    per = usage.get('byPeriod') or []
    tip = r['tipEnd'] or []
    if len(per) >= 2:
        last = per[-1]
        joined = ' | '.join(tip)
        for v in (last['tokensIn'], last['tokensOut'], last['tokensIn'] + last['tokensOut']):
            if '{:,}'.format(v) not in joined:
                p.append('tip lacks exact %s: %s' % ('{:,}'.format(v), joined))
                break
        if not tip or tip[0] != last['period']:
            p.append('tip head %r (expected %s)' % (tip[:1], last['period']))
        if r['tipLeft'] != per[-2]['period']:
            p.append('ArrowLeft tip %r' % r['tipLeft'])
    ov = [c for c in (d.js(LABEL_OVERLAP) or []) if c['overlaps']]
    if ov:
        p.append('overlapping labels')
    out = d.js(UC2_OUTSIDE) or []
    if out:
        p.append('labels outside their card: %r' % out[:2])
    mv = d.js(UC2_MOVE) or []
    if not all(mv):
        p.append('a switch moved when tapped %r' % mv)
    # the choice is remembered per browser (re-query: the click replaced the chart)
    d.js("document.querySelector('.uc-line').closest('.uc-card').querySelectorAll('.uc-sw-btn')[1].click()")
    goto(d, '#/ai-settings?tab=usage', 3.0)
    rem = d.js("(() => { const c = document.querySelector('.uc-line').closest('.uc-card'); return [localStorage.getItem('xexam.uc.period'), c.querySelector('.uc-title').textContent, [...c.querySelectorAll('.uc-sw-btn')].map(b => b.getAttribute('aria-pressed'))] })()")
    if not rem or rem[0] != 'tokens' or rem[1] != T[1] or rem[2] != ['false', 'true']:
        p.append('not remembered after a reload: %r' % (rem,))
    hit = d.js(UC2_HIT) or {}
    if hit.get('gap') and not hit.get('tipOnInputLine'):
        p.append('no tip on the input line')
    d.js("document.querySelector('.uc-line').closest('.uc-card').querySelectorAll('.uc-sw-btn')[0].click(); localStorage.setItem('xexam.uc.period', 'calls')")
    rc.rec('W-UC-2', 'ai-settings', 'by-time', 'admin', size, lang, theme, not p,
           '; '.join(p[:3]) or 'switch, areas, legend, K/M ticks, exact tip, keys and the remembered choice are right', d if p else None)


def ab1_case(rc, lang, theme, size):
    """A-OWNER-27 / 28 / 28b: the AI+题集 badge (with its tip and the caution line) on the live question card and on the review
    cards of the result page; one AI bank is generated for it and deleted again"""
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/', 2.0)
    started = d.js("""(async () => { const post = async (u, b) => { const r = await fetch(u, {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(b)}); return [r.status, await r.json().catch(() => null)]; };
      const banks = (await (await fetch('/api/banks')).json()).banks; const g = banks.find(b => !b.ai && b.sheets.length);
      if (!g) return ['no source bank'];
      const before = (await (await fetch('/api/ai/admin/limits')).json()).limits.genMinSource;
      await post('/api/ai/admin/limits', {genMinSource: 5});
      let s; try { s = await post('/api/ai/gen/start', {source: 'scope', sheets: g.sheets.map(x => ({bank: g.id, sheet: x.name})), name: 'Walk AI set'}); }
      finally { await post('/api/ai/admin/limits', {genMinSource: before}); }
      if (s[0] !== 202) return ['start ' + s[0]];
      const id = s[1].job.id;
      for (let i = 0; i < 240; i++) { await window.__wait(500); const j = (await (await fetch('/api/ai/gen/job?id=' + id)).json()).job;
        if (!['queued', 'running'].includes(j.state)) return [j.state, j.bankId, id]; }
      return ['timeout', null, id] })()""")
    state, bank, job = (started + [None, None])[:3]
    if state != 'done' or not bank:
        rc.rec('W-AB-1', 'quiz', 'ai-badge', 'admin', size, lang, theme, None, 'no AI bank to look at (%s)' % state)
        return
    try:
        goto(d, '#/', 2.0)
        if not d.js("(() => { const b = [...document.querySelectorAll('.gen-card .btn.primary')][0]; if (!b) return false; b.click(); return true })()"):
            rc.rec('W-AB-1', 'quiz', 'ai-badge', 'admin', size, lang, theme, False, 'no practice button on the finished AI card', d)
            return
        time.sleep(1.0)
        d.js(CHECKS_JS)
        d.js("window.__start('practice')")
        until(d, "location.hash.indexOf('quiz') >= 0 && document.querySelector('.stem')", 10)
        time.sleep(0.6)
        d.js(CHECKS_JS)
        card = d.js("""(() => { const b = document.querySelector('.stem') && document.querySelector('.ai-badge.ai-set'); if (!b) return null;
          const host = b.closest('.has-tip'); const tip = host ? host.querySelector(':scope > .tip') : null; return {tip: tip ? tip.textContent.trim() : ''} })()""")
        problems = []
        if card is None:
            problems.append('no AI+题集 badge on the question card')
        elif not re.search(r'AI', card['tip']) or not re.search(r'有误|wrong|誤り', card['tip']):
            problems.append('the badge tip lacks the author line or the caution: %s' % card['tip'][:60])
        d.js('(%s)(5)' % FINISH_JS)
        time.sleep(1.0)
        d.js(CHECKS_JS)
        if d.js("!document.querySelector('.ai-badge.ai-set')") and d.js("!!document.querySelector('.rv-card, .review-card, .result-item')"):
            problems.append('no AI+题集 badge on the review cards of the result')
        rc.rec('W-AB-1', 'quiz', 'ai-badge', 'admin', size, lang, theme, not problems,
               '; '.join(problems) or 'AI+题集 badge with its tip on the question card and on the result', d if problems else None)
    finally:
        d.js("""(async () => { const post = (u, b) => fetch(u, {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(b)});
          await post('/api/ai/gen/dismiss', {id: %d}); await post('/api/banks/delete', {id: %d, withRecords: true}); return 1 })()""" % (job or 0, bank))


TOP_SUBMIT = """(() => { const vis = e => !!e && e.getClientRects().length > 0 && getComputedStyle(e).display !== 'none';
  const r = e => { const b = e.getBoundingClientRect(); return {l: b.left, r: b.right, t: b.top, b: b.bottom, h: b.height, w: b.width} };
  const rail = document.querySelector('.rail .btn.primary'); const sub = document.querySelector('.qbar .qsubmit');
  const cnt = document.querySelector('.qbar .qcount'); const clock = document.querySelector('.qbar .qtime'); const exit = document.querySelector('.qbar .qexit');
  const over = (a, b) => a && b && a.l < b.r - 0.5 && b.l < a.r - 0.5 && a.t < b.b - 0.5 && b.t < a.b - 0.5;
  const rs = vis(sub) ? r(sub) : null, rc = vis(clock) ? r(clock) : null, re = vis(exit) ? r(exit) : null, rn = vis(cnt) ? r(cnt) : null;
  return {rail: vis(rail), sub: !!rs, count: rn ? ((vis(cnt.querySelector('.qcount-s')) ? cnt.querySelector('.qcount-s') : cnt.querySelector('.qcount-l')).textContent.trim()) : null, subH: rs ? Math.round(rs.h) : 0, subW: rs ? Math.round(rs.w) : 0,
    overlap: !!(over(rs, rc) || over(rs, re) || over(rn, rs) || over(rn, rc)), inside: !rs || (rs.r <= innerWidth + 0.5 && rs.l >= -0.5),
    leftOfClock: !rs || !rc || rs.r <= rc.l + 0.5,
    colors: sub ? (() => { const c = getComputedStyle(sub); const q = document.createElement('i'); q.style.color = 'var(--go-exam)'; document.body.appendChild(q); const ex = getComputedStyle(q).color; q.style.color = 'var(--go-practice)'; const pr = getComputedStyle(q).color; q.remove();
      return {bg: c.backgroundColor, fg: c.color, ex, pr, tip: sub.getAttribute('aria-label'), lbl: (sub.querySelector('.lbl') || {}).textContent || ''} })() : null, label: !!sub && vis(sub.querySelector('.lbl')), side: document.documentElement.scrollWidth <= innerWidth + 1} })()"""


def as1_case(rc, lang, theme, size):
    """A-OWNER-22 / 24b / 25: up to the rail line (880 px) the answer sheet has no finish button (folded or open) and the top
    row has the count + a 交卷 / 完成 pill left of the clock (no overlap); wider, the rail keeps its button
    and the top row has neither"""
    for kind in ('practice', 'exam'):
        d = rc.dev('sam', lang, theme, size)
        start_quiz(d, kind)
        toggle = '[data-focus="rail-toggle"], .rail-toggle'
        states = [d.js(TOP_SUBMIT)]
        if size[0] <= 600 and d.js("!!document.querySelector('%s') && getComputedStyle(document.querySelector('.rail-toggle')).display !== 'none'" % toggle):
            d.tap_sel(toggle)
            time.sleep(0.4)
            states.append(d.js(TOP_SUBMIT))
            d.tap_sel(toggle)
            time.sleep(0.3)
        narrow = size[0] <= 880
        s0 = states[0]
        if narrow and s0['colors']:
            # A-OWNER-44 / 44b: exam green, practice orange (the Home button colours), white text, the full name as the tip
            cl = s0['colors']
            want = {'practice': ('完成练习', 'Finish practice', '練習を終了'), 'exam': ('完成考试', 'Finish exam', '試験を終了')}[kind]
            s0['sub'] = bool(s0['sub']) and cl['bg'] == (cl['ex'] if kind == 'exam' else cl['pr']) and cl['fg'] == 'rgb(255, 255, 255)'                 and cl['tip'] in want and (not s0['label'] or cl['lbl'] in want)
        if narrow:
            ok = (not any(x['rail'] for x in states) and s0['sub'] and s0['count'] and not s0['overlap'] and s0['inside']
                  and s0['leftOfClock'] and s0['side'])  # the pill is 30 px high with a 44 px tap area (A-OWNER-60; W-L60 measures that)
        else:
            ok = s0['rail'] and not s0['sub'] and not s0['count']
        rc.rec('W-AS-1v', 'quiz', 'top-submit-' + kind, 'sam', size, lang, theme, ok,
               'sheet button %s; top row: count %r, button %s (%dx%d px, label %s), overlap %s' % (
                   'shown' if any(x['rail'] for x in states) else 'hidden', s0['count'], 'shown' if s0['sub'] else 'hidden',
                   s0['subW'], s0['subH'], 'shown' if s0['label'] else 'hidden', s0['overlap']), d if not ok else None)


def ft1_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/about', 1.6)
    r = d.js("""(() => { const f = document.querySelector('.main .foot, .foot'); if (!f) return null;
      const note = f.querySelector('.foot-note') || f.firstElementChild; const fr = f.getBoundingClientRect();
      const gh = f.querySelector('.gh-line');
      return {gap: note ? Math.round(note.getBoundingClientRect().top - fr.top) : 0, gap2: note && gh ? Math.round(gh.getBoundingClientRect().top - note.getBoundingClientRect().bottom) : -1,
        line: note ? Math.round(parseFloat(getComputedStyle(note).lineHeight) || 18) : 18} })()""")
    if r is None:
        rc.rec('W-FT-1', 'about', 'footer', 'sam', size, lang, theme, None, 'no footer on this page')
        return
    # A-OWNER-20: one blank text line between the divider and the text, and one between the text and the GitHub line
    lo, hi = r['line'] - 5, r['line'] + 6
    ok = lo <= r['gap'] <= hi and lo <= r['gap2'] <= hi
    rc.rec('W-FT-1', 'about', 'footer', 'sam', size, lang, theme, ok, 'divider to text %d px, text to GitHub line %d px (one %d px line each)' % (r['gap'], r['gap2'], r['line']), d if not ok else None)


def si1_case(rc, lang, theme, size):
    d = rc.dev(None, lang, theme, size)
    goto(d, '#/', 1.6)
    r = d.js("""(() => { const c = document.querySelector('.auth-card'), l = document.querySelector('.auth-lang'), b = document.querySelector('.auth-bottom');
      if (!c || !l || !b) return null; const cr = c.getBoundingClientRect(), lr = l.getBoundingClientRect(), br = b.getBoundingClientRect();
      return {gap: Math.round(lr.top - cr.bottom), bottomGap: Math.round(innerHeight - br.bottom), vh: innerHeight, sh: document.documentElement.scrollHeight} })()""")
    if r is None:
        rc.rec('W-SI-1', 'signin', 'lang-switch', None, size, lang, theme, None, 'not the sign-in page')
        return
    problems = []
    if not 10 <= r['gap'] <= 24:
        problems.append('the language switch is %d px under the card (want about 16)' % r['gap'])
    if r['sh'] <= r['vh'] + 1 and r['bottomGap'] > 40:
        problems.append('the GitHub line is %d px above the screen bottom' % r['bottomGap'])
    rc.rec('W-SI-1', 'signin', 'lang-switch', None, size, lang, theme, not problems,
           '; '.join(problems) or 'switch %d px under the card, GitHub line at the bottom' % r['gap'], d if problems else None)


# ---------------------------------------------------------------------------------------------------- v1.1 L-54: search, sort, paging
PT_STATE = """((id, rowSel) => { const q = s => document.querySelector(s); const root = q('#' + id + '-q') && q('#' + id + '-q').closest('.ptb');
  if (!root) return null; const vis = e => !!e && getComputedStyle(e).visibility !== 'hidden' && e.getClientRects().length > 0;
  const rows = [...root.querySelectorAll(rowSel)].map(e => e.textContent.trim());
  const th = [...root.querySelectorAll('th[aria-sort]')]; const on = th.find(e => e.getAttribute('aria-sort') !== 'none');
  const r = e => { if (!e) return null; const b = e.getBoundingClientRect(); return {t: Math.round(b.top + scrollY), l: Math.round(b.left), w: b.width, h: b.height} };
  const of = root.querySelector('.ptb-of'); const cur = root.querySelector('.ptb-pg.on'); const act = document.activeElement;
  return {rows, of: of ? of.textContent : '', cur: cur ? cur.textContent : '', q: q('#' + id + '-q').value,
    sortCol: on ? on.textContent.replace(/[▲▼↕]/g, '').trim() : '', sortDir: on ? on.getAttribute('aria-sort') : '', nSortable: th.length,
    selVisible: vis(root.querySelector('.ptb-sel')), selVal: root.querySelector('.ptb-sel').value, dirLabel: root.querySelector('.ptb-dir').getAttribute('aria-label') || '',
    headBtns: [...root.querySelectorAll('thead .ptb-sort')].map(e => vis(e)), tableHidden: root.querySelector('.ais-tablewrap').hidden,
    none: !root.querySelector('.ptb-none').hidden ? root.querySelector('.ptb-none').textContent : '',
    prevOff: q('#' + id + '-prev').getAttribute('aria-disabled') === 'true', nextOff: q('#' + id + '-next').getAttribute('aria-disabled') === 'true',
    pages: [...root.querySelectorAll('.ptb-pg:not(.ptb-step)')].map(e => e.textContent), focus: act ? act.id : '',
    tools: r(root.querySelector('.ptb-tools')), search: r(q('#' + id + '-q')), pager: r(root.querySelector('.ptb-pager')),
    sizes: [...root.querySelectorAll('.ptb-q, .ptb-pg, .ptb-dir, .ptb-sel')].filter(vis).map(e => [e.className.split(' ')[0], Math.round(e.getBoundingClientRect().width), Math.round(e.getBoundingClientRect().height)]),
    label: !!root.querySelector('nav[aria-label]'), live: !!root.querySelector('[role=status][aria-live=polite]'),
    side: document.documentElement.scrollWidth <= innerWidth + 1, rowN: rows.length} })"""


def pt(d, id_, row_sel):
    return d.js('(%s)(%s, %s)' % (PT_STATE, json.dumps(id_), json.dumps(row_sel)))


def pt_set_select(d, id_, value):
    d.js("""(() => { const s = document.getElementById(%s + '-sort'); s.value = %s; s.dispatchEvent(new Event('change', {bubbles: true})); return 1 })()""" % (json.dumps(id_), json.dumps(value)))


def g_l54(rc):
    if not any(rc.want(i) for i in ('W-PT-1', 'W-PT-2', 'W-JC-1', 'W-AB-2', 'W-GB-2')):
        return
    if rc.want('W-PT-1') or rc.want('W-PT-2'):
        rc.paging_users()
        for gid, tab, id_, row in (('W-PT-1', 'usage', 'usage-users', 'tbody > tr > th'), ('W-PT-2', 'users', 'ai-users', 'tbody > tr:not(.ais-featrow) > th.ais-user')):
            if not rc.want(gid):
                continue
            for lang in rc.langs(['zh']):
                for size in rc.sizes([(360, 740), (390, 844), (844, 390), (768, 1024), (1280, 800)], lambda s: True):
                    rc.guard(gid, 'ai-settings', pt_case, rc, gid, tab, id_, row, lang, rc.themes(['light'])[0], size)
            if not rc.args.themes and not rc.args.full:
                for size in ((390, 844), (1280, 800)):
                    rc.guard(gid, 'ai-settings', pt_case, rc, gid, tab, id_, row, 'zh', 'dark', size)
    if rc.want('W-JC-1'):
        for size in rc.sizes([(360, 740), (390, 844), (844, 390), (1280, 800)], lambda s: True):
            rc.guard('W-JC-1', 'ai-settings', jc1_case, rc, 'zh', rc.themes(['light'])[0], size)
    if rc.want('W-AB-2') or rc.want('W-GB-2'):
        for size in rc.sizes([(390, 844), (1280, 800)], lambda s: True):
            rc.guard('W-AB-2', 'banks', ab2_case, rc, 'zh', rc.themes(['light'])[0], size)


def pt_case(rc, gid, tab, id_, row, lang, theme, size):
    """A-OWNER-54 / 54b: search, sortable columns and 10 per page on the "by user" table and on the AI+ users list"""
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings?tab=' + tab, 2.5)
    cards = tab != 'usage' and size[0] <= 900   # A-OWNER-59: the usage table is a mini table on phones, never cards
    first_head = '#%s-q' % id_
    problems = []
    s0 = pt(d, id_, row)
    if not s0:
        rc.rec(gid, 'ai-settings', 'paging', 'admin', size, lang, theme, None, 'no table in the %s tab yet' % tab)
        return
    if s0['rowN'] != 10:
        problems.append('page 1 shows %d rows, not 10' % s0['rowN'])
    m = re.search(r'(\d+)\D+(\d+)', s0['of'])
    if not m or m.group(1) != '1' or int(m.group(2)) < 2:
        problems.append('pager text "%s" (no page 1 of 2+)' % s0['of'])
    if lang == 'zh' and not re.fullmatch(r'第 1 / \d+ 页', s0['of']):
        problems.append('pager text is not "第 1 / y 页": %s' % s0['of'])
    if not s0['prevOff'] or s0['nextOff']:
        problems.append('arrows on page 1: prev off=%s next off=%s' % (s0['prevOff'], s0['nextOff']))
    if not s0['sortCol'] or s0['sortDir'] == 'none':
        problems.append('no active sort column is marked')
    if not (s0['label'] and s0['live']):
        problems.append('pager nav label / live status missing')
    if not s0['side']:
        problems.append('sideways scroll')
    # keyboard / screen reader: header buttons are real buttons (focusable) on wide screens, hidden in the card layout
    if cards:
        if any(s0['headBtns']):
            problems.append('hidden card header still has a focusable sort button')
        if not s0['selVisible']:
            problems.append('no sort select on the card layout')
    else:
        if not s0['headBtns'] or not all(s0['headBtns']):
            problems.append('sort buttons missing or hidden in the header')
        if s0['selVisible']:
            problems.append('the sort select shows on the wide layout')
    if d.js("matchMedia('(pointer: coarse)').matches"):
        small = [x for x in s0['sizes'] if x[2] < 43.5 or (x[0] == 'ptb-pg' and x[1] < 43.5)]
        if small:
            problems.append('targets under 44 px: %s' % small[:3])
    names0 = s0['rows']
    # next page: the focus stays in the pager, the toolbar does not move
    d.tap_sel('#%s-next' % id_)
    s1 = pt(d, id_, row)
    if s1['cur'] != '2' or s1['rows'] == names0 or not s1['rows']:
        problems.append('next did not show page 2 (%s)' % s1['of'])
    if not s1['focus'].startswith(id_ + '-'):
        problems.append('the focus left the pager after paging (%s)' % s1['focus'])
    if abs(s1['tools']['t'] - s0['tools']['t']) > 1 or abs(s1['search']['l'] - s0['search']['l']) > 1:
        problems.append('the toolbar moved when paging')
    if s1['sortCol'] != s0['sortCol'] or s1['sortDir'] != s0['sortDir']:
        problems.append('paging changed the sort')
    # sort by name: a header click on wide screens, the select on cards; back to page 1
    head_btn = '.ptb th.ptb-th:first-child .ptb-sort'
    if cards:
        pt_set_select(d, id_, 'user')
    else:
        d.tap_sel('#%s' % id_ if False else head_btn)
    s2 = pt(d, id_, row)
    if s2['cur'] != '1':
        problems.append('sorting did not return to page 1 (%s)' % s2['of'])
    if s2['sortDir'] not in ('ascending', 'descending'):
        problems.append('sort direction not marked')
    # the same column again (or the order button) reverses
    if cards:
        d.tap_sel('#%s-dir' % id_)
    else:
        d.tap_sel(head_btn)
    s3 = pt(d, id_, row)
    if s3['sortDir'] == s2['sortDir'] or s3['cur'] != '1':
        problems.append('a second click did not reverse the order (%s -> %s)' % (s2['sortDir'], s3['sortDir']))
    n2 = [r.lower() for r in s2['rows']]
    n3 = [r.lower() for r in s3['rows']]
    up, down = (n2, n3) if s2['sortDir'] == 'ascending' else (n3, n2)
    if up != sorted(up) or down != sorted(down, reverse=True):
        problems.append('names are not in order / not reversed')
    # search: page 1, the focus stays in the box, only matching rows, the sort is kept; paging keeps both
    d.tap_sel(first_head)
    d.type('walk-pg')
    time.sleep(0.4)
    s4 = pt(d, id_, row)
    if s4['cur'] != '1' or not s4['rows'] or any('walk-pg' not in r for r in s4['rows']):
        problems.append('search did not filter (%s)' % s4['rows'][:2])
    if s4['focus'] != id_ + '-q':
        problems.append('the search box lost the focus while typing (%s)' % s4['focus'])
    if abs(s4['search']['t'] - s0['search']['t']) > 1:
        problems.append('the search box moved')
    if s4['sortCol'] != s3['sortCol'] or s4['sortDir'] != s3['sortDir']:
        problems.append('searching changed the sort')
    d.js("document.getElementById('%s-q').blur()" % id_)
    if '2' in s4['pages']:
        d.tap_sel('#%s-p2' % id_)
        s5 = pt(d, id_, row)
        if s5['q'] != 'walk-pg' or any('walk-pg' not in r for r in s5['rows']) or s5['cur'] != '2':
            problems.append('paging dropped the search')
        if s5['sortDir'] != s4['sortDir']:
            problems.append('paging dropped the sort')
    else:
        problems.append('12 matching users but no page 2 (%s)' % s4['pages'])
    # nothing found
    d.tap_sel(first_head)
    d.js("document.getElementById('%s-q').select()" % id_)
    d.type('zzzz-none')
    time.sleep(0.4)
    s6 = pt(d, id_, row)
    want = {'zh': '没有找到匹配的用户', 'en': 'No matching users found', 'ja': '一致するユーザーが見つかりません'}[lang]
    if s6['none'] != want or not s6['tableHidden'] or s6['rowN'] != 0:
        problems.append('empty search: text "%s", table hidden=%s' % (s6['none'], s6['tableHidden']))
    if s6['cur'] != '1' or not (s6['prevOff'] and s6['nextOff']):
        problems.append('empty search pager is not 1 / 1')
    rc.rec(gid, 'ai-settings', 'paging-' + tab, 'admin', size, lang, theme, not problems,
           '; '.join(problems[:4]) or 'search, sort, 10 per page, empty text, focus, toolbar fixed (%s)' % ('cards' if cards else 'table'),
           d if problems or theme == 'dark' else None)


def jc1_case(rc, lang, theme, size):
    """A-OWNER-51: the AI bank jobs are cards of short lines up to 900 px (a chip, no sideways scroll), the table when wider"""
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings?tab=gen', 2.5)
    r = d.js("""(() => { const t = document.querySelector('.ais-genjobs'); if (!t) return null; const rows = [...t.querySelectorAll('tbody tr')];
      if (!rows.length) return {n: 0}; const row = rows[0]; const chip = row.querySelector('.ais-jchip');
      const lines = new Set([...row.children].filter(e => getComputedStyle(e).display !== 'none' && getComputedStyle(e).display !== 'contents').map(e => Math.round(e.getBoundingClientRect().top / 4))).size;
      return {n: rows.length, grid: getComputedStyle(row).display === 'grid', chip: !!chip && chip.getBoundingClientRect().width > 0,
        theadHidden: getComputedStyle(t.querySelector('thead')).position === 'absolute', lines, text: row.innerText.slice(0, 120),
        side: document.documentElement.scrollWidth <= innerWidth + 1, over: [...row.querySelectorAll('*')].some(e => e.getBoundingClientRect().right > innerWidth + 1)} })()""")
    state = 'cards' if size[0] <= 900 else 'wide'
    if not r or not r['n']:
        rc.rec('W-JC-1', 'ai-settings', state, 'admin', size, lang, theme, None, 'no AI bank job to look at')
        return
    problems = []
    if not r['side'] or r['over']:
        problems.append('sideways scroll / content past the screen')
    if size[0] <= 900:
        if not r['grid'] or not r['theadHidden']:
            problems.append('not a card (grid=%s, head hidden=%s)' % (r['grid'], r['theadHidden']))
        if not r['chip']:
            problems.append('no status chip')
        if r['lines'] > 5:
            problems.append('%d lines in the card' % r['lines'])
    # A-OWNER-55 / 59: "YYYY-MM-DD HH:MM", one grid (times, names and chips each in one column), the tip and names on the figures,
    # no box glyph, K / M (never 万) in the figures
    f = d.js("""(() => { const rows = [...document.querySelectorAll('.ais-genjobs tbody tr')];
      const col = (sel, key) => [...new Set(rows.map(r => { const e = r.querySelector(sel); return e && e.getBoundingClientRect().width ? Math.round(e.getBoundingClientRect()[key]) : null }).filter(x => x != null))];
      const st = rows[0].querySelector('.ais-stats');
      return {times: rows.map(r => (r.querySelector('.ais-when') || {}).textContent), lefts: col('.ais-when', 'left'), names: col('.ais-uname', 'left'),
        chips: col('.ais-jchip', 'right'), tabbable: !!st && st.tabIndex === 0, sr: st ? [...st.querySelectorAll('.sr-only')].map(e => e.textContent.trim()) : [],
        text: document.querySelector('.ais-genjobs').innerText} })()""")
    if any(not re.fullmatch(r'\d{4}-\d\d-\d\d \d\d:\d\d', (x or '').strip()) for x in f['times']):
        problems.append('time is not YYYY-MM-DD HH:MM: %s' % f['times'][:2])
    if size[0] <= 900 and (len(f['lefts']) > 1 or len(f['names']) > 1 or len(f['chips']) > 1):
        problems.append('the cards do not share one grid (time left %s, name left %s, chip right %s)' % (f['lefts'], f['names'], f['chips']))
    if not f['tabbable'] or len([x for x in f['sr'] if x]) < 3:
        problems.append('the figures have no tip target / screen-reader names (%s)' % f['sr'])
    if '万' in f['text'] or '□' in f['text'] or '�' in f['text']:
        problems.append('the job list holds 万 or a box glyph')
    rc.rec('W-JC-1', 'ai-settings', state, 'admin', size, lang, theme, not problems,
           '; '.join(problems) or 'job card: %d lines, chip, no sideways scroll (%s)' % (r['lines'], r['text'].replace('\n', ' | ')[:60]), d if problems else None)


def ab2_case(rc, lang, theme, size):
    """L-53 badge word (the practice set for a bank generated from a scope) and L-52 (an AI bank among the ticked sheets
    greys the generate button); one AI bank is generated and deleted again"""
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/', 2.0)
    started = d.js("""(async () => { const post = async (u, b) => { const r = await fetch(u, {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(b)}); return [r.status, await r.json().catch(() => null)]; };
      const banks = (await (await fetch('/api/banks')).json()).banks; const g = banks.find(b => !b.ai && b.sheets.length);
      if (!g) return ['no source bank'];
      const before = (await (await fetch('/api/ai/admin/limits')).json()).limits.genMinSource;
      await post('/api/ai/admin/limits', {genMinSource: 5});
      let s; try { s = await post('/api/ai/gen/start', {source: 'scope', sheets: g.sheets.map(x => ({bank: g.id, sheet: x.name})), name: 'Walk AI badge set'}); }
      finally { await post('/api/ai/admin/limits', {genMinSource: before}); }
      if (s[0] !== 202) return ['start ' + s[0]];
      const id = s[1].job.id;
      for (let i = 0; i < 240; i++) { await window.__wait(500); const j = (await (await fetch('/api/ai/gen/job?id=' + id)).json()).job;
        if (!['queued', 'running'].includes(j.state)) return [j.state, j.bankId, id]; }
      return ['timeout', null, id] })()""")
    state, bank, job = (started + [None, None])[:3]
    if state != 'done' or not bank:
        rc.rec('W-AB-2', 'banks', 'badge-word', 'admin', size, lang, theme, None, 'no AI bank to look at (%s)' % state)
        return
    try:
        problems = []
        goto(d, '#/banks', 2.0)
        word = {'zh': '练题集', 'en': 'practice set', 'ja': '練習問題集'}[lang]
        grab = "(() => { const b = document.querySelector('.ai-badge.ai-set'); return b ? b.textContent.trim() : null })()"
        badge = d.js(grab)
        if badge is None:
            problems.append('no AI+ badge on the AI bank in Bank management')
        elif word not in badge:
            problems.append('badge word is "%s", expected "%s"' % (badge, word))
        goto(d, '#/', 2.0)
        home = d.js(grab)
        if home is not None and word not in home:
            problems.append('Home badge word is "%s"' % home)
        rc.rec('W-AB-2', 'banks', 'badge-word', 'admin', size, lang, theme, not problems,
               '; '.join(problems) or 'AI+ badge says "%s" in Bank management (Home: %s)' % (badge, home), d if problems else None)
        problems = []
        g = d.js("""(() => { const b = document.querySelector('.home-setup-head .gen-btn'); if (!b) return null;
          const dd = b.getAttribute('aria-describedby') && document.getElementById(b.getAttribute('aria-describedby'));
          return {aria: b.getAttribute('aria-disabled'), dis: b.disabled, tip: (dd ? dd.textContent : '') || b.getAttribute('title') || b.getAttribute('aria-label') || '',
            all: document.querySelector('.home-setup-head').innerText} })()""")
        if g is None:
            rc.rec('W-GB-2', 'home', 'greyed', 'admin', size, lang, theme, None, 'no AI generate button on Home')
        else:
            if g['aria'] != 'true' and not g['dis']:
                problems.append('the generate button is not greyed with an AI bank in the scope')
            rc.rec('W-GB-2', 'home', 'greyed', 'admin', size, lang, theme, not problems,
                   '; '.join(problems) or 'button greyed (aria-disabled=%s); reason: %s' % (g['aria'], (g['tip'] or g['all'])[:60].replace('\n', ' ')), d if problems else None)
    finally:
        d.js("""(async () => { const post = (u, b) => fetch(u, {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(b)});
          await post('/api/ai/gen/dismiss', {id: %d}); await post('/api/banks/delete', {id: %d, withRecords: true}); return 1 })()""" % (job or 0, bank))


# ---- sprint 10 (L-55, L-57, L-59): AI bank job cards need jobs, the import name box, the usage mini table and K / M
def ensure_jobs(rc, d, n=2):
    """At least `n` finished AI bank jobs for the job cards (W-JC-1): generated from the first source bank, kept (made-up data)"""
    return d.js("""(async (n) => { const post = async (u, b) => { const r = await fetch(u, {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(b)}); return [r.status, await r.json().catch(() => null)]; };
      const have = async () => ((await (await fetch('/api/ai/admin/gen/jobs')).json().catch(() => ({}))).jobs || []).length;
      const banks = (await (await fetch('/api/banks')).json()).banks; const g = banks.find(b => !b.ai && b.sheets.length);
      if (!g) return 'no source bank';
      const before = (await (await fetch('/api/ai/admin/limits')).json()).limits;
      await post('/api/ai/admin/limits', {genMinSource: 5, genPerDay: 20});
      try {
        for (let k = await have(); k < n; k++) {
          const s = await post('/api/ai/gen/start', {source: 'scope', sheets: g.sheets.map(x => ({bank: g.id, sheet: x.name})), name: 'Walk job ' + k});
          if (s[0] !== 202) return 'start ' + s[0];
          for (let i = 0; i < 240; i++) { await window.__wait(500); const j = (await (await fetch('/api/ai/gen/job?id=' + s[1].job.id)).json()).job;
            if (!['queued', 'running'].includes(j.state)) break; }
          await post('/api/ai/gen/dismiss', {id: s[1].job.id});
        }
      } finally { await post('/api/ai/admin/limits', {genMinSource: before.genMinSource, genPerDay: before.genPerDay}); }
      return 'ok' })(%d)""" % n)


# Wraps fetch in the page: the usage answer gets its token figures multiplied (so K / M show), the status gets a made-up import
# job waiting for the confirm, and POST /api/ai/import/confirm is answered by the script (the bodies are kept in window.__cfm)
FAKE_JS = """(() => { if (window.__fake) return 1; window.__fake = true; window.__cfm = []; window.__cfmAnswers = []; const real = window.fetch.bind(window); window.__realFetch = window.fetch;
  const grow = (o) => { if (Array.isArray(o)) o.forEach(grow); else if (o && typeof o === 'object') for (const k of Object.keys(o)) {
    if (/^tokens(In|Out|Cached)$/.test(k) && typeof o[k] === 'number') o[k] = o[k] * 4321 + 1234; else grow(o[k]); } };
  window.fetch = async (u, init) => { const url = String(u);
    if (url.includes('/api/ai/import/confirm')) { window.__cfm.push(JSON.parse(init.body)); const a = window.__cfmAnswers.shift() || {status: 400, body: {code: 'ai.gen.badName'}};
      return new Response(JSON.stringify(a.body), {status: a.status, headers: {'Content-Type': 'application/json'}}); }
    const r = await real(u, init);
    if (url.includes('/api/ai/admin/usage')) { const j = await r.clone().json(); grow(j); return new Response(JSON.stringify(j), {status: r.status, headers: {'Content-Type': 'application/json'}}); }
    if (/\\/api\\/ai\\/status(\\?|$)/.test(url) && window.__fakeJob) { const j = await r.clone().json(); if (j && j.gen) j.gen.job = window.__fakeJob;
      return new Response(JSON.stringify(j), {status: r.status, headers: {'Content-Type': 'application/json'}}); }
    return r; }; return 1 })()"""


def g_l60(rc):
    if not rc.want('W-L60') or not rc.walk.wants_page('quiz'):
        return
    for lang in rc.langs(['zh']):
        for size in rc.sizes([(320, 640), (360, 740), (390, 844), (430, 932), (1280, 800)], lambda s: True):
            rc.guard('W-L60', 'quiz', l60_case, rc, lang, rc.themes(['light'])[0], size)
    if not rc.args.themes and not rc.args.full:
        rc.guard('W-L60', 'quiz', l60_case, rc, 'zh', 'dark', (390, 844))


L60_JS = """(() => { const b = e => { const r = e.getBoundingClientRect(); return {x: r.left, r: r.right, y: r.top, bt: r.bottom, h: r.height, w: r.width} };
  const bar = document.querySelector('.qbar'); const sub = document.querySelector('.btn.qsubmit'); const cnt = document.querySelector('.qcount'); const r = bar.querySelector('.r');
  const vis = e => e && e.getClientRects().length > 0;
  const items = [...r.children].filter(vis).map(e => ({c: e.className.split(' ')[0], ...b(e)}));
  let overlap = ''; for (let i = 0; i < items.length; i++) for (let j = i + 1; j < items.length; j++) if (items[i].x < items[j].r - 0.5 && items[j].x < items[i].r - 0.5) overlap += items[i].c + '/' + items[j].c + ' ';
  const cs = sub ? getComputedStyle(sub) : null; const after = sub ? getComputedStyle(sub, '::after') : null;
  const vtxt = e => [...e.children].filter(c => getComputedStyle(c).position !== 'absolute' && c.getClientRects().length).map(c => c.textContent).join('');
  return {bar: b(bar), sub: sub && vis(sub) ? b(sub) : null, cnt: cnt && vis(cnt) ? b(cnt) : null, cntText: cnt ? vtxt(cnt) : '', cntAria: cnt ? cnt.textContent : '',
    label: sub ? [...sub.querySelectorAll('.lbl')].filter(vis).map(e => e.textContent).join('') : '', border: cs ? cs.borderTopWidth : '', shadow: cs ? cs.boxShadow : '',
    hitH: after ? parseFloat(after.height) || 0 : 0, hitW: after ? parseFloat(after.width) || 0 : 0, overlap, side: document.documentElement.scrollWidth <= innerWidth + 1,
    barKids: [...bar.children].filter(vis).length, rBottom: b(r).bt, rTop: b(r).y, safeTop: parseFloat(getComputedStyle(bar).paddingTop) || 0,
    wrap: items.length ? Math.max(...items.map(i => i.y)) - Math.min(...items.map(i => i.y)) : 0} })()"""


L60_POS_JS = """(() => { const q = document.querySelector('.qbar .ql'); const vis = e => e.getClientRects().length > 0;
  const parts = [...q.querySelectorAll('.pos-full .num, .pos-short .num')].filter(vis);
  const tops = [...q.querySelectorAll('.pos-full, .pos-short, .qkind')].filter(vis).map(e => e.getBoundingClientRect().top);
  const lines = parts.length ? Math.max(...parts.map(e => Math.round(e.getBoundingClientRect().height / (parseFloat(getComputedStyle(e).lineHeight) || 20)))) : 0;
  return {lines: Math.max(1, lines), spread: tops.length ? Math.max(...tops) - Math.min(...tops) : 0, text: parts.map(e => e.textContent).join(' ')} })()"""


def l60_case(rc, lang, theme, size):
    """A-OWNER-60: the finish pill sits inside the top bar (bar height unchanged, pill <= 32 px tall, 44 px tap area on touch, one frame),
    the count reads "<word> a/n" with spacing (only "a/n" up to 360 px), no overlap, no wrap, no sideways scroll; exam and practice"""
    for user, kind in (('riley', 'exam'), ('sam', 'practice')):
        d = rc.dev(user, lang, theme, size)
        start_quiz(d, kind, 51)
        time.sleep(0.4)
        r = d.js(L60_JS)
        problems = []
        want_word = cat(lang, 'quiz.top.answered').split('{')[0].strip()
        if size[0] > 880:
            if r['sub'] or r['cnt']:
                problems.append('the pill / count show on the wide layout')
        else:
            if not r['sub']:
                problems.append('no finish pill')
            else:
                phone_bar = 36 + r['safeTop'] if size[0] > size[1] else 44 + r['safeTop']
                if r['bar']['h'] > phone_bar + 1.5:
                    problems.append('bar %d px (max %d)' % (r['bar']['h'], phone_bar + 1))
                if r['sub']['h'] > 32.5:
                    problems.append('the pill is %d px tall' % r['sub']['h'])
                if r['sub']['y'] < r['bar']['y'] - 0.5 or r['sub']['bt'] > r['bar']['bt'] + 0.5:
                    problems.append('the pill sticks out of the bar')
                if r['border'] not in ('0px', '') or r['shadow'] != 'none':
                    problems.append('a second frame (border %s, shadow %s)' % (r['border'], r['shadow'][:30]))
                if d.js("matchMedia('(pointer: coarse)').matches") and (r['hitH'] < 43.5 or r['hitW'] < 43.5 and r['sub']['w'] < 43.5):
                    problems.append('tap area %dx%d' % (r['hitW'], r['hitH']))
            if not r['cnt']:
                problems.append('no count')
            else:
                nums = r['cntText'].strip()
                if not re.fullmatch(r'(%s ?)?0/51' % re.escape(want_word), nums) or (size[0] > 360 and not nums.startswith(want_word)):
                    problems.append('count reads "%s"' % nums)
                if r['sub'] and r['sub']['x'] - r['cnt']['r'] < 6:
                    problems.append('count and pill %d px apart' % (r['sub']['x'] - r['cnt']['r']))
        if r['overlap']:
            problems.append('overlap: ' + r['overlap'])
        if not r['side'] or r['wrap'] > 12:
            problems.append('side scroll or wrapped row (spread %d)' % r['wrap'])
        # the position counter itself ("1 / 12", "第 1 题 共 12 题") stays on one line next to the kind tag (owner rule: one line)
        start_quiz(d, kind, 12)
        time.sleep(0.4)
        pos = d.js(L60_POS_JS)
        if pos['lines'] > 1 or pos['spread'] > 6:
            problems.append('the position "%s" wraps (%d lines, spread %d px)' % (pos['text'], pos['lines'], pos['spread']))
        rc.rec('W-L60', 'quiz', kind, user, size, lang, theme, not problems,
               '; '.join(problems) or 'bar %dpx, pill %s, count "%s", label "%s", tap %dx%d' % (r['bar']['h'], round(r['sub']['h']) if r['sub'] else '-', r['cntText'].strip(), r['label'], r['hitW'] or 0, r['hitH'] or 0), d if problems else None)


def g_s10(rc):
    if not any(rc.want(i) for i in ('W-L59', 'W-L57', 'W-JC-1')) or not rc.walk.wants_page('ai-settings'):
        return
    if rc.want('W-JC-1'):
        d = rc.dev('admin', 'zh', 'light', (390, 844))
        goto(d, '#/', 2.0)
        rc.notes.append('jobs for the cards: %s' % ensure_jobs(rc, d))
    if rc.want('W-L59'):
        rc.paging_users()
        for lang in rc.langs(['zh']):
            for size in rc.sizes([(360, 740), (390, 844), (1280, 800)], lambda s: True):
                rc.guard('W-L59', 'ai-settings', l59_case, rc, lang, rc.themes(['light'])[0], size)
        if not rc.args.themes and not rc.args.full:
            rc.guard('W-L59', 'ai-settings', l59_case, rc, 'zh', 'dark', (390, 844))
    if rc.want('W-L57'):
        for lang in rc.langs(['zh']):
            for size in rc.sizes([(360, 740), (390, 844), (1280, 800)], lambda s: True):
                rc.guard('W-L57', 'banks', l57_case, rc, lang, rc.themes(['light'])[0], size)
        if not rc.args.themes and not rc.args.full:
            rc.guard('W-L57', 'banks', l57_case, rc, 'zh', 'dark', (390, 844))


def l73_case(rc, lang, theme, size):
    """L-73 (A-OWNER-79): the guide's "use an outside AI" part has six numbered steps, the prompt in a box that scrolls inside
    itself (no sideways scroll of the page) and a copy button that is a real tap target"""
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/banks', 2.0)
    d.js("(() => { document.getElementById('bk-guide-d').open = true; const subs = [...document.querySelectorAll('.bk-sub')]; subs[subs.length - 1].open = true; return 1 })()")
    time.sleep(0.4)
    r = d.js("""(() => { const pre = document.getElementById('bk-prompt'), btn = document.getElementById('bk-prompt-copy'); if (!pre || !btn) return null;
      pre.scrollIntoView({block: 'center'});
      const sub = pre.closest('.bk-sub'), cr = pre.getBoundingClientRect(), br = btn.getBoundingClientRect();
      return {steps: sub.querySelectorAll('ol li').length, inner: pre.scrollHeight > pre.clientHeight + 4, fits: cr.right <= innerWidth + 0.5 && cr.left >= -0.5,
              mono: /mono|Consolas|Menlo/i.test(getComputedStyle(pre).fontFamily), btnH: Math.round(br.height), btnW: Math.round(br.width),
              text: pre.textContent.length, btnIn: br.right <= innerWidth + 0.5} })()""")
    problems = []
    if r is None:
        rc.rec('W-L73', 'banks', 'outside-ai', 'admin', size, lang, theme, False, 'the prompt box or the copy button is missing')
        return
    if not d.js(NO_SIDE_SCROLL):
        problems.append('sideways scroll')
    if r['steps'] != 6:
        problems.append('%d steps instead of 6' % r['steps'])
    if not r['inner']:
        problems.append('the prompt box does not scroll inside itself')
    if not (r['fits'] and r['btnIn']):
        problems.append('the prompt box or the copy button is past the screen edge')
    if not r['mono']:
        problems.append('the prompt is not monospace')
    if r['btnH'] < 32 or r['btnW'] < 44:
        problems.append('copy button %dx%d' % (r['btnW'], r['btnH']))
    if r['text'] < 500:
        problems.append('the prompt text is only %d characters' % r['text'])
    rc.rec('W-L73', 'banks', 'outside-ai', 'admin', size, lang, theme, not problems, '; '.join(problems) or 'steps 6, prompt box scrolls inside, copy button %dx%d' % (r['btnW'], r['btnH']), d)


def l59_case(rc, lang, theme, size):
    """A-OWNER-59: the by-user table is a mini table on phones (header row, own sideways scroll, first column fixed, 10 per page),
    K / M figures in every language (never 万), exact figures with separators at the top"""
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings?tab=feature', 2.5)
    d.js(FAKE_JS)
    d.tap_sel('#ais-tab-usage', wait=2.5)
    d.js("(() => { const s = document.getElementById('ai-days'); s.value = s.options[s.options.length - 1].value; s.dispatchEvent(new Event('change', {bubbles: true})); return 1 })()")
    time.sleep(2.5)
    r = d.js("""(() => { const root = document.querySelector('.ptb-mini'); if (!root) return null; const q = (s) => [...root.querySelectorAll(s)];
      const vis = (e) => { const c = getComputedStyle(e), b = e.getBoundingClientRect(); return c.display !== 'none' && c.visibility !== 'hidden' && b.width > 0 && b.height > 0 };
      const wrap = root.querySelector('.ais-tablewrap'); const first = root.querySelector('tbody th');
      const cells = q('tbody td.ais-c').filter(vis).map(e => e.innerText.trim()).filter(x => !x.includes('/'));
      return {head: q('thead th').filter(vis).length, heads: q('thead .ptb-sort').map(e => Math.round(e.getBoundingClientRect().height)), sel: q('.ptb-sortsel').some(vis),
        cardRows: q('tbody tr').some(e => getComputedStyle(e).display === 'grid'), rows: q('tbody tr').length, cells, sticky: first ? getComputedStyle(first).position : '',
        own: wrap.scrollWidth, ownW: wrap.clientWidth, page: document.documentElement.scrollWidth <= innerWidth + 1,
        tops: [...document.querySelectorAll('.ais-total dd')].map(e => e.textContent.trim()), body: document.querySelector('.ais-sec, .panel').innerText} })()""")
    d.js("window.fetch = window.__realFetch; window.__fake = false; 1")
    if not r:
        rc.rec('W-L59', 'ai-settings', 'mini-table', 'admin', size, lang, theme, None, 'no by-user table yet')
        return
    problems = []
    phone_w = size[0] <= 560
    if not r['page']:
        problems.append('the page scrolls sideways')
    if r['rows'] != 10:
        problems.append('%d rows on page 1' % r['rows'])
    if r['sel'] or r['cardRows']:
        problems.append('cards / sort select instead of the table')
    if not r['head']:
        problems.append('no header row')
    if '万' in r['body']:
        problems.append('万 in the usage tab')
    bad_top = [x for x in r['tops'] if not re.fullmatch(r'\d{1,3}(,\d{3})*', x)]
    if bad_top:
        problems.append('top figures are not exact with separators: %s' % bad_top)
    if phone_w:
        if min(r['heads'] or [0]) < 31.5:
            problems.append('header taps under 32 px: %s' % r['heads'])
        if r['sticky'] != 'sticky':
            problems.append('the name column is not fixed (%s)' % r['sticky'])
        # ja (A-OWNER-77 short headers, Token in English) fits 360 px without scrolling: nothing to prove there
        if r['own'] <= r['ownW'] and size[0] <= 360 and lang != 'ja':
            problems.append('the table does not scroll inside its own box (%d / %d)' % (r['own'], r['ownW']))
        wrong = [x for x in r['cells'] if not re.fullmatch(r'\d+(\.\d)?[KM]?', x)]
        if wrong:
            problems.append('cells are not compact K / M figures: %s' % wrong[:3])
        elif not any(re.search(r'[KM]$', x) for x in r['cells']):
            problems.append('no K / M figure among the cells %s' % r['cells'][:6])
    else:
        if any(re.search(r'[KM]$', x) for x in r['cells']):
            problems.append('the wide table shows compact figures')
    rc.rec('W-L59', 'ai-settings', 'mini-table', 'admin', size, lang, theme, not problems,
           '; '.join(problems) or 'mini table: header, %d rows, K / M figures, top %s' % (r['rows'], r['tops'][:3]), d if problems else None)


def l57_case(rc, lang, theme, size):
    """A-OWNER-57: the confirm dialog of an AI import has a required bank name box, filled in advance; the import button stays
    off while the trimmed name is empty (reason shown); the body carries the name; a server refusal shows and keeps the dialog"""
    d = rc.dev('admin', lang, theme, size)
    job_js = """window.__fakeJob = {id: 987001, state: 'running', phase: 'confirm', source: 'file', name: 'Made-up notes_admin_20261113120000', from: [], fromCount: 0,
      file: {name: 'Made-up notes.docx', type: 'docx', size: 2048}, mode: 'questions', found: 8, read: {done: 3, total: 3}, confirmBy: Math.floor(Date.now() / 1000) + 3000,
      made: 0, goal: 8, calls: 3, createdAt: Math.floor(Date.now() / 1000) - 60, startedAt: Math.floor(Date.now() / 1000) - 50, bankId: null, error: null, charged: false}; 1"""
    # the status read at page load already holds the made-up job (the status is read at most every 60 s afterwards)
    script = d.cdp.call('Page.addScriptToEvaluateOnNewDocument', source=FAKE_JS + ';' + job_js)
    try:
        goto(d, '#/banks', 1.0)
        d.reload(3.0)
        opened = until(d, "!!document.querySelector('dialog.gen-cfm[open]')", 14)
        if not opened:
            rc.rec('W-L57', 'banks', 'name-box', 'admin', size, lang, theme, False, 'the confirm dialog did not open', d)
            return
        l57_checks(rc, d, lang, theme, size)
    finally:
        try:
            d.cdp.call('Page.removeScriptToEvaluateOnNewDocument', identifier=(script or {}).get('identifier'))
        except Exception:
            pass
        d.reload(1.5)


def l57_checks(rc, d, lang, theme, size):
    need = cat(lang, 'aigen.imp.cfm.nameNeed')
    r = d.js("""(() => { const dlg = document.querySelector('dialog.gen-cfm'); const n = dlg.querySelector('input.gen-cfm-name'); const go = dlg.querySelector('.acts .btn.primary');
      const b = (e) => e.getBoundingClientRect(); const lab = dlg.querySelector('label[for="' + n.id + '"]');
      return {value: n.value, label: lab ? lab.textContent : '', ph: n.placeholder, h: Math.round(b(n).height), btnH: Math.round(b(go).height), off: go.disabled,
        side: document.documentElement.scrollWidth <= innerWidth + 1, inView: b(dlg).right <= innerWidth + 1 && b(dlg).left >= -1} })()""")
    problems = []
    if r['value'] != 'Made-up notes_admin_20261113120000':
        problems.append('the name box is not pre-filled (%s)' % r['value'])
    if r['label'] != cat(lang, 'aigen.imp.cfm.nameLabel') or r['ph'] != cat(lang, 'aigen.imp.cfm.namePh'):
        problems.append('label / placeholder: %s / %s' % (r['label'], r['ph']))
    if r['off']:
        problems.append('the import button is off with a name filled in')
    if d.js("matchMedia('(pointer: coarse)').matches") and (r['h'] < 43.5 or r['btnH'] < 43.5):
        problems.append('targets under 44 px (box %d, button %d)' % (r['h'], r['btnH']))
    if not r['side'] or not r['inView']:
        problems.append('the dialog runs past the screen')
    d.js("""(() => { const n = document.querySelector('input.gen-cfm-name'); n.value = '   '; n.dispatchEvent(new Event('input', {bubbles: true})); return 1 })()""")
    e = d.js("""(() => { const dlg = document.querySelector('dialog.gen-cfm'); return {off: dlg.querySelector('.acts .btn.primary').disabled, need: dlg.querySelector('.gen-cfm-need').textContent,
      invalid: dlg.querySelector('input.gen-cfm-name').getAttribute('aria-invalid')} })()""")
    if not e['off'] or e['need'] != need or e['invalid'] != 'true':
        problems.append('empty name: button off=%s, reason "%s" (want "%s")' % (e['off'], e['need'], need))
    # a refusal of the server (the made-up answer) shows under the box; the dialog stays open and the name is kept
    d.js("""(() => { const n = document.querySelector('input.gen-cfm-name'); n.value = '  Walk name  '; n.dispatchEvent(new Event('input', {bubbles: true}));
      document.querySelector('dialog.gen-cfm .acts .btn.primary').click(); return 1 })()""")
    time.sleep(1.0)
    f = d.js("""(() => { const dlg = document.querySelector('dialog.gen-cfm'); return {open: !!dlg && dlg.open, err: dlg ? dlg.querySelector('.gen-cfm-err').textContent : '',
      shown: dlg ? !dlg.querySelector('.gen-cfm-err').hidden : false, sent: window.__cfm} })()""")
    if not f['open'] or not f['shown'] or not f['err']:
        problems.append('a refused name did not stay as an error in the open dialog (%s)' % f)
    if not f['sent'] or f['sent'][0].get('name') != 'Walk name' or f['sent'][0].get('id') != 987001:
        problems.append('the confirm request carries %s' % f['sent'])
    d.js("document.querySelector('dialog.gen-cfm').close('cancel'); window.__fakeJob = null; 1")
    rc.rec('W-L57', 'banks', 'name-box', 'admin', size, lang, theme, not problems,
           '; '.join(problems) or 'name box pre-filled, empty -> "%s" and the button off, refusal shown, trimmed name sent' % need, d if problems else None)


def g_tk(rc):
    """W-TK-1 (A-OWNER-67c/67d, 68): the token line stays on one line at every width / language; the cancel button is a red outline button"""
    if not rc.want('W-TK-1') or not rc.walk.wants_page('banks'):
        return
    for lang in rc.langs(['zh', 'en', 'ja']):
        for size in rc.sizes([(320, 640), (360, 740), (390, 844), (430, 932), (1280, 800)], lambda s: True):
            for source in ('file', 'bank'):
                rc.guard('W-TK-1', 'banks', tk_case, rc, lang, rc.themes(['light'])[0], size, source)
                if size[0] <= 360:  # worst case: 3-digit calls and 6-digit tokens (the count drops, the numbers never cut)
                    rc.guard('W-TK-1', 'banks', tk_case, rc, lang, rc.themes(['light'])[0], size, source, True)
    if not rc.args.themes and not rc.args.full:
        rc.guard('W-TK-1', 'banks', tk_case, rc, 'zh', 'dark', (390, 844), 'bank')


TK_JS = """(() => { const u = document.querySelector('.gen-usage'); const c = document.querySelector('.gen-cancel'); if (!u || !c) return null;
  const cs = getComputedStyle(u); const lh = parseFloat(cs.lineHeight) || 18; const r = document.createRange(); r.selectNodeContents(u);
  const lines = new Set([...r.getClientRects()].map(x => Math.round(x.top))).size;
  const rgb = (v) => (v.match(/[\d.]+/g) || []).slice(0, 3).map(Number);
  const lum = (a) => { const f = a.map(x => { x /= 255; return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4) }); return 0.2126 * f[0] + 0.7152 * f[1] + 0.0722 * f[2] };
  const bgOf = (e) => { for (; e; e = e.parentElement) { const b = getComputedStyle(e).backgroundColor; const m = b.match(/[\d.]+/g) || []; if (m.length >= 3 && (m.length < 4 || +m[3] > 0.9)) return rgb(b) } return [255, 255, 255] };
  const cc = getComputedStyle(c); const fg = lum(rgb(cc.color)); const bg = lum(bgOf(c)); const ratio = (Math.max(fg, bg) + 0.05) / (Math.min(fg, bg) + 0.05);
  const cb = c.getBoundingClientRect(); const ub = u.getBoundingClientRect();
  return {text: u.textContent, lines, fit: u.scrollWidth <= u.clientWidth, fs: parseFloat(cs.fontSize), nowrap: cs.whiteSpace, tab: cs.fontVariantNumeric, right: ub.right, vw: innerWidth,
    side: document.documentElement.scrollWidth <= innerWidth + 1, cH: cb.height, ic: !!c.querySelector('svg'), border: cc.borderTopColor, ratio, label: c.textContent.trim(),
    tip: (u.closest('.has-tip').querySelector('.tip') || {}).textContent, coarse: matchMedia('(pointer: coarse)').matches} })()"""


def tk_case(rc, lang, theme, size, source, big=False):
    d = rc.dev('admin', lang, theme, size)
    now = 'Math.floor(Date.now() / 1000)'
    job_js = """window.__fakeJob = {id: 987002, state: 'running', phase: 'writing', source: '%s', name: 'Made-up bank_admin_20261113120000', from: [], fromCount: 0,
      file: %s, mode: null, found: null, read: null, confirmBy: null, made: 4, goal: 20, calls: 9, createdAt: %s - 60, startedAt: %s - 50, bankId: null, error: null, charged: false,
      usage: {calls: %s, tokensIn: %s, tokensOut: %s, tokensCacheWrite: %s}}; 1""" % (
        source, "{name: 'Made-up notes.docx', type: 'docx', size: 2048}" if source == 'file' else 'null', now, now, *((120, 999949, 987654, 1234567) if big else (12, 1434, 21907, 13000)))
    script = d.cdp.call('Page.addScriptToEvaluateOnNewDocument', source=FAKE_JS + ';' + job_js)
    try:
        goto(d, '#/banks', 1.0)
        d.reload(3.0)
        if not until(d, "!!document.querySelector('.gen-usage')", 12):
            rc.rec('W-TK-1', 'banks', source, 'admin', size, lang, theme, False, 'no token line on the running card', d)
            return
        time.sleep(0.6)
        r = d.js(TK_JS)
    finally:
        try:
            d.cdp.call('Page.removeScriptToEvaluateOnNewDocument', identifier=(script or {}).get('identifier'))
        except Exception:
            pass
        d.js("window.__fakeJob = null; 1")
        d.reload(1.5)
    problems = []
    if r is None:
        problems.append('no token line or no cancel button')
    else:
        if r['lines'] != 1 or not r['fit'] or r['nowrap'] != 'nowrap':
            problems.append('token line not on one line (lines %s, fit %s, %s): %s' % (r['lines'], r['fit'], r['nowrap'], r['text']))
        if not 12 <= r['fs'] <= 13 or 'tabular' not in r['tab']:
            problems.append('token line type %s / %s' % (r['fs'], r['tab']))
        if ('999.9K/987.7K' if big else '1.4K/21.9K') not in r['text'].replace(' ', ''):
            problems.append('numbers missing in "%s"' % r['text'])
        # A-OWNER-74b: the cache-write figure ends the line (words may shorten, the number never)
        if ('1.2M' if big else '13K') not in r['text'].replace(' ', ''):
            problems.append('cache-write figure missing in "%s"' % r['text'])
        # A-OWNER-74b: the cache words shorten before the call count goes
        calls = 120 if big else 12
        long_words = cat(lang, 'aigen.card.cw').split('{n}')[0].strip(' ,，、')
        if not re.search(r'(?<![\d.,])%d(?![\d.,KM])' % calls, r['text']) and long_words in r['text']:
            problems.append('the call count is gone while the cache words are still long: "%s"' % r['text'])
        if not any(x in (r['tip'] or '') for x in (('1,234,567', '1.234.567', '1 234 567') if big else ('13,000', '13.000', '13 000'))):
            problems.append('the tip has no exact cache-write number: %s' % r['tip'])
        if r['right'] > r['vw'] + 0.5 or not r['side']:
            problems.append('the token line runs past the screen')
        if not any(x in (r['tip'] or '') for x in (('999,949', '999.949', '999 949') if big else ('1,434', '1.434', '1 434'))):
            problems.append('the tip has no exact numbers: %s' % r['tip'])
        if r['coarse'] and r['cH'] < 43.5:
            problems.append('cancel button %s px high' % r['cH'])
        if not r['ic'] or r['ratio'] < 4.5 or r['label'] != cat(lang, 'aigen.card.cancel') and r['label'] != cat(lang, 'aigen.imp.card.cancel'):
            problems.append('cancel button: icon %s, contrast %.2f, label "%s"' % (r['ic'], r['ratio'], r['label']))
    rc.rec('W-TK-1', 'banks', source, 'admin', size, lang, theme, not problems,
           '; '.join(problems) or 'one line "%s", cancel button red %.1f:1' % (r['text'], r['ratio']), d if problems else None)


# ---------------------------------------------------------------------------------------------------- entry point
# ---------------------------------------------------------------------------------------------------- sprint 13
# W-RS-9: a not-completed hotspot question is not judged: its items carry no right / wrong tag and no "n of m right" line
RS9_JS = """(() => { const hs = [...document.querySelectorAll('.rv')].filter(x => x.querySelector('.hs'));
  return {n: hs.length, tags: hs.reduce((a, x) => a + x.querySelectorAll('.hs-item .tag').length, 0), verdict: hs.length ? (hs[0].querySelector('.verdict') || {}).textContent || '' : ''} })()"""


def g_s13(rc):
    if rc.want('W-RS-9') and rc.walk.wants_page('result'):
        for lang in rc.langs(['zh']):
            rc.guard('W-RS-9', 'result', rs9_case, rc, lang, rc.themes(['light'])[0], (1280, 800))
    if rc.want('W-IM-3') and rc.walk.wants_page('banks'):
        for lang in rc.langs(['zh']):
            rc.guard('W-IM-3', 'banks', im3_case, rc, lang, rc.themes(['light'])[0], (390, 844))


def rs9_case(rc, lang, theme, size):
    d = rc.dev('riley', lang, theme, size)
    start_quiz(d, 'exam', 51)
    time.sleep(0.4)
    d.js("""(async () => { const w = ms => new Promise(r => setTimeout(r, ms)); const click = s => { const e = document.querySelector(s); if (e && !e.disabled) { e.click(); return true } return false };
      click('[data-act="submit"]') || click('[data-act="finish"]') || click('.rail .btn.primary'); await w(500);
      click('dialog[open] .acts .btn.primary') || click('dialog[open] .btn.primary'); await w(2500) })()""")
    time.sleep(3)
    r = d.js(RS9_JS)
    problems = []
    if not r['n']:
        problems.append('no hotspot card on the result page')
    else:
        if r['tags']:
            problems.append('%d right / wrong tags on a not-completed hotspot question' % r['tags'])
        if len(re.findall(r'\d+', r['verdict'])) > 1:   # "n items and you got k right" has two numbers, the plain rule one
            problems.append('verdict %r' % r['verdict'])
    rc.rec('W-RS-9', 'result', 'hotspot-open', 'riley', size, lang, theme, not problems,
           '; '.join(problems) or '%d hotspot cards, no tags, verdict "%s"' % (r['n'], r['verdict'][:40]), d if problems else None)


IM3_JS = """(() => { const c = document.querySelector('.gen-file'); if (!c) return null;
  const labs = [...c.querySelectorAll('.gen-cnt')].map(e => { const cs = getComputedStyle(e); return {t: e.textContent, bg: cs.backgroundColor, fg: cs.color} });
  return {labs, line: (c.querySelector('.gen-line') || {}).textContent || ''} })()"""


def im3_case(rc, lang, theme, size):
    """A-OWNER-70 / 72 / 73: an import of 60 made-up questions: the confirm card names the file's count and the limit (two red labels with
    white text) and the line "found 60, the first 50 are imported"; the confirm dialog repeats them; the done card says "50 of 60"."""
    u = rc.session('lee')
    filler = 'This made-up sample sentence only pads the stem. ' * 3
    text = '\n'.join('Q%d. Sample question %d: %s Which choice is the made-up one?\nA. First\nB. Second\nC. Third\nD. Fourth' % (i, i, filler) for i in range(1, 61)) + '\n'
    a = u.call('/api/ai/import/upload', raw=text.encode(), headers={'Content-Type': 'application/octet-stream', 'X-Filename': 'sample-q.txt', 'X-Ui-Lang': lang})
    jid = a.get('id') or a.get('job', {}).get('id')
    j = {}
    t0 = time.time()
    while time.time() - t0 < 240:
        j = u.call('/api/ai/gen/job?id=%s' % jid)
        j = j.get('job', j)
        if j.get('state') in ('failed', 'done') or j.get('phase') == 'confirm':
            break
        time.sleep(1)
    problems = []
    d = rc.dev('lee', lang, theme, size)
    try:
        if j.get('phase') != 'confirm' or j.get('total') != 60 or j.get('taken') != 50:
            problems.append('job state %s / %s total %s taken %s' % (j.get('state'), j.get('phase'), j.get('total'), j.get('taken')))
        else:
            goto(d, '#/banks', 2.5)
            r = d.js(IM3_JS)
            want_line = fill(cat(lang, 'aigen.imp.card.confirmQCut'), n=50, found=60)
            want = [fill(cat(lang, 'aigen.imp.cnt.file'), n=60), fill(cat(lang, 'aigen.imp.cnt.limit'), n=50)]
            if not r or [x['t'] for x in r['labs']] != want:
                problems.append('labels %s (want %s)' % (r and [x['t'] for x in r['labs']], want))
            elif any(x['bg'] != 'rgb(185, 28, 28)' or x['fg'] != 'rgb(255, 255, 255)' for x in r['labs']):
                problems.append('the labels are not white on red: %s' % r['labs'])
            if r and r['line'].replace(chr(160), ' ') != want_line.replace(chr(160), ' '):
                problems.append('line %r (want %r)' % (r['line'], want_line))
            # the confirm dialog
            d.js("(() => { const b = [...document.querySelectorAll('.gen-file .btn.primary, .gen-file .btn.ai')][0]; if (b) b.click() })()")
            time.sleep(0.8)
            dl = d.js("(() => { const x = document.querySelector('dialog.gen-cfm[open]'); return x ? {labs: [...x.querySelectorAll('.gen-cnt')].map(e => e.textContent), body: x.querySelector('p:not(.gen-counts)').textContent} : null })()")
            if not dl or dl['labs'] != want or '60' not in dl['body']:
                problems.append('confirm dialog %r' % (dl,))
            d.js("(() => { const x = document.querySelector('dialog.gen-cfm[open]'); if (x) x.close() })()")
    finally:
        try:
            u.call('/api/ai/gen/cancel', {'id': jid})
        except Exception:
            pass
    rc.rec('W-IM-3', 'banks', 'import-confirm', 'lee', size, lang, theme, not problems,
           '; '.join(problems) or 'two red labels "%s" / "%s"; the line and the dialog name 60 and 50' % (want[0], want[1]), d if problems else None)


GROUPS = [g_shared, g_toast, g_pinned, g_contrast, g_dialogs, g_menu, g_first_line, g_bar, g_panel, g_greyed, g_landscape,
          g_result, g_records, g_home, g_chat, g_users, g_logs_settings, g_ai_settings,
          g_sh8, g_sh9, g_sh10, g_qz9, g_as2, g_as3, g_ch4, g_ch5, g_us2, g_caps, g_v11, g_s10, g_l54, g_l60, g_tk, g_s13]


def run(walk):
    """Called by walk.py with the running stack; returns the exit code (1 when a P1 check failed)."""
    rc = Run(walk)
    rc.set_caps(WALK_CAPS)
    try:
        for group in GROUPS:
            rc.guard('W-GROUP', 'all', group, rc)
    finally:
        rc.set_caps(DEFAULT_CAPS)
        rc.close()
    return rc.write_report()
