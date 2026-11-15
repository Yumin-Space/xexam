# -*- coding: utf-8 -*-
"""v1.3 walk checks: the new screens of v1.3 on phones and desktop (zh / en / ja; 393x852, 360x740, 320x640, 1280x800).

Every case loads one screen, runs the shared measures (nothing sticks out sideways, touch hit areas >= 44 px on phones,
nothing covers a control) and the checks of what the screen must show:

  W-V13-CV   AI Chat: a message with 2 versions shows `< n/m >` (arrows are real buttons) and a regenerate button
  W-V13-ED   AI Chat: tapping Edit on the last question opens the edit banner with Cancel; Cancel closes it
  W-V13-MM   AI Chat, administrator in free mode: the model menu is one select inside the screen, laid invisibly over the
             round brain button; the composer is one box (the input above one bar with the brain button left and Send
             right; the empty box is at most 120 px high at 393 px wide)
  W-V13-AC   AI settings > Connections and models: status cards, connection cards, slot rows, switches card, the
             connection form (Add)
  W-V13-AT   AI settings > the other four tabs
  W-V13-US   Users: the sign-in info eye (closed and open), its clear button and the last-active line of every row
  W-V13-LG   Logs: the Activity tab (rows, settings popover) and the Service tab
  W-V13-SI   Sign-in page: the "keep me signed in" checkbox and its hint
  W-V13-RS   Result page: the retry-save banner when the attempt could not be saved, its retry button

Made-up data only (the stub provider, demo model names, example.com hosts). Registered in checks.py (GROUPS_V13).
"""
import json
import re
import time
import urllib.request

import seed
from checks import CHECKS_JS, LANGS, W, goto, make_result, phone, sz, until

SIZES = [(393, 852), (360, 740), (320, 640), (1280, 800)]

# What a thumb meets: the shared measures of checks.py, as one list of problems
COMMON_JS = """(() => { const o = window.__walk.overflow(); const a = window.__walk.audit({touch: %s});
  const p = [];
  if (o.doc > 1 || o.offenders.length) p.push('sideways: ' + o.doc + ' px; ' + o.offenders.join('; '));
  if (%s && a.small.length) p.push('small: ' + a.small.slice(0, 4).join('; '));
  if (a.covered.length) p.push('covered: ' + a.covered.slice(0, 4).join('; '));
  return p })()"""


def common(d, size):
    d.js(CHECKS_JS)
    touch = 'true' if phone(size) else 'false'
    return d.js(COMMON_JS % (touch, touch)) or []


def verdict(rc, cid, page, state, user, lang, theme, size, d, problems, ok_text):
    rc.rec(cid, page, state, user, size, lang, theme, not problems, '; '.join(problems) or ok_text, d if problems else None)


def post_stream(rc, user, path, payload):
    """POSTs `payload` to an AI route that streams and reads the stream to its end (the made-up provider answers)."""
    base = rc.walk.base
    req = urllib.request.Request(base + path, data=json.dumps(payload).encode(), method='POST',
                                 headers={'Origin': base, 'Content-Type': 'application/json',
                                          'Cookie': '%s=%s' % (seed.COOKIE, rc.walk.stack.cookie(user))})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read().decode('utf-8')


def versions_chat(rc):
    """A chat of sam whose last answer has 2 versions (one regenerate through the API)."""
    if 'v13-chat' in rc.cache:
        return rc.cache['v13-chat']
    from checks import ai_send
    cid = ai_send(rc.walk.base, rc.walk.stack.cookie('sam'), 'Give me one tip about a tidy desk.', 'en', None)
    s = rc.session('sam')
    msgs = s.call('/api/ai/chat?id=%d' % cid).get('messages', [])
    last = [m for m in msgs if m['role'] == 'assistant'][-1]
    post_stream(rc, 'sam', '/api/ai/regenerate', {'chatId': cid, 'messageId': last['id'], 'lang': 'en'})
    rc.cache['v13-chat'] = cid
    return cid


def cases(rc, cid, fn, users_sizes=None, langs=None):
    if not rc.want(cid):
        return
    for lang in rc.langs(langs or LANGS):
        for size in rc.sizes(SIZES, lambda s: True):
            rc.guard(cid, 'v13', fn, rc, lang, rc.themes(['light'])[0], size)


# ------------------------------------------------------------------------------------------------ AI Chat
VER_JS = """(() => { const v = document.querySelector('.ai-ver'); if (!v) return null; const b = [...v.querySelectorAll('.ai-ver-btn')];
  const r = v.getBoundingClientRect(); const n = v.querySelector('.ai-ver-n');
  const reg = document.querySelector('.ai-regen');
  return {text: n ? n.textContent : '', btns: b.length, enabled: b.filter(x => !x.disabled).length, label: v.getAttribute('aria-label'),
    inside: r.left >= 0 && r.right <= innerWidth, reg: !!reg, regLabel: reg ? (reg.getAttribute('aria-label') || '') : '' } })()"""


def cv_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/ai/%d' % versions_chat(rc), 2.4)
    p = common(d, size)
    r = d.js(VER_JS)
    if not r:
        p.append('no version switch on a message with 2 versions')
    else:
        if r['text'] != '2/2':
            p.append('version text "%s", wanted 2/2' % r['text'])
        if r['btns'] != 2 or r['enabled'] != 1:
            p.append('arrows: %d buttons, %d enabled (want 2 / 1)' % (r['btns'], r['enabled']))
        if not r['label'] or not r['inside'] or not r['reg'] or not r['regLabel']:
            p.append('label "%s", inside %s, regenerate %s' % (r['label'], r['inside'], r['reg']))
        # the older version: tap the left arrow, the text becomes 1/2
        d.js("document.getElementById('ai-ver-prev-1') && document.getElementById('ai-ver-prev-1').click(); 1")
        time.sleep(1.0)
        t1 = d.js("(document.querySelector('.ai-ver-n') || {}).textContent")
        if t1 != '1/2':
            p.append('after the left arrow the text is "%s"' % t1)
        # back to the newest version (the chosen branch is stored in the chat), so every run starts from 2/2
        d.js("document.getElementById('ai-ver-next-1') && document.getElementById('ai-ver-next-1').click(); 1")
        time.sleep(1.0)
    verdict(rc, 'W-V13-CV', 'ai', 'versions', 'sam', lang, theme, size, d, p, 'versions 2/2 -> 1/2, regenerate present, no overflow')


def ed_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/ai/%d' % versions_chat(rc), 2.4)
    p = []
    d.js("(() => { const b = [...document.querySelectorAll('.ai-edit')].pop(); if (b && !b.disabled) b.click(); return 1 })()")
    time.sleep(0.8)
    r = d.js("""(() => { const b = document.getElementById('ai-edit-banner'); const c = document.getElementById('ai-edit-cancel'); if (!b || b.hidden || !c) return null;
      const br = b.getBoundingClientRect(), cr = c.getBoundingClientRect(); return {inside: br.left >= 0 && br.right <= innerWidth && cr.right <= innerWidth + 0.5, h: cr.height} })()""")
    if not r:
        p.append('no edit banner after tapping Edit')
    else:
        if not r['inside']:
            p.append('banner or Cancel outside the screen')
        if phone(size) and r['h'] < 30:
            p.append('Cancel is %d px high' % r['h'])
        p += common(d, size)
        d.js("document.getElementById('ai-edit-cancel').click(); 1")
        time.sleep(0.5)
        if d.js("(() => { const b = document.getElementById('ai-edit-banner'); return !!b && !b.hidden })()"):
            p.append('Cancel did not close the banner')
    verdict(rc, 'W-V13-ED', 'ai', 'edit-banner', 'sam', lang, theme, size, d, p, 'edit banner opens, fits, Cancel closes it')


def mm_case(rc, lang, theme, size):
    s = rc.session('admin')
    d = rc.dev('admin', lang, theme, size)
    was = s.call('/api/ai/status').get('free')
    s.call('/api/ai/free', {'free': True})
    try:
        goto(d, '#/ai', 2.2)
        d.js("(() => { const b = document.querySelector('.ai-new, #ai-new'); if (b) b.click(); return 1 })()")
        time.sleep(1.0)
        p = []
        r = d.js("""(() => { const s = document.getElementById('ai-model'); if (!s || !s.getClientRects().length) return null; const b = s.getBoundingClientRect();
          return {inside: b.left >= 0 && b.right <= innerWidth && b.bottom <= innerHeight + 1, h: b.height, groups: s.querySelectorAll('optgroup').length, opts: s.options.length, first: s.options[0].textContent} })()""")
        if not r:
            p.append('no model menu for an administrator in free mode')
        else:
            if not r['inside']:
                p.append('the model menu is outside the screen')
            if phone(size) and r['h'] < 36:
                p.append('menu %d px high' % r['h'])
            if 'demo-chat-small' not in r['first']:
                p.append('the default entry names no model: "%s"' % r['first'])
            if r['opts'] < 2:
                p.append('only %d option(s)' % r['opts'])
        # The composer is ONE box: the input on top, one bar under it with the brain button (left) and Send (right)
        c = d.js("""(() => { const q = (s) => document.querySelector(s); const box = q('#ai-composer .ai-box'), bar = q('#ai-composer .ai-bar'),
          inp = q('#ai-input'), pick = q('#ai-model-pick'), sel = q('#ai-model'), send = q('#ai-send'); if (!box || !bar || !inp || !pick || !sel || !send) return null;
          const R = (e) => e.getBoundingClientRect(); const b = R(box), a = R(bar), i = R(inp), k = R(pick), n = R(send); const mid = (r) => (r.top + r.bottom) / 2;
          const within = (r, o) => r.left >= o.left - 0.5 && r.right <= o.right + 0.5 && r.top >= o.top - 0.5 && r.bottom <= o.bottom + 0.5;
          return {inBox: box.contains(inp) && bar.parentElement === box && within(i, b) && within(a, b), inBar: bar.contains(pick) && bar.contains(send) && pick.contains(sel),
            oneRow: Math.abs(mid(k) - mid(n)) <= 1 && k.top >= a.top - 2.5 && k.bottom <= a.bottom + 2.5 && n.top >= a.top - 2.5 && n.bottom <= a.bottom + 2.5,
            above: i.bottom <= a.top + 0.5, sides: k.right <= n.left && k.left - b.left < 24 && b.right - n.right < 24,
            hidden: getComputedStyle(sel).opacity === '0', name: ((q('.ai-model-face') || {}).textContent || '').trim(), label: sel.getAttribute('aria-label') || '',
            dot: !!q('.ai-model-dot:not([hidden])'), custom: sel.value !== '', h: b.height, empty: inp.value === '', oldRow: !!q('.ai-composer-side')} })()""")
        if not c:
            p.append('the composer has no box / bar / brain button / Send')
        else:
            if not c['inBox'] or not c['inBar']:
                p.append('the input and the bar are not inside the one box, or a button is outside the bar')
            if not c['oneRow'] or not c['sides']:
                p.append('the brain button (left) and Send (right) are not on one bar')
            if not c['above']:
                p.append('the input is not above the bar')
            if not c['hidden'] or c['name']:
                p.append('the model button shows text ("%s") or the select is visible' % c['name'])
            if not c['label']:
                p.append('the model select has no name')
            if c['dot'] != c['custom']:
                p.append('the dot (%s) does not follow the choice (not the default: %s)' % (c['dot'], c['custom']))
            if c['oldRow']:
                p.append('the old composer side column is still there')
            if size[0] == 393 and c['empty'] and c['h'] > 120:
                p.append('the empty box is %d px high (at most 120)' % c['h'])
        p += common(d, size)
        verdict(rc, 'W-V13-MM', 'ai', 'model-menu', 'admin', lang, theme, size, d, p, 'model menu inside the screen (%s groups, %s options)' % (r and r['groups'], r and r['opts']))
    finally:
        s.call('/api/ai/free', {'free': bool(was)})


# ------------------------------------------------------------------------------------------------ AI settings
def ac_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings', 2.6)
    p = []
    r = d.js("""(() => ({cards: document.querySelectorAll('.ais-scard').length, conns: document.querySelectorAll('li.ais-conn:not(.ais-conn-add)').length,
      slots: document.querySelectorAll('.ais-slot').length, sw: !!document.getElementById('ais-switches-h'), add: !!document.querySelector('.ais-conn-add button'),
      chips: document.querySelectorAll('.ais-chip').length, http: document.querySelectorAll('.ais-conn-http').length}))()""")
    if r['cards'] < 1:
        p.append('no status cards')
    if r['conns'] < 2:
        p.append('%d connection cards (the stack seeds 2)' % r['conns'])
    if r['slots'] < 1:
        p.append('no model slot rows')
    if not r['sw']:
        p.append('no feature switches card')
    if not r['add']:
        p.append('no Add connection button')
    p += common(d, size)
    # the form of a new connection
    if r['add']:
        d.js("document.querySelector('.ais-conn-add button').click(); 1")
        time.sleep(0.8)
        f = d.js("""(() => { const f = document.getElementById('ais-conn-form'); if (!f) return null; f.scrollIntoView({block: 'start'}); const b = f.getBoundingClientRect();
          return {inside: b.left >= 0 && b.right <= innerWidth + 0.5, inputs: f.querySelectorAll('input, select').length} })()""")
        if not f:
            p.append('Add did not open the connection form')
        else:
            if not f['inside']:
                p.append('the connection form sticks out of the screen')
            p += ['form ' + x for x in common(d, size)]
    verdict(rc, 'W-V13-AC', 'ai-settings', 'connections', 'admin', lang, theme, size, d, p,
            '%d status cards, %d connections, %d slots, switches card, form' % (r['cards'], r['conns'], r['slots']))


def near_miss(text):
    """True for a 'small:' line whose every item has a hit area of at least 40 px each way (a 2 px clip by a neighbour's tap margin)."""
    import re
    if not text.startswith('small:'):
        return False
    return all(int(m.group(1)) >= 40 and int(m.group(2)) >= 40 for m in (re.search(r'hit (\d+)x(\d+)', i) for i in text[6:].split(';')) if m) and 'sideways' not in text


def at_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    p = []
    for tab in ('users', 'usage', 'safety', 'gen'):
        goto(d, '#/ai-settings?tab=' + tab, 2.2)
        if not d.js("!!document.getElementById('ais-card') && document.getElementById('ais-tab-%s').getAttribute('aria-pressed') === 'true'" % tab):
            p.append('%s: the tab did not open' % tab)
            continue
        p += ['%s: %s' % (tab, x) for x in common(d, size) if 'ptb-sort' not in x.split(';')[0] and not near_miss(x)]
        if tab == 'users' and p:
            p.append('debug ' + json.dumps(d.js("""(() => { const f = [...document.querySelectorAll('.ais-calls-fig')].map(e => { const b = e.getBoundingClientRect(); return [Math.round(b.left), Math.round(b.right), e.textContent] })
              .sort((a, b) => b[1] - a[1])[0]; const t = document.querySelector('.ais-users'); const tb = t.getBoundingClientRect(); const tr = t.querySelector('tbody tr'); const cs = tr ? getComputedStyle(tr) : {};
              return {fig: f, table: [Math.round(tb.left), Math.round(tb.right)], vw: innerWidth, trDisplay: cs.display, trW: tr ? Math.round(tr.getBoundingClientRect().width) : 0} })()""")))
    verdict(rc, 'W-V13-AT', 'ai-settings', 'tabs', 'admin', lang, theme, size, d, p, 'four tabs open, nothing sticks out')


# ------------------------------------------------------------------------------------------------ Users / Logs
def us_case(rc, lang, theme, size):
    s = rc.session('admin')
    d = rc.dev('admin', lang, theme, size)
    p = []
    try:
        for on in (False, True):
            s.call('/api/users/signin-info', {'enabled': on})
            goto(d, '#/users', 2.2)
            # v1.4: two tabs; the list is the first, the eye and the Clear button are on the sign-in tab
            r = d.js("""(() => ({order: [...document.querySelectorAll('.us-row')].map(r => [...r.querySelectorAll('.us-facts .us-meta')].map(m => m.textContent.trim())),
                facts: document.querySelectorAll('.us-meta').length,
                tabs: [...document.querySelectorAll('.us-tabs button')].map(b => [b.id, b.getAttribute('aria-pressed'), Math.round(b.getBoundingClientRect().height), b.getAttribute('aria-controls')]),
                card: !!document.getElementById('us-card'), eyeHere: !!document.getElementById('us-signin')}))()""")
            if [t[0] for t in r['tabs']] != ['us-tab-users', 'us-tab-signin'] or [t[1] for t in r['tabs']] != ['true', 'false']:
                p.append('the users tab is not the first one shown: %s' % r['tabs'])
            if not r['card'] or r['eyeHere']:
                p.append('the users tab shows the sign-in panel or has no card')
            if phone(size) and any(t[2] < 43.5 for t in r['tabs']):
                p.append('a user management tab is under 44 px high')
            p += ['users tab: ' + x for x in common(d, size)]
            goto(d, '#/users?tab=signin', 2.2)
            r.update(d.js("""(() => { const sw = document.getElementById('us-signin');
              const b = sw ? sw.getBoundingClientRect() : null;
              const cl = document.getElementById('us-signin-clear'); const cb = cl ? cl.getBoundingClientRect() : null;
              const big = !!(b && b.height >= 43.5 && cb && cb.height >= 43.5);
              return {sw: !!sw, on: sw ? (sw.getAttribute('aria-pressed') === 'true') : null, desc: !!document.getElementById('us-signin-d'), title: !!document.getElementById('us-signin-t'),
                st: sw ? (sw.querySelector('.us-eye-txt') || {textContent: ''}).textContent.trim() : '', noHint: !document.querySelector('.us-signin-hint'),
                clear: !!cl, big: big, clearInside: !cb || (cb.left >= 0 && cb.right <= innerWidth),
                inside: !b || (b.left >= 0 && b.right <= innerWidth), listHere: !!document.querySelector('.us-rows'),
                pressed: (document.getElementById('us-tab-signin') || {getAttribute: () => null}).getAttribute('aria-pressed')} })()"""))
            if r['pressed'] != 'true' or r['listHere']:
                p.append('the sign-in tab did not open alone')
            if not r['sw']:
                p.append('eye %s: no sign-in info eye button' % ('open' if on else 'closed'))
            if not r['title'] or not r['desc']:
                p.append('eye %s: the sign-in info title or description is missing' % ('open' if on else 'closed'))
            if not r['noHint']:
                p.append('eye %s: the old hint line is still there' % ('open' if on else 'closed'))
            if not re.search(r'(当前状态|Status|現在の状態)[:：]?\s*\S', r['st'] or ''):
                p.append('eye %s: no visible status text on the eye button (%r)' % ('open' if on else 'closed', r['st']))
            # fact lines (v1.4): last sign-in, [sign-in IP, sign-in place], last active, [active IP, active place, device], records;
            # every date has the full year
            for lines in r['order']:
                kinds = []
                for ln in lines:
                    if re.match(r'(最后登录 IP|最終ログイン IP|Last sign-in IP)', ln): kinds.append('loginIp')
                    elif re.match(r'(登录所在地|ログイン場所|Sign-in location)', ln): kinds.append('loginPlace')
                    elif re.match(r'(最后活动 IP|最終アクティブ IP|Last active IP)', ln): kinds.append('activeIp')
                    elif re.match(r'(活动所在地|アクティブ場所|Active location)', ln): kinds.append('activePlace')
                    elif re.match(r'(最后活动设备|最終アクティブ端末|Last active device)', ln): kinds.append('activeDevice')
                    elif re.match(r'(最后登录|最終ログイン|Last sign-in|从未登录|ログイン履歴なし|Never signed in)', ln): kinds.append('lastLogin')
                    elif re.match(r'(最后活动|最終アクティブ|Last active|从未活动|アクティブ履歴なし|Never active)', ln): kinds.append('lastActive')
                    else: kinds.append('records')
                    if kinds[-1] in ('lastLogin', 'lastActive') and re.search(r'\d', ln) and not re.search(r'\b20\d\d-\d\d-\d\d \d\d:\d\d', ln):
                        p.append('a date without the full year: %s' % ln)
                active = 'lastActive' in kinds and not any(re.match(r'(从未活动|アクティブ履歴なし|Never active)', x) for x in lines)
                want = ['lastLogin'] + (['loginIp', 'loginPlace'] if on and 'loginIp' in kinds else []) + ['lastActive']
                want += (['activeIp', 'activePlace'] if on and 'activeIp' in kinds else []) + (['activeDevice'] if 'activeDevice' in kinds and active else []) + ['records']
                if kinds != want:
                    p.append('fact lines out of order: %s' % kinds)
                    break
            if not r['clear']:
                p.append('eye %s: no red Clear records button' % ('open' if on else 'closed'))
            elif not r['clearInside']:
                p.append('the Clear records button is outside the screen')
            if r['sw'] and r['on'] != on:
                p.append('the eye shows %s, the server has %s' % (r['on'], on))
            if not r['inside']:
                p.append('the eye button is outside the screen')
            if not r['big']:
                p.append('the eye or Clear records button is under 44 px high')
            if not r['facts']:
                p.append('no last-active line on the rows')
            p += ['%s: %s' % ('on' if on else 'off', x) for x in common(d, size)]
    finally:
        s.call('/api/users/signin-info', {'enabled': False})
    verdict(rc, 'W-V13-US', 'users', 'signin-info', 'admin', lang, theme, size, d, p, 'eye closed / open, last-active lines, nothing sticks out')


def lg_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/logs', 2.4)
    p = []
    r = d.js("""(() => ({rows: document.querySelectorAll('.act-list > li').length, tabs: document.querySelectorAll('.lg-tabs button').length,
      tools: !!document.querySelector('.act-tools')}))()""")
    if r['tabs'] != 2:
        p.append('%d log tabs (want 2)' % r['tabs'])
    if not r['rows']:
        p.append('no activity rows')
    p += common(d, size)
    # open the first row, then the settings popover (a button inside the tools)
    d.js("(() => { const b = document.querySelector('.act-main'); if (b) b.click(); return 1 })()")
    time.sleep(0.5)
    p += ['row open: ' + x for x in common(d, size)]
    d.js("(() => { const b = document.querySelector('.act-tools button[aria-expanded]'); if (b) b.click(); return 1 })()")
    time.sleep(0.5)
    if d.js("!!document.querySelector('.act-pop')"):
        p += ['settings: ' + x for x in common(d, size)]
        if not d.js("(() => { const b = document.querySelector('.act-pop').getBoundingClientRect(); return b.left >= 0 && b.right <= innerWidth + 0.5 })()"):
            p.append('the settings popover sticks out of the screen')
    # the Service tab
    d.js("(() => { const b = [...document.querySelectorAll('.lg-tabs button')][1]; if (b) b.click(); return 1 })()")
    time.sleep(1.0)
    p += ['service: ' + x for x in common(d, size)]
    verdict(rc, 'W-V13-LG', 'logs', 'activity', 'admin', lang, theme, size, d, p, '%d activity rows, popover, service tab, nothing sticks out' % r['rows'])


# ------------------------------------------------------------------------------------------------ Sign-in / Result
def si_case(rc, lang, theme, size):
    d = rc.dev(None, lang, theme, size)
    goto(d, '#/', 1.8)
    p = []
    r = d.js("""(() => { const c = document.getElementById('auth-remember'); const l = document.querySelector('.auth-remember-l'); const h = document.getElementById('auth-remember-h');
      if (!c || !l || !h) return null; const lr = l.getBoundingClientRect(), hr = h.getBoundingClientRect();
      return {checked: c.checked, hint: (h.textContent || '').trim(), hit: Math.max(lr.height, c.getBoundingClientRect().height), inside: lr.right <= innerWidth && hr.right <= innerWidth + 0.5,
        order: hr.top >= lr.bottom - 1} })()""")
    if not r:
        rc.rec('W-V13-SI', 'signin', 'remember', None, size, lang, theme, None, 'not the sign-in page')
        return
    if not r['hint']:
        p.append('the hint is empty')
    if not r['inside']:
        p.append('the checkbox row or its hint sticks out')
    if not r['order']:
        p.append('the hint is not under the checkbox')
    if phone(size) and r['hit'] < 44:
        p.append('the checkbox row is %d px high' % r['hit'])
    p += common(d, size)
    verdict(rc, 'W-V13-SI', 'signin', 'remember', None, lang, theme, size, d, p, 'keep-signed-in checkbox with its hint, row %d px high' % r['hit'])


FAIL_ATTEMPT = """(() => { if (window.__failAttempt) return 1; window.__failAttempt = true; const real = window.fetch.bind(window);
  window.fetch = async (u, init) => { if (String(u).includes('/api/attempt')) return new Response(JSON.stringify({code: 'net.failed'}), {status: 503, headers: {'Content-Type': 'application/json'}});
    return real(u, init) }; return 1 })()"""


def rs_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/', 0.8)
    script = d.cdp.call('Page.addScriptToEvaluateOnNewDocument', source=FAIL_ATTEMPT)    # survives the reload the quiz start does
    try:
        got = make_result(d, 5)
    finally:
        d.cdp.call('Page.removeScriptToEvaluateOnNewDocument', identifier=(script or {}).get('identifier'))
    p = []
    r = d.js("""(() => { const b = document.querySelector('.res-unsaved'); if (!b) return null; const btn = b.querySelector('button'); const br = b.getBoundingClientRect();
      return {inside: br.left >= 0 && br.right <= innerWidth + 0.5, btn: !!btn, h: btn ? btn.getBoundingClientRect().height : 0} })()""")
    if not r:
        rc.rec('W-V13-RS', 'result', 'unsaved', 'sam', size, lang, theme, None, 'no result page / banner (%s)' % got, d)
        return
    if not r['btn']:
        p.append('no retry button')
    if not r['inside']:
        p.append('the banner sticks out of the screen')
    if phone(size) and r['btn'] and r['h'] < 44:
        p.append('the retry button is %d px high' % r['h'])
    p += common(d, size)
    verdict(rc, 'W-V13-RS', 'result', 'unsaved', 'sam', lang, theme, size, d, p, 'retry-save banner with a %d px button' % r['h'])
    d.reload(1.0)


def g_v13(rc):
    for cid, fn in (('W-V13-CV', cv_case), ('W-V13-ED', ed_case), ('W-V13-MM', mm_case), ('W-V13-AC', ac_case), ('W-V13-AT', at_case),
                    ('W-V13-US', us_case), ('W-V13-LG', lg_case), ('W-V13-SI', si_case), ('W-V13-RS', rs_case)):
        cases(rc, cid, fn)


GROUPS_V13 = [g_v13]
