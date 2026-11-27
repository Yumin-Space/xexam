# -*- coding: utf-8 -*-
"""v1.2 checks of AI Chat on phones.

W-CH-82 (A-OWNER-82): the history list and memory screens are fixed frames. With the owner's 5 chats and the GitHub line the
list body has nothing to scroll; a drag in any direction (up, down, sideways, diagonal) on the empty area under the rows, the
rows, the GitHub line, the list header, the title row, the top bar and the memory pane is cancelled; a long list still
scrolls (up / down) and never sideways; the conversation keeps the v1.1 rule (a vertical drag on its head is cancelled, a
sideways one is left to the browser). Real CDP touch swipes move no frame, no document scroll and no visual viewport.

W-CH-83 (A-OWNER-83): the AI Chat header (v2.3 phone: menu button, title, free-mode tag, new chat, more; wider: title row) and the top bar (brand, account label,
menu button) never overlap or spill, in zh / en / ja, for an administrator with and without free mode and for a user, at
text zoom 1, 1.25 and 1.5 (iOS text size).

W-CH-88 (A-OWNER-88): an open conversation does not move when the keyboard opens, while typing (the composer grows) or when
it closes, scrolled to the end, the middle or the top: every message keeps its on-screen y (within 1 px), the list keeps its
scrollTop and top edge and only gets shorter from the bottom. With the keyboard open the list still scrolls to its very end,
where the last message sits right above the composer; a message sent with the keyboard open scrolls to the end. Both the
iOS layout (the window keeps its height; --vvh / --kb / html.kb-open as utils/dom.ts sets them) and a real viewport shrink
(Device.keyboard).
"""
import time

from checks import CHECKS_JS, LANGS, ai_send, goto, open_memories, open_sidebar, until  # noqa: F401

SIZES = [(320, 640), (360, 740), (375, 667), (390, 844), (393, 852), (402, 874), (430, 932)]
GUARD_SIZES = [(320, 640), (375, 667), (393, 852), (430, 932)]

# Keeps n rows in the history list (clones of the first one when there are fewer) and hides every other item of the list body
# except the rows and the GitHub line
ROWS = """(n => { const ul = document.querySelector('.ai-chats'); if (!ul || !ul.firstElementChild) return -1;
  const rows = Array.from(ul.children); rows.slice(n).forEach(r => r.remove());
  for (let i = ul.children.length; i < n; i++) ul.appendChild(rows[0].cloneNode(true));
  document.querySelectorAll('.ai-list-body > :not(.ai-chats-wrap):not(.gh-line)').forEach(e => { e.style.display = 'none' });
  return ul.children.length })"""

# A synthetic, cancelable one-finger drag: touchstart and three moves of (dx, dy) each at a point of `sel` ('top', 'mid' or
# 'below' = the empty area between the last row and the GitHub line). Answers whether any move was cancelled.
DRAG = """((sel, at, dx, dy) => { const e = document.querySelector(sel); if (!e || !e.getClientRects().length) return null;
  const b = e.getBoundingClientRect(); let x = b.left + b.width / 2, y = b.top + Math.min(b.height / 2, 30);
  if (at === 'mid') y = (Math.max(b.top, 0) + Math.min(b.bottom, innerHeight * 0.7)) / 2;
  if (at === 'below') { const ul = document.querySelector('.ai-chats').getBoundingClientRect();
    const gh = document.querySelector('.ai-list-body > .gh-line'); y = (ul.bottom + (gh ? gh.getBoundingClientRect().top : b.bottom)) / 2; }
  const target = document.elementFromPoint(x, y); if (!target) return null;
  const mk = (type, xx, yy) => { const t = new Touch({ identifier: 9, target, clientX: xx, clientY: yy, pageX: xx, pageY: yy });
    return new TouchEvent(type, { touches: type === 'touchend' ? [] : [t], targetTouches: type === 'touchend' ? [] : [t], changedTouches: [t], bubbles: true, cancelable: true }); };
  target.dispatchEvent(mk('touchstart', x, y)); let any = false;
  for (let i = 1; i <= 3; i++) { const ev = mk('touchmove', x + dx * i, y + dy * i); target.dispatchEvent(ev); any = any || ev.defaultPrevented; }
  target.dispatchEvent(mk('touchend', x + dx * 3, y + dy * 3));
  return any })"""

STATE = """(() => { const r = s => { const e = document.querySelector(s); if (!e || !e.getClientRects().length) return null;
    const b = e.getBoundingClientRect(); return [Math.round(b.top * 10) / 10, Math.round(b.left * 10) / 10, Math.round(b.width * 10) / 10]; };
  const vv = window.visualViewport || {}; const lb = document.querySelector('.ai-list-body');
  return { frame: ['.topbar', '.ai-head', '.ai-list-head', '.ai-list', '.ai-conv-head', '.ai-composer', '.ai-mem'].map(r),
    win: [scrollX, scrollY, document.scrollingElement.scrollTop, document.scrollingElement.scrollLeft, vv.offsetTop || 0, vv.offsetLeft || 0],
    st: lb ? lb.scrollTop : -1, sl: lb ? lb.scrollLeft : -1, over: lb ? lb.scrollHeight - lb.clientHeight : -1 } })()"""

DIRS = (('up', 0, -20), ('down', 0, 20), ('left', -20, 0), ('right', 20, 0), ('diag', 14, -14), ('diag2', -14, 14))


def touch_swipe(d, x, y, dx, dy, steps=8):
    """A real CDP touch swipe from (x, y) by (dx, dy)."""
    def pt(i):
        return [{'x': x + dx * i / steps, 'y': y + dy * i / steps, 'radiusX': 6, 'radiusY': 6, 'force': 1, 'id': 1}]
    d.cdp.call('Input.dispatchTouchEvent', type='touchStart', touchPoints=pt(0))
    for i in range(1, steps + 1):
        d.cdp.call('Input.dispatchTouchEvent', type='touchMove', touchPoints=pt(i))
        time.sleep(0.016)
    d.cdp.call('Input.dispatchTouchEvent', type='touchEnd', touchPoints=[])
    time.sleep(0.45)


# The AI Chat page has rendered (a busy machine needs longer than the fixed settle time) / the drawer is open and shows rows
READY = "document.querySelector('.page-ai .ai-conv-head') && (innerWidth >= 720 || document.getElementById('ai-drawer-btn'))"
OPEN = """(() => { const a = document.querySelector('aside.ai-list'); const b = document.getElementById('ai-drawer-btn');
  return !!a && a.getClientRects().length > 0 && !!document.querySelector('.ai-chats > li') && (!b || !b.getClientRects().length || b.getAttribute('aria-expanded') === 'true') })()"""


def open_list(d, settle):
    """#/ai with the history list open (the phone drawer), waiting for the page instead of trusting a fixed delay."""
    goto(d, '#/ai', settle)
    until(d, READY, 10)
    open_sidebar(d)
    until(d, OPEN, 5)


def fixed_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    rc.chat_id('sam')
    problems = []
    open_list(d, 2.0)     # v2.3: the history list is the phone drawer
    phone_drawer = d.js("!!document.getElementById('ai-drawer-btn') && document.getElementById('ai-drawer-btn').getClientRects().length > 0")
    if d.js_call(ROWS, 5) != 5:
        rc.rec('W-CH-82', 'ai', 'fixed-frame', 'sam', size, lang, theme, False, 'no history list on screen', d)
        return
    time.sleep(0.3)
    st = d.js(STATE)
    gh = d.js("(() => { const e = document.querySelector('.ai-list-body > .gh-line'); return !!e && e.offsetParent !== null })()")
    if not gh:
        problems.append('the GitHub line is not in the list')
    if st['over'] > 0:
        problems.append('5 rows: the list body has %s px to scroll' % st['over'])
    spots = (('.ai-list-body', 'below', 'empty area'), ('.ai-chats', 'top', 'rows'), ('.ai-list-body > .gh-line', 'top', 'GitHub line'),
             ('.ai-list-head', 'top', 'list header'), ('.ai-head', 'top', 'title row'), ('.topbar', 'top', 'top bar'))
    for sel, at, name in spots:
        for dn, dx, dy in DIRS:
            res = d.js_call(DRAG, sel, at, dx, dy)
            if res is None and name in ('list header', 'title row'):
                break       # v2.3: no such element on a phone any more (the drawer has no head row, the title row is gone)
            if res is not True:
                problems.append('%s: %s drag not cancelled' % (name, dn))
    # Real touch swipes on the owner's spot: nothing moves
    before = d.js(STATE)
    p = d.js("""(() => { const ul = document.querySelector('.ai-chats').getBoundingClientRect(); const gh = document.querySelector('.ai-list-body > .gh-line').getBoundingClientRect();
      return [innerWidth / 2, (ul.bottom + gh.top) / 2] })()""")
    for dn, dx, dy in DIRS:
        if dn == 'left' and phone_drawer:
            continue        # v2.3: a left swipe of the phone drawer closes it (checked below)
        touch_swipe(d, p[0], p[1], dx * 6, dy * 6)
    after = d.js(STATE)
    for k in ('frame', 'win', 'st', 'sl'):
        if before[k] != after[k]:
            problems.append('swipes on the empty area moved %s (%s -> %s)' % (k, before[k], after[k]))
    if phone_drawer:
        touch_swipe(d, p[0], p[1], -120, 0)
        time.sleep(0.8)
        if d.js("document.getElementById('ai-drawer-btn').getAttribute('aria-expanded')") != 'false':
            problems.append('a left swipe did not close the drawer')
    # A long list scrolls up / down, never sideways
    open_list(d, 1.5)
    d.js_call(ROWS, 40)
    time.sleep(0.3)
    d.js("document.querySelector('.ai-list-body').scrollTop = 40")
    for dn, dx, dy in (('up', 0, -20), ('down', 0, 20), ('diag', 6, -20)):
        if d.js_call(DRAG, '.ai-chats', 'mid', dx, dy) is not False:
            problems.append('long list: %s drag cancelled' % dn)
    for dn, dx, dy in (('left', -20, 0), ('right', 20, 0)):
        if d.js_call(DRAG, '.ai-chats', 'mid', dx, dy) is not True:
            problems.append('long list: %s drag not cancelled' % dn)
    before = d.js(STATE)
    r = d.rect('.ai-list-body', 0, scroll=False)
    touch_swipe(d, r['cx'], r['cy'], 0, -200)
    mid = d.js(STATE)
    touch_swipe(d, r['cx'], r['cy'], 160 if phone_drawer else -160, 4)     # v2.3: a left swipe closes the drawer, so it swipes right
    after = d.js(STATE)
    if mid['st'] <= before['st']:
        problems.append('long list did not scroll (%s -> %s)' % (before['st'], mid['st']))
    if before['frame'] != after['frame'] or before['win'] != after['win'] or after['sl'] != 0:
        problems.append('long list swipes moved the frame or went sideways')
    # Memory: a fixed frame too
    goto(d, '#/ai', 1.5)
    until(d, READY, 10)
    open_memories(d, 0.6)
    until(d, "document.querySelector('.ai-mem')", 5)
    for sel, at, name in (('.ai-mem', 'mid', 'memory pane'), ('.ai-head', 'top', 'memory title row')):
        for dn, dx, dy in DIRS:
            res = d.js_call(DRAG, sel, at, dx, dy)
            follow = d.js("(() => { const e = document.querySelector('.ai-mem'); return !!e && e.scrollHeight - e.clientHeight >= 4 })()")
            if res is None:
                if name != 'memory pane':
                    break       # v2.3: the memory header is the chat-style header row, not the old title row
                problems.append('%s: not on screen' % name)
                break
            if res is not True and not (follow and dn in ('up', 'down', 'diag', 'diag2') and name == 'memory pane'):
                problems.append('%s: %s drag not cancelled' % (name, dn))
    # The conversation: the v1.1 rule, unchanged (A-OWNER-82 constraint)
    goto(d, '#/ai/%d' % rc.chat_id('sam'), 2.2)
    until(d, "document.querySelector('.page-ai .ai-conv-head') && document.querySelector('.ai-scroll .ai-msg')", 10)
    for dn, dx, dy, want in (('up', 0, -20, True), ('down', 0, 20, True), ('left', -20, 0, False), ('right', 20, 0, False)):
        if d.js_call(DRAG, '.ai-conv-head', 'top', dx, dy) is not want:
            problems.append('conversation head: %s drag %s (v1.1: %s)' % (dn, 'cancelled' if not want else 'left alone', 'left alone' if not want else 'cancelled'))
    rc.rec('W-CH-82', 'ai', 'fixed-frame', 'sam', size, lang, theme, not problems,
           '; '.join(problems) or '5 rows + GitHub line: 0 px to scroll, every drag on the frame cancelled; long list scrolls; conversation v1.1',
           d if problems else None)


# Every visible part of the title row and the top bar inside its row, no two overlapping, no label cut
HEAD_JS = """(() => { const vis = e => e && e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden';
  const R = e => e.getBoundingClientRect();
  const hit = (a, b) => a.left < b.right - 0.5 && b.left < a.right - 0.5 && a.top < b.bottom - 0.5 && b.top < a.bottom - 0.5;
  const bad = [];
  const pairs = (row, parts, tag) => { const rr = R(row); for (let i = 0; i < parts.length; i++) { const a = R(parts[i]);
    if (a.left < rr.left - 0.5 || a.right > rr.right + 0.5) bad.push(tag + ' outside: ' + (parts[i].className || parts[i].tagName));
    for (let j = i + 1; j < parts.length; j++) if (hit(a, R(parts[j]))) bad.push(tag + ' overlap: ' + (parts[i].className || parts[i].tagName) + ' / ' + (parts[j].className || parts[j].tagName)); } };
  // v2.3: a phone has the chat header row (menu button, title, free-mode tag, new chat, more); wider screens keep the title row
  const phoneHead = document.querySelector('.page-ai .ai-conv-head');
  const head = innerWidth < 720 && vis(phoneHead) ? phoneHead : document.querySelector('.page-ai .ai-head');
  if (vis(head)) {
    const isChat = head === phoneHead;
    const parts = isChat ? ['#ai-drawer-btn', '.ai-conv-title, #ai-conv-t, h2, h1', '.ai-mode-tag', '#ai-new', '#ai-chat-more'].map(q => head.querySelector(q)).filter(vis)
      : [head.querySelector('.ai-title h1'), head.querySelector('.ai-mode-tag')].filter(vis);
    pairs(head, parts, 'title row');
    for (const e of [...head.querySelectorAll('.ai-mode-tag > [aria-hidden]')].filter(vis))
      if (R(e).right > R(e.closest('.ai-mode-tag')).right + 0.5) bad.push('title row: cut ' + e.className);
  }
  const bar = document.querySelector('.topbar');
  if (vis(bar)) {
    pairs(bar, [bar.querySelector('.top-brand'), bar.querySelector('.acct-btn'), bar.querySelector('.menu-wrap > .menu-btn')].filter(vis), 'top bar');
    const ab = bar.querySelector('.acct-btn');
    if (ab) for (const k of ab.children) if (vis(k) && (R(k).right > R(ab).right + 0.5 || R(k).left < R(ab).left - 0.5)) bad.push('top bar: spills ' + k.className);
  }
  return { bad, head: vis(head) ? head.className : '', bar: vis(bar) ? bar.className : '', docX: document.documentElement.scrollWidth - innerWidth } })()"""


def head_case(rc, lang, theme, size, user, free):
    label = '%s%s' % (user, '+free' if free else '')
    rc.chat_id(user)
    d = rc.dev(user, lang, theme, size)
    goto(d, '#/ai', 2.0)
    # a busy machine can still show "Loading..." after the fixed settle time: wait for the header row itself
    until(d, "document.querySelector('.page-ai .ai-conv-head, .page-ai .ai-head') && document.querySelector('.topbar')", 10)
    bad = []
    seen = set()
    for zoom in (1, 1.25, 1.5):
        d.js("document.body.style.zoom = '%s'; dispatchEvent(new Event('resize')); 1" % zoom)
        time.sleep(0.35)
        r = d.js(HEAD_JS)
        if not r['head']:
            bad.append('z%s: no title row on screen' % zoom)
        bad += ['z%s %s' % (zoom, b) for b in r['bad']]
        if r['docX'] > 0:
            bad.append('z%s: the page scrolls sideways (%s px)' % (zoom, r['docX']))
        seen.add('z%s:%s|%s' % (zoom, ' '.join(c for c in r['head'].split() if c.startswith('fit-')) or '-', ' '.join(c for c in r['bar'].split() if c.startswith('fit-')) or '-'))
    d.js("document.body.style.zoom = ''; dispatchEvent(new Event('resize')); 1")
    rc.rec('W-CH-83', 'ai', 'head-' + label, user, size, lang, theme, not bad, '; '.join(bad) or ' '.join(sorted(seen)), d if bad else None)


def g_fixed(rc):
    """W-CH-82: the history list and memory screens never move; lists scroll; the conversation keeps its v1.1 rule."""
    if not rc.want('W-CH-82') or not rc.walk.wants_page('ai'):
        return
    for theme in rc.themes(['dark']):
        for size in rc.sizes(GUARD_SIZES, ok=lambda s: s[0] < 720 and s[1] > s[0]):
            rc.guard('W-CH-82', 'ai', fixed_case, rc, rc.langs(['zh'])[0], theme, size)


def g_head(rc):
    """W-CH-83: the AI Chat title row and the top bar never overlap (zh / en / ja, text zoom 1-1.5)."""
    if not rc.want('W-CH-83') or not rc.walk.wants_page('ai'):
        return
    s = rc.session('admin')
    try:
        for user, free in (('admin', True), ('admin', False), ('sam', False)):
            if user == 'admin':
                s.call('/api/ai/free', {'free': free})
            for lang in rc.langs(LANGS):
                for size in rc.sizes(SIZES, ok=lambda z: z[0] < 720 and z[1] > z[0]):
                    rc.guard('W-CH-83', 'ai', head_case, rc, lang, rc.themes(['light'])[0], size, user, free)
    finally:
        s.call('/api/ai/free', {'free': False})


KB_SNAP = """(() => { const sc = document.getElementById('ai-scroll'); const c = document.getElementById('ai-composer');
  if (!sc || !sc.getClientRects().length || !c) return null; const ms = [...sc.querySelectorAll('.ai-msg')];
  const last = ms[ms.length - 1];
  return { ys: ms.map(m => m.getBoundingClientRect().top), st: sc.scrollTop, max: sc.scrollHeight - sc.clientHeight, ch: sc.clientHeight,
    top: sc.getBoundingClientRect().top, lastB: last ? last.getBoundingClientRect().bottom : null, compTop: c.getBoundingClientRect().top,
    winY: scrollY, bar: document.querySelector('.topbar').getBoundingClientRect().top } })()"""
KB_SCROLL = """(w => { const sc = document.getElementById('ai-scroll');
  sc.scrollTop = w === 'end' ? sc.scrollHeight : w === 'mid' ? Math.round((sc.scrollHeight - sc.clientHeight) / 2) : 0; return 1 })"""
# The iOS keyboard as utils/dom.ts sees it: the window keeps its height, the visual viewport loses `kb` px
KB_IOS = """(kb => { const r = document.documentElement; r.style.setProperty('--vvh', (innerHeight - kb) + 'px');
  r.style.setProperty('--kb', kb + 'px'); r.classList.toggle('kb-open', kb > 0); return 1 })"""
KB_FOCUS = "(() => { document.getElementById('ai-input').focus({ preventScroll: true }); return 1 })()"
KB_BLUR = "(() => { document.getElementById('ai-input').blur(); return 1 })()"
KB_CLEAR = "(() => { const t = document.getElementById('ai-input'); t.value = ''; t.dispatchEvent(new Event('input')); return 1 })()"


def kb_moved(a, b):
    """The largest change of a message's on-screen y between two snapshots (px)."""
    if not a or not b or len(a['ys']) != len(b['ys']) or not a['ys']:
        return 999.0
    return max(abs(x - y) for x, y in zip(a['ys'], b['ys']))


def kb_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    cid = rc.chat_id('sam')
    if 'kb-long-chat' not in rc.cache:
        # the chat must overflow a tall phone screen: two more made-up exchanges (once per run)
        rc.cache['kb-long-chat'] = True
        for text in ('Please list three more tips about knife care.', 'And three tips about pan care, in short lines.'):
            try:
                ai_send(rc.walk.base, rc.walk.stack.cookie('sam'), text, 'zh', cid)
            except Exception:
                break
    goto(d, '#/ai/%d' % cid, 2.2)
    kb = round(size[1] * 0.4)
    problems, notes = [], []
    first = d.js(KB_SNAP)
    if not first or first['max'] < 40:
        rc.rec('W-CH-88', 'ai', 'keyboard-still', 'sam', size, lang, theme, False, 'no conversation that scrolls on screen', d)
        return
    for mode in ('ios', 'resize'):
        for where in ('end', 'mid', 'top'):
            d.js_call(KB_SCROLL, where)
            time.sleep(0.3)
            before = d.js(KB_SNAP)
            d.js(KB_FOCUS)
            if mode == 'ios':
                d.js_call(KB_IOS, kb)
            else:
                d.keyboard(True)
            time.sleep(0.5)
            opened = d.js(KB_SNAP)
            d.type('Made-up draft line 1\nMade-up draft line 2\nMade-up draft line 3')
            time.sleep(0.4)
            typed = d.js(KB_SNAP)
            d.js(KB_CLEAR)
            d.js(KB_BLUR)
            if mode == 'ios':
                d.js_call(KB_IOS, 0)
            else:
                d.keyboard(False)
            time.sleep(0.5)
            closed = d.js(KB_SNAP)
            for step, snap in (('open', opened), ('typing', typed), ('closed', closed)):
                m = kb_moved(before, snap)
                if m > 1.01:
                    problems.append('%s %s %s: messages moved %.1f px' % (mode, where, step, m))
                if snap and (snap['winY'] != 0 or abs(snap['bar'] - before['bar']) > 1.01 or abs(snap['top'] - before['top']) > 1.01):
                    problems.append('%s %s %s: the frame moved (scrollY %s, list top %.1f -> %.1f)' % (mode, where, step, snap['winY'], before['top'], snap['top']))
            notes.append('%s %s %d->%d->%dpx' % (mode, where, before['ch'], opened['ch'] if opened else -1, typed['ch'] if typed else -1))
    # Keyboard open: the list scrolls to its very end, the last message right above the composer
    d.js(KB_FOCUS)
    d.js_call(KB_IOS, kb)
    time.sleep(0.4)
    d.js_call(KB_SCROLL, 'top')
    time.sleep(0.2)
    d.js_call(KB_SCROLL, 'end')
    time.sleep(0.3)
    e = d.js(KB_SNAP)
    if not e or e['lastB'] is None or e['lastB'] > e['compTop'] + 0.5:
        problems.append('keyboard open, at the end: last message bottom %s > composer top %s' % (e and e['lastB'], e and e['compTop']))
    # Send with the keyboard open: the conversation follows the end
    d.js_call(KB_SCROLL, 'top')
    time.sleep(0.3)
    n0 = len(d.js(KB_SNAP)['ys'])
    d.type('Made-up follow-up question.')
    d.tap_sel('#ai-send', wait=0.6)
    until(d, "document.querySelectorAll('#ai-scroll .ai-msg').length >= %d && !document.querySelector('#ai-send.stop')" % (n0 + 2), 30)
    time.sleep(0.5)
    f = d.js(KB_SNAP)
    if len(f['ys']) < n0 + 2 or f['max'] - f['st'] > 2:
        problems.append('send with the keyboard open: %d new messages, %.1f px from the end' % (len(f['ys']) - n0, f['max'] - f['st']))
    elif f['lastB'] > f['compTop'] + 0.5:
        problems.append('after the send the last message is under the composer (%.1f > %.1f)' % (f['lastB'], f['compTop']))
    d.js(KB_BLUR)
    d.js_call(KB_IOS, 0)
    d.js("(() => { document.documentElement.style.removeProperty('--kb'); return 1 })()")
    gap = (e['compTop'] - e['lastB']) if e and e['lastB'] is not None else 0
    rc.rec('W-CH-88', 'ai', 'keyboard-still', 'sam', size, lang, theme, not problems,
           '; '.join(problems) or 'still within 1 px; end above the composer (gap %.1f); send follows; %s' % (gap, ', '.join(notes[:1])),
           d if problems else None)


def g_kb(rc):
    """W-CH-88: the conversation never moves when the keyboard opens, while typing or when it closes."""
    if not rc.want('W-CH-88') or not rc.walk.wants_page('ai'):
        return
    for size in rc.sizes([(393, 852), (360, 740)], ok=lambda z: z[0] < 720 and z[1] > z[0]):
        rc.guard('W-CH-88', 'ai', kb_case, rc, rc.langs(['zh'])[0], rc.themes(['dark'])[0], size)


GROUPS_V12_AI = [g_fixed, g_head, g_kb]
