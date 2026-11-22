# -*- coding: utf-8 -*-
"""v2.5 walk checks: the Home dashboard (#/), the Exam & practice page (#/study) and the menu entry. Phone sizes 320 / 375 /
430 px wide, both themes, all three languages (the matrix follows --langs / --themes / --widths; by default the three
languages and both themes). Every failing case keeps a screenshot in the output folder.

  W-V25-DASH  dashboard layout (v2.6: 4 to 6 row tiles, then the leaderboard chart, the most-missed board and the AI card): no sideways scroll; the hero tile is >= 72 px high, the row tiles >= 88 px, none overlaps another,
              the icons are 28 px; no tile label is cut off (one line fits, or a clamped second line); no leaderboard point label
              (.dash-rank .dash-pt aria-label) contains an @; the ranking JSON of /api/dashboard has no @ either
  W-V25-AI    (retired in v2.6: the amber AI button and its dialog are gone; the AI card is checked by W-V26-CAP in checks_v26.py)
  W-V25-RS    a saved session shows the resume card on the dashboard: visible, inside the width, no sideways scroll
  W-V25-MN    the menu lists Exam & practice as the second entry and opens #/study
  W-V25-RB    the result page's back button lands on #/study (the exam setup), not on the dashboard
  W-V25-ST    the Exam & practice page keeps its setup (bank list and start bar) and its address is #/study

Made-up accounts and data only (the sample users of seed_extra: e-mail logins with plain, long and short display names; v2.6 the
ranking shows display names).
Registered in checks.py (GROUPS_V25); run alone with --only 'W-V25-*'.
"""
import re
import time

from checks import LANGS, SETUP, goto, make_result, phone, start_quiz, until
from checks_v13 import common, verdict
from checks_v22 import HELPERS, js

PHONES = [(320, 640), (375, 812), (430, 932)]
DESKTOP = (1280, 800)
STUDY_RE = re.compile(r'考试&练习|Exam & practice|試験・練習')

# Waits until the dashboard has its data (the boards are drawn, no skeleton left)
READY = "document.querySelector('.page-dash') && document.querySelector('.dash-board') && !document.querySelector('.dash-board-skel')"

DASH_JS = """
const q = (s) => [...document.querySelectorAll(s)].filter(vis);
const hero = q('.dash-hero')[0]; const tiles = q('.dash-row .dash-tile'); const all = hero ? [hero].concat(tiles) : tiles;
const hb = hero ? box(hero) : null;
const icons = q('.dash-tile .dash-ico').map((e) => { const b = box(e); return [Math.round(b.w), Math.round(b.h)]; });
const cut = [];
for (const e of q('.dash-hero-label, .dash-hero-sub, .dash-tile-label, .dash-sub, .dash-note')) {
  const cs = getComputedStyle(e); const clamped = cs.webkitLineClamp && cs.webkitLineClamp !== 'none';
  if (e.scrollWidth > e.clientWidth + 1) cut.push(nm(e) + ' is cut sideways');
  else if (clamped && e.scrollHeight > e.clientHeight + 1) cut.push(nm(e) + ' needs more than 2 lines');
}
const names = q('.dash-rank .dash-pt').map((e) => ({text: (e.getAttribute('aria-label') || '').trim()}));
const h1 = q('.dash-hello h1')[0];
return {sideways: document.documentElement.scrollWidth - innerWidth, hero: hb ? Math.round(hb.h) : null,
  tiles: tiles.map((e) => Math.round(box(e).h)), overlap: overlaps(all), icons, cut, names, tileCount: tiles.length,
  h1: h1 ? {cut: h1.scrollWidth > h1.clientWidth + 1, ell: getComputedStyle(h1).textOverflow === 'ellipsis'} : null,
  tileRight: tiles.length ? Math.max(...tiles.map((e) => box(e).r)) : 0, vw: innerWidth};"""

API_NAMES_JS = """(async () => { const r = await fetch('/api/dashboard'); if (!r.ok) return {status: r.status};
  const j = await r.json(); return {status: 200, names: (j.ranking.top || []).map((e) => e.name), body: JSON.stringify(j)} })()"""

MENU_JS = """
const items = [...document.querySelectorAll('.nav-pop .nav-list .nav-link')].filter(vis);
return items.map((e) => (e.querySelector('.nav-label') || e).textContent.trim());"""

SETUP_JS = """
return {hash: location.hash, bank: !!document.querySelector('.home-setup, .home-bank, .home-bankhead'), go: !!document.querySelector('.home-go'),
  h1: (document.querySelector('h1') || {}).textContent || '', sideways: document.documentElement.scrollWidth - innerWidth};"""


def cases(rc, cid, page, fn, sizes):
    if not rc.want(cid) or not rc.walk.wants_page(page):
        return
    for lang in rc.langs(LANGS):
        for theme in rc.themes(['light', 'dark']):
            for size in rc.sizes(sizes, lambda s: True):
                rc.guard(cid, page, fn, rc, lang, theme, size)


# ------------------------------------------------------------------------------------------------ W-V25-DASH
def dash_case(rc, lang, theme, size):
    d = rc.dev('lee', lang, theme, size)
    goto(d, '#/', 1.6)
    until(d, READY, 8.0)
    time.sleep(0.4)
    r = js(d, DASH_JS)
    p = []
    if not r or r['hero'] is None:
        rc.rec('W-V25-DASH', 'home', 'layout', 'lee', size, lang, theme, False, 'no dashboard hero tile on #/', d)
        return
    if r['sideways'] > 1:
        p.append('the page is %d px wider than the viewport' % r['sideways'])
    if r['hero'] < 72:
        p.append('the hero tile is %d px high (< 72)' % r['hero'])
    if r['tileCount'] < 2:
        p.append('only %d row tiles' % r['tileCount'])
    p += ['row tile %d px high (< 88)' % h for h in r['tiles'] if h < 88]
    p += ['overlap: ' + o for o in (r['overlap'] or [])[:3]]
    p += ['a tile icon is %dx%d (want 28)' % tuple(i) for i in r['icons'] if tuple(i) != (28, 28)]
    p += r['cut'][:4]
    if r['tileRight'] > r['vw'] + 0.5:
        p.append('a tile sticks out on the right')
    if r['h1'] and r['h1']['cut'] and not r['h1']['ell']:
        p.append('the greeting is cut without an ellipsis')
    for n in r['names']:
        if '@' in n['text']:
            p.append('a ranking name contains @ (%s)' % n['text'])
    api = d.js(API_NAMES_JS) or {}
    if api.get('status') != 200:
        p.append('GET /api/dashboard answered %s' % api.get('status'))
    elif any('@' in str(n) for n in api['names']) or '@' in api['body']:
        p.append('the /api/dashboard body contains an @ (names: %s)' % api['names'])
    p += ['common: ' + x for x in common(d, size)[:3]]
    verdict(rc, 'W-V25-DASH', 'home', 'layout', 'lee', lang, theme, size, d, p,
            'hero %d px, tiles %s px, icons 28, labels whole, %d ranking names without @' % (r['hero'], r['tiles'], len(r['names'])))


# ------------------------------------------------------------------------------------------------ W-V25-RS
def rs_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    start_quiz(d, 'practice')
    d.js("window.__answer && window.__answer(true); 1")
    time.sleep(0.4)
    d.tap_sel('.qexit')
    time.sleep(1.0)
    goto(d, '#/', 1.8)
    until(d, READY, 8.0)
    r = js(d, """const c = document.querySelector('.page-dash .home-resume'); if (!c || !vis(c)) return null; const b = box(c);
      const btns = [...c.querySelectorAll('button')].filter(vis).map((e) => box(e));
      return {l: b.l, r: b.r, h: b.h, buttons: btns.length, out: btns.some((x) => x.r > innerWidth + 0.5 || x.l < -0.5),
        sideways: document.documentElement.scrollWidth - innerWidth};""")
    p = []
    if not r:
        rc.rec('W-V25-RS', 'home', 'resume', 'sam', size, lang, theme, False, 'no resume card on the dashboard after leaving a quiz', d)
        return
    if r['l'] < -0.5 or r['r'] > size[0] + 0.5:
        p.append('the resume card sticks out (%d..%d)' % (r['l'], r['r']))
    if r['buttons'] < 2:
        p.append('the resume card shows %d buttons (want Continue and Discard)' % r['buttons'])
    if r['out']:
        p.append('a resume button is outside the screen')
    if r['sideways'] > 1:
        p.append('the page is %d px wider than the viewport' % r['sideways'])
    verdict(rc, 'W-V25-RS', 'home', 'resume', 'sam', lang, theme, size, d, p, 'resume card with Continue and Discard, inside the width')
    d.js("Object.keys(localStorage).filter(k => /session\\.v1$/.test(k)).forEach(k => localStorage.removeItem(k)); 1")


# ------------------------------------------------------------------------------------------------ W-V25-MN
def mn_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    goto(d, '#/', 1.4)
    d.tap_sel('#menu-btn')
    time.sleep(0.6)
    items = js(d, MENU_JS) or []
    p = []
    if len(items) < 3:
        p.append('the menu lists %d entries' % len(items))
    elif not STUDY_RE.search(items[1]):
        p.append('the second entry is "%s", not Exam & practice (entries: %s)' % (items[1], ' | '.join(items[:4])))
    else:
        d.js("(() => { const l = [...document.querySelectorAll('.nav-pop .nav-list .nav-link')].filter(e => e.getClientRects().length)[1]; l && l.click(); return 1 })()")
        time.sleep(0.9)
        h = d.js('location.hash')
        if h != '#/study':
            p.append('the entry opened %s, not #/study' % h)
    verdict(rc, 'W-V25-MN', 'menu', 'study-entry', 'sam', lang, theme, size, d, p, 'Exam & practice is the second entry and opens #/study')


# ------------------------------------------------------------------------------------------------ W-V25-RB
def rb_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    make_result(d, 4)
    ok = d.js("(() => { const a = document.querySelector('.score .acts'); if (!a) return false; const b = [...a.children].filter(e => e.getClientRects().length).pop(); if (!b) return false; b.scrollIntoView({block: 'center'}); b.click(); return true })()")
    time.sleep(1.2)
    h = d.js('location.hash')
    p = []
    if not ok:
        p.append('no result page action row')
    elif h != '#/study':
        p.append('the back button opened %s, not #/study' % h)
    verdict(rc, 'W-V25-RB', 'result', 'back', 'sam', lang, theme, size, d, p, 'the back button lands on #/study')


# ------------------------------------------------------------------------------------------------ W-V25-ST
def st_case(rc, lang, theme, size):
    d = rc.dev('lee', lang, theme, size)
    goto(d, SETUP, 1.8)
    r = js(d, SETUP_JS)
    p = []
    if r['hash'] != '#/study':
        p.append('the address is %s' % r['hash'])
    if not r['bank'] or not r['go']:
        p.append('the setup is missing (bank list %s, start bar %s)' % (r['bank'], r['go']))
    if not STUDY_RE.search(r['h1']):
        p.append('the page title is "%s"' % r['h1'].strip())
    if r['sideways'] > 1:
        p.append('the page is %d px wider than the viewport' % r['sideways'])
    p += ['common: ' + x for x in common(d, size)[:3]]
    verdict(rc, 'W-V25-ST', 'study', 'setup', 'lee', lang, theme, size, d, p, 'exam setup at #/study, titled Exam & practice')


def g_v25(rc):
    cases(rc, 'W-V25-DASH', 'home', dash_case, PHONES + [DESKTOP])
    cases(rc, 'W-V25-RS', 'home', rs_case, PHONES)
    cases(rc, 'W-V25-MN', 'menu', mn_case, PHONES)
    cases(rc, 'W-V25-RB', 'result', rb_case, PHONES[:2])
    cases(rc, 'W-V25-ST', 'study', st_case, PHONES + [DESKTOP])


GROUPS_V25 = [g_v25]
