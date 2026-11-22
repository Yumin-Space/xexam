# -*- coding: utf-8 -*-
"""v2.6 walk checks: the final Home (hand-written mode on by default: hero, six tiles three per row, the leaderboard line chart,
the most-missed parabola board, the folded AI card last), the Profile page, centred narrow pages, the paged System log and the
first-sign-in name screens. Phone sizes 320 / 375 / 430 px wide plus a desktop width; the matrix follows --langs / --themes /
--widths (by default the three languages and both themes). Every failing case keeps a screenshot in the output folder.

  W-V26-RANK Home leaderboard (user lee): a hand-drawn line chart (.dash-rank .dash-pt buttons): every point >= 44 x 44 px, inside
             the width, no overlap, an aria-label each, place 1 highest; every SVG text stays inside the board; before a tap the
             hint shows, a tap shows the full line (.dash-rank-pick, no @) and presses exactly one point
  W-V26-POD  Home most-missed parabola board (lee: five places, .dash-wrong .dash-pt): each point a button >= 44 x 44 px, no
             overlap, nothing cut or outside the width, every SVG text inside the board; a tap on place 1 opens the question
             under the board (stem + the correct answer + "bank, sheet, answered, wrong" line), a second tap closes it again
             (the hint comes back); on a desktop Escape clears a pinned pick
  W-V26-CAP  AI study assessment card (lee has one saved text, made through the made-up provider on first use): it is the LAST block
             (after the most-missed board), folded on arrival (aria-expanded false, body hidden), the toggle is >= 44 px high and
             inside the card, title + AI+ badge do not overlap; opening it shows the whole text and a >= 44 px action button inside
             the card, never a model name; leaving Home and coming back folds it again; a user without AI+ and without a saved text
             (riley) has no card
  W-V26-TL   page tiles: 5 or 6 tiles are rows of three (a wide screen may also show one row); 4 tiles are a 2 x 2 grid up to 480 px
             wide and one row wider; fewer tiles one row; the order is records, wrong answers, AI Chat, banks, settings, profile;
             every tile >= 88 px high, nothing sticks out; hand mode is on by default (html[data-hand]) with the Aa switch visible
  W-V26-PRF  Profile page (#/profile, user sam): account, username and password sections; every control is >= 44 px high on phones,
             nothing overflows, the password row opens its three fields in place; Settings has no account section any more; the menu's
             user row is a >= 44 px link to #/profile
  W-V26-DSK  Home layout (user lee): at 1280 x 800 the tiles are ONE row, the leaderboard and the most-missed panels sit side by side
             (equal columns, no overlap, boards inside their panel), the hero is above the tiles, the AI card below and as wide as the
             page, the page is at most 1080 px wide and centred, nothing overflows; at 390 px everything is one column
  W-V26-CTR  desktop 1280 / 1440 px: Settings, Profile, Users (administrator), AI settings and About are centred in the page (equal
             space on both sides)
  W-V26-LOG  System log (administrator): a pager shows on at least one of the two tabs and fits (inside the width, buttons >= 44 px
             on phones, no overlap, no sideways scroll)
  W-V26-NAME first sign-in by e-mail: a new user (initial password) sees the new-password page with the username field above the
             password fields; a user who only lacks a username sees the name-only page (one field, Save, Sign out). Controls >= 44 px,
             nothing sticks out, a too long name and a name with an @ each show an inline error. The name-only page needs the
             user's name removed from the database, which works with the cargo backend only (skipped with the Docker backend)

Made-up accounts and data only. Registered in checks.py (GROUPS_V26); run alone with --only 'W-V26-*'.
"""
import json
import os
import sqlite3
import time

import seed
from checks import goto, phone, until
from checks_v13 import common, post_stream, verdict
from checks_v25 import DESKTOP, PHONES, READY, cases
from checks_v22 import js

# Wide screens the centring check also looks at
DESKTOPS = [(1280, 800), (1440, 900)]

# Shared: every SVG text of a board stays inside the board's box (a label that leaves it is cut by the board)
BOARD_TEXT_OUT = """
const textOut = (root) => { const bd = document.querySelector(root + ' .dash-board'); if (!bd) return 0; const bb = box(bd);
  return [...bd.querySelectorAll('svg text')].filter((e) => { const r = e.getBoundingClientRect();
    return r.width > 0 && (r.left < bb.l - 1 || r.right > bb.r + 1 || r.top < bb.t - 1 || r.bottom > bb.b + 1); }).length; };
"""

# The points of one board (ROOT = '.dash-rank' or '.dash-wrong'): size, overlap, order, aria, bounds
BOARD_JS = BOARD_TEXT_OUT + """
const root = 'ROOT'; const bd = document.querySelector(root + ' .dash-board'); if (!bd) return null;
const pts = [...document.querySelectorAll(root + ' .dash-pt')].filter(vis);
const bs = pts.map(box); const bb = box(bd);
return {n: pts.length, minW: bs.length ? Math.round(Math.min(...bs.map((b) => b.w))) : 0, minH: bs.length ? Math.round(Math.min(...bs.map((b) => b.h))) : 0,
  overlap: overlaps(pts), noAria: pts.filter((e) => !e.getAttribute('aria-label')).length, tops: bs.map((b) => Math.round(b.t)),
  left: bs.length ? Math.min(...bs.map((b) => b.l)) : 0, right: bs.length ? Math.max(...bs.map((b) => b.r)) : 0, vw: innerWidth,
  svgOut: (() => { const sv = bd.querySelector('svg'); if (!sv) return true; const b = box(sv); return b.r > innerWidth + 0.5 || b.l < -0.5; })(),
  textOut: textOut(root), boardBottom: bb.b, sideways: document.documentElement.scrollWidth - innerWidth};"""

# The detail under a board: the shown question (most-missed board) or the shown line (leaderboard)
PICK_JS = """
const root = 'ROOT'; const r = document.querySelector(root + ' .dash-pick'); if (!r) return null;
const lay = r.querySelector('.dash-pick-layer.on') || r; const q = (s) => lay.querySelector(s);
const pressed = [...document.querySelectorAll(root + ' .dash-pt')].filter((e) => e.getAttribute('aria-pressed') === 'true').length;
const b = box(r); const line = q('.dash-rank-line');
return {stem: q('.dash-pick-stem') ? q('.dash-pick-stem').textContent.trim() : '', ans: q('.dash-pick-ans-l') ? q('.dash-pick-ans-l').textContent.trim() : '',
  meta: q('.dash-pick-meta') ? q('.dash-pick-meta').textContent.trim() : '', line: line ? line.textContent.trim() : '', hint: !!q('.dash-pick-hint'),
  pressed, l: b.l, r: b.r, t: b.t, live: r.getAttribute('aria-live') || ((r.querySelector('[aria-live]') || {}).getAttribute ? r.querySelector('[aria-live]').getAttribute('aria-live') : null), boardBottom: box(document.querySelector(root + ' .dash-board')).b,
  cut: r.scrollWidth > r.clientWidth + 1, sideways: document.documentElement.scrollWidth - innerWidth};"""

CAP_JS = """
const c = document.querySelector('.dash-ai'); if (!c) return null;
const page = document.querySelector('.page-dash'), wrong = document.querySelector('.dash-wrong');
const title = c.querySelector('.dash-ai-title'), badge = c.querySelector('.dash-ai-toggle .ai-badge'), tog = c.querySelector('.dash-ai-toggle');
const body = document.getElementById('dash-ai-body');
const text = c.querySelector('.dash-ai-text'), btn = c.querySelector('.dash-ai-btn, .aip-sum-btn');
const cb = box(c), tb = tog ? box(tog) : null;
const btnb = btn ? box(btn) : null;
return {l: cb.l, r: cb.r, t: cb.t, b: cb.b, vw: innerWidth, sideways: document.documentElement.scrollWidth - innerWidth,
  afterWrong: wrong ? cb.t >= box(wrong).b - 0.5 : null, last: !!page && page.lastElementChild === c,
  tog: !!tog, togH: tb ? Math.round(tb.h) : 0, togInside: !!tb && tb.l >= cb.l - 0.5 && tb.r <= cb.r + 0.5,
  expanded: tog ? tog.getAttribute('aria-expanded') : null, bodyHidden: !!body && body.hidden,
  headOverlap: title && badge ? overlaps([title, badge]) : [], badgeInside: !badge || (box(badge).r <= (tb ? tb.r : cb.r) + 0.5),
  titleCut: !!title && title.scrollWidth > title.clientWidth + 1, text: text ? text.textContent.trim() : '', textDim: !!text && text.classList.contains('dim'),
  textCut: !!text && text.scrollWidth > text.clientWidth + 1, btn: !!btn, btnH: btnb ? Math.round(btnb.h) : 0, btnW: btnb ? Math.round(btnb.w) : 0,
  btnInside: !btnb || (btnb.l >= cb.l - 0.5 && btnb.r <= cb.r + 0.5 && btnb.b <= cb.b + 0.5),
  btnDisabled: !!btn && btn.getAttribute('aria-disabled') === 'true', btnText: btn ? btn.textContent.trim() : '',
  note: (c.querySelector('.dash-note') || {}).textContent || '', model: /demo-|gpt|claude|gemini/i.test(c.textContent || '')};"""

TILES_JS = """
const row = document.querySelector('.dash-row'); if (!row) return null;
const tiles = [...row.querySelectorAll('.dash-tile')].filter(vis); const bs = tiles.map(box);
const uniq = (xs) => [...new Set(xs.map((x) => Math.round(x)))];
const hand = document.querySelector('.hand-btn');
return {n: tiles.length, dataN: row.getAttribute('data-n'), tops: uniq(bs.map((b) => b.t)), lefts: uniq(bs.map((b) => b.l)),
  hrefs: tiles.map((e) => e.getAttribute('href')), hand: document.documentElement.hasAttribute('data-hand'), handBtn: !!hand && vis(hand),
  minH: Math.round(Math.min(...bs.map((b) => b.h))), minW: Math.round(Math.min(...bs.map((b) => b.w))), overlap: overlaps(tiles),
  out: bs.some((b) => b.r > innerWidth + 0.5 || b.l < -0.5), sideways: document.documentElement.scrollWidth - innerWidth, vw: innerWidth,
  cut: tiles.filter((e) => { const l = e.querySelector('.dash-tile-label'); return l && l.scrollWidth > l.clientWidth + 1; }).length};"""

# The owner's tile order (the ones a user cannot see are left out)
TILE_ORDER = ['#/records', '#/records/wrong', '#/ai', '#/banks', '#/settings', '#/profile']

# Every control of the profile page: its height, and whether it is inside the screen width
PROFILE_JS = """
const page = document.querySelector('.page-profile'); if (!page) return null;
const ctl = [...page.querySelectorAll('button, a[href], input:not([type=hidden]):not(.sr-only), select, textarea')].filter(vis);
const small = ctl.filter((e) => box(e).h < %d - 0.5).map((e) => nm(e) + ' ' + Math.round(box(e).h) + 'px');
const out = ctl.filter((e) => { const b = box(e); return b.r > innerWidth + 0.5 || b.l < -0.5; }).map(nm);
return {n: ctl.length, small, out, sideways: document.documentElement.scrollWidth - innerWidth, secs: page.querySelectorAll('.pf-sec').length,
  login: (page.querySelector('.pf-login') || {}).textContent || '', nameInput: !!page.querySelector('#pf-name'),
  pwToggle: !!page.querySelector('#pf-pw-toggle'), account: !!document.querySelector('#pf-account-h')};"""

PW_JS = """
const f = document.querySelector('#pf-pw-form'); if (!f) return null;
const fields = [...f.querySelectorAll('input:not(.sr-only)')].filter(vis);
const bs = fields.map(box);
return {open: !f.hidden && vis(f), n: fields.length, minH: Math.round(Math.min(...bs.map((b) => b.h))),
  out: bs.some((b) => b.r > innerWidth + 0.5 || b.l < -0.5), overlap: overlaps(fields), sideways: document.documentElement.scrollWidth - innerWidth};"""

SETTINGS_JS = """
return {page: !!document.querySelector('.page-settings'), account: !!document.querySelector('.set-username, .set-pw, #set-pw-form, [id^="set-pw"]'),
  sideways: document.documentElement.scrollWidth - innerWidth};"""

MENU_ROW_JS = """
const a = document.querySelector('.nav-account a.nav-who, .nav-pop a.nav-who'); if (!a) return null;
const b = box(a); const out = document.querySelector('.nav-account button, .nav-pop .nav-account .btn');
return {h: Math.round(b.h), w: Math.round(b.w), href: a.getAttribute('href'), inside: b.r <= innerWidth + 0.5 && b.l >= -0.5,
  signOut: !!out};"""

# The narrow page's left and right space inside the viewport (a page that sits left has the left gap much smaller)
CENTRE_JS = """
const p = document.querySelector(%s); if (!p) return null;
const b = box(p); return {l: Math.round(b.l), r: Math.round(innerWidth - b.r), w: Math.round(b.w), vw: innerWidth};"""

PAGER_JS = """
const out = []; const tabs = [...document.querySelectorAll('.lg-tabs button')];
const pagers = [...document.querySelectorAll('.ptb-pager, .lg-pager, nav[class*="pager"], [class*="pager"]')].filter(vis);
const p = pagers[0]; if (!p) return {found: false, tabs: tabs.length};
const pb = box(p); const ctl = [...p.querySelectorAll('button, a')].filter(vis); const cb = ctl.map(box);
return {found: true, tabs: tabs.length, l: pb.l, r: pb.r, vw: innerWidth, ctl: ctl.length, minH: cb.length ? Math.round(Math.min(...cb.map((b) => b.h))) : 0,
  minW: cb.length ? Math.round(Math.min(...cb.map((b) => b.w))) : 0, overlap: overlaps(ctl), text: p.textContent.replace(/\\s+/g, ' ').trim().slice(0, 60),
  out: cb.some((b) => b.r > innerWidth + 0.5 || b.l < -0.5), sideways: document.documentElement.scrollWidth - innerWidth,
  disabled: ctl.filter((e) => e.getAttribute('aria-disabled') === 'true' || e.disabled).length};"""


def ensure_assess(rc):
    """lee (an administrator, AI+ always, 60 records) gets one saved assessment through the made-up provider, once per run. The
    opened AI card then shows a text and a "wait until tomorrow" button. Returns a short state text for the report."""
    if 'v26-assess' in rc.cache:
        return rc.cache['v26-assess']
    state = 'none'
    try:
        s = rc.session('lee')
        cur = s.call('/api/ai/assess/latest')
        if cur.get('assess'):
            state = 'saved'
        elif cur.get('remaining') != 0 and not cur.get('running'):
            post_stream(rc, 'lee', '/api/ai/assess/ask', {'lang': 'en'})
            state = 'made' if s.call('/api/ai/assess/latest').get('assess') else 'not saved'
    except Exception as e:  # the checks below then judge the card that is there
        state = 'error %s' % str(e)[:60]
    rc.cache['v26-assess'] = state
    return state


# ------------------------------------------------------------------------------------------------ W-V26-RANK
def board_problems(r, what):
    """The checks both boards share: size, overlap, aria-labels, inside the width, SVG texts inside the board."""
    p = []
    if r['minW'] < 44 or r['minH'] < 44:
        p.append('%s: a point is %d x %d px (< 44)' % (what, r['minW'], r['minH']))
    p += ['%s overlap: %s' % (what, o) for o in (r['overlap'] or [])[:3]]
    if r['noAria']:
        p.append('%s: %d points without an aria-label' % (what, r['noAria']))
    if r['right'] > r['vw'] + 0.5 or r['left'] < -0.5 or r['svgOut']:
        p.append('%s sticks out (%d..%d of %d px)' % (what, r['left'], r['right'], r['vw']))
    if r['textOut']:
        p.append('%s: %d texts leave the board' % (what, r['textOut']))
    if r['sideways'] > 1:
        p.append('the page is %d px wider than the viewport' % r['sideways'])
    return p


def rank_case(rc, lang, theme, size):
    d = rc.dev('lee', lang, theme, size)
    goto(d, '#/', 1.8)
    until(d, READY, 8.0)
    until(d, "document.querySelector('.dash-rank .dash-pt')", 6.0)
    time.sleep(0.5)
    r = js(d, BOARD_JS.replace('ROOT', '.dash-rank'))
    if not r or not r['n']:
        rc.rec('W-V26-RANK', 'home', 'chart', 'lee', size, lang, theme, False, 'no leaderboard points on #/ (.dash-rank .dash-pt)', d)
        return
    p = board_problems(r, 'leaderboard')
    if r['tops'] != sorted(r['tops']):
        p.append('place 1 is not the highest point (tops %s)' % r['tops'])
    state = 'chart'
    if not p:
        before = js(d, PICK_JS.replace('ROOT', '.dash-rank'))
        if not before or not before['hint']:
            p.append('before any tap the detail does not show the hint')
        d.tap_sel('.dash-rank .dash-pt', 0)
        time.sleep(0.4)
        after = js(d, PICK_JS.replace('ROOT', '.dash-rank'))
        if not after or not after['line']:
            p.append('a tap on place 1 did not show its line')
        else:
            state = 'detail'
            if '@' in after['line']:
                p.append('the detail line contains an @ (%s)' % after['line'][:40])
            if after['pressed'] != 1:
                p.append('%d points are pressed after one tap (want 1)' % after['pressed'])
            if after['live'] != 'polite':
                p.append('the detail region is not aria-live polite')
            if after['t'] < after['boardBottom'] - 0.5:
                p.append('the detail starts above the bottom of the board')
            if after['l'] < -0.5 or after['r'] > r['vw'] + 0.5 or after['cut'] or after['sideways'] > 1:
                p.append('the detail sticks out sideways')
    p += ['common: ' + x for x in common(d, size)[:3]]
    verdict(rc, 'W-V26-RANK', 'home', state, 'lee', lang, theme, size, d, p,
            'line chart with %d points >= 44 px, labels inside the board, a tap shows the line' % r['n'])


# ------------------------------------------------------------------------------------------------ W-V26-POD
def pod_case(rc, lang, theme, size):
    d = rc.dev('lee', lang, theme, size)
    goto(d, '#/', 1.8)
    until(d, READY, 8.0)
    until(d, "document.querySelector('.dash-wrong .dash-pt')", 6.0)
    time.sleep(0.5)
    r = js(d, BOARD_JS.replace('ROOT', '.dash-wrong'))
    if not r or not r['n']:
        rc.rec('W-V26-POD', 'home', 'board', 'lee', size, lang, theme, False, 'no most-missed points on #/ (lee has 60 records with wrong answers)', d)
        return
    p = []
    if r['n'] < 5:
        p.append('only %d places on the board (the seed gives lee five)' % r['n'])
    p += board_problems(r, 'most-missed board')
    state = 'board'
    if not p:
        # a tap opens the question of the first place under the board
        before = js(d, PICK_JS.replace('ROOT', '.dash-wrong'))
        if not before or not before['hint']:
            p.append('before any pick the detail does not show the hint')
        d.tap_sel('.dash-wrong .dash-pt-1')
        time.sleep(0.4)
        open_ = js(d, PICK_JS.replace('ROOT', '.dash-wrong'))
        if not open_ or not open_['stem'] or not open_['meta']:
            p.append('a tap on place 1 did not show the question (stem %r)' % ((open_ or {}).get('stem', '')[:30]))
        else:
            state = 'detail'
            if not open_['ans']:
                p.append('the detail does not show the correct answer')
            if open_['pressed'] != 1:
                p.append('%d points are pressed after one tap (want 1)' % open_['pressed'])
            if open_['live'] != 'polite':
                p.append('the detail region is not aria-live polite')
            if open_['t'] < open_['boardBottom'] - 0.5:
                p.append('the detail starts above the bottom of the board')
            if open_['l'] < -0.5 or open_['r'] > r['vw'] + 0.5 or open_['cut'] or open_['sideways'] > 1:
                p.append('the detail sticks out sideways')
            if not phone(size):
                d.key('Escape', 'Escape', 27)
                time.sleep(0.3)
                esc = js(d, PICK_JS.replace('ROOT', '.dash-wrong'))
                if esc and esc['pressed'] != 0:
                    p.append('Escape did not clear the pinned pick')
            else:
                d.tap_sel('.dash-wrong .dash-pt-1')
                time.sleep(0.35)
                again = js(d, PICK_JS.replace('ROOT', '.dash-wrong'))
                if again and (again['pressed'] != 0 or not again['hint']):
                    p.append('a second tap did not close the question (pressed %d, hint %s)' % (again['pressed'], again['hint']))
    p += ['common: ' + x for x in common(d, size)[:3]]
    verdict(rc, 'W-V26-POD', 'home', state, 'lee', lang, theme, size, d, p,
            'most-missed board with %d points >= 44 px, a tap opens the question and its answer' % r['n'])


# ------------------------------------------------------------------------------------------------ W-V26-CAP
def cap_folded_problems(c):
    p = []
    if c['l'] < -0.5 or c['r'] > c['vw'] + 0.5:
        p.append('the card sticks out (%d..%d of %d px)' % (c['l'], c['r'], c['vw']))
    if c['sideways'] > 1:
        p.append('the page is %d px wider than the viewport' % c['sideways'])
    if c['afterWrong'] is False:
        p.append('the card is not below the most-missed board')
    if not c['last']:
        p.append('the card is not the last block of the page')
    if not c['tog']:
        p.append('the card has no toggle button')
        return p
    if c['togH'] < 44:
        p.append('the toggle is %d px high (< 44)' % c['togH'])
    if not c['togInside']:
        p.append('the toggle leaves the card')
    p += ['title and badge overlap: ' + o for o in (c['headOverlap'] or [])[:2]]
    if c['titleCut'] or not c['badgeInside']:
        p.append('the title or the AI+ badge is cut')
    return p


def cap_case(rc, lang, theme, size):
    made = ensure_assess(rc)
    d = rc.dev('lee', lang, theme, size)
    goto(d, '#/', 1.8)
    until(d, READY, 8.0)
    until(d, "document.querySelector('.dash-ai')", 6.0)
    time.sleep(0.4)
    c = js(d, CAP_JS)
    if not c:
        rc.rec('W-V26-CAP', 'home', 'card', 'lee', size, lang, theme, False,
               'no AI card on #/ for an AI+ user (assessment: %s)' % made, d)
        return
    p = cap_folded_problems(c)
    # folded on arrival
    if c['expanded'] != 'false' or not c['bodyHidden']:
        p.append('the card is not folded on arrival (aria-expanded %s, body hidden %s)' % (c['expanded'], c['bodyHidden']))
    # open it: the text and the button show
    if c['tog']:
        d.tap_sel('.dash-ai-toggle')
        time.sleep(0.5)
        c = js(d, CAP_JS) or c
        if c['expanded'] != 'true' or c['bodyHidden']:
            p.append('a tap on the toggle did not open the card (aria-expanded %s)' % c['expanded'])
        if c['r'] > c['vw'] + 0.5 or c['sideways'] > 1:
            p.append('the opened card sticks out sideways')
        if c['textCut']:
            p.append('the text is cut sideways')
        if not c['text']:
            p.append('the opened card has no text')
        if c['model']:
            p.append('the card shows a model name')
        if c['btn']:
            if c['btnH'] < 44:
                p.append('the action button is %d px high (< 44)' % c['btnH'])
            if not c['btnInside']:
                p.append('the action button leaves the card')
        elif made in ('saved', 'made'):
            p.append('no action button although a text is saved (lee has AI+)')
        if c['btn'] and c['btnDisabled'] and not c['note']:
            p.append('the button is disabled without a reason line')
        # leaving Home and coming back (in the app, no reload) folds it again
        d.js("location.hash = '#/settings'; 1")
        time.sleep(0.8)
        d.js("location.hash = '#/'; 1")
        until(d, READY, 8.0)
        until(d, "document.querySelector('.dash-ai')", 6.0)
        time.sleep(0.4)
        back = js(d, CAP_JS)
        if not back or back['expanded'] != 'false' or not back['bodyHidden']:
            p.append('the card is not folded again after leaving Home and coming back')
    p += ['common: ' + x for x in common(d, size)[:3]]
    verdict(rc, 'W-V26-CAP', 'home', 'card', 'lee', lang, theme, size, d, p,
            'AI card last, folded, toggle %d px, opened: %d characters, button "%s" %d px high' % (c['togH'], len(c['text']), c['btnText'], c['btnH']))


def cap_none_case(rc, lang, theme, size):
    """riley has no AI+ and no saved text: the AI card is not drawn at all."""
    d = rc.dev('riley', lang, theme, size)
    goto(d, '#/', 1.8)
    until(d, READY, 8.0)
    time.sleep(0.5)
    shown = d.js("!!document.querySelector('.dash-ai')")
    verdict(rc, 'W-V26-CAP', 'home', 'no-card', 'riley', lang, theme, size, d,
            ['a user without AI+ and without a saved assessment sees the AI card'] if shown else [],
            'no AI card without AI+ and without a saved text')


# ------------------------------------------------------------------------------------------------ W-V26-TL
def tiles_case(rc, lang, theme, size):
    for user in ('lee', 'sam', 'riley'):
        d = rc.dev(user, lang, theme, size)
        goto(d, '#/', 1.6)
        until(d, READY, 8.0)
        time.sleep(0.3)
        t = js(d, TILES_JS)
        if not t:
            rc.rec('W-V26-TL', 'home', 'tiles', user, size, lang, theme, False, 'no tile row on #/', d)
            continue
        p = []
        n, w = t['n'], t['vw']
        if str(n) != str(t['dataN']):
            p.append('data-n is %s for %d tiles' % (t['dataN'], n))
        # 5 or 6 tiles: rows of three (a wide screen may also show them in one row); 4: 2 x 2 up to 480 px; else one row
        if n >= 5:
            wants = [((n + 2) // 3, 3)] + ([(1, n)] if w >= 900 else [])
        elif n == 4 and w <= 480:
            wants = [(2, 2)]
        else:
            wants = [(1, n)]
        rows, cols = wants[0]
        if (len(t['tops']), len(t['lefts'])) not in wants:
            p.append('%d tiles at %d px: %d rows x %d columns, want %d x %d' % (n, w, len(t['tops']), len(t['lefts']), rows, cols))
        idx = [TILE_ORDER.index(h) if h in TILE_ORDER else -1 for h in t['hrefs']]
        if -1 in idx or idx != sorted(idx):
            p.append('tile order %s, want the order %s' % (t['hrefs'], TILE_ORDER))
        if not t['hand']:
            p.append('hand mode is not on by default (html[data-hand] missing)')
        if not t['handBtn']:
            p.append('the Aa hand-mode switch is not visible in the top bar')
        if t['minH'] < 88:
            p.append('a tile is %d px high (< 88)' % t['minH'])
        if t['minW'] < 44:
            p.append('a tile is %d px wide' % t['minW'])
        p += ['overlap: ' + o for o in (t['overlap'] or [])[:2]]
        if t['out']:
            p.append('a tile sticks out')
        if t['cut']:
            p.append('%d tile labels are cut sideways' % t['cut'])
        if t['sideways'] > 1:
            p.append('the page is %d px wider than the viewport' % t['sideways'])
        verdict(rc, 'W-V26-TL', 'home', 'tiles-%d' % n, user, lang, theme, size, d, p,
                '%d tiles as %d x %d, each >= 88 px high' % (n, rows, cols))


# ------------------------------------------------------------------------------------------------ W-V26-DSK
DSK_JS = """
const pg = document.querySelector('.page-dash'); if (!pg) return null;
const b = (s) => { const e = document.querySelector(s); return e && vis(e) ? box(e) : null; };
const hero = b('.dash-hero'), row = b('.dash-row'), rank = b('.dash-rank'), wrong = b('.dash-wrong'), ai = b('.dash-ai'), page = box(pg);
const tiles = [...document.querySelectorAll('.dash-row .dash-tile')].filter(vis).map(box);
const boards = ['.dash-rank', '.dash-wrong'].map((s) => { const p = b(s), d = b(s + ' .dash-board'); return p && d ? d.l >= p.l - 0.5 && d.r <= p.r + 0.5 : null; });
return {hero, row, rank, wrong, ai, page, vw: innerWidth, tops: [...new Set(tiles.map((x) => Math.round(x.t)))].length, tiles: tiles.length,
  boards, sideways: document.documentElement.scrollWidth - innerWidth};"""


def dsk_check(rc, lang, theme, size, wide):
    d = rc.dev('lee', lang, theme, size)
    goto(d, '#/', 1.6)
    until(d, READY, 8.0)
    time.sleep(0.4)
    r = js(d, DSK_JS)
    if not r or not r['rank'] or not r['wrong']:
        rc.rec('W-V26-DSK', 'home', 'layout', 'lee', size, lang, theme, False, 'no Home panels on #/', d)
        return
    p = []
    if r['sideways'] > 1:
        p.append('the page is %d px wider than the viewport' % r['sideways'])
    if None in r['boards'] or not all(r['boards']):
        p.append('a board sticks out of its panel')
    rk, wr, ai, hero, row, pg = r['rank'], r['wrong'], r['ai'], r['hero'], r['row'], r['page']
    if wide:
        if r['tops'] != 1:
            p.append('tiles are in %d rows, want one' % r['tops'])
        if abs(rk['t'] - wr['t']) > 1 or rk['r'] > wr['l'] + 0.5:
            p.append('the leaderboard and the most missed panels are not side by side')
        if abs(rk['w'] - wr['w']) > 1.5:
            p.append('the two panels differ in width (%d / %d)' % (rk['w'], wr['w']))
        if hero and row and not hero['b'] <= row['t'] + 0.5:
            p.append('the hero is not above the tiles')
        if row and rk['t'] < row['b'] - 0.5:
            p.append('the panels start above the tile row')
        if pg['w'] > 1081 or abs((pg['l'] - 0) - (r['vw'] - pg['r'])) > 24:
            p.append('page is %d px wide and not centred (%d / %d)' % (pg['w'], pg['l'], r['vw'] - pg['r']))
        if ai and (ai['t'] < max(rk['b'], wr['b']) - 0.5 or abs(ai['w'] - pg['w']) > 1.5):
            p.append('the AI card is not a full-width row below the panels')
        if hero and abs(hero['w'] - pg['w']) > 1.5:
            p.append('the hero is not full width')
    else:
        if rk['b'] > wr['t'] + 0.5 or abs(rk['l'] - wr['l']) > 1 or abs(rk['w'] - wr['w']) > 1:
            p.append('on a phone the two panels are not one column')
        if ai and ai['t'] < wr['b'] - 0.5:
            p.append('the AI card is not below the panels')
    verdict(rc, 'W-V26-DSK', 'home', 'wide' if wide else 'column', 'lee', lang, theme, size, d, p,
            'wide: hero, one tile row, two panels side by side, AI card full width' if wide else 'phone: single column')


def dsk_case(rc, lang, theme, size):
    dsk_check(rc, lang, theme, size, size[0] >= 900)


# ------------------------------------------------------------------------------------------------ W-V26-PRF
def prf_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/profile', 1.8)
    until(d, "document.querySelector('.page-profile')", 6.0)
    time.sleep(0.3)
    need = 44 if phone(size) else 36
    r = js(d, PROFILE_JS % need)
    if not r:
        rc.rec('W-V26-PRF', 'profile', 'page', 'sam', size, lang, theme, False, 'no .page-profile on #/profile', d)
        return
    p = []
    if not (r['account'] and r['nameInput'] and r['pwToggle']) or r['secs'] < 3:
        p.append('sections missing (account %s, name field %s, password row %s, %d sections)' % (r['account'], r['nameInput'], r['pwToggle'], r['secs']))
    if '@' not in r['login']:
        p.append('the login line shows "%s", not an e-mail' % r['login'][:30])
    if r['small']:
        p.append('small targets: %s' % ', '.join(r['small'][:4]))
    if r['out']:
        p.append('outside the screen: %s' % ', '.join(r['out'][:3]))
    if r['sideways'] > 1:
        p.append('the page is %d px wider than the viewport' % r['sideways'])
    # the password row opens in place with its three fields
    d.tap_sel('#pf-pw-toggle')
    time.sleep(0.4)
    w = js(d, PW_JS)
    if not w or not w['open'] or w['n'] < 3:
        p.append('the password row did not open three fields (%r)' % (w,))
    else:
        if w['minH'] < need:
            p.append('a password field is %d px high (< %d)' % (w['minH'], need))
        if w['out'] or w['sideways'] > 1:
            p.append('the password fields stick out sideways')
        p += ['overlap: ' + o for o in (w['overlap'] or [])[:2]]
    p += ['common: ' + x for x in common(d, size)[:3]]
    verdict(rc, 'W-V26-PRF', 'profile', 'page', 'sam', lang, theme, size, d, p,
            '%d controls >= %d px, three sections, the password row opens in place' % (r['n'], need))

    # Settings lost the account section
    goto(d, '#/settings', 1.6)
    s = js(d, SETTINGS_JS)
    verdict(rc, 'W-V26-PRF', 'settings', 'no-account', 'sam', lang, theme, size, d,
            (['Settings still has an account / password section'] if s and s['account'] else []) +
            (['no Settings page'] if not s or not s['page'] else []) + (['sideways scroll'] if s and s['sideways'] > 1 else []),
            'Settings has no account section')

    # The menu's user row is a link to the profile page
    goto(d, '#/', 1.4)
    d.tap_sel('#menu-btn')
    time.sleep(0.6)
    m = js(d, MENU_ROW_JS)
    p = []
    if not m:
        p.append('no user link (a.nav-who) in the menu')
    else:
        if m['href'] != '#/profile':
            p.append('the user row links to %s' % m['href'])
        if m['h'] < 44:
            p.append('the user row is %d px high (< 44)' % m['h'])
        if not m['inside'] or not m['signOut']:
            p.append('the user row is outside the screen or the sign-out button is gone')
        else:
            d.tap_sel('.nav-account a.nav-who, .nav-pop a.nav-who')
            time.sleep(0.8)
            if d.js('location.hash') != '#/profile':
                p.append('a tap on the user row opened %s' % d.js('location.hash'))
    verdict(rc, 'W-V26-PRF', 'menu', 'user-row', 'sam', lang, theme, size, d, p, 'the menu user row is a >= 44 px link that opens #/profile')


# ------------------------------------------------------------------------------------------------ W-V26-CTR
CENTRE_PAGES = [('settings', '#/settings', 'sam', '.page-settings'), ('profile', '#/profile', 'sam', '.page-profile'),
                ('users', '#/users', 'admin', '.page-users'), ('ai-settings', '#/ai-settings?tab=users', 'admin', '.page-ais'),
                ('about', '#/about', 'sam', '.page-about')]


def ctr_case(rc, lang, theme, size):
    for page, route, user, sel in CENTRE_PAGES:
        if not rc.walk.wants_page(page):
            continue
        d = rc.dev(user, lang, theme, size)
        goto(d, route, 2.0)
        c = js(d, CENTRE_JS % json.dumps(sel))
        if not c:
            rc.rec('W-V26-CTR', page, 'centred', user, size, lang, theme, False, 'no %s on %s' % (sel, route), d)
            continue
        gap = abs(c['l'] - c['r'])
        p = [] if gap <= 2 else ['%s sits %d px from the left and %d px from the right edge' % (sel, c['l'], c['r'])]
        verdict(rc, 'W-V26-CTR', page, 'centred', user, lang, theme, size, d, p,
                '%s is centred (%d px both sides, %d px wide)' % (sel, c['l'], c['w']))


# ------------------------------------------------------------------------------------------------ W-V26-LOG
def log_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/logs', 2.2)
    found = []
    p = []
    for tab in (0, 1):
        d.js("(() => { const b = document.querySelectorAll('.lg-tabs button')[%d]; if (b) b.click(); return 1 })()" % tab)
        time.sleep(1.3)
        r = js(d, PAGER_JS)
        if not r or not r['found']:
            continue
        found.append(tab)
        if r['l'] < -0.5 or r['r'] > r['vw'] + 0.5 or r['out']:
            p.append('tab %d: the pager sticks out (%d..%d of %d px)' % (tab, r['l'], r['r'], r['vw']))
        if r['sideways'] > 1:
            p.append('tab %d: the page is %d px wider than the viewport' % (tab, r['sideways']))
        if r['ctl'] < 2:
            p.append('tab %d: the pager has %d controls (want previous and next)' % (tab, r['ctl']))
        if phone(size) and (r['minH'] < 44 or r['minW'] < 44):
            p.append('tab %d: a pager control is %d x %d px (< 44)' % (tab, r['minW'], r['minH']))
        p += ['tab %d overlap: %s' % (tab, o) for o in (r['overlap'] or [])[:2]]
    if not found:
        p.append('no pager on either log tab')
    verdict(rc, 'W-V26-LOG', 'logs', 'pager', 'admin', lang, theme, size, d, p, 'the pager fits on tab(s) %s' % found)


# ------------------------------------------------------------------------------------------------ W-V26-NAME
NAME_JS = """
const q = (s) => document.querySelector(s);
const ctl = [q('#auth-name'), q('#auth-new'), q('#auth-repeat'), q('.auth-submit'), ...document.querySelectorAll('.auth-acts .btn.quiet')]
  .filter(Boolean).filter(vis);
const bs = ctl.map(box);
const inputs = [...document.querySelectorAll('.auth-form input:not(.sr-only):not([type=hidden])')].filter(vis);
return {name: !!q('#auth-name'), pw: !!q('#auth-new'), nameTop: q('#auth-name') ? box(q('#auth-name')).t : null, pwTop: q('#auth-new') ? box(q('#auth-new')).t : null,
  small: ctl.filter((e) => box(e).h < NEED - 0.5).map((e) => nm(e) + ' ' + Math.round(box(e).h) + 'px'),
  out: bs.some((b) => b.r > innerWidth + 0.5 || b.l < -0.5), sideways: document.documentElement.scrollWidth - innerWidth,
  inputs: inputs.length, quiet: document.querySelectorAll('.auth-acts .btn.quiet').length, err: !!q('.auth-msg')};"""

SET_JS = """
const set = (id, v) => { const e = document.getElementById(id); if (!e) return false;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(e, v);
  e.dispatchEvent(new Event('input', {bubbles: true})); return true; };
const vals = %s; for (const k of Object.keys(vals)) set(k, vals[k]);
const f = document.querySelector('.auth-form'); if (f) f.requestSubmit(); return !!f;"""


def fill_submit(d, vals):
    d.js('(() => { %s })()' % (SET_JS % json.dumps(vals)))
    time.sleep(0.7)


def name_case(rc, lang, theme, size):
    import device
    base = rc.walk.base
    admin = rc.session('admin')
    stamp = int(time.time() * 1000)
    login = 'newbie-%d@example.com' % stamp
    admin.create_user(login)
    need = 44 if phone(size) else 36
    d = device.Device(rc.walk.chrome, base, None, lang=lang, theme=theme, width=size[0], height=size[1])
    try:
        goto(d, '#/', 1.6)
        until(d, "document.getElementById('auth-user')", 8.0)
        fill_submit(d, {'auth-user': login, 'auth-pass': seed.INITIAL})
        # (1) new user: the new-password page with the username field above the password fields
        if not until(d, "document.getElementById('auth-name') && document.getElementById('auth-new')", 8.0):
            rc.rec('W-V26-NAME', 'signin', 'new-user', None, size, lang, theme, False, 'no new-password page with a username field after the first sign-in', d)
            return
        r = js(d, NAME_JS.replace('NEED', str(need)))
        p = []
        if r['nameTop'] is None or r['pwTop'] is None or r['nameTop'] >= r['pwTop']:
            p.append('the username field is not above the password fields')
        if r['small']:
            p.append('small targets: %s' % ', '.join(r['small'][:4]))
        if r['out'] or r['sideways'] > 1:
            p.append('the page sticks out sideways (%d px)' % r['sideways'])
        for what, vals in (('21 characters', {'auth-name': 'x' * 21}), ('an @', {'auth-name': 'a@b'})):
            vals = dict(vals, **{'auth-new': seed.DEMO_PASSWORD, 'auth-repeat': seed.DEMO_PASSWORD})
            fill_submit(d, vals)
            if not js(d, NAME_JS.replace('NEED', str(need)))['err']:
                p.append('a name with %s shows no inline error' % what)
        verdict(rc, 'W-V26-NAME', 'signin', 'new-user', None, lang, theme, size, d, p,
                'username above the password fields, targets >= %d px, errors shown inline' % need)
        # finish the first sign-in with a good name
        fill_submit(d, {'auth-name': 'Newbie%d' % (stamp % 100000), 'auth-new': seed.DEMO_PASSWORD, 'auth-repeat': seed.DEMO_PASSWORD})
        until(d, "document.querySelector('.page-dash')", 8.0)

        # (2) a user who only lacks a username: needs the name removed from the database (cargo backend)
        data = getattr(rc.walk.stack.backend, 'data', None)
        db = os.path.join(data, 'xexam.db') if data else ''
        if not db or not os.path.isfile(db):
            rc.rec('W-V26-NAME', 'signin', 'name-only', None, size, lang, theme, None,
                   'skipped: the name-only page needs the user name cleared in the database (cargo backend only)')
            return
        with sqlite3.connect(db, timeout=15) as con:
            con.execute('UPDATE users SET display_name = NULL WHERE username = ?', (login,))
        d.nav('#/', 1.8)
        if not until(d, "document.getElementById('auth-name') && !document.getElementById('auth-new')", 8.0):
            rc.rec('W-V26-NAME', 'signin', 'name-only', None, size, lang, theme, False, 'no name-only page for a user without a username', d)
            return
        r = js(d, NAME_JS.replace('NEED', str(need)))
        p = []
        if r['inputs'] != 1:
            p.append('the name-only page has %d fields (want 1)' % r['inputs'])
        if r['quiet'] < 1:
            p.append('no Sign out button on the name-only page')
        if r['small']:
            p.append('small targets: %s' % ', '.join(r['small'][:4]))
        if r['out'] or r['sideways'] > 1:
            p.append('the page sticks out sideways (%d px)' % r['sideways'])
        for what, vals in (('nothing', {'auth-name': ''}), ('an @', {'auth-name': 'a@b'}), ('21 characters', {'auth-name': 'x' * 21})):
            fill_submit(d, vals)
            if not js(d, NAME_JS.replace('NEED', str(need)))['err']:
                p.append('%s typed as a name shows no inline error' % what)
        d.tap_sel('.auth-acts .btn.quiet')
        if not until(d, "document.getElementById('auth-user')", 6.0):
            p.append('Sign out did not lead to the sign-in page')
        verdict(rc, 'W-V26-NAME', 'signin', 'name-only', None, lang, theme, size, d, p,
                'one field, Save and Sign out >= %d px, errors inline, Sign out works' % need)
    finally:
        d.close()


def g_v26(rc):
    cases(rc, 'W-V26-RANK', 'home', rank_case, PHONES + [DESKTOP])
    cases(rc, 'W-V26-POD', 'home', pod_case, PHONES + [DESKTOP])
    cases(rc, 'W-V26-CAP', 'home', cap_case, PHONES + [DESKTOP])
    cases(rc, 'W-V26-CAP', 'home', cap_none_case, PHONES[:2])
    cases(rc, 'W-V26-TL', 'home', tiles_case, PHONES + [DESKTOP])
    cases(rc, 'W-V26-DSK', 'home', dsk_case, [(390, 844), DESKTOP])
    cases(rc, 'W-V26-PRF', 'profile', prf_case, PHONES + [DESKTOP])
    cases(rc, 'W-V26-CTR', 'settings', ctr_case, DESKTOPS)
    cases(rc, 'W-V26-LOG', 'logs', log_case, PHONES + [DESKTOP])
    cases(rc, 'W-V26-NAME', 'signin', name_case, PHONES + [DESKTOP])


GROUPS_V26 = [g_v26]
