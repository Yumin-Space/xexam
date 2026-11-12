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
P2_STATES = {('W-US-1', 'bank-row'), ('W-AS-1', 'usage-cards'), ('W-US-2', 'forbidden'), ('W-SH-10', 'short-page')}
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
    d.js(CLEAR_SESSION)
    goto(d, '#/')
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
    d.tap_sel('.opt', 0)            # a single-choice question is checked at once: Check answer greys out, Next stays in its own place
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
    ok = bool(r) and r['x'] >= -0.5 and r['r'] <= size[0] + 0.5 and 0 <= r['y'] < size[1] and bool(re.search('无法使用|Unavailable|使用不可', r['text']))
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
            problems.append('AI 解析 sent %d ask requests (expected 1)' % len(asks))
        d.net_clear()
        d.tap_sel('.rv .aip-ask')
        time.sleep(0.8)
        if d.net_requests('/api/ai/study/ask'):
            problems.append('a second press of AI 解析 asked again')
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


# ---- W-CH-4: the memory editor counts characters (n / 200); 201 are refused, 200 save
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
    if digits(count) != '0200':
        problems.append('counter reads %r (want 0 / 200)' % count)
    # 201 characters typed: the field keeps at most 200 (maxlength), or the add is refused with the catalog text
    d.js("document.getElementById('ai-mem-input').focus()")
    d.type('记' * 201)
    time.sleep(0.3)
    n = d.js("document.getElementById('ai-mem-input').value.length")
    if n > 200:
        d.net_clear()
        d.tap_sel('#ai-mem-add')
        time.sleep(0.8)
        posts = [u for m, u in d.net_requests('/api/ai/memories') if m == 'POST']
        err = fill(cat(lang, 'ai.err.memoryTooLong'), n=200)
        if posts and err not in d.js('document.body.innerText'):
            problems.append('201 characters were sent and no %r shown' % err)
    before = len(rc.session('sam').call('/api/ai/memories').get('memories', []))
    text = ('练习记忆' * 50)[:200]
    d.js_call(SET_VALUE, '#ai-mem-input', text)
    time.sleep(0.3)
    d.tap_sel('#ai-mem-add')
    time.sleep(1.2)
    mems = rc.session('sam').call('/api/ai/memories').get('memories', [])
    saved = [m for m in mems if m.get('content') == text]
    if len(mems) != before + 1 or not saved:
        problems.append('200 characters did not save (%d -> %d memories)' % (before, len(mems)))
    for m in saved:     # leave the made-up user as it was
        try:
            rc.session('sam').call('/api/ai/memories/delete', {'id': m['id']})
        except Exception:
            pass
    rc.rec('W-CH-4', 'ai', 'memory-length', 'sam', size, lang, theme, not problems,
           '; '.join(problems) or 'counter 0 / 200; 201 refused (field keeps %d); 200 saved' % min(n, 200), d if problems else None)


# ---- W-AS-3: four category tabs under the status chips
AS3_SIZES = [(360, 740), (390, 844), (768, 1024), (1280, 800)]
AS3_CONTENT = {'feature': '#ai-limits-save', 'users': '.ais-users', 'usage': '.ais-totals', 'safety': '#ai-limits-save'}


def g_as3(rc):
    if not rc.want('W-AS-3') or not rc.walk.wants_page('ai-settings'):
        return
    for lang in rc.langs(LANGS):
        for size in rc.sizes([(360, 740), (390, 844), (1280, 800)], lambda s: s in AS3_SIZES):
            rc.guard('W-AS-3', 'ai-settings', as3_case, rc, lang, rc.themes(['light'])[0], size)


def as3_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings', 2.5)
    d.js(CHECKS_JS)
    problems = []
    r = d.js("""(() => { const t = [...document.querySelectorAll('.ais-tabs button')];
      return {ids: t.map(b => b.id), pressed: t.filter(b => b.getAttribute('aria-pressed') === 'true').map(b => b.id),
              chips: !!document.querySelector('.ais-status')} })()""")
    if r['ids'] != ['ais-tab-feature', 'ais-tab-users', 'ais-tab-usage', 'ais-tab-safety']:
        problems.append('tabs %s' % ','.join(r['ids']))
    if r['pressed'] != ['ais-tab-feature']:
        problems.append('#/ai-settings opens %s' % (r['pressed'] or 'no tab'))
    row = W(d, 'oneRow', '.ais-tabs', 'button', 4)
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
           '; '.join(problems) or '4 tabs in one row; each tab replaces the hash, keeps the chips and survives a reload', d if problems else None)


# ---- W-AS-2: the answer and memory length in characters
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
      return {groups: g, reply: v('ai-lim-replyMaxChars'), mem: v('ai-lim-memoryChars'), msg: !!document.getElementById('ai-lim-messageChars'),
              hint: (document.getElementById('ai-lim-replyMaxChars-hint2') || {}).innerText || '', live: (document.getElementById('ai-lim-replyMaxChars-live') || {}).innerText || ''} })()""")
    if r['groups'] != ['caps', 'length', 'server']:
        problems.append('groups %s' % ','.join(r['groups']))
    if r['reply'] != '1000' or r['mem'] != '200' or r['msg']:
        problems.append('fields reply %s, memory %s, per-message field %s' % (r['reply'], r['mem'], r['msg']))
    if r['hint'].strip() != cat(lang, 'ai.set.limit.replyMaxCharsHint'):
        problems.append('hint %r' % r['hint'])
    if '1500' not in digits(r['live']) or '1000' not in digits(r['live']):
        problems.append('live line %r' % r['live'])
    d.js_call(SET_VALUE, '#ai-lim-replyMaxChars', '2000')
    time.sleep(0.4)
    live = d.js("(document.getElementById('ai-lim-replyMaxChars-live') || {}).innerText || ''")
    if '3000' not in digits(live) or '2000' not in digits(live):
        problems.append('after 2000 the live line reads %r' % live)
    if W(d, 'overflow')['doc'] > 1:
        problems.append('the page scrolls sideways')
    if lang == 'zh' and size == (390, 844):
        admin = rc.session('admin')
        for bad in ('0', '16001'):
            d.js_call(SET_VALUE, '#ai-lim-replyMaxChars', bad)
            d.tap_sel('#ai-limits-save', wait=1.2)
            err = d.js("(() => { const f = document.getElementById('ai-lim-replyMaxChars'); const box = f && f.closest('.ais-length'); const e = box && box.querySelector('.ais-ferr'); return e ? e.innerText : null })()")
            now = admin.call('/api/ai/admin/limits').get('limits', {}).get('replyMaxChars')
            if not err or now != 1000:
                problems.append('%s: error %r, stored %s' % (bad, err, now))
        d.js_call(SET_VALUE, '#ai-lim-replyMaxChars', '16000')
        d.tap_sel('#ai-limits-save', wait=1.5)
        now = admin.call('/api/ai/admin/limits').get('limits', {}).get('replyMaxChars')
        if now != 16000:
            problems.append('16000 did not save (stored %s)' % now)
        admin.call('/api/ai/admin/limits', {'replyMaxChars': 1000, 'memoryChars': 200})
    rc.rec('W-AS-2', 'ai-settings', 'lengths', 'admin', size, lang, theme, not problems,
           '; '.join(problems) or 'caps, length, server; 1000 / 200, live line 1500 / 1000 then 3000 / 2000%s' % (
               '; 0 and 16001 refused, 16000 saved' if lang == 'zh' and size == (390, 844) else ''), d if problems else None)


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
            grab('single-after', False)
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
      if (gr.top - prev.bottom > 8.5) out.problems.push('gap above the GitHub line ' + Math.round(gr.top - prev.bottom) + ' px');
      const ft = shell.getBoundingClientRect().top;
      if (note.getBoundingClientRect().top - ft > 12.5) out.problems.push('rule to disclaimer ' + Math.round(note.getBoundingClientRect().top - ft) + ' px');
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


# ---------------------------------------------------------------------------------------------------- entry point
GROUPS = [g_shared, g_toast, g_pinned, g_contrast, g_dialogs, g_menu, g_first_line, g_bar, g_panel, g_greyed, g_landscape,
          g_result, g_records, g_home, g_chat, g_users, g_logs_settings, g_ai_settings,
          g_sh8, g_sh9, g_sh10, g_qz9, g_as2, g_as3, g_ch4, g_us2, g_caps]


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
