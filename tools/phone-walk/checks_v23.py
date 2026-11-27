# -*- coding: utf-8 -*-
"""v2.3 walk checks: AI Chat laid out like a chat app (the history is a drawer on phones and a sidebar on tablets / desktops, the
page always opens a new chat). Every case takes screenshots into the output folder (rc.rec with the device), passed or not.

  W-V23-EN   entering AI Chat (#/ai) opens a new chat: the greeting and the composer are on screen, the address is #/ai/new, no
             history list is on screen on a phone (a sidebar is on a tablet / desktop)
  W-V23-DR   phone drawer: the menu button opens it (width about min(320, 82 % of the screen), inside the screen, full height,
             dialog semantics, focus inside, backdrop beside it); a backdrop tap, Esc and the back button close it (the back
             button stays on AI Chat) and the focus returns to the menu button
  W-V23-SE   the search box filters the history live (case-insensitive), a made-up word leaves no row, clearing restores all
  W-V23-MEM  the memory row of the sidebar opens the memories view (the drawer closes on a phone), the back control returns to
             the conversation
  W-V23-HD   phone chat header row (new chat and an open chat, administrator in free mode too): menu button, title, free-mode
             tag, new-chat and more buttons fit at 320 px without overlap or overflow, 44 px targets, the row is 44 px high
  W-V23-SB   tablet / desktop sidebar: always visible, no menu button, search / new chat / memory row, grouped history with
             headings and a row menu button, the width follows the layout (240 / 272 px)

  W-V23-DL   (fix round) delete from the phone drawer: the confirm dialog keeps Tab and Esc (the drawer does not take them), deleting
             the open chat or all chats ends on a clean new chat whose address stays #/ai/new after the drawer closes (and after
             a reload), never an error for the deleted chat; the red menu items have no shadow
  W-V23-GONE a chat id that no longer exists in the address opens a real new-chat draft (#/ai/new) with no error
  W-V23-RO   a paused (read-only) user does not land on an empty new chat: the latest chat opens, no new-chat button
  W-V23-FW   closing the drawer leaves no dead forward entry (Forward twice, then Back, leaves AI Chat)
  W-V23-KB   the drawer's bottom follows the keyboard height (--kb) and the drawer stays inside the screen with the keyboard up
  W-V23-LS   short landscape phones (under 720 px wide): the whole drawer content scrolls as one and reaches Delete all
  W-V23-LT   "Jump to latest" scrolls to the newest message and never focuses the input (no keyboard on a phone)
  W-V23-CTR  an EMPTY new chat has the greeting, the composer and the reply-language hint as one group centred in the visible area
             under the chat head (the composer's centre in the middle third, below the greeting, above the hint, no horizontal
             line above it, same background as the conversation); with the keyboard emulated (--vvh / --kb) the composer stays
             above it; an open chat with messages has the composer back at the bottom
  W-V23-CMP  the composer box is ONE compact pill row when empty (brain, input, send on one line, fully rounded); a long line
             switches it to the expanded layout (the input on top at the full width, the controls in a row under it, a large
             rounded rectangle) with the focus kept; many lines grow the input up to 8 lines / 40 % of the visible height and
             then it scrolls inside; clearing the text returns to the compact row
  W-V23-NT   the administrator notice is under the composer (and in the memory view) but not in the drawer, and it fits ONE line at
             375 / 393 / 430 px; over 900 px it is the one line under the page title instead (never both)

Made-up accounts and data only; the screenshots stay in the output folder. Registered in checks.py (GROUPS_V23); run alone with
--only 'W-V23-*'.
"""
import re
import time

from checks import ai_send, goto, open_sidebar, until
from checks_v22 import DESKTOP, PHONES, go, js, skip

TABLET = (768, 1024)
LANDS = [(568, 320), (667, 375)]

GREETING_RE = {'zh': re.compile(r'有什么可以帮你'), 'en': re.compile(r'How can I help'), 'ja': re.compile(r'何をお手伝い')}
GROUP_RE = re.compile(r'今天|昨天|7 天内|更早|Today|Yesterday|Previous 7 days|Older|今日|昨日|過去 7 日間|それ以前')
BACK_RE = re.compile(r'返回对话|Back to chat|チャットに戻る')

# Shared in-page helpers for this file (after the v22 ones): the drawer element, the sidebar element, the open state
V23 = """
const drawerBtn = () => document.getElementById('ai-drawer-btn');
const sidebar = () => { const b = drawerBtn(); const id = b && b.getAttribute('aria-controls'); return (id && document.getElementById(id)) || document.querySelector('aside.ai-list'); };
const isOpen = () => { const b = drawerBtn(); return !!b && b.getAttribute('aria-expanded') === 'true'; };
"""


def jv(d, body):
    return js(d, V23 + body)


def tap(d, sel, wait=0.6):
    d.tap_sel(sel, wait=wait)


def verdict(rc, cid, page, state, user, size, lang, theme, d, problems, ok_text):
    rc.rec(cid, page, state, user, size, lang, theme, not problems, '; '.join(problems) or ok_text, d)


# ------------------------------------------------------------------------------------------------ W-V23-EN
EN_JS = """const ta = document.getElementById('ai-input'); const side = sidebar(); const sb = side && vis(side) ? box(side) : null;
  const hits = [...document.querySelectorAll('.page-ai *')].filter((e) => vis(e) && e.children.length === 0 && e.textContent.trim().length < 80);
  return {hash: location.hash, composer: !!ta && vis(ta), texts: hits.map((e) => e.textContent.trim()),
    sideShown: !!sb && sb.r > 1 && sb.l < innerWidth - 1, sideLeft: sb ? sb.l : null, btn: vis(drawerBtn()),
    rows: [...document.querySelectorAll('.ai-chat-open')].filter((e) => vis(e) && box(e).r > 1 && box(e).l < innerWidth).length};"""


def en_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    rc.chat_id('sam')           # the user has at least one chat: the old phone flow would have shown the list first
    goto(d, '#/ai', 2.4)
    r = jv(d, EN_JS)
    p = []
    wide = size[0] >= 720
    if not r['hash'].startswith('#/ai/new'):
        p.append('the address is %s (want #/ai/new)' % r['hash'])
    if not r['composer']:
        p.append('no composer on screen')
    if not any(GREETING_RE[lang].search(t) for t in r['texts']):
        p.append('the greeting is not on screen')
    if wide:
        if not r['sideShown']:
            p.append('no sidebar on screen')
        if r['btn']:
            p.append('a menu button is shown beside the always-visible sidebar')
    else:
        if r['sideShown'] or r['rows']:
            p.append('the history list is on screen on a phone (%d rows)' % r['rows'])
        if not r['btn']:
            p.append('no menu button')
    verdict(rc, 'W-V23-EN', 'ai', 'new-chat', 'sam', size, lang, theme, d, p,
            'new chat with greeting and composer; %s' % ('sidebar on screen' if wide else 'no list on screen'))


# ------------------------------------------------------------------------------------------------ W-V23-DR
DR_JS = """const side = sidebar(); const b = drawerBtn(); if (!side || !b) return null;
  const sb = box(side); const open = isOpen();
  const backs = [...document.querySelectorAll('[class*="backdrop"], [class*="scrim"]')].filter((e) => vis(e) && !side.contains(e));
  const px = innerWidth - 4, py = innerHeight / 2; const hit = document.elementFromPoint(px, py);
  return {open, shown: vis(side) && sb.r > 1, l: sb.l, r: sb.r, w: sb.w, t: sb.t, b: sb.b, vw: innerWidth, vh: innerHeight,
    role: side.getAttribute('role'), modal: side.getAttribute('aria-modal'), label: side.getAttribute('aria-label') || '',
    focusInside: side.contains(document.activeElement), focusOnDrawer: document.activeElement === side,
    focusOnSearch: document.activeElement && document.activeElement.id === 'ai-search', btnFocus: document.activeElement === b, focusOut: !side.contains(document.activeElement),
    backdrop: backs.length, edgeIsDrawer: !!hit && side.contains(hit), hash: location.hash, sideways: document.documentElement.scrollWidth - innerWidth};"""


def dr_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/ai', 2.4)
    p = []
    c = jv(d, DR_JS)
    if not c:
        skip(rc, 'W-V23-DR', 'ai', 'drawer', 'sam', size, lang, theme, 'no menu button / drawer on the page')
        return
    if c['open'] or c['shown']:
        p.append('the drawer is open or visible before any tap')
    tap(d, '#ai-drawer-btn', 0.9)
    c = jv(d, DR_JS)
    want = min(320, 0.82 * c['vw'])
    if not c['open'] or not c['shown']:
        p.append('the menu button did not open the drawer')
    else:
        if abs(c['w'] - want) > 3:
            p.append('drawer %.0f px wide (want about %.0f)' % (c['w'], want))
        if c['l'] < -0.5 or c['r'] > c['vw'] + 0.5:
            p.append('the drawer leaves the screen (%.0f..%.0f of %d)' % (c['l'], c['r'], c['vw']))
        if c['b'] - c['t'] < c['vh'] * 0.9:
            p.append('the drawer is only %.0f px high of %d' % (c['b'] - c['t'], c['vh']))
        if c['role'] != 'dialog' or c['modal'] != 'true' or not c['label']:
            p.append('dialog semantics missing (role %s, aria-modal %s, label "%s")' % (c['role'], c['modal'], c['label']))
        if not c['focusInside'] or c['focusOnSearch']:
            p.append('focus is %s (want the drawer itself, not the search box)' % ('on the search box' if c['focusOnSearch'] else 'outside the drawer'))
        if not c['backdrop']:
            p.append('no backdrop beside the drawer')
        if c['edgeIsDrawer']:
            p.append('no free strip right of the drawer to tap')
        if c['sideways'] > 1:
            p.append('sideways scroll: %d px' % c['sideways'])
    # 1. a backdrop tap closes it, the focus returns to the menu button
    d.tap(c['vw'] - 4, c['vh'] / 2, wait=0.8)
    c2 = jv(d, DR_JS)
    if c2['open'] or c2['shown']:
        p.append('a backdrop tap did not close the drawer')
    elif not c2['focusOut']:
        p.append('after a tap close the focus is left inside the hidden drawer')
    # 2. Esc closes it
    tap(d, '#ai-drawer-btn', 0.8)
    d.key('Escape', 'Escape', 27)
    time.sleep(0.7)
    if jv(d, DR_JS)['open']:
        p.append('Esc did not close the drawer')
    elif not jv(d, DR_JS)['btnFocus']:
        p.append('after an Escape close the focus is not on the menu button')
    # 3. the back button closes it and stays on AI Chat
    tap(d, '#ai-drawer-btn', 0.8)
    d.js('history.back(); 1')
    time.sleep(0.9)
    c3 = jv(d, DR_JS)
    if c3['open']:
        p.append('the back button did not close the drawer')
    if not c3['hash'].startswith('#/ai'):
        p.append('the back button left AI Chat (%s)' % c3['hash'])
    # the drawer is shown once more for the screenshot
    if not p:
        tap(d, '#ai-drawer-btn', 0.8)
    verdict(rc, 'W-V23-DR', 'ai', 'drawer', 'sam', size, lang, theme, d, p,
            '%.0f px wide of %d, dialog, backdrop; backdrop / Esc / back close it, focus returns' % (c['w'], c['vw']))


# ------------------------------------------------------------------------------------------------ W-V23-SE
SE_JS = """const side = sidebar(); if (!side) return null; const rows = [...side.querySelectorAll('.ai-chat-open')].filter(vis);
  const s = document.getElementById('ai-search');
  return {n: rows.length, texts: rows.map((e) => e.innerText.trim()), search: !!s && vis(s), value: s ? s.value : null,
    note: side.innerText.length};"""

SET_SEARCH = """((v) => { const e = document.getElementById('ai-search'); if (!e) return false; e.focus();
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(e, v);
  e.dispatchEvent(new Event('input', {bubbles: true})); return true })"""


def se_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    rc.chat_id('sam')
    goto(d, '#/ai', 2.4)
    open_sidebar(d)
    a = jv(d, SE_JS)
    if not a or not a['search']:
        rc.rec('W-V23-SE', 'ai', 'search', 'sam', size, lang, theme, False, 'no search box (#ai-search) in the sidebar', d)
        return
    if a['n'] == 0:
        skip(rc, 'W-V23-SE', 'ai', 'search', 'sam', size, lang, theme, 'no history rows to filter')
        return
    p = []
    word = (a['texts'][0].split() or [''])[0]
    q = word.upper() if word.upper() != word else word.lower()
    d.js_call(SET_SEARCH, q)
    time.sleep(0.5)
    b = jv(d, SE_JS)
    if b['n'] < 1:
        p.append('searching "%s" (case changed) leaves no row' % q)
    elif b['n'] > a['n']:
        p.append('the search added rows')
    bad = [t for t in b['texts'] if q.lower() not in t.lower()]
    if bad and b['n'] < a['n']:
        p.append('a row without the word stays: "%s"' % bad[0][:40])
    d.js_call(SET_SEARCH, 'zzqq-no-such-chat-zz')
    time.sleep(0.5)
    c = jv(d, SE_JS)
    if c['n'] != 0:
        p.append('a made-up word still shows %d row(s)' % c['n'])
    d.js_call(SET_SEARCH, '')
    time.sleep(0.5)
    e = jv(d, SE_JS)
    if e['n'] != a['n']:
        p.append('clearing the search shows %d rows (was %d)' % (e['n'], a['n']))
    verdict(rc, 'W-V23-SE', 'ai', 'search', 'sam', size, lang, theme, d, p,
            '"%s": %d of %d rows, a made-up word: none, cleared: all %d' % (q, b['n'], a['n'], e['n']))


# ------------------------------------------------------------------------------------------------ W-V23-MEM
MEM_JS = """const side = sidebar(); const b = drawerBtn();
  const view = document.getElementById('ai-mem-input') || document.getElementById('ai-mdelall') || document.querySelector('.ai-mem');
  const back = [...document.querySelectorAll('.page-ai button, .page-ai a')].find((e) => vis(e) && BACKRE.test(e.textContent + ' ' + (e.getAttribute('aria-label') || '')));
  const ta = document.getElementById('ai-input');
  return {memShown: !!view && vis(view), back: !!back, backH: back ? box(back).h : 0, backIn: back ? inView(box(back)) : false,
    drawerOpen: isOpen(), composer: !!ta && vis(ta), hash: location.hash, sideways: document.documentElement.scrollWidth - innerWidth};""".replace('BACKRE', '/' + BACK_RE.pattern + '/')


SIDE_RECT_JS = """const side = document.querySelector('aside.ai-list'); if (!side) return null; const b = box(side);
  const card = document.querySelector('.ai-conv, .ai-memwrap'); const m = card ? box(card) : b;
  return {h: b.h, t: b.t, memT: m.t, memH: m.h};"""

MEM_BAR_JS = """const bar = document.querySelector('.ai-mem-bar-t'); const n = document.querySelector('.ai-mem-n');
  return {bar: bar ? bar.textContent.trim() : '', bare: !!n && vis(n) && box(n).w > 2 && box(n).h > 2};"""


def mem_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/ai', 2.4)
    # a fresh device can still be booting ("loading the banks") after the fixed settle: wait for the AI Chat page first
    until(d, "document.getElementById('ai-input')", 10)
    open_sidebar(d)
    until(d, "(() => { const e = document.getElementById('ai-mem-open'); return !!e && e.getClientRects().length > 0 })()", 6)
    if not jv(d, "return !!document.getElementById('ai-mem-open') && vis(document.getElementById('ai-mem-open'));"):
        rc.rec('W-V23-MEM', 'ai', 'memories', 'sam', size, lang, theme, False, 'no memory row (#ai-mem-open) in the sidebar', d)
        return
    p = []
    before = jv(d, SIDE_RECT_JS) if size[0] >= 720 else None
    tap(d, '#ai-mem-open', 1.0)
    m = jv(d, MEM_JS)
    if size[0] < 720:
        n = jv(d, MEM_BAR_JS)
        if not n or not re.search(r'\d+\s*(/|of)\s*\d+', n['bar']):
            p.append('the memory bar has no count next to the title (%r)' % (n and n['bar']))
        if n and n['bare']:
            p.append('a bare count line is shown in the panel')
    else:
        after = jv(d, SIDE_RECT_JS)
        if before and after:
            if abs(before['h'] - after['h']) > 1.5:
                p.append('the sidebar is %.0f px high in the memory view (%.0f in the chat view)' % (after['h'], before['h']))
            if abs(before['t'] - after['t']) > 1.5 or abs(after['memT'] - after['t']) > 1.5:
                p.append('columns not aligned: sidebar top %.0f, memory card top %.0f' % (after['t'], after['memT']))
            if abs(after['memH'] - after['h']) > 1.5:
                p.append('the memory card is %.0f px high, the sidebar %.0f' % (after['memH'], after['h']))
        if jv(d, "return [...document.querySelectorAll('.page-ai .gh-line')].filter(vis).length;") > 1:
            p.append('the GitHub line is shown more than once')
    if not m['memShown']:
        p.append('the memory row did not open the memories view')
    if size[0] < 720 and m['drawerOpen']:
        p.append('the drawer stays open over the memories view')
    if not m['back']:
        p.append('no back-to-chat control in the memories view')
    else:
        if size[0] < 720 and m['backH'] < 43.5:
            p.append('the back control is %.0f px high' % m['backH'])
        if not m['backIn']:
            p.append('the back control is outside the screen')
    if m['sideways'] > 1:
        p.append('sideways scroll: %d px' % m['sideways'])
    rc.rec('W-V23-MEM', 'ai', 'memories', 'sam', size, lang, theme, not p, '; '.join(p) or 'memories view open, back control inside the screen', d)
    if m['back']:
        d.js("(() => { const b = [...document.querySelectorAll('.page-ai button, .page-ai a')].find((e) => /%s/.test(e.textContent + ' ' + (e.getAttribute('aria-label') || ''))); if (b) b.click(); return 1 })()" % BACK_RE.pattern)
        time.sleep(1.0)
        m2 = jv(d, MEM_JS)
        q = []
        if m2['memShown']:
            q.append('the memories view is still shown after Back')
        if not m2['composer']:
            q.append('no conversation / composer after Back')
        if not m2['hash'].startswith('#/ai'):
            q.append('Back left AI Chat (%s)' % m2['hash'])
        # leaving AI Chat from the memory view and coming back enters as usual: a new chat, not the memory view
        open_sidebar(d, 0.4)
        d.js("(() => { const b = document.getElementById('ai-mem-open'); if (b) b.click(); return 1 })()")
        time.sleep(0.8)
        d.js("location.hash = '#/'; 1")
        time.sleep(1.0)
        d.js("location.hash = '#/ai'; 1")
        time.sleep(1.6)
        m3 = jv(d, MEM_JS)
        if m3['memShown'] or not m3['composer']:
            q.append('coming back to AI Chat from the memory view shows the memory view again')
        verdict(rc, 'W-V23-MEM', 'ai', 'back-to-chat', 'sam', size, lang, theme, d, q, 'Back returns to the conversation; re-entering opens a new chat')


# ------------------------------------------------------------------------------------------------ W-V23-HD
HD_JS = """const head = document.querySelector('.page-ai .ai-conv-head'); if (!head || !vis(head)) return null;
  const find = (q) => head.querySelector(q);
  const parts = {menu: find('#ai-drawer-btn'), title: find('.ai-conv-title, #ai-conv-t, h2, h1'), tag: find('.ai-mode-tag'), add: find('#ai-new'), more: find('#ai-chat-more')};
  const shown = Object.keys(parts).filter((k) => vis(parts[k]));
  const hb = box(head); const out = {};
  for (const k of shown) { const r = box(parts[k]); out[k] = {w: r.w, h: r.h, inside: r.l >= hb.l - 0.5 && r.r <= hb.r + 0.5 && inView(r)}; }
  const app = document.getElementById('menu-btn'); const mi = parts.menu ? parts.menu.querySelector('svg') : null; const ai = app ? app.querySelector('svg') : null;
  const gap = parts.tag && vis(parts.tag) && vis(parts.title) ? box(parts.tag).l - box(parts.title).r : null;
  const cut = [...head.querySelectorAll('.ai-mode-tag > *, .ai-mode-tag')].filter((e) => vis(e) && !e.classList.contains('sr-only')).some((e) => e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflow === 'hidden' && !e.closest('.ai-conv-title'));
  return {h: hb.h, shown, parts: out, overlap: overlaps(shown.map((k) => parts[k])), sideways: document.documentElement.scrollWidth - innerWidth,
    sameIcon: !!mi && !!ai && mi.innerHTML === ai.innerHTML, gap, title: parts.title ? parts.title.textContent.trim() : '', expanded: parts.menu ? parts.menu.getAttribute('aria-expanded') : null,
    label: parts.menu ? parts.menu.getAttribute('aria-label') || '' : '', tagCut: cut};"""


def hd_judge(h, existing, free):
    p = []
    if not h:
        return ['no chat header row on the phone']
    if h['h'] > 45.5:
        p.append('the header row is %.0f px high (limit 44 + the 1 px hairline)' % h['h'])
    for need in ('menu', 'add') + (('more',) if existing else ()):
        if need not in h['shown']:
            p.append('no %s button' % need)
    if not existing and 'more' in h['shown']:
        p.append('the more button is shown for a new chat')
    if free and 'tag' not in h['shown']:
        p.append('no free-mode tag after the title')
    if 'title' not in h['shown']:
        p.append('no title')
    for k, v in h['parts'].items():
        if not v['inside']:
            p.append('%s sticks out of the row / screen' % k)
        if k in ('menu', 'add', 'more') and (v['w'] < 43.5 or v['h'] < 43.5):
            p.append('%s target is %.0fx%.0f px' % (k, v['w'], v['h']))
    if h['overlap']:
        p.append('overlap: ' + ', '.join(h['overlap']))
    if h['sideways'] > 1:
        p.append('sideways scroll: %d px' % h['sideways'])
    if h['tagCut']:
        p.append('the free-mode tag is cut')
    if h['sameIcon']:
        p.append('the history button has the same icon as the app menu button')
    if free and h['gap'] is not None and h['gap'] > 16:
        p.append('the free-mode tag is %.0f px after the title (want it right after it)' % h['gap'])
    if not h['label']:
        p.append('the menu button has no aria-label')
    if h['expanded'] != 'false':
        p.append('menu button aria-expanded is %s while the drawer is closed' % h['expanded'])
    return p


def hd_case(rc, lang, theme, size):
    if size[0] >= 720:
        return      # the phone header row only exists below 720 px (wider screens keep the title row)
    for user, free in (('sam', False), ('admin', True)):
        was = None
        if free:
            was = rc.session('admin').call('/api/ai/status').get('free')
            rc.session('admin').call('/api/ai/free', {'free': True})
        try:
            cid = rc.chat_id(user)
            d = rc.dev(user, lang, theme, size)
            for state, route, existing in (('new', '#/ai', False), ('chat', '#/ai/%d' % cid, True)):
                goto(d, route, 2.4)
                h = jv(d, HD_JS)
                p = hd_judge(h, existing, free)
                rc.rec('W-V23-HD', 'ai', '%s-%s%s' % ('header', state, '-free' if free else ''), user, size, lang, theme, not p,
                       '; '.join(p) or 'row %.0f px: %s' % (h['h'], ', '.join(h['shown'])), d)
        finally:
            if free and was is not None:
                try:
                    rc.session('admin').call('/api/ai/free', {'free': bool(was)})
                except Exception:
                    pass


# ------------------------------------------------------------------------------------------------ W-V23-SB
SB_JS = """const side = document.querySelector('aside.ai-list'); if (!side || !vis(side)) return null; const sb = box(side);
  const text = side.innerText; const rows = [...side.querySelectorAll('.ai-chat-open')].filter(vis);
  const menus = [...side.querySelectorAll('[id^="ai-cmenu-"]')].filter(vis);
  const need = (id) => { const e = document.getElementById(id); return !!e && vis(e) && side.contains(e); };
  return {w: sb.w, inside: inView(sb), btn: vis(drawerBtn()), search: need('ai-search'), add: need('ai-new'), mem: need('ai-mem-open'),
    groups: GROUPRE.test(text), ghLines: [...document.querySelectorAll('.page-ai .gh-line')].filter(vis).length,
    delShadow: (() => { const e = document.getElementById('ai-delall'); return e ? getComputedStyle(e).boxShadow : 'none'; })(), rows: rows.length, menus: menus.length, hash: location.hash, sideways: document.documentElement.scrollWidth - innerWidth,
    rowsIn: rows.every((e) => box(e).r <= sb.r + 0.5 && box(e).l >= sb.l - 0.5)};""".replace('GROUPRE', '/' + GROUP_RE.pattern + '/')


def sb_case(rc, lang, theme, size):
    if size[0] < 720:
        return      # drawer mode (W-V23-DR / -EN cover it): the always-visible sidebar exists from 720 px on
    d = rc.dev('sam', lang, theme, size)
    rc.chat_id('sam')
    goto(d, '#/ai', 2.4)
    # the history arrives after the page ("Loading..." in the sidebar): wait for the rows before measuring
    until(d, "document.querySelector('aside.ai-list .ai-chat-open')", 10)
    s = jv(d, SB_JS)
    if not s:
        rc.rec('W-V23-SB', 'ai', 'sidebar', 'sam', size, lang, theme, False, 'no sidebar on screen', d)
        return
    p = []
    want = 240 if size[0] <= 900 else 272
    if abs(s['w'] - want) > 4:
        p.append('sidebar %.0f px wide (want %d)' % (s['w'], want))
    if not s['inside']:
        p.append('the sidebar leaves the screen')
    if s['btn']:
        p.append('a menu button is shown')
    for k, name in (('search', 'search box'), ('add', 'new chat button'), ('mem', 'memory row')):
        if not s[k]:
            p.append('no %s' % name)
    if s['rows'] == 0:
        p.append('no history rows')
    else:
        if not s['groups']:
            p.append('no group heading (today / yesterday / ...)')
        if s['menus'] < s['rows']:
            p.append('%d rows but %d row menu buttons' % (s['rows'], s['menus']))
        if not s['rowsIn']:
            p.append('a row sticks out of the sidebar')
    if s['sideways'] > 1:
        p.append('sideways scroll: %d px' % s['sideways'])
    if s['delShadow'] != 'none':
        p.append('Delete all chats keeps the button shadow (%s)' % s['delShadow'][:40])
    if s['ghLines'] > 1:
        p.append('the GitHub line is shown %d times' % s['ghLines'])
    verdict(rc, 'W-V23-SB', 'ai', 'sidebar', 'sam', size, lang, theme, d, p,
            '%.0f px sidebar, %d rows with headings and row menus, search / new chat / memory row' % (s['w'], s['rows']))


# ------------------------------------------------------------------------------------------------ W-V23-DL
STATE_JS = """const err = document.querySelector('.page-ai .ai-conv .err, .page-ai .ai-conv [role="alert"]');
  const ta = document.getElementById('ai-input'); const dlg = document.querySelector('dialog[open]');
  const ae = document.activeElement;
  return {hash: location.hash, err: err ? err.textContent.trim().slice(0, 80) : null, composer: !!ta && vis(ta), open: isOpen(),
    dialog: !!dlg, focusInDialog: !!dlg && !!ae && dlg.contains(ae), focusId: ae ? (ae.id || ae.tagName) : null,
    msgs: document.querySelectorAll('.ai-msg').length, title: (document.getElementById('ai-conv-t') || {}).textContent || '',
    delShadow: (() => { const e = document.querySelector('.ai-delthis, #ai-delall'); return e ? getComputedStyle(e).boxShadow : 'none'; })()};"""


def mk_chat(rc, user, text):
    return ai_send(rc.walk.base, rc.walk.stack.cookie(user), text, 'zh', None)


def chat_ids(rc, user):
    return [c['id'] for c in rc.session(user).call('/api/ai/chats').get('chats', [])]


def dl_case(rc, lang, theme, size):
    user = 'lee'
    if size[0] >= 720:
        # --full runs every size: from 720 px the history is the always-visible sidebar (v2.3), there is no drawer to test
        skip(rc, 'W-V23-DL', 'ai', 'delete-open-chat', user, size, lang, theme, 'no drawer at %dx%d (sidebar mode)' % tuple(size))
        return
    d = rc.dev(user, lang, theme, size)
    p = []
    a = mk_chat(rc, user, 'Walk delete test A: one tip about knives, please.')
    b = mk_chat(rc, user, 'Walk delete test B: one tip about pans, please.')
    # 1. delete the open chat from the drawer: Tab and Esc belong to the confirm dialog
    goto(d, '#/ai/%d' % a, 2.4)
    tap(d, '#ai-drawer-btn', 0.8)
    tap(d, '#ai-cmenu-%d' % a, 0.5)
    tap(d, '#ai-cdel-%d' % a, 0.8)
    st = jv(d, STATE_JS)
    if not st['dialog']:
        p.append('the delete confirm did not open')
    else:
        d.key('Tab', 'Tab', 9)
        time.sleep(0.2)
        st = jv(d, STATE_JS)
        if not st['focusInDialog']:
            p.append('Tab left the confirm dialog (focus on %s)' % st['focusId'])
        d.key('Escape', 'Escape', 27)
        time.sleep(0.5)
        st = jv(d, STATE_JS)
        if st['dialog'] or not st['open']:
            p.append('Esc did not close just the dialog (dialog %s, drawer open %s)' % (st['dialog'], st['open']))
        tap(d, '#ai-cmenu-%d' % a, 0.5)
        tap(d, '#ai-cdel-%d' % a, 0.8)
        d.js("document.querySelector('dialog[open] .acts .btn.danger').click(); 1")
        time.sleep(1.4)
    # the drawer is still open: close it with a tap beside it
    d.tap(size[0] - 4, size[1] / 2, wait=1.0)
    st = jv(d, STATE_JS)
    if a in chat_ids(rc, user):
        p.append('the chat was not deleted')
    if st['err']:
        p.append('an error shows after the delete and the drawer close: %s' % st['err'])
    if not st['hash'].startswith('#/ai/new'):
        p.append('the address is %s after the drawer closed (want #/ai/new)' % st['hash'])
    if not st['composer'] or st['msgs']:
        p.append('the screen is not a clean new chat (composer %s, %d messages)' % (st['composer'], st['msgs']))
    goto(d, st['hash'] or '#/ai', 2.0)
    st2 = jv(d, STATE_JS)
    if st2['err']:
        p.append('a reload shows an error: %s' % st2['err'])
    rc.rec('W-V23-DL', 'ai', 'delete-open-chat', user, size, lang, theme, not p,
           '; '.join(p) or 'deleted from the drawer: new chat, #/ai/new, no error, Tab / Esc stay in the dialog', d)
    # 2. delete all chats from the drawer
    q = []
    goto(d, '#/ai/%d' % b, 2.4)
    tap(d, '#ai-drawer-btn', 0.8)
    st = jv(d, STATE_JS)
    if st['delShadow'] != 'none':
        q.append('the red menu items keep a shadow (%s)' % st['delShadow'][:40])
    tap(d, '#ai-delall', 0.8)
    if not jv(d, STATE_JS)['dialog']:
        q.append('Delete all chats did not open the confirm')
    else:
        d.key('Tab', 'Tab', 9)
        time.sleep(0.2)
        if not jv(d, STATE_JS)['focusInDialog']:
            q.append('Tab left the confirm dialog')
        d.js("document.querySelector('dialog[open] .acts .btn.danger').click(); 1")
        time.sleep(1.4)
    d.tap(size[0] - 4, size[1] / 2, wait=1.0)
    st = jv(d, STATE_JS)
    if chat_ids(rc, user):
        q.append('chats are left after Delete all')
    if st['err']:
        q.append('an error shows after Delete all and the drawer close: %s' % st['err'])
    if not st['hash'].startswith('#/ai/new'):
        q.append('the address is %s after Delete all (want #/ai/new)' % st['hash'])
    if not st['composer']:
        q.append('no composer after Delete all')
    verdict(rc, 'W-V23-DL', 'ai', 'delete-all', user, size, lang, theme, d, q, 'Delete all from the drawer: new chat, #/ai/new, no error')


# ------------------------------------------------------------------------------------------------ W-V23-GONE
def gone_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/ai/987654', 2.6)
    st = jv(d, STATE_JS)
    p = []
    if st['err']:
        p.append('an error is shown for the missing chat: %s' % st['err'])
    if not st['hash'].startswith('#/ai/new'):
        p.append('the address is %s (want #/ai/new)' % st['hash'])
    if not st['composer']:
        p.append('no usable composer')
    if size[0] < 720:
        tap(d, '#ai-drawer-btn', 0.8)
        d.tap(size[0] - 4, size[1] / 2, wait=0.9)
        st = jv(d, STATE_JS)
        if '987654' in st['hash']:
            p.append('the missing chat is back in the address after the drawer closed')
    verdict(rc, 'W-V23-GONE', 'ai', 'missing-chat', 'sam', size, lang, theme, d, p, 'a missing chat opens a new-chat draft, no error')


# ------------------------------------------------------------------------------------------------ W-V23-RO
def ro_case(rc, lang, theme, size):
    rc.chat_id('sam')
    admin = rc.session('admin')
    uid = admin.user_id('sam')
    p = []
    try:
        admin.call('/api/ai/admin/suspend', {'userId': uid})
        d = rc.dev('sam', lang, theme, size)
        goto(d, '#/ai', 2.8)
        st = jv(d, STATE_JS)
        extra = jv(d, """return {add: !!document.querySelector('.ai-conv-head #ai-new'),
          line: !!document.getElementById('ai-readonly') && vis(document.getElementById('ai-readonly')), greeting: !!document.getElementById('ai-greeting')};""")
        if not st['msgs']:
            p.append('a paused user with chats lands on an empty pane (the latest chat should open)')
        if not re.match(r'#/ai/\d+', st['hash']):
            p.append('the address is %s (want the latest chat)' % st['hash'])
        if extra['add']:
            p.append('the header shows a new-chat button to a paused user')
        if not extra['line']:
            p.append('the paused line is missing')
        if extra['greeting']:
            p.append('a paused user sees the new-chat greeting')
        rc.rec('W-V23-RO', 'ai', 'paused-user', 'sam', size, lang, theme, not p,
               '; '.join(p) or 'paused: latest chat open (%d messages), no new-chat button, paused line shown' % st['msgs'], d)
    finally:
        try:
            admin.call('/api/ai/admin/resume', {'userId': uid})
        except Exception:
            pass


# ------------------------------------------------------------------------------------------------ W-V23-FW
def fw_case(rc, lang, theme, size):
    if size[0] >= 720:
        # --full runs every size: from 720 px the history is the always-visible sidebar (v2.3), there is no drawer to close
        skip(rc, 'W-V23-FW', 'ai', 'forward', 'sam', size, lang, theme, 'no drawer at %dx%d (sidebar mode)' % tuple(size))
        return
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/', 1.6)
    d.js("location.hash = '#/ai'; 1")
    time.sleep(1.8)
    tap(d, '#ai-drawer-btn', 0.8)
    d.tap(size[0] - 4, size[1] / 2, wait=1.0)
    d.js('history.back(); 1')
    time.sleep(1.0)
    home = d.js('location.hash')
    d.js('history.forward(); 1')
    time.sleep(0.9)
    d.js('history.forward(); 1')
    time.sleep(1.0)
    mid = d.js('location.hash')
    d.js('history.back(); 1')
    time.sleep(1.0)
    end = d.js('location.hash')
    p = []
    if home not in ('#/', '', '#'):
        p.append('the first back did not leave AI Chat (%s)' % home)
    if not mid.startswith('#/ai'):
        p.append('Forward did not return to AI Chat (%s)' % mid)
    if end.startswith('#/ai'):
        p.append('a dead forward entry: back after Forward, Forward still shows AI Chat (%s)' % end)
    verdict(rc, 'W-V23-FW', 'ai', 'forward', 'sam', size, lang, theme, d, p, 'no dead forward entry after closing the drawer')


# ------------------------------------------------------------------------------------------------ W-V23-KB
def kb_case(rc, lang, theme, size):
    if size[0] >= 720:
        # --full runs every size: from 720 px the history is the always-visible sidebar (v2.3), there is no drawer to lift
        skip(rc, 'W-V23-KB', 'ai', 'drawer-keyboard', 'sam', size, lang, theme, 'no drawer at %dx%d (sidebar mode)' % tuple(size))
        return
    d = rc.dev('sam', lang, theme, size)
    rc.chat_id('sam')
    goto(d, '#/ai', 2.4)
    tap(d, '#ai-drawer-btn', 0.9)
    d.js("document.documentElement.style.setProperty('--kb', '300px'); 1")
    time.sleep(0.4)
    r = jv(d, """const s = sidebar(); const b = box(s); return {bottom: b.b, vh: innerHeight};""")
    d.js("document.documentElement.style.removeProperty('--kb'); 1")
    p = []
    if abs(r['bottom'] - (r['vh'] - 300)) > 2:
        p.append('the drawer ends at %.0f with a 300 px keyboard (want %.0f)' % (r['bottom'], r['vh'] - 300))
    verdict(rc, 'W-V23-KB', 'ai', 'drawer-keyboard', 'sam', size, lang, theme, d, p, 'the drawer bottom follows --kb')


# ------------------------------------------------------------------------------------------------ W-V23-LS
def ls_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    rc.chat_id('sam')
    goto(d, '#/ai', 2.4)
    if not jv(d, "return !!drawerBtn() && vis(drawerBtn());"):
        skip(rc, 'W-V23-LS', 'ai', 'drawer-landscape', 'sam', size, lang, theme, 'no menu button at this size')
        return
    until(d, "document.querySelector('aside.ai-list .ai-chat-open')", 10)
    tap(d, '#ai-drawer-btn', 0.9)
    # short (<= 500 px high, ai.css): the whole drawer is the one scroller; a taller phone (--full also runs portrait sizes) keeps
    # the v2.3 layout: the drawer is fixed and the list body (rows, Delete all, the GitHub line) is the scroller under the head
    short = size[1] <= 500
    r = jv(d, """const s = sidebar(); const body = document.getElementById('ai-list-body'); const cs = getComputedStyle(s); const bs = getComputedStyle(body);
      const sc = SHORT ? s : body; sc.scrollTop = sc.scrollHeight; const del = document.getElementById('ai-delall'); const gh = s.querySelector('.gh-line');
      const out = {sideY: cs.overflowY, bodyY: bs.overflowY, delIn: del ? inView(box(del)) : null,
        noticeIn: gh && vis(gh) ? inView(box(gh)) : null, notice: !!s.querySelector('.ai-notice-btn')};
      sc.scrollTop = 0; const row = s.querySelector('.ai-chat-open'); out.rowIn = row ? inView(box(row)) : null; return out;""".replace('SHORT', 'true' if short else 'false'))
    p = []
    if short and (r['sideY'] not in ('auto', 'scroll') or r['bodyY'] in ('auto', 'scroll')):
        p.append('the drawer is not one scroller (drawer %s, list %s)' % (r['sideY'], r['bodyY']))
    if not short and (r['bodyY'] not in ('auto', 'scroll') or r['sideY'] in ('auto', 'scroll')):
        p.append('the list body is not the one scroller of the tall drawer (drawer %s, list %s)' % (r['sideY'], r['bodyY']))
    if r['delIn'] is False:
        p.append('Delete all chats cannot be scrolled into view')
    if r['noticeIn'] is False:
        p.append('the GitHub line cannot be scrolled into view')
    if r['notice']:
        p.append('the administrator notice is in the drawer')
    if r['rowIn'] is False:
        p.append('the first history row is not reachable at the top')
    verdict(rc, 'W-V23-LS', 'ai', 'drawer-landscape', 'sam', size, lang, theme, d, p,
            ('the drawer scrolls as one' if short else 'the list scrolls inside the fixed drawer') + '; foot and GitHub line reachable')


# ------------------------------------------------------------------------------------------------ W-V23-NT
NT_JS = """const s = document.querySelector('.ai-composer .ai-notice-btn span'); const side = sidebar();
  let lines = 0; if (s && vis(s)) { const rg = document.createRange(); rg.selectNodeContents(s); lines = new Set([...rg.getClientRects()].map((r) => Math.round(r.top))).size; }
  const top = document.querySelector('.page-ai .ai-notice-top span'); let topLines = 0;
  if (top && vis(top)) { const rg = document.createRange(); rg.selectNodeContents(top); topLines = new Set([...rg.getClientRects()].map((r) => Math.round(r.top))).size; }
  const tb = top && vis(top) ? box(top) : null; const h1 = document.querySelector('.page-ai .ai-head h1');
  return {shown: !!s && vis(s), lines, inDrawer: !!side && !!side.querySelector('.ai-notice-btn'), top: !!tb, topLines,
    topIn: !!tb && inView(tb), underTitle: !!tb && !!h1 && vis(h1) && tb.t >= box(h1).b - 1,
    docScroll: !document.documentElement.classList.contains('no-doc-scroll')};"""


def nt_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/ai', 2.4)
    until(d, "document.getElementById('ai-input')", 10)
    r = jv(d, NT_JS)
    p = []
    if r['docScroll']:
        # over 900 px (not a short sideways phone) the page scrolls as a whole and the notice is the line under the page
        # title (.ai-notice-top, the design since v1.1); under the composer it shows only on the full-height chat (<= 900 px)
        if not r['top']:
            p.append('no administrator notice under the page title')
        else:
            if r['topLines'] != 1:
                p.append('the notice under the title wraps to %d lines at %d px (want one line)' % (r['topLines'], size[0]))
            if not r['underTitle']:
                p.append('the notice is not under the page title')
            if not r['topIn']:
                p.append('the notice under the title is outside the screen')
        if r['shown']:
            p.append('the notice is shown twice (under the title and under the composer)')
        if r['inDrawer']:
            p.append('the administrator notice is in the sidebar')
        verdict(rc, 'W-V23-NT', 'ai', 'notice', 'sam', size, lang, theme, d, p, 'notice under the page title on %d line(s), not under the composer, none in the sidebar' % r['topLines'])
        return
    if not r['shown']:
        p.append('no administrator notice under the composer')
    elif r['lines'] != 1 and size[0] >= 375:
        p.append('the notice wraps to %d lines at %d px (want one line)' % (r['lines'], size[0]))
    if r['top']:
        p.append('the notice is shown twice (under the composer and under the page title)')
    if r['inDrawer']:
        p.append('the administrator notice is in the drawer')
    verdict(rc, 'W-V23-NT', 'ai', 'notice', 'sam', size, lang, theme, d, p, 'notice under the composer on %d line(s), none in the drawer' % r['lines'])


# ------------------------------------------------------------------------------------------------ W-V23-LT
def lt_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    cid = rc.chat_id('sam')
    if 'kb-long-chat' not in rc.cache:
        # the chat must overflow a phone screen: two more made-up exchanges (once per run; W-CH-88 uses the same)
        rc.cache['kb-long-chat'] = True
        for text in ('Please list three more tips about knife care.', 'And three tips about pan care, in short lines.'):
            try:
                ai_send(rc.walk.base, rc.walk.stack.cookie('sam'), text, 'zh', cid)
            except Exception:
                break
    goto(d, '#/ai/%d' % cid, 2.4)
    d.js("(() => { const sc = document.getElementById('ai-scroll'); sc.scrollTop = 0; sc.dispatchEvent(new Event('scroll')); return 1 })()")
    time.sleep(0.5)
    if not jv(d, "const b = document.getElementById('ai-latest'); return !!b && vis(b);"):
        skip(rc, 'W-V23-LT', 'ai', 'jump-latest', 'sam', size, lang, theme, 'the conversation does not scroll here (no jump button)')
        return
    tap(d, '#ai-latest', 0.8)
    r = jv(d, """const sc = document.getElementById('ai-scroll'); const ae = document.activeElement;
      return {atEnd: sc.scrollHeight - sc.scrollTop - sc.clientHeight < 30, input: !!ae && (ae.id === 'ai-input' || ae.tagName === 'TEXTAREA'),
        focus: ae ? (ae.id || ae.tagName) : null};""")
    p = []
    if not r['atEnd']:
        p.append('the conversation is not at its end after the tap')
    if r['input']:
        p.append('the tap focused the input (keyboard would open)')
    # the keyboard is open (the input has the focus, the user scrolled up while typing): the tap keeps the focus on the input
    d.js("(() => { const sc = document.getElementById('ai-scroll'); sc.scrollTop = 0; sc.dispatchEvent(new Event('scroll')); document.getElementById('ai-input').focus({ preventScroll: true }); return 1 })()")
    time.sleep(0.5)
    if jv(d, "const b = document.getElementById('ai-latest'); return !!b && vis(b);"):
        tap(d, '#ai-latest', 0.8)
        r2 = jv(d, """const sc = document.getElementById('ai-scroll'); const ae = document.activeElement;
          return {atEnd: sc.scrollHeight - sc.scrollTop - sc.clientHeight < 30, input: !!ae && ae.id === 'ai-input'};""")
        if not r2['atEnd']:
            p.append('with the input focused the tap did not scroll to the end')
        if not r2['input']:
            p.append('with the input focused the tap took the focus away (the keyboard would close)')
    else:
        p.append('no jump button with the input focused')
    verdict(rc, 'W-V23-LT', 'ai', 'jump-latest', 'sam', size, lang, theme, d, p, 'scrolled to the end, focus on %s (not the input)' % r['focus'])


# ------------------------------------------------------------------------------------------------ W-V23-CTR
CTR_JS = """const ta = document.getElementById('ai-input'); const form = ta && ta.closest('.ai-form'); if (!form) return null;
  const f = box(form); const head = document.getElementById('ai-conv-head'); const hb = head ? box(head) : {b: 0};
  const g = document.getElementById('ai-greeting'); const hint = document.getElementById('ai-empty-hint');  // v2.3: the hint is gone (hT stays null)
  const vv = window.visualViewport; const vh = vv ? vv.height : innerHeight;
  const dock = document.getElementById('ai-composer'); const ds = dock ? getComputedStyle(dock) : null;
  const bg = (e) => { for (; e; e = e.parentElement) { const c = getComputedStyle(e).backgroundColor; if (c && c !== 'rgba(0, 0, 0, 0)' && c !== 'transparent') return c; } return ''; };
  const conv = document.querySelector('.ai-conv');
  const note = document.querySelector('.ai-conv .ai-notice-btn'); const nb = note && vis(note) ? box(note) : null;
  const lines = hint ? (() => { const rg = document.createRange(); rg.selectNodeContents(hint); return new Set([...rg.getClientRects()].map((q) => Math.round(q.top))).size; })() : 0;
  return {nT: nb ? nb.t : null, nB: nb ? nb.b : null, hintLines: lines, hintTxt: hint ? hint.textContent : '', fc: (f.t + f.b) / 2, ft: f.t, fb: f.b, headB: hb.b, vh, gB: g ? box(g).b : null, hT: hint ? box(hint).t : null,
    border: ds ? ds.borderTopWidth : '0px', formBorder: getComputedStyle(form).borderTopWidth,
    dockBg: dock ? bg(dock.style.display === 'contents' ? form : dock) : '', convBg: conv ? bg(conv) : '', wide: innerWidth > 900,
    cardB: conv ? box(conv).b : vh};"""


def ctr_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    cid = rc.chat_id('sam')
    goto(d, '#/ai/new', 2.4)
    p = []
    r = jv(d, CTR_JS)
    if not r:
        skip(rc, 'W-V23-CTR', 'ai', 'new-chat-centred', 'sam', size, lang, theme, 'no composer')
        return
    area = r['cardB'] - r['headB'] if r['wide'] else r['vh'] - r['headB']
    lo, hi = r['headB'] + area / 3, r['headB'] + 2 * area / 3
    short = size[1] <= 500
    if not lo - 1 <= r['fc'] <= hi + 1:
        p.append('the composer centre %.0f is outside the middle third %.0f..%.0f' % (r['fc'], lo, hi))
    if r['gB'] is not None and r['gB'] > r['ft'] + 1:
        p.append('the greeting (bottom %.0f) is not above the composer (top %.0f)' % (r['gB'], r['ft']))
    if r['hT'] is not None and r['hT'] < r['fb'] - 1:
        p.append('the hint (top %.0f) is not below the composer (bottom %.0f)' % (r['hT'], r['fb']))
    if r['hintTxt']:
        p.append('the language hint is still shown: %s' % r['hintTxt'])
    if r['nB'] is not None:
        bottom = r['cardB'] if r['wide'] else r['vh']
        if r['nT'] < r['fb'] - 1:
            p.append('the admin notice (top %.0f) is not below the composer (bottom %.0f)' % (r['nT'], r['fb']))
        if bottom - r['nB'] > 24:
            p.append('the admin notice ends %.0f px above the bottom (it must keep the bottom of the screen)' % (bottom - r['nB']))
    if r['formBorder'] != '0px':
        p.append('a line above the composer (%s)' % r['formBorder'])
    if r['dockBg'] and r['convBg'] and r['dockBg'] != r['convBg'] and not r['wide']:
        p.append('the dock background %s differs from the conversation %s' % (r['dockBg'], r['convBg']))
    if size[0] < 720 and not short:
        d.js("document.documentElement.style.setProperty('--vvh', '%dpx'); document.documentElement.style.setProperty('--kb', '%dpx'); document.documentElement.classList.add('kb-open'); 1"
             % (round(size[1] * 0.55), size[1] - round(size[1] * 0.55)))
        time.sleep(0.5)
        k = jv(d, CTR_JS)
        d.js("document.documentElement.style.removeProperty('--vvh'); document.documentElement.style.removeProperty('--kb'); document.documentElement.classList.remove('kb-open'); 1")
        if k['fb'] > round(size[1] * 0.55) + 1 or k['ft'] < k['headB'] - 1:
            p.append('with the keyboard the composer is %.0f..%.0f (visible area ends at %d, head at %.0f)' % (k['ft'], k['fb'], round(size[1] * 0.55), k['headB']))
    goto(d, '#/ai/%d' % cid, 2.4)
    m = jv(d, CTR_JS)
    if m:
        if m['gB'] is not None:
            p.append('an open chat still shows the greeting')
        bottom = m['cardB'] if m['wide'] else m['vh']
        if bottom - m['fb'] > 80:
            p.append('with messages the composer ends %.0f px above the bottom' % (bottom - m['fb']))
    verdict(rc, 'W-V23-CTR', 'ai', 'new-chat-centred', 'sam', size, lang, theme, d, p,
            'composer centre %.0f in %.0f..%.0f, greeting above, hint below, no line; at the bottom with messages' % (r['fc'], lo, hi))


# ------------------------------------------------------------------------------------------------ W-V23-CMP
CMP_JS = """const ta = document.getElementById('ai-input'); const bx = ta && ta.closest('.ai-box'); if (!bx) return null;
  const b = box(bx), i = box(ta), s = box(document.getElementById('ai-send')); const cs = getComputedStyle(bx);
  const vv = window.visualViewport; const vh = vv ? vv.height : innerHeight; const lh = parseFloat(getComputedStyle(ta).lineHeight) || 22;
  return {compact: bx.classList.contains('compact'), bh: b.b - b.t, bw: b.w, iw: i.w, it: i.t, ib: i.b, st: s.t, sb: s.b, il: i.l, sl: s.l,
    radius: parseFloat(cs.borderTopLeftRadius), focus: document.activeElement === ta, ih: i.b - i.t, over: getComputedStyle(ta).overflowY,
    sh: ta.scrollHeight, ch: ta.clientHeight, vh, lh, sideways: document.documentElement.scrollWidth - innerWidth};"""


def cmp_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/ai/new', 2.4)
    p = []
    r = jv(d, CMP_JS)
    if not r:
        skip(rc, 'W-V23-CMP', 'ai', 'composer-grow', 'sam', size, lang, theme, 'no composer')
        return
    if not r['compact']:
        p.append('the empty box is not compact')
    if r['bh'] > 60:
        p.append('the compact box is %.0f px high (want one row, at most 60)' % r['bh'])
    if not (r['it'] < r['sb'] and r['st'] < r['ib']):
        p.append('input and send are not on one row')
    if r['radius'] < r['bh'] / 2 - 1:
        p.append('the compact radius %.0f is not a full pill (%.0f px high)' % (r['radius'], r['bh']))
    d.js("document.getElementById('ai-input').focus(); 1")
    d.type('This is a deliberately long line of made-up words that cannot fit the compact pill on one line at any phone width, ok')
    time.sleep(0.5)
    e = jv(d, CMP_JS)
    if e['compact']:
        p.append('a long line did not switch to the expanded layout')
    if e['iw'] < e['bw'] * 0.85:
        p.append('expanded input is %.0f px of a %.0f px box (want the full width)' % (e['iw'], e['bw']))
    if not e['ib'] <= e['st'] + 1:
        p.append('the controls are not under the text in the expanded layout')
    if not 16 <= e['radius'] <= 32:
        p.append('expanded radius %.0f (want a large rounded rectangle, 16..32)' % e['radius'])
    if not e['focus']:
        p.append('the input lost the focus on the switch')
    d.type('\n'.join('line %d' % n for n in range(1, 20)))
    time.sleep(0.5)
    g = jv(d, CMP_JS)
    cap = min(8 * g['lh'], 0.4 * g['vh']) + 24
    if g['ih'] > cap + 2:
        p.append('the input is %.0f px high, over the cap %.0f' % (g['ih'], cap))
    if g['over'] != 'auto' or g['sh'] <= g['ch'] + 1:
        p.append('the input does not scroll inside after the cap (overflow %s)' % g['over'])
    if g['sideways'] > 1:
        p.append('the page scrolls sideways by %d px' % g['sideways'])
    shot = d
    d.js("(() => { const t = document.getElementById('ai-input'); t.value = ''; t.dispatchEvent(new Event('input', {bubbles: true})); return 1 })()")
    time.sleep(0.4)
    c = jv(d, CMP_JS)
    if not c['compact'] or c['bh'] > 60:
        p.append('clearing the text did not return to the compact row (compact %s, %.0f px high)' % (c['compact'], c['bh']))
    # one long word (no spaces): it breaks at the box edge, so every line is full and the box expands
    d.js("(() => { const t = document.getElementById('ai-input'); t.focus(); t.value = ''; t.dispatchEvent(new Event('input', {bubbles: true})); t.value = 'ssssssssss'.repeat(30); t.dispatchEvent(new Event('input', {bubbles: true})); return 1 })()")
    time.sleep(0.5)
    w = jv(d, CMP_JS)
    lw = d.js("(() => { const t = document.getElementById('ai-input'); const cs = getComputedStyle(t); return [t.scrollWidth - t.clientWidth, cs.overflowWrap, cs.wordBreak] })()")
    if w and w['compact']:
        p.append('a long word without spaces left the box compact')
    if lw[0] > 1 or lw[1] not in ('anywhere', 'break-word'):
        p.append('a long word does not wrap at the box edge (scroll %s, overflow-wrap %s)' % (lw[0], lw[1]))
    d.js("(() => { const t = document.getElementById('ai-input'); t.value = ''; t.dispatchEvent(new Event('input', {bubbles: true})); return 1 })()")
    time.sleep(0.4)
    # keep the expanded state in the screenshot of this case
    d.type('x ' * 60)
    time.sleep(0.4)
    verdict(rc, 'W-V23-CMP', 'ai', 'composer-grow', 'sam', size, lang, theme, shot, p,
            'compact %.0f px -> expanded (full width, radius %.0f) -> capped at %.0f px and scrolling -> compact again' % (r['bh'], e['radius'], g['ih']))


# ------------------------------------------------------------------------------------------------ run
def g_v23(rc):
    go(rc, 'W-V23-EN', 'ai', en_case, PHONES + [TABLET, DESKTOP])
    go(rc, 'W-V23-DR', 'ai', dr_case, PHONES)
    go(rc, 'W-V23-SE', 'ai', se_case, PHONES[:1] + [DESKTOP])
    go(rc, 'W-V23-MEM', 'ai', mem_case, PHONES + [DESKTOP])
    go(rc, 'W-V23-HD', 'ai', hd_case, PHONES)
    go(rc, 'W-V23-SB', 'ai', sb_case, [TABLET, DESKTOP])
    go(rc, 'W-V23-DL', 'ai', dl_case, PHONES[1:])
    go(rc, 'W-V23-GONE', 'ai', gone_case, PHONES[1:2] + [DESKTOP])
    go(rc, 'W-V23-RO', 'ai', ro_case, PHONES[:1] + [DESKTOP])
    go(rc, 'W-V23-FW', 'ai', fw_case, PHONES[:1])
    go(rc, 'W-V23-KB', 'ai', kb_case, PHONES[:1])
    go(rc, 'W-V23-LS', 'ai', ls_case, LANDS)
    if rc.args.full and not rc.args.widths and rc.want('W-V23-LS') and rc.walk.wants_page('ai'):
        # --full swaps the sizes for MATRIX, which has no phone under 720 x 500: the short sideways phones run as well
        for lang in rc.langs([]):
            for size in LANDS:
                rc.guard('W-V23-LS', 'ai', ls_case, rc, lang, rc.themes(['light'])[0], size)
    go(rc, 'W-V23-LT', 'ai', lt_case, [(393, 852), (360, 740)])
    go(rc, 'W-V23-CTR', 'ai', ctr_case, PHONES + LANDS[1:] + [TABLET, DESKTOP])
    go(rc, 'W-V23-CMP', 'ai', cmp_case, PHONES[:1] + PHONES[2:] + [DESKTOP])
    go(rc, 'W-V23-NT', 'ai', nt_case, [(375, 667), (393, 852), (430, 932), (320, 640)])


GROUPS_V23 = [g_v23]
