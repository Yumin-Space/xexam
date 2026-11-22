# -*- coding: utf-8 -*-
"""v2.2 walk checks: the phone account button, the quiz theme toggle, the AI Chat strength button, the strength dropdown of the
AI settings model rows, the AI jobs list and the centered footer. Every case takes screenshots into the output folder (rc.rec
with the device), passed or not.

  W-V22-AB   phone top bar: the brand is the word Xexam without the logo icon (wider screens show the icon), the account button shows the person icon, the compact AI+ badge and the short admin tag without any
             overlap, the user name is hidden; a tap shows the name tip (closes on a tap elsewhere and on Esc); the menu button
             opens the menu; on a desktop width the name is shown and the button opens the menu
  W-V22-QT   quiz top row (practice and exam): the theme toggle is there, switches dark / light and back, and overlaps nothing
             (timer, finish button, count, exit, position)
  W-V22-CS   AI Chat (administrator, free mode): the strength button sits beside the brain button, its menu opens (Default first,
             items inside the screen, one checked, Esc closes it), the composer bar does not overflow
  W-V22-MS   AI settings, model rows: every strength dropdown starts with Default (empty value) whose text says no parameter is sent, fits
  W-V22-GJ   AI settings, AI bank tab: the jobs list is a paged table, cards on phones, the details toggle opens and closes
             (skipped when the stack has no job yet)
  W-V22-FT   footer disclaimer (and the shutdown button when there is one) is centered when it is short and not wider than the
             page when it wraps

Made-up accounts and data only; the screenshots stay in the output folder. Registered in checks.py (GROUPS_V22); run alone with
--only 'W-V22-*'.
"""
import re
import time

import seed
from checks import LANGS, SETUP, goto, phone, start_quiz, until
from checks_v13 import common

PHONES = [(393, 852), (360, 740), (320, 640)]
DESKTOP = (1280, 800)

DEFAULT_RE = re.compile(r'默认|Default|既定')

# Shared in-page helpers: visibility, boxes, overlaps of a list of elements
HELPERS = """
const vis = (e) => !!e && e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden' && getComputedStyle(e).display !== 'none';
const box = (e) => { const r = e.getBoundingClientRect(); return {l: r.left, t: r.top, r: r.right, b: r.bottom, w: r.width, h: r.height}; };
const inView = (r) => r.l >= -0.5 && r.r <= innerWidth + 0.5 && r.t >= -0.5 && r.b <= innerHeight + 0.5;
const nm = (e) => (e.id ? '#' + e.id : '') + (e.className && typeof e.className === 'string' ? '.' + e.className.trim().split(/\\s+/).join('.') : '') || e.tagName;
const overlaps = (els) => { const out = []; const bs = els.map((e) => [e, box(e)]);
  for (let i = 0; i < bs.length; i++) for (let j = i + 1; j < bs.length; j++) { const a = bs[i][1], b = bs[j][1];
    if (Math.min(a.r, b.r) - Math.max(a.l, b.l) > 0.5 && Math.min(a.b, b.b) - Math.max(a.t, b.t) > 0.5) out.push(nm(bs[i][0]) + ' x ' + nm(bs[j][0])); }
  return out; };
"""


def js(d, body):
    """Runs `body` (the text of a function body that returns a value) after the shared helpers."""
    return d.js('(() => { %s %s })()' % (HELPERS, body))


def go(rc, cid, page, fn, sizes):
    if not rc.want(cid) or not rc.walk.wants_page(page):
        return
    for lang in rc.langs(LANGS):
        for size in rc.sizes(sizes, lambda s: True):
            rc.guard(cid, page, fn, rc, lang, rc.themes(['light'])[0], size)


def skip(rc, cid, page, state, user, size, lang, theme, why):
    rc.rec(cid, page, state, user, size, lang, theme, None, why)


def shot_name(parts):
    return '; '.join(parts)


# ------------------------------------------------------------------------------------------------ W-V22-AB
ACCT_JS = """const b = document.getElementById('acct-btn'); if (!b) return null;
  const nameEl = b.querySelector('.acct-name'), ico = b.querySelector('.acct-ico'), badge = b.querySelector('.ai-badge'), role = b.querySelector('.acct-role');
  const parts = [ico, badge, role].filter((e) => vis(e)); const bb = box(b);
  const others = ['#menu-btn', '#theme-btn', '.logo', '.brand'].map((s) => document.querySelector(s)).filter((e) => vis(e) && !b.contains(e));
  const logo = document.querySelector('.top-brand .logo'), bname = document.querySelector('.top-brand .brand-name');
  return {logoShown: vis(logo) && box(logo).w > 1, brandNameShown: vis(bname) && box(bname).w > 1, nameShown: vis(nameEl) && box(nameEl).w > 1, icon: vis(ico), badge: vis(badge), role: vis(role),
    inside: inView(bb), h: bb.h, w: bb.w, partOverlap: overlaps(parts), partsOut: parts.filter((e) => { const r = box(e); return r.l < bb.l - 0.5 || r.r > bb.r + 0.5; }).map(nm),
    barOverlap: overlaps([b].concat(others)), scrollW: b.scrollWidth, clientW: b.clientWidth, menuOpen: !!document.getElementById('nav-menu')};"""

TIP_JS = """const w = document.querySelector('.acct-tipwrap'); const t = document.getElementById('acct-tip'); if (!w || !t) return null;
  const open = w.classList.contains('tip-open') && vis(t); const r = box(t);
  return {open, inside: open && inView(r), text: t.textContent.trim(), menuOpen: !!document.getElementById('nav-menu')};"""

# (user, AI+ badge expected (None = paused: only dark badge), administrator)
ACCT_USERS = [('admin', True, True), ('sam', True, False), ('alex', None, False), ('riley', False, False)]


def ab_case(rc, lang, theme, size):
    for user, ai, admin in ACCT_USERS:
        d = rc.dev(user, lang, theme, size)
        goto(d, '#/', 1.8)
        p = []
        a = js(d, ACCT_JS)
        if not a:
            skip(rc, 'W-V22-AB', 'home', 'account-' + user, user, size, lang, theme, 'no account button')
            continue
        if phone(size):
            if a['nameShown']:
                p.append('the user name is still shown on a phone')
            # v2.3 (owner): the brand is the word "Xexam" only on a phone; the logo icon shows on wider screens
            if a['logoShown']:
                p.append('the logo icon is shown in the top bar of a phone')
            if not a['brandNameShown']:
                p.append('the brand name Xexam is missing in the top bar of a phone')
            if not a['icon']:
                p.append('no person icon')
            if ai is True and not a['badge']:
                p.append('no AI+ badge')
            if admin and not a['role']:
                p.append('no admin tag')
            if a['partOverlap']:
                p.append('overlap inside the button: ' + ', '.join(a['partOverlap']))
            if a['partsOut']:
                p.append('sticks out of the button: ' + ', '.join(a['partsOut']))
            if a['barOverlap']:
                p.append('overlap in the top bar: ' + ', '.join(a['barOverlap']))
            if not a['inside']:
                p.append('the button is outside the screen')
            if a['h'] < 43.5:
                p.append('the button is %d px high' % a['h'])
            if a['scrollW'] > a['clientW'] + 1:
                p.append('the button content is wider than the button')
            rc.rec('W-V22-AB', 'home', 'account-' + user, user, size, lang, theme, not p, '; '.join(p) or 'icon, badge and tag without overlap, name hidden', d)
            # a tap: the name tip, not the menu
            q = []
            d.tap_sel('#acct-btn', wait=0.5)
            t = js(d, TIP_JS)
            if not t or not t['open']:
                q.append('a tap on the account button shows no tip')
            else:
                if not t['inside']:
                    q.append('the tip is outside the screen')
                want = seed.name_of(user)
                if want not in t['text']:
                    q.append('the tip does not name the user %r: %r' % (want, t['text'][:60]))
            if t and t['menuOpen']:
                q.append('a tap on the account button opened the menu on a phone')
            rc.rec('W-V22-AB', 'home', 'tip-' + user, user, size, lang, theme, not q, '; '.join(q) or 'tap shows the name tip', d)
            # a tap elsewhere closes it, Esc closes it
            r = []
            d.tap(size[0] / 2, size[1] * 0.6, wait=0.4)
            t2 = js(d, TIP_JS)
            if t2 and t2['open']:
                r.append('a tap elsewhere did not close the tip')
            d.tap_sel('#acct-btn', wait=0.5)
            d.key('Escape', 'Escape', 27)
            time.sleep(0.4)
            t3 = js(d, TIP_JS)
            if t3 and t3['open']:
                r.append('Esc did not close the tip')
            # the menu button opens the menu
            d.tap_sel('#menu-btn', wait=0.6)
            if not d.js("!!document.getElementById('nav-menu')"):
                r.append('the menu button did not open the menu')
            rc.rec('W-V22-AB', 'home', 'menu-' + user, user, size, lang, theme, not r,
                   '; '.join(r) or 'the tip closes on a tap elsewhere and on Esc, the menu button opens the menu', d)
        else:
            if not a['nameShown']:
                p.append('the user name is hidden on a desktop width')
            if not a['logoShown']:
                p.append('the logo icon is missing in the top bar on a desktop width')
            if not a['brandNameShown']:
                p.append('the brand name Xexam is missing in the top bar on a desktop width')
            d.tap_sel('#acct-btn', wait=0.6)
            if not d.js("!!document.getElementById('nav-menu')"):
                p.append('the account button did not open the menu on a desktop width')
            rc.rec('W-V22-AB', 'home', 'account-' + user, user, size, lang, theme, not p, '; '.join(p) or 'name shown, the button opens the menu', d)
        d.key('Escape', 'Escape', 27)
        time.sleep(0.3)


# ------------------------------------------------------------------------------------------------ W-V22-QT
QBAR_JS = """const bar = document.querySelector('.qbar'); if (!bar) return null; const r = bar.querySelector('.r'); const tb = document.getElementById('theme-btn');
  const kids = r ? [...r.children].filter(vis) : []; const ql = bar.querySelector('.ql');
  const els = kids.concat(ql && vis(ql) ? [ql] : []);
  // the area a thumb gets (the box plus an invisible ::after extension), like W-SH-3: elementFromPoint around the centre
  const hit = (e) => { const r = e.getBoundingClientRect(); const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    const inside = (x, y) => { const h = document.elementFromPoint(x, y); return !!h && (e === h || e.contains(h)); };
    if (!inside(cx, cy)) return {w: 0, h: 0};
    const reach = (dx, dy) => { let n = 0; while (n < 40 && inside(cx + dx * (n + 1), cy + dy * (n + 1))) n++; return n; };
    return {w: reach(-1, 0) + reach(1, 0) + 1, h: reach(0, -1) + reach(0, 1) + 1}; };
  return {has: !!tb && vis(tb), inRight: !!tb && !!r && r.contains(tb), label: tb ? (tb.getAttribute('aria-label') || '') : '', h: tb ? hit(tb).h : 0, w: tb ? hit(tb).w : 0,
    overlap: overlaps(els), out: els.filter((e) => box(e).l < -0.5 || box(e).r > innerWidth + 0.5).map(nm), sideways: document.documentElement.scrollWidth - innerWidth,
    bg: getComputedStyle(document.body).backgroundColor, timer: !!bar.querySelector('.qtime')};"""


def qt_case(rc, lang, theme, size):
    for kind in ('practice', 'exam'):
        d = rc.dev('sam', lang, theme, size)
        start_quiz(d, kind, 5)
        a = js(d, QBAR_JS)
        if not a:
            skip(rc, 'W-V22-QT', 'quiz', kind, 'sam', size, lang, theme, 'no quiz bar (the quiz did not start)')
            continue
        p = []
        if not a['has']:
            p.append('no theme toggle in the quiz top row')
            rc.rec('W-V22-QT', 'quiz', kind, 'sam', size, lang, theme, False, shot_name(p), d)
            continue
        if not a['inRight']:
            p.append('the toggle is not in the right group of the bar')
        if phone(size) and (a['h'] < 43.5 or a['w'] < 43.5):
            p.append('the toggle is %dx%d px' % (a['w'], a['h']))
        if a['overlap']:
            p.append('overlap in the top row: ' + ', '.join(a['overlap']))
        if a['out']:
            p.append('outside the screen: ' + ', '.join(a['out']))
        if a['sideways'] > 1:
            p.append('sideways scroll: %d px' % a['sideways'])
        p += ['measure ' + x for x in common(d, size)]
        rc.rec('W-V22-QT', 'quiz', kind, 'sam', size, lang, theme, not p, '; '.join(p) or 'toggle in the top row, no overlap%s' % (', with a timer' if a['timer'] else ''), d)
        # toggle: the background and the label change, a second tap restores them
        q = []
        d.tap_sel('#theme-btn', wait=0.7)
        b = js(d, QBAR_JS)
        if not b or b['bg'] == a['bg'] or b['label'] == a['label']:
            q.append('the toggle did not switch the look (bg %s -> %s, label %r -> %r)' % (a['bg'], b and b['bg'], a['label'], b and b['label']))
        elif b['overlap']:
            q.append('overlap after switching: ' + ', '.join(b['overlap']))
        rc.rec('W-V22-QT', 'quiz', kind + '-toggled', 'sam', size, lang, theme, not q, '; '.join(q) or 'toggle switches the look, no overlap', d)
        d.tap_sel('#theme-btn', wait=0.7)
        c = js(d, QBAR_JS)
        if c and (c['bg'] != a['bg'] or c['label'] != a['label']):
            rc.rec('W-V22-QT', 'quiz', kind + '-restore', 'sam', size, lang, theme, False, 'the second tap did not restore the look', d)


# ------------------------------------------------------------------------------------------------ W-V22-CS
COMPOSER_JS = """const q = (s) => document.querySelector(s); const br = q('#ai-model-btn'), ef = q('#ai-effort-btn'), send = q('#ai-send'), bar = q('#ai-composer .ai-bar'), box0 = q('#ai-composer .ai-box');
  if (!br || !bar) return null; const parts = [br, ef, send].filter(vis); const bb = box(bar).w > 0 ? box(bar) : box(box0);
  return {effort: !!ef && vis(ef), text: ef ? ef.textContent.trim() : '', label: ef ? (ef.getAttribute('aria-label') || '') : '', overlap: overlaps(parts),
    brainLeftOfEffort: !!ef && vis(ef) && box(br).r <= box(ef).l + 0.5, sameRow: !!ef && vis(ef) && Math.abs((box(br).t + box(br).b) / 2 - (box(ef).t + box(ef).b) / 2) <= 2,
    effortH: ef && vis(ef) ? box(ef).h : 0, effortW: ef && vis(ef) ? box(ef).w : 0, compact: !!box0 && box0.classList.contains('compact'), inBar: parts.every((e) => { const r = box(e); return r.l >= bb.l - 0.5 && r.r <= bb.r + 0.5; }),
    barScroll: bar.scrollWidth - bar.clientWidth, boxScroll: box0 ? box0.scrollWidth - box0.clientWidth : 0, sideways: document.documentElement.scrollWidth - innerWidth};"""

POP_JS = """const pop = document.getElementById('ai-effort-pop'); if (!pop) return null; const items = [...pop.querySelectorAll('.ai-effort-item')]; const pb = box(pop);
  return {n: items.length, first: items[0] ? items[0].textContent.trim() : '', checked: items.filter((e) => e.getAttribute('aria-checked') === 'true').length,
    popInside: inView(pb), itemsOut: items.filter((e) => { const r = box(e); return r.l < pb.l - 0.5 || r.r > pb.r + 0.5 || !inView(r); }).length,
    smallest: Math.min.apply(null, items.map((e) => box(e).h)), scrollW: pop.scrollWidth - pop.clientWidth, sideways: document.documentElement.scrollWidth - innerWidth,
    expanded: (document.getElementById('ai-effort-btn') || {getAttribute: () => null}).getAttribute('aria-expanded')};"""


SECTION_JS = """const pop = document.getElementById('ai-model-pop'); const sel = document.getElementById('ai-model-str');
  if (!pop || !sel) return null; const pb = box(pop); const sb = box(sel); const lab = document.getElementById('ai-model-str-l');
  const cn = document.getElementById('ai-model-conn'); const ms = document.getElementById('ai-model-sel');
  return {pop: true, n: sel.options.length, first: sel.options[0].textContent.trim(), value: sel.value, h: sb.h, w: sb.w, label: lab ? lab.textContent.trim() : '',
    sameStyle: !!cn && getComputedStyle(cn).fontSize === getComputedStyle(sel).fontSize && getComputedStyle(cn).borderTopWidth === getComputedStyle(sel).borderTopWidth && Math.abs(box(cn).h - sb.h) < 1,
    after: !!ms && box(ms).b <= sb.t + 1, inside: sb.l >= pb.l - 0.5 && sb.r <= pb.r + 0.5, sideways: document.documentElement.scrollWidth - innerWidth};"""


def cs_case(rc, lang, theme, size):
    s = rc.session('admin')
    d = rc.dev('admin', lang, theme, size)
    was = s.call('/api/ai/status').get('free')
    s.call('/api/ai/free', {'free': True})
    try:
        goto(d, '#/ai', 2.2)
        d.js("(() => { const b = document.querySelector('.ai-new, #ai-new'); if (b) b.click(); return 1 })()")
        time.sleep(1.2)
        if phone(size):
            # owner (v2.3): in the compact one-row box the strength is the last section of the brain picker; there is no button
            c0 = js(d, COMPOSER_JS)
            if not c0:
                skip(rc, 'W-V22-CS', 'ai', 'composer', 'admin', size, lang, theme, 'no composer on the AI Chat page')
                return
            k = []
            if not c0['compact']:
                k.append('the empty box is not compact')
            if c0['effort']:
                k.append('the separate strength button is shown in the compact row')
            d.tap_sel('#ai-model-btn', wait=0.7)
            m0 = js(d, SECTION_JS)
            if not m0:
                k.append('the brain picker has no strength dropdown in the compact row')
            else:
                if not DEFAULT_RE.search(m0['first']):
                    k.append('the first strength option is not Default: %r' % m0['first'])
                if m0['n'] < 2:
                    k.append('%d strength options' % m0['n'])
                if not m0['sameStyle']:
                    k.append('the strength dropdown does not look like the API / model dropdowns')
                if not m0['after']:
                    k.append('the strength dropdown is not below the model dropdown')
                if m0['h'] < 36:
                    k.append('the strength dropdown is %d px high' % m0['h'])
                if not m0['inside'] or m0['sideways'] > 1:
                    k.append('the strength dropdown sticks out of the picker / the page')
                d.js("(() => { const s = document.getElementById('ai-model-str'); s.selectedIndex = 1; s.dispatchEvent(new Event('change', {bubbles: true})); return 1 })()")
                time.sleep(0.5)
                m1 = js(d, SECTION_JS)
                if not m1:
                    k.append('the picker closed after a strength pick')
                elif m1['value'] == m0['value']:
                    k.append('the pick did not change the strength dropdown')
            rc.rec('W-V22-CS', 'ai', 'compact-picker', 'admin', size, lang, theme, not k, '; '.join(k) or 'strength is the last section of the brain picker, 44 px items, no button in the row', d)
            d.key('Escape', 'Escape', 27)
            time.sleep(0.4)
            d.js("(() => { const p = document.getElementById('ai-model-pop'); const b = document.getElementById('ai-model-close'); if (p && b) b.click(); return 1 })()")
            # two lines of text expand the box: the labelled strength button appears, the picker has no strength section any more
            d.js("(() => { const t = document.getElementById('ai-input'); if (t) { t.focus(); t.value = 'a' + String.fromCharCode(10) + 'b'; t.dispatchEvent(new Event('input', {bubbles: true})); } return 1 })()")
            time.sleep(0.6)
            d.tap_sel('#ai-model-btn', wait=0.7)
            m2 = js(d, SECTION_JS)
            if m2:
                rc.rec('W-V22-CS', 'ai', 'expanded-picker', 'admin', size, lang, theme, False, 'the brain picker still has the strength section in the expanded box', d)
            d.js("(() => { const b = document.getElementById('ai-model-close'); if (b) b.click(); return 1 })()")
            time.sleep(0.4)
        c = js(d, COMPOSER_JS)
        if not c:
            skip(rc, 'W-V22-CS', 'ai', 'composer', 'admin', size, lang, theme, 'no composer on the AI Chat page')
            return
        p = []
        if not c['effort']:
            p.append('no strength button for an administrator in free mode')
            rc.rec('W-V22-CS', 'ai', 'composer', 'admin', size, lang, theme, False, shot_name(p), d)
            return
        if not c['brainLeftOfEffort'] or not c['sameRow']:
            p.append('the strength button is not beside the brain button')
        if c['overlap']:
            p.append('overlap in the composer bar: ' + ', '.join(c['overlap']))
        if not c['inBar']:
            p.append('a composer button sticks out of the bar')
        if c['barScroll'] > 1 or c['boxScroll'] > 1 or c['sideways'] > 1:
            p.append('composer overflow (bar %d, box %d, page %d px)' % (c['barScroll'], c['boxScroll'], c['sideways']))
        if phone(size) and c['effortH'] < 36:
            p.append('the strength button is %d px high' % c['effortH'])
        if phone(size) and c['compact'] and (c['effortH'] < 43.5 or c['effortW'] < 43.5 or c['effortW'] > 60):
            p.append('the icon-only strength button of the compact row is %dx%d px (want 44x44)' % (c['effortW'], c['effortH']))
        if not c['label'] or not c['text']:
            p.append('the strength button has no text or no name (%r / %r)' % (c['text'], c['label']))
        p += ['measure ' + x for x in common(d, size)]
        rc.rec('W-V22-CS', 'ai', 'composer', 'admin', size, lang, theme, not p, '; '.join(p) or 'strength button beside the brain button, no overflow', d)
        # the menu
        q = []
        d.tap_sel('#ai-effort-btn', wait=0.6)
        m = js(d, POP_JS)
        if not m:
            q.append('the strength menu did not open')
        else:
            if m['expanded'] != 'true':
                q.append('aria-expanded is %r while the menu is open' % m['expanded'])
            if not DEFAULT_RE.search(m['first']):
                q.append('the first item is not Default: %r' % m['first'])
            if m['n'] < 2:
                q.append('%d items only' % m['n'])
            if m['checked'] != 1:
                q.append('%d items are checked (want 1)' % m['checked'])
            if not m['popInside'] or m['itemsOut']:
                q.append('the menu or %d item(s) outside the screen / the menu' % m['itemsOut'])
            if phone(size) and m['smallest'] < 40:
                q.append('an item is %d px high' % m['smallest'])
            if m['scrollW'] > 1 or m['sideways'] > 1:
                q.append('the menu scrolls sideways')
        rc.rec('W-V22-CS', 'ai', 'menu', 'admin', size, lang, theme, not q, '; '.join(q) or 'menu with Default first, %s items' % (m and m['n']), d)
        d.key('Escape', 'Escape', 27)
        time.sleep(0.4)
        if d.js("!!document.getElementById('ai-effort-pop')"):
            rc.rec('W-V22-CS', 'ai', 'esc', 'admin', size, lang, theme, False, 'Esc did not close the strength menu', d)
    finally:
        s.call('/api/ai/free', {'free': bool(was)})


# ------------------------------------------------------------------------------------------------ W-V22-MS
ROWS_JS = """const sels = [...document.querySelectorAll('.ais-effort-level')]; return sels.map((s) => { const r = box(s); const row = s.closest('.ais-effort') || s.parentElement;
  const hint = row ? row.querySelector('.ais-effort-hint') : null; const rb = row ? box(row) : r;
  return {id: s.id, first: s.options[0] ? s.options[0].value : null, firstText: s.options[0] ? s.options[0].textContent.trim() : '', n: s.options.length, value: s.value,
    shown: vis(s), inside: r.l >= -0.5 && r.r <= innerWidth + 0.5, rowScroll: row ? row.scrollWidth - row.clientWidth : 0,
    hint: hint ? vis(hint) : false, hintOverlap: hint && vis(hint) ? overlaps([s, hint]).length : 0, h: r.h, rowOut: rb.r > innerWidth + 0.5}; });"""


def ms_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings', 2.8)
    rows = js(d, ROWS_JS) or []
    if not rows:
        skip(rc, 'W-V22-MS', 'ai-settings', 'models', 'admin', size, lang, theme, 'no strength dropdowns (no effort levels on this stack)')
        return
    p = []
    for r in rows:
        tag = r['id'] or 'row'
        if r['first'] != '' or not DEFAULT_RE.search(r['firstText']):
            p.append('%s: first option is %r / %r, not Default' % (tag, r['first'], r['firstText']))
        if r['n'] < 2:
            p.append('%s: only %d option(s)' % (tag, r['n']))
        if not r['shown'] or not r['inside'] or r['rowOut']:
            p.append('%s: dropdown hidden or outside the screen' % tag)
        if r['rowScroll'] > 1:
            p.append('%s: the row scrolls sideways (%d px)' % (tag, r['rowScroll']))
        # v2.2 (owner): the explanation is part of the option text itself, in brackets
        if not ('(' in r['firstText'] or '（' in r['firstText']):
            p.append('%s: the Default option does not say that no parameter is sent: %r' % (tag, r['firstText']))
        if phone(size) and r['h'] < 36:
            p.append('%s: dropdown %d px high' % (tag, r['h']))
    d.js("(() => { const s = document.querySelector('.ais-effort-level'); if (s) s.scrollIntoView({block: 'center'}); return 1 })()")
    time.sleep(0.4)
    p += ['measure ' + x for x in common(d, size) if 'ptb-sort' not in x.split(';')[0]]
    rc.rec('W-V22-MS', 'ai-settings', 'models', 'admin', size, lang, theme, not p, '; '.join(p) or '%d strength dropdowns start with Default (no parameter sent)' % len(rows), d)


# ------------------------------------------------------------------------------------------------ W-V22-GJ
JOBS_JS = """const root = document.getElementById('ai-genjobs'); if (!root) return null; const wrap = root.closest('.ptb') || root.parentElement;
  const rows = [...document.querySelectorAll('.ais-genjob')]; const first = rows[0];
  return {jobs: rows.length, ptb: !!document.querySelector('.ptb'), search: !!document.getElementById('ai-genjobs-q'), sort: !!document.getElementById('ai-genjobs-sort'),
    pager: !!document.querySelector('.ptb-pager'), display: first ? getComputedStyle(first).display : '', toggles: document.querySelectorAll('.ais-genmore-b').length,
    sideways: document.documentElement.scrollWidth - innerWidth, wrapScroll: wrap ? wrap.scrollWidth - wrap.clientWidth : 0};"""

DETAIL_JS = """const b = [...document.querySelectorAll('.ais-genmore-b')].filter(vis)[0]; if (!b) return null;
  const row = b.closest('tr'); const nxt = row ? row.nextElementSibling : null; const full = nxt && nxt.classList.contains('ais-genfull') ? nxt : null;
  const tb = box(b); const fb = full && vis(full) ? box(full) : null;
  return {expanded: b.getAttribute('aria-expanded'), shown: full ? vis(full) : null, wide: fb ? fb.r - innerWidth : 0, fullScroll: fb ? full.scrollWidth - full.clientWidth : 0,
    h: tb.h, w: tb.w, fields: full ? full.querySelectorAll('dt').length : 0};"""


def gj_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/ai-settings?tab=gen', 2.8)
    until(d, "document.getElementById('ai-genjobs') || document.querySelector('.ais-note')", 6)
    j = js(d, JOBS_JS)
    if not j:
        skip(rc, 'W-V22-GJ', 'ai-settings', 'jobs', 'admin', size, lang, theme, 'no jobs list on the page (the stack has no generation or import job yet)')
        return
    p = []
    if not j['ptb'] or not j['search'] or not j['sort']:
        p.append('the jobs list is not a paged table with search and sort (paged %s, search %s, sort %s)' % (j['ptb'], j['search'], j['sort']))
    if j['jobs'] and j['display'] == 'table-row' and size[0] < 900:
        p.append('on a phone the jobs are table rows, not cards')
    if j['sideways'] > 1 or j['wrapScroll'] > 1:
        p.append('sideways overflow (page %d, table %d px)' % (j['sideways'], j['wrapScroll']))
    if j['jobs'] > 10 and not j['pager']:
        p.append('%d jobs on one page without a pager' % j['jobs'])
    if j['jobs'] and not j['toggles']:
        p.append('no details toggle on the jobs')
    p += ['measure ' + x for x in common(d, size) if 'ptb-sort' not in x.split(';')[0]]
    d.js("(() => { const e = document.getElementById('ai-genjobs'); if (e) e.scrollIntoView({block: 'start'}); return 1 })()")
    time.sleep(0.3)
    rc.rec('W-V22-GJ', 'ai-settings', 'jobs', 'admin', size, lang, theme, not p, '; '.join(p) or '%d jobs on the page, paged table, cards on phones' % j['jobs'], d)
    if not j['toggles']:
        return
    # the details toggle: open, then close
    q = []
    d.tap_sel('.ais-genmore-b', wait=0.6)
    o = js(d, DETAIL_JS)
    if not o or o['expanded'] != 'true' or not o['shown']:
        q.append('the details toggle did not open the details (%s)' % (o and (o['expanded'], o['shown']),))
    else:
        if o['fields'] < 3:
            q.append('only %d detail fields' % o['fields'])
        if o['wide'] > 0.5 or o['fullScroll'] > 1:
            q.append('the details stick out of the screen (%d px)' % max(o['wide'], o['fullScroll']))
        if phone(size) and (o['h'] < 40 or o['w'] < 40):
            q.append('the toggle is %dx%d px' % (o['w'], o['h']))
        q += ['open ' + x for x in common(d, size) if 'ptb-sort' not in x.split(';')[0]]
    rc.rec('W-V22-GJ', 'ai-settings', 'details-open', 'admin', size, lang, theme, not q, '; '.join(q) or 'details open, %s fields' % (o and o['fields']), d)
    d.tap_sel('.ais-genmore-b', wait=0.6)
    c = js(d, DETAIL_JS)
    if c and (c['expanded'] != 'false' or c['shown']):
        rc.rec('W-V22-GJ', 'ai-settings', 'details-closed', 'admin', size, lang, theme, False,
               'the details toggle did not close the details (%s)' % ((c['expanded'], c['shown']),), d)


# ------------------------------------------------------------------------------------------------ W-V22-FT
FOOT_JS = """const f = document.querySelector('.foot'); const n = document.querySelector('.foot-note'); if (!f || !n) return null; const fb = box(f), nb = box(n);
  const cs = getComputedStyle(f); const inL = fb.l + (parseFloat(cs.paddingLeft) || 0), inR = fb.r - (parseFloat(cs.paddingRight) || 0);
  const lineH = parseFloat(getComputedStyle(n).lineHeight) || 20; const lines = Math.round(nb.h / lineH);
  const btn = f.querySelector('.btn.quiet'); const bb = btn && vis(btn) ? box(btn) : null;
  return {lines, left: nb.l - inL, right: inR - nb.r, wide: nb.r > innerWidth + 0.5 || nb.l < -0.5, align: getComputedStyle(n).textAlign,
    btn: bb ? {left: bb.l - inL, right: inR - bb.r} : null, sideways: document.documentElement.scrollWidth - innerWidth};"""


def ft_case(rc, lang, theme, size):
    for route, page in ((SETUP, 'home'), ('#/settings', 'settings')):
        if not rc.walk.wants_page(page):
            continue
        d = rc.dev('sam', lang, theme, size)
        goto(d, route, 1.8)
        f = js(d, FOOT_JS)
        if not f:
            skip(rc, 'W-V22-FT', page, 'footer', 'sam', size, lang, theme, 'no footer on %s' % route)
            continue
        q = []
        if f['wide']:
            q.append('the disclaimer sticks out of the screen')
        if f['sideways'] > 1:
            q.append('sideways scroll: %d px' % f['sideways'])
        if f['lines'] <= 1:
            if abs(f['left'] - f['right']) > 1.5:
                q.append('a one-line disclaimer is not centered (left gap %.1f, right gap %.1f px)' % (f['left'], f['right']))
        else:
            if f['left'] < -0.5 or f['right'] < -0.5:
                q.append('the wrapped disclaimer is wider than the page')
            if f['align'] == 'center':
                q.append('the wrapped disclaimer is centered line by line (it must read left to right)')
        if f['btn'] and abs(f['btn']['left'] - f['btn']['right']) > 1.5:
            q.append('the shutdown button is not centered (%.1f / %.1f px)' % (f['btn']['left'], f['btn']['right']))
        d.js("window.scrollTo(0, document.body.scrollHeight); 1")
        time.sleep(0.3)
        rc.rec('W-V22-FT', page, 'footer', 'sam', size, lang, theme, not q,
               '; '.join(q) or '%d line(s), gaps %.1f / %.1f px' % (f['lines'], f['left'], f['right']), d)


# ------------------------------------------------------------------------------------------------ run
def g_v22(rc):
    go(rc, 'W-V22-AB', 'home', ab_case, PHONES + [DESKTOP])
    go(rc, 'W-V22-QT', 'quiz', qt_case, PHONES + [DESKTOP])
    go(rc, 'W-V22-CS', 'ai', cs_case, PHONES + [DESKTOP])
    go(rc, 'W-V22-MS', 'ai-settings', ms_case, PHONES + [DESKTOP])
    go(rc, 'W-V22-GJ', 'ai-settings', gj_case, PHONES + [DESKTOP])
    go(rc, 'W-V22-FT', 'home', ft_case, PHONES + [DESKTOP])


GROUPS_V22 = [g_v22]
