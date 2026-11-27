# -*- coding: utf-8 -*-
"""v1.2 checks of the quiz page (A-OWNER-81): the top-row finish pill always shows its text."""
import time

from checks import LANGS, goto, start_quiz  # noqa: F401

SIZES = [(320, 640), (360, 740), (375, 667), (390, 844), (393, 852), (402, 874), (430, 932), (740, 360), (844, 390)]

# The visible label, the pill and the row, measured in the page
PILL_JS = """(() => {
  const b = document.querySelector('.qbar .qsubmit'); const row = document.querySelector('.qbar');
  if (!b || !row || !b.getClientRects().length) return { found: false };
  const vis = [...b.querySelectorAll('.lbl, .lbl-s')].filter(e => e.getClientRects().length);
  // v2.7: each label carries the other mode's word as an invisible ghost (.same-w-ghost) so both modes keep one width; read the shown word only
  const tx = e => { const c = e.cloneNode(true); c.querySelectorAll('.same-w-ghost').forEach(g => g.remove()); return c.textContent.trim() };
  const txt = vis.map(tx).join('');
  const ghostShown = [...b.querySelectorAll('.same-w-ghost')].some(g => g.getClientRects().length && getComputedStyle(g).visibility !== 'hidden');
  const br = b.getBoundingClientRect(), rr = row.getBoundingClientRect();
  // v2.7: on phones the position is the answer-sheet button, a small word over "1 / n"; the numbers line (.qpos-n) must stay one line
  const pos = row.querySelector('h1.l'); const num = pos ? (pos.querySelector('.pos-short .qpos-n') || pos.querySelector('.pos-short')) : null;
  const nr = num && num.getClientRects().length ? num.getBoundingClientRect() : null;
  const phone = matchMedia('(max-width: 600px), (orientation: landscape) and (max-height: 500px)').matches;
  const kids = [...row.querySelectorAll('.ql, .r, .r > *')].filter(e => e.getClientRects().length);
  const tops = kids.map(e => { const r = e.getBoundingClientRect(); return r.top + r.height / 2 });
  return { found: true, txt, ghostShown, phone, tight: row.classList.contains('tight'), left: br.left, right: br.right, w: br.width, h: br.height,
    rowRight: rr.right, rowLeft: rr.left, scrollOver: row.scrollWidth > row.clientWidth + 1,
    numOneLine: !nr || nr.height < 26, spread: Math.max(...tops) - Math.min(...tops),
    lblClipped: vis.some(e => e.scrollWidth > e.clientWidth + 1) } })()"""

# (full label, short label). v2.2: phones show the short label in every language and both modes (the same form in practice and
# exam; zh 完成 too); from 601 px to the rail line (880 px) the full label shows
WANT = {'practice': {'zh': ('完成练习', '完成'), 'en': ('Finish practice', 'Finish'), 'ja': ('練習を終了', '終了')},
        'exam': {'zh': ('完成考试', '完成'), 'en': ('Finish exam', 'Finish'), 'ja': ('試験を終了', '終了')}}


def pill_case(rc, lang, theme, size):
    for user, kind in (('sam', 'practice'), ('riley', 'exam')):
        d = rc.dev(user, lang, theme, size)
        start_quiz(d, kind, 51)
        time.sleep(0.4)
        r = d.js(PILL_JS) or {}
        if not r.get('found'):
            rc.rec('W-QZ-81', 'quiz', kind, user, size, lang, theme, False, 'no finish pill in the top row', d)
            continue
        want = WANT[kind][lang]
        want = want if isinstance(want, tuple) else (want,)
        bad = []
        if r['txt'] not in want:
            bad.append('label %r, want one of %r' % (r['txt'], want))
        elif r['txt'] != want[1 if r['phone'] else 0]:
            bad.append('%s label on a %s' % ('full' if r['phone'] else 'short', 'phone' if r['phone'] else 'tablet-wide row'))
        if r['ghostShown']:
            bad.append('the other mode\'s ghost label is visible')
        if r['right'] > r['rowRight'] + 0.5 or r['left'] < r['rowLeft'] - 0.5:
            bad.append('pill outside the row')
        if r['scrollOver']:
            bad.append('row overflows')
        if r['spread'] > 6:
            bad.append('row is not one line (%.0f px)' % r['spread'])
        if not r['numOneLine']:
            bad.append('"1 / n" breaks')
        if r['lblClipped']:
            bad.append('label clipped')
        rc.rec('W-QZ-81', 'quiz', kind, user, size, lang, theme, not bad,
               '; '.join(bad) or 'label %r%s, pill %.0fx%.0f' % (r['txt'], ' (short)' if r['tight'] else '', r['w'], r['h']), d if bad else None)


def g_pill(rc):
    """W-QZ-81: the finish pill always shows its text (the short label on phones, the full one from 601 px; never the ghost)."""
    if not rc.want('W-QZ-81') or not rc.walk.wants_page('quiz'):
        return
    for lang in rc.langs(LANGS):
        for theme in rc.themes(['light']):
            for size in rc.sizes(SIZES, ok=lambda s: s[0] <= 880 or min(s) < 500):
                rc.guard('W-QZ-81', 'quiz', pill_case, rc, lang, theme, size)


GROUPS_V12_QUIZ = [g_pill]
