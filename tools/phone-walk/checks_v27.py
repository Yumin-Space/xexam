# -*- coding: utf-8 -*-
"""v2.7 walk checks: the phone answer sheet drawer, the shared pager in User management and Study records, and the
administrator's read-only view of a user's study records. zh / en / ja, light and dark (the matrix follows --langs / --themes /
--widths). Every failing case keeps a screenshot in the output folder. Made-up accounts and data only.

  W-V27-QD  quiz (sam, practice, 51 questions), phones 320 / 360 / 375 / 393 / 430 px wide: the "1 / n" button (#qz-pos) opens the
            answer sheet as a drawer from the left. Closed: the button is in the top row (aria-expanded false), the drawer is hidden and
            off screen, the top row has no overlap and nothing sticks out sideways. Open: the drawer sits flush left and completely on
            screen, a backdrop covers the page, the page behind does not scroll, the close button is >= 44 px, the tile list scrolls
            inside the drawer. It closes by a tap on the backdrop, by its close button and by Escape; a tile tap closes it and goes
            to that question. At 1280 px the sheet stays a static side rail (no dialog role, no backdrop, no #qz-pos button).
            History (phone, 320 / 393 light): the drawer pushes one history entry. Back closes it and stays on the quiz; one more Back
            leaves the quiz in one step (also after a close by button or tile: no stale entry); Forward over the entry a close
            left behind does not reopen it.
  W-V27-QT  the quiz top row, exam (riley) AND practice (sam), phones 320 / 360 / 393 / 430, light and dark: screenshots kept for
            every case; at 360-430 the answer-sheet button (#qz-pos) and the two-line count (.qcount-w over .qcount-s) are both
            visible and inside the width, and the exam and practice rows put them at the same x (+-1 px).
  W-V27-PG  the shared pager: User management (admin, #us-*) and Study records (lee, #rec-*) show it (also "Page 1 / n"), inside the
            width, no sideways scroll, buttons >= 44 px on phones and without overlap; page 2 works (the current page is marked, the
            rows change, the page size is 5 / 10 on phones and 10 / 20 wide for records).
  W-V27-UR  the administrator opens a user's study records from the book icon in User management (#us-records-<id>, found on any
            page of the list): a read-only list (name title, "view only" note, back link, no delete, no delete-all, no AI summary), a
            record opens read-only (no retry / retake / delete / AI buttons), Back returns to the same list page, and the list's
            back link returns to User management on the same page.

Registered in checks.py (GROUPS_V27); run alone with --only 'W-V27-*'.
"""
import json
import os
import time

from checks import goto, phone, start_quiz, until
from checks_v25 import DESKTOP, cases
from checks_v13 import verdict
from checks_v22 import js

QD_PHONES = [(320, 640), (360, 740), (375, 812), (393, 852), (430, 932)]
QT_PHONES = [(320, 640), (360, 740), (393, 852), (430, 932)]
PG_SIZES = [(320, 640), (390, 844), DESKTOP]
UR_SIZES = [(375, 812), DESKTOP]

# ---------------------------------------------------------------------------------------------------- QD

# The top row of the quiz: every visible direct control inside the width, no two overlapping, no sideways scroll; plus the
# drawer state (hidden / on screen), read in one go
QD_JS = """
const q = document.querySelector('.qbar'); const rail = document.getElementById('qz-rail'); const pos = document.getElementById('qz-pos');
const back = document.getElementById('qz-backdrop'); const close = rail ? rail.querySelector('.rail-close') : null;
const kids = q ? [...q.querySelectorAll('button, a, .qtime, .qcount, .qkind, h1 .pos-full, .tag')].filter(vis) : [];
const rb = rail ? box(rail) : null; const cb = close && vis(close) ? box(close) : null;
const body = rail ? rail.querySelector('.rail-body') : null;
return {pos: !!pos && vis(pos), expanded: pos ? pos.getAttribute('aria-expanded') : null,
  railVis: !!rail && getComputedStyle(rail).visibility !== 'hidden', railOpen: !!rail && rail.classList.contains('open'),
  railBox: rb, role: rail ? rail.getAttribute('role') : null, modal: rail ? rail.getAttribute('aria-modal') : null,
  backVis: !!back && vis(back) && !back.hidden, backBox: back && vis(back) ? box(back) : null,
  locked: document.documentElement.classList.contains('qz-sheet-open'),
  closeBox: cb, bodyScrolls: body ? body.scrollHeight > body.clientHeight + 1 : false, bodyTop: body ? body.scrollTop : 0,
  tiles: rail ? [...rail.querySelectorAll('.tile')].filter((e) => getComputedStyle(e).visibility !== 'hidden').length : 0,
  tileBig: rail ? [...rail.querySelectorAll('.tile')].filter((e) => { const b = box(e); return b.w > 44.5 || b.h > 44.5; }).length : 0,
  cur: (() => { const t = rail ? [...rail.querySelectorAll('.tile')].findIndex((e) => e.classList.contains('cur')) : -1; return t; })(),
  posNum: pos && pos.querySelector('.num') ? pos.querySelector('.num').textContent.trim() : '',
  overlap: overlaps(kids), out: kids.filter((e) => { const b = box(e); return b.r > innerWidth + 0.5 || b.l < -0.5; }).length,
  sideways: document.documentElement.scrollWidth - innerWidth, vw: innerWidth, vh: innerHeight, scrollY: scrollY};"""


def qd_state(d):
    return js(d, QD_JS)


def qd_open(d):
    d.tap_sel('#qz-pos')
    until(d, "document.getElementById('qz-rail') && document.getElementById('qz-rail').classList.contains('open')", 3.0)
    time.sleep(0.45)


def qd_closed(d, seconds=3.0):
    ok = until(d, "(() => { const r = document.getElementById('qz-rail'); return !!r && !r.classList.contains('open') && getComputedStyle(r).visibility === 'hidden' })()", seconds)
    time.sleep(0.2)
    return ok


def qd_wide_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    start_quiz(d, 'practice', 51)
    r = js(d, """const rail = document.getElementById('qz-rail'); const pos = document.getElementById('qz-pos'); const back = document.getElementById('qz-backdrop');
      return {rail: !!rail && vis(rail), left: rail ? box(rail).l : -1, role: rail ? rail.getAttribute('role') : null, pos: !!pos && vis(pos),
              back: !!back && vis(back), sideways: document.documentElement.scrollWidth - innerWidth};""")
    p = []
    if not r or not r['rail']:
        p.append('the answer sheet is not visible')
    else:
        if r['role']:
            p.append('the wide rail has a dialog role (%s)' % r['role'])
        if r['pos']:
            p.append('the drawer button shows on a wide screen')
        if r['back']:
            p.append('a backdrop shows on a wide screen')
        if r['sideways'] > 1:
            p.append('sideways scroll %d px' % r['sideways'])
    verdict(rc, 'W-V27-QD', 'quiz', 'rail-wide', 'sam', lang, theme, size, d, p, 'static side rail: no dialog role, no backdrop, no drawer button')


def in_quiz(d):
    return bool(d.js("location.hash.indexOf('#/quiz') === 0"))


def left_quiz(d, seconds=4.0):
    return until(d, "location.hash.indexOf('#/quiz') !== 0", seconds)


def qd_history(rc, lang, theme, size, d):
    """The drawer's pushed history entry: Back closes it, one more Back leaves the quiz, no stale entry after any close."""
    p = []
    # (a) Back closes the drawer and stays on the quiz; one more Back leaves it
    qd_open(d)
    d.js('history.back(); 1')
    if not qd_closed(d):
        p.append('history: Back did not close the drawer')
    if not in_quiz(d):
        p.append('history: Back from the open drawer left the quiz (hash %s)' % d.js('location.hash'))
    else:
        d.js('history.back(); 1')
        if not left_quiz(d):
            p.append('history: the Back after the drawer closed did not leave the quiz in one step (hash %s)' % d.js('location.hash'))
    # (b) closed by the button / the backdrop / the tile: one Back leaves the quiz (no stale entry); Forward does not reopen the drawer
    for how in ('button', 'backdrop', 'tile'):
        start_quiz(d, 'practice', 51)
        qd_open(d)
        if how == 'button':
            d.tap_sel('#qz-rail .rail-close')
        elif how == 'backdrop':
            s = qd_state(d)
            rb = s['railBox']
            d.tap((rb['r'] + s['vw']) / 2, s['vh'] / 2)
        else:
            d.tap_sel('#qz-rail .tile', 2)
        if not qd_closed(d):
            p.append('history: the drawer did not close by the %s' % how)
            continue
        time.sleep(0.4)
        d.js('history.forward(); 1')
        time.sleep(0.8)
        if not in_quiz(d):
            p.append('history (%s): Forward left the quiz (hash %s)' % (how, d.js('location.hash')))
        elif qd_state(d)['railOpen']:
            p.append('history (%s): Forward over the stale entry reopened the drawer' % how)
        d.js('history.back(); 1')
        if not left_quiz(d):
            p.append('history (%s): one Back did not leave the quiz (hash %s)' % (how, d.js('location.hash')))
    return p


def qd_case(rc, lang, theme, size):
    if not phone(size):
        return qd_wide_case(rc, lang, theme, size)
    d = rc.dev('sam', lang, theme, size)
    start_quiz(d, 'practice', 51)
    p = []
    # (1) closed
    s = qd_state(d)
    if not s['pos'] or s['expanded'] != 'false':
        p.append('closed: no #qz-pos button with aria-expanded=false (expanded=%s)' % s['expanded'])
    if s['railVis'] or s['railOpen']:
        p.append('closed: the drawer is not hidden')
    if s['overlap']:
        p.append('top row overlap: %s' % '; '.join(s['overlap'][:3]))
    if s['out']:
        p.append('%d top-row controls stick out of the width' % s['out'])
    if s['sideways'] > 1:
        p.append('sideways scroll %d px' % s['sideways'])
    if not s['pos']:
        verdict(rc, 'W-V27-QD', 'quiz', 'drawer', 'sam', lang, theme, size, d, p, '')
        return
    # (2) open
    qd_open(d)
    s = qd_state(d)
    rb, vw, vh = s['railBox'], s['vw'], s['vh']
    if s['expanded'] != 'true' or not s['railOpen'] or not s['railVis']:
        p.append('open: the drawer did not open (expanded=%s)' % s['expanded'])
    if not rb or rb['l'] > 0.5 or rb['r'] > vw + 0.5 or rb['t'] < -0.5 or rb['b'] > vh + 0.5:
        p.append('open: the drawer is not flush left and fully on screen (%s)' % (json.dumps({k: round(v) for k, v in rb.items()}) if rb else 'no box'))
    if s['role'] != 'dialog' or s['modal'] != 'true':
        p.append('open: no dialog role / aria-modal (%s / %s)' % (s['role'], s['modal']))
    if not s['backVis']:
        p.append('open: no backdrop')
    elif s['backBox'] and (s['backBox']['w'] < vw - 1 or s['backBox']['h'] < vh - 1):
        p.append('open: the backdrop does not cover the page')
    if not s['locked']:
        p.append('open: the page behind is not locked')
    cb = s['closeBox']
    if not cb or cb['w'] < 43.5 or cb['h'] < 43.5 or cb['r'] > (rb['r'] if rb else vw) + 0.5:
        p.append('open: close button %s' % ('missing' if not cb else '%dx%d px' % (round(cb['w']), round(cb['h']))))
    if s['tileBig']:
        p.append('%d tiles larger than 44 px' % s['tileBig'])
    if s['bodyScrolls']:
        d.js("document.querySelector('#qz-rail .rail-body').scrollTop = 120; 1")
        time.sleep(0.2)
        s2 = qd_state(d)
        if s2['bodyTop'] < 1:
            p.append('open: the tile list does not scroll inside the drawer')
        if s2['scrollY'] != s['scrollY']:
            p.append('open: scrolling the list moved the page behind')
        d.js("document.querySelector('#qz-rail .rail-body').scrollTop = 0; 1")
    # (3) closes: backdrop, close button, Escape
    if rb:
        d.tap((rb['r'] + vw) / 2, vh / 2)
    if not qd_closed(d):
        p.append('a tap on the backdrop did not close the drawer')
        d.js("document.getElementById('qz-rail').querySelector('.rail-close').click(); 1")
        qd_closed(d)
    qd_open(d)
    d.tap_sel('#qz-rail .rail-close')
    if not qd_closed(d):
        p.append('the close button did not close the drawer')
        d.key('Escape', 'Escape', 27)
        qd_closed(d)
    qd_open(d)
    d.key('Escape', 'Escape', 27)
    if not qd_closed(d):
        p.append('Escape did not close the drawer')
        d.tap_sel('#qz-rail .rail-close')
        qd_closed(d)
    s = qd_state(d)
    if s['locked']:
        p.append('the page stays locked after closing')
    # (4) a tile tap closes and goes to that question
    qd_open(d)
    d.tap_sel('#qz-rail .tile', 4)
    time.sleep(0.5)
    if not qd_closed(d):
        p.append('a tile tap did not close the drawer')
    s = qd_state(d)
    if s['posNum'] != '5':
        p.append('after the tile 5 the position reads "%s"' % s['posNum'])
    if s['overlap'] or s['out']:
        p.append('top row problem after the jump (overlap %d, out %d)' % (len(s['overlap']), s['out']))
    if size[0] in (320, 393) and theme == 'light':
        p += qd_history(rc, lang, theme, size, d)
    verdict(rc, 'W-V27-QD', 'quiz', 'drawer', 'sam', lang, theme, size, d, p,
            'drawer from the left, fully on screen, scrolls, closes by backdrop / button / Escape / tile, top row clean')


# ---------------------------------------------------------------------------------------------------- QT

QT_JS = """
const pos = document.getElementById('qz-pos'); const w = document.querySelector('.qbar .qcount-w'); const s = document.querySelector('.qbar .qcount-s');
const ok = (e) => !!e && vis(e) && box(e).w > 0 && box(e).h > 0 && box(e).l >= -0.5 && box(e).r <= innerWidth + 0.5;
const full = document.querySelector('.qbar .qcount-l'); const cnt = document.querySelector('.qbar .qcount');
return {pos: ok(pos), word: ok(w), nums: ok(s), cnt: ok(cnt), cntText: cnt ? cnt.textContent.trim() : '', posX: pos && vis(pos) ? box(pos).l : null, countX: w && vis(w) ? box(w).l : null,
  sideways: document.documentElement.scrollWidth - innerWidth};"""


def qt_row(rc, d, kind, tag, lang, theme, size):
    start_quiz(d, kind, 51)
    time.sleep(0.4)
    d.shot(os.path.join(rc.walk.out, 'W-V27-QT-%s-%s-%s-%dx%d.png' % (kind, lang, theme, size[0], size[1])))
    return js(d, QT_JS)


def qt_case(rc, lang, theme, size):
    p = []
    rows = {}
    phone = size[0] <= 600 or size[1] <= 500   # the PHONE media query: up to 600 px wide, or a short landscape screen
    for kind, user in (('practice', 'sam'), ('exam', 'riley')):
        d = rc.dev(user, lang, theme, size)
        r = qt_row(rc, d, kind, user, lang, theme, size)
        rows[kind] = r
        if not phone and size[0] > 880:
            # Desktop: the sheet and its count are the rail beside the card; the top row has neither (v2.7)
            if r['pos']:
                p.append('%s: the answer-sheet button shows on a desktop screen (the sheet is beside the card there)' % kind)
        elif not phone:
            # Tablet 601-880 px: the sheet stays in the page, so no sheet button; the count is the one-line sentence
            if r['pos']:
                p.append('%s: the answer-sheet button shows on a screen over 600 px (the sheet is in the page there)' % kind)
            if not r['cnt'] or len(r['cntText']) < 8:
                p.append('%s: the one-line count sentence is not visible inside the width ("%s")' % (kind, r['cntText']))
        elif size[0] >= 360:
            if not r['pos']:
                p.append('%s: the answer-sheet button is not visible inside the width' % kind)
            # Only at 360 px wide in en / ja may the count drop out (v2.7: the narrowest phones; the meter shows the
            # progress), but then in both modes alike, never one line cut off; everywhere else it must show
            if not (size[0] <= 360 and lang in ('en', 'ja')) or (r['word'] or r['nums']):
                if not r['word']:
                    p.append('%s: the count word (line 1) is not visible' % kind)
                if not r['nums']:
                    p.append('%s: the count numbers (line 2) are not visible' % kind)
        if r['sideways'] > 1:
            p.append('%s: sideways scroll %d px' % (kind, r['sideways']))
    a, b = rows['practice'], rows['exam']
    if phone and size[0] >= 360:
        if (a['word'] and a['nums']) != (b['word'] and b['nums']):
            p.append('the two-line count shows in one mode only (practice %s / exam %s)' % (a['word'], b['word']))
        for key, what in (('posX', 'answer-sheet button'), ('countX', 'count')):
            if key == 'countX' and a[key] is None and b[key] is None:
                continue
            if a[key] is None or b[key] is None or abs(a[key] - b[key]) > 1:
                p.append('%s x differs, practice %s / exam %s' % (what, a[key], b[key]))
    verdict(rc, 'W-V27-QT', 'quiz', 'top-row', 'sam+riley', lang, theme, size, d, p,
            'exam and practice top rows: button and two-line count visible, same x')


# ---------------------------------------------------------------------------------------------------- PG

PAGER_JS = """
const nav = document.querySelector('.ptb-pager'); if (!nav) return null;
nav.scrollIntoView({block: 'center'});
const nb = box(nav); const btns = [...nav.querySelectorAll('.ptb-pg')].filter(vis);
const hit = (e) => (window.__walk && window.__walk.hitBox) ? window.__walk.hitBox(e) : box(e);
const bb = btns.map(hit);
const on = nav.querySelector('.ptb-pg.on'); const of = nav.querySelector('.ptb-of');
return {n: btns.length, of: of ? of.textContent.trim() : '', on: on ? on.textContent.trim() : '',
  inside: nb.l >= -0.5 && nb.r <= innerWidth + 0.5 && nb.t >= -0.5 && nb.b <= innerHeight + 0.5 && bb.every((b) => b.l >= -0.5 && b.r <= innerWidth + 0.5),
  small: bb.filter((b) => b.w < 43.5 || b.h < 43.5).length, overlap: overlaps(btns), sideways: document.documentElement.scrollWidth - innerWidth,
  prevOff: (document.querySelector('.ptb-step[id$="-prev"]') || {getAttribute: () => null}).getAttribute('aria-disabled') === 'true',
  nextOff: (document.querySelector('.ptb-step[id$="-next"]') || {getAttribute: () => null}).getAttribute('aria-disabled') === 'true'};"""


def rows_of(d, sel):
    return d.js("[...document.querySelectorAll(%s)].map(e => (e.getAttribute('data-rec') || e.id || e.textContent || '').trim().slice(0, 40))" % json.dumps(sel))


def pager_verdict(rc, cid_state, d, size, lang, theme, user, id_, row_sel, max_rows, p):
    """Checks the pager of the open page and page 2; problems are appended to p."""
    r = js(d, PAGER_JS)
    if not r:
        p.append('%s: no pager' % id_)
        return
    if not r['inside']:
        p.append('%s: the pager is outside the viewport' % id_)
    if r['sideways'] > 1:
        p.append('%s: sideways scroll %d px' % (id_, r['sideways']))
    if r['overlap']:
        p.append('%s: pager buttons overlap: %s' % (id_, '; '.join(r['overlap'][:2])))
    if phone(size) and r['small']:
        p.append('%s: %d pager buttons under 44 px' % (id_, r['small']))
    if not r['of'] or r['on'] != '1':
        p.append('%s: the pager shows "%s" with page "%s" marked (want page 1)' % (id_, r['of'], r['on']))
    rows0 = rows_of(d, row_sel)
    if max_rows and len(rows0) > max_rows:
        p.append('%s: %d rows on a page (at most %d)' % (id_, len(rows0), max_rows))
    if r['nextOff']:
        p.append('%s: only one page, cannot test page 2' % id_)
        return
    d.tap_sel('#%s-next' % id_)
    time.sleep(0.6)
    r2 = js(d, PAGER_JS)
    rows1 = rows_of(d, row_sel)
    if not r2 or r2['on'] != '2':
        p.append('%s: page 2 is not marked after Next (%s)' % (id_, r2['on'] if r2 else 'no pager'))
    if not rows1 or rows1 == rows0:
        p.append('%s: the rows did not change on page 2' % id_)
    if r2 and not r2['inside']:
        p.append('%s: the pager is outside the viewport on page 2' % id_)
    if r2 and r2['sideways'] > 1:
        p.append('%s: sideways scroll on page 2' % id_)


def pg_case(rc, lang, theme, size):
    rc.paging_users()
    # User management: 5 per page on phones, 10 wide
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/users', 1.8)
    until(d, "document.querySelector('.ptb-pager')", 8.0)
    p = []
    pager_verdict(rc, 'users', d, size, lang, theme, 'admin', 'us', '.us-rows > li', 5 if phone(size) else 10, p)
    verdict(rc, 'W-V27-PG', 'users', 'pager', 'admin', lang, theme, size, d, p, 'pager shown, inside the width, targets ok, page 2 works')
    # Study records (lee, 60 records): 10 per page on phones, 20 wide
    d = rc.dev('lee', lang, theme, size)
    goto(d, '#/records', 1.8)
    until(d, "document.querySelector('.rec-table a[data-rec]')", 8.0)
    p = []
    pager_verdict(rc, 'records', d, size, lang, theme, 'lee', 'rec', '.rec-table a[data-rec]', 10 if phone(size) else 20, p)
    verdict(rc, 'W-V27-PG', 'records', 'pager', 'lee', lang, theme, size, d, p, 'pager shown, inside the width, targets ok, page 2 works')


# ---------------------------------------------------------------------------------------------------- UR

# Read-only view: nothing that changes data or starts a session
FORBIDDEN_JS = """
const q = (s) => [...document.querySelectorAll('.main ' + s)].filter(vis).length;
return {del: q('.rec-act, .rec-del, button.btn.danger'), ai: q('.rec-ai'), acts: q('.rec-acts'), primary: q('button.btn.primary'),
  note: q('.rec-target-note'), back: q('a.rec-back'), rows: q('.rec-table a[data-rec]'), title: (document.querySelector('.main h1') || {textContent: ''}).textContent.trim(),
  sideways: document.documentElement.scrollWidth - innerWidth, hash: location.hash};"""

USERS_JS = """
const on = document.querySelector('.ptb-pager .ptb-pg.on'); return {page: on ? on.textContent.trim() : '', hash: location.hash,
  has: !!document.getElementById('us-next')};"""


def ur_case(rc, lang, theme, size):
    rc.paging_users()
    uid = rc.session('admin').user_id('lee')
    d = rc.dev('admin', lang, theme, size)
    goto(d, '#/users', 1.8)
    until(d, "document.getElementById('us-next')", 8.0)
    p = []
    # find lee's book icon: step through the pages of the list (a different page than 1 shows the page is kept later)
    sel = '#us-records-%d' % uid
    found = False
    for _ in range(12):
        if d.js("!!document.querySelector('%s')" % sel):
            found = True
            break
        if d.js("document.getElementById('us-next').getAttribute('aria-disabled') === 'true'"):
            break
        d.tap_sel('#us-next')
        time.sleep(0.4)
    if not found:
        verdict(rc, 'W-V27-UR', 'users', 'book-icon', 'admin', lang, theme, size, d, ['no book icon %s on any page of the user list' % sel], '')
        return
    page_before = js(d, USERS_JS)['page']
    r = js(d, "const e = document.querySelector('%s'); if (!e) return null; e.scrollIntoView({block: 'center'}); const b = box(e); return {w: b.w, h: b.h, off: e.disabled || e.getAttribute('aria-disabled') === 'true', label: e.getAttribute('aria-label') || ''};" % sel)
    if not r or r['off']:
        p.append('the book icon of a user with records is greyed out')
    elif phone(size) and (r['w'] < 43.5 or r['h'] < 43.5):
        p.append('the book icon is %dx%d px' % (round(r['w']), round(r['h'])))
    d.tap_sel(sel)
    if not until(d, "location.hash.indexOf('/users/%d/records') >= 0 && document.querySelector('.rec-table')" % uid, 8.0):
        p.append('no records list after the tap (hash %s)' % d.js('location.hash'))
        verdict(rc, 'W-V27-UR', 'users', 'records', 'admin', lang, theme, size, d, p, '')
        return
    time.sleep(0.5)
    f = js(d, FORBIDDEN_JS)
    if not f['note'] or not f['back']:
        p.append('list: no "view only" note or back link (note %d, back %d)' % (f['note'], f['back']))
    if f['del'] or f['ai']:
        p.append('list: %d delete controls and %d AI summary controls show' % (f['del'], f['ai']))
    if not f['rows']:
        p.append('list: no record rows')
    if f['sideways'] > 1:
        p.append('list: sideways scroll %d px' % f['sideways'])
    # the target list pages too: go to page 2 and open a record from there
    if d.js("!!document.getElementById('rec-next') && document.getElementById('rec-next').getAttribute('aria-disabled') !== 'true'"):
        d.tap_sel('#rec-next')
        time.sleep(0.5)
    rec_page = d.js("(document.querySelector('.ptb-pager .ptb-pg.on') || {textContent: ''}).textContent.trim()")
    d.tap_sel('.rec-table a[data-rec]')
    if not until(d, "location.hash.indexOf('/users/%d/records/') >= 0" % uid, 8.0):
        p.append('a record row did not open its read-only detail (hash %s)' % d.js('location.hash'))
    else:
        time.sleep(0.8)
        f2 = js(d, FORBIDDEN_JS)
        if f2['acts'] or f2['del'] or f2['primary']:
            p.append('detail: action buttons show (acts %d, delete %d, primary %d)' % (f2['acts'], f2['del'], f2['primary']))
        if not f2['back']:
            p.append('detail: no back link')
        if f2['sideways'] > 1:
            p.append('detail: sideways scroll %d px' % f2['sideways'])
        d.tap_sel('a.rec-back')
        if not until(d, "location.hash.replace(/\\/$/, '').endsWith('/users/%d/records') && document.querySelector('.rec-table')" % uid, 8.0):
            p.append('detail Back did not return to the user\'s list (hash %s)' % d.js('location.hash'))
        else:
            time.sleep(0.5)
            back_page = d.js("(document.querySelector('.ptb-pager .ptb-pg.on') || {textContent: ''}).textContent.trim()")
            if back_page != rec_page:
                p.append('detail Back returned to list page %s, was %s' % (back_page, rec_page))
    # the list's back link: User management on the same page
    d.tap_sel('a.rec-back')
    if not until(d, "location.hash === '#/users' && document.getElementById('us-next')", 8.0):
        p.append('the list back link did not return to User management (hash %s)' % d.js('location.hash'))
    else:
        time.sleep(0.5)
        page_after = js(d, USERS_JS)['page']
        if page_after != page_before:
            p.append('User management came back on page %s, was page %s' % (page_after, page_before))
    verdict(rc, 'W-V27-UR', 'users', 'records', 'admin', lang, theme, size, d, p,
            "a user's records read-only (no delete / AI / retry), detail and Back keep the page, User management keeps its page")


def g_v27(rc):
    cases(rc, 'W-V27-QD', 'quiz', qd_case, QD_PHONES + [DESKTOP])
    cases(rc, 'W-V27-QT', 'quiz', qt_case, QT_PHONES)
    cases(rc, 'W-V27-PG', 'users', pg_case, PG_SIZES)
    cases(rc, 'W-V27-UR', 'users', ur_case, UR_SIZES)


GROUPS_V27 = [g_v27]
