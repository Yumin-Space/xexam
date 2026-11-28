# -*- coding: utf-8 -*-
"""v3.1 walk checks, part 1: question bank categories. Made-up accounts and data only: the seed (tools/screenshots/seed.py)
creates the categories `Science` (holding `General knowledge`) and `Everyday life` (holding `Kitchen basics`); the 51-question
walk bank `Long sample bank` stays uncategorised; the plain user alex is granted only `General knowledge` (so alex sees only
`Science`), riley has every bank. All checks run in zh / en / ja; the ones that only look (CAT-1, CAT-5) in light and dark,
the ones that change data (CAT-2, CAT-3, CAT-6) in light only, and every one of them restores the seed.

  W-V31-CAT-1  banks (admin)   `.bk-cat` sections in category order with `.bk-cathead` names, "Uncategorised" last; every
                               `.bk-bank` still has `.bk-bankhead` / `.bk-bankname` / `.bk-box` / `.bk-bankacts`; folding hides the
                               group's cards and unfolding shows them again; no sideways scroll; fold buttons >= 44 px on phones
  W-V31-CAT-2  banks (admin)   `#bk-catmanage` opens the manager; create the made-up category `Walk test`, rename it, move it up,
                               delete it; the list follows each step (and the server agrees); every control >= 44 px on phones;
                               the dialog fits the width
  W-V31-CAT-3  banks (admin)   select two banks, `#bk-movecat`, pick `Science`: both cards appear under Science; they are moved
                               back at the end (the seed is restored through the API even when a step fails)
  W-V31-CAT-4  banks (sam)     a bank manager sees neither `#bk-catmanage` nor `#bk-movecat`; the categories are still shown
  W-V31-CAT-5  study (admin, alex)  `.home-cat` groups; `.home-catbox` ticks and clears every sheet of its banks and shows the
                               indeterminate state for a partial selection; folding; alex sees only `Science` (and no
                               "Uncategorised" group)
  W-V31-CAT-6  users (admin)   the bank dialog shows `.us-cat-row` rows after `.us-later-row`; ticking one ticks its banks;
                               Cancel leaves the user unchanged
  W-V31-CAT-7  records (admin) the bank filter `<select>` has `<optgroup>`s named after the categories

Registered in checks.py (GROUPS_V31_CAT); run alone with --only 'W-V31-CAT-*'. The DOM hooks are the ones of the contract
(section D.9 / D.13): the manager's buttons are found by their icon (the path of the up / down / pencil / trash mini icon) and
by the inline text field, never by a language-dependent label.
"""
import json
import re
import time

from checks import LANGS, SETUP, goto, phone, until
from checks_v13 import common, verdict
from checks_v22 import js

PHONES = [(393, 852), (320, 640)]
DESKTOP = (1280, 800)
UNCAT_RE = re.compile(r'未分类|未分類|Uncategori[sz]ed', re.I)
TEST_CAT = 'Walk test'
TEST_CAT_2 = 'Walk test 2'

# Waits for the bank list of the Banks page (cards drawn)
BANKS_READY = "document.querySelector('.page-banks .bk-bank')"
HIT = "const hit = (e) => { const b = window.__walk.hitBox(e); return {w: b.r - b.l, h: b.b - b.t}; };"
# the mini icons are told apart by the first path of their drawing (domain/mini-icon-shapes.ts)
ICON_UP, ICON_DOWN, ICON_PENCIL, ICON_TRASH = 'M8 13V3.5', 'M8 3v9.5', 'M10.6 2.6', 'M2.8 4.3h10.4'


def cat_state(rc):
    """(banks, categories) as the administrator's GET /api/banks answers them (the ground truth of every check)."""
    r = rc.session('admin').call('/api/banks')
    return r['banks'], r.get('categories', [])


def bank_id(banks, name):
    return next(b['id'] for b in banks if b['name'].startswith(name))


def restore_seed(rc):
    """Puts the seed back: General knowledge in Science, Kitchen basics in Everyday life, the walk bank uncategorised, no
    `Walk test` category left (used in `finally`, so a failed step never leaks into the next check)."""
    s = rc.session('admin')
    banks, cats = cat_state(rc)
    by_name = {c['name']: c['id'] for c in cats}
    for c in cats:
        if c['name'] in (TEST_CAT, TEST_CAT_2):
            s.call('/api/banks/categories', {'op': 'delete', 'id': c['id']})
    banks, cats = cat_state(rc)
    by_name = {c['name']: c['id'] for c in cats}
    plan = (('General knowledge', by_name.get('Science')), ('Kitchen basics', by_name.get('Everyday life')),
            ('Long sample bank', None))
    for name, cid in plan:
        try:
            bid = bank_id(banks, name)
        except StopIteration:
            continue
        have = next(b['categoryId'] for b in banks if b['id'] == bid)
        if have != cid:
            s.call('/api/banks/category', {'ids': [bid], 'categoryId': cid})


def cases(rc, cid, page, fn, sizes, themes=None):
    if not rc.want(cid) or not rc.walk.wants_page(page):
        return
    for lang in rc.langs(LANGS):
        for theme in (themes or rc.themes(['light', 'dark'])):
            for size in rc.sizes(sizes, lambda s: True):
                rc.guard(cid, page, fn, rc, lang, theme, size)


def light_cases(rc, cid, page, fn, sizes):
    cases(rc, cid, page, fn, sizes, rc.themes(['light'])[:1])


def open_banks(d, user_seconds=2.0):
    goto(d, '#/banks', user_seconds)
    until(d, BANKS_READY, 15.0)
    time.sleep(0.3)


# ------------------------------------------------------------------------------------------------ W-V31-CAT-1

CAT1_JS = HIT + """
const secs = [...document.querySelectorAll('.bk-cat')];
const parts = ['.bk-bankhead', '.bk-bankname', '.bk-box', '.bk-bankacts'];
const out = secs.map((s) => {
  const head = s.querySelector('.bk-cathead'), nm = s.querySelector('.bk-catname'), fold = s.querySelector('.bk-catfold');
  const cards = [...s.querySelectorAll('.bk-bank')];
  return {head: !!head, name: nm ? nm.textContent.trim() : null, fold: !!fold, foldBox: fold ? hit(fold) : null,
    expanded: fold ? fold.getAttribute('aria-expanded') : null, cards: cards.length, shown: cards.filter(vis).length,
    missing: cards.map((c) => parts.filter((p) => !c.querySelector(p))).filter((m) => m.length).length};
});
return {secs: out, allCards: document.querySelectorAll('.bk-bank').length, outside: [...document.querySelectorAll('.bk-bank')].filter((c) => !c.closest('.bk-cat')).length,
  sideways: document.documentElement.scrollWidth - innerWidth};"""


def cat1_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    banks, cats = cat_state(rc)
    open_banks(d)
    r = js(d, CAT1_JS)
    p = []
    names = [x['name'] for x in r['secs']]
    want = [c['name'] for c in cats]
    has_uncat = any(b['categoryId'] is None for b in banks)
    if len(r['secs']) != len(want) + (1 if has_uncat else 0):
        p.append('%d .bk-cat sections, expected %d (%s%s)' % (len(r['secs']), len(want) + (1 if has_uncat else 0), ', '.join(want),
                                                             ' + uncategorised' if has_uncat else ''))
    elif names[:len(want)] != want:
        p.append('the groups are in the order %s, expected %s' % (names[:len(want)], want))
    elif has_uncat and not UNCAT_RE.search(names[-1] or ''):
        p.append('the last group is "%s", expected the "Uncategorised" group' % names[-1])
    for x in r['secs']:
        if not x['head'] or not x['name']:
            p.append('a group has no .bk-cathead / .bk-catname')
            break
    if r['allCards'] != len(banks) or r['outside']:
        p.append('%d bank cards on the page (%d outside a group), the server has %d banks' % (r['allCards'], r['outside'], len(banks)))
    if any(x['missing'] for x in r['secs']):
        p.append('a bank card lost one of .bk-bankhead / .bk-bankname / .bk-box / .bk-bankacts')
    if r['sideways'] > 1:
        p.append('the page is %d px wider than the viewport' % r['sideways'])
    if phone(size):
        small = [x['foldBox'] for x in r['secs'] if x['foldBox'] and min(x['foldBox']['w'], x['foldBox']['h']) < 43.5]
        if small:
            p.append('a fold button is %dx%d px (< 44)' % (round(small[0]['w']), round(small[0]['h'])))
    folded = next((i for i, x in enumerate(r['secs']) if x['fold'] and x['cards']), None)
    if folded is None:
        p.append('no group with a fold button and cards to fold')
    else:
        probe = "(() => { const s = document.querySelectorAll('.bk-cat')[%d]; const f = s.querySelector('.bk-catfold');" \
                " return {exp: f.getAttribute('aria-expanded'), shown: [...s.querySelectorAll('.bk-bank')].filter((e) => e.getClientRects().length > 0).length} })()" % folded
        d.js("document.querySelectorAll('.bk-cat')[%d].querySelector('.bk-catfold').scrollIntoView({block: 'center'}); 1" % folded)
        time.sleep(0.2)
        d.tap_sel('.bk-catfold', folded, 0.5)
        after = d.js(probe)
        if after['shown'] != 0 or after['exp'] != 'false':
            p.append('after folding the group still shows %d cards (aria-expanded %s)' % (after['shown'], after['exp']))
        d.tap_sel('.bk-catfold', folded, 0.5)
        again = d.js(probe)
        if again['shown'] != r['secs'][folded]['cards'] or again['exp'] != 'true':
            p.append('after unfolding the group shows %d of %d cards (aria-expanded %s)' % (again['shown'], r['secs'][folded]['cards'], again['exp']))
    p += ['common: ' + x for x in common(d, size)[:3]]
    verdict(rc, 'W-V31-CAT-1', 'banks', 'groups', 'admin', lang, theme, size, d, p,
            '%d groups in order (%s), cards intact, folding works, fold buttons >= 44 px on phones' % (len(r['secs']), ', '.join(n or '?' for n in names)))


# ------------------------------------------------------------------------------------------------ W-V31-CAT-2

# The manager dialog: the create field is the dialog's first text input; every other control is found by its icon
DLG_JS = HIT + """
const dlg = document.querySelector('dialog[open]');
if (!dlg) return {open: false};
const r = box(dlg);
const btns = [...dlg.querySelectorAll('button')].filter(vis);
const small = btns.map((b) => [b, hit(b)]).filter(([b, h]) => Math.min(h.w, h.h) < 43.5 && !b.matches('.dlg-x, [data-dismiss]'))
  .map(([b, h]) => (b.getAttribute('aria-label') || b.textContent.trim() || b.className).slice(0, 24) + ' ' + Math.round(h.w) + 'x' + Math.round(h.h));
const text = [...dlg.querySelectorAll('input:not([type=checkbox]):not([type=radio])')].map((i) => i.value);
return {open: true, l: r.l, r: r.r, w: r.w, vw: innerWidth, small, inputs: text.length, sideways: document.documentElement.scrollWidth - innerWidth,
  text: dlg.textContent.replace(/\\s+/g, ' ').slice(0, 400)};"""

# The row of a category in the manager: the smallest element that holds the name (as text or as the value of an input) and
# at least two buttons
ROW_FN = """
const rowOf = (name) => {
  const dlg = document.querySelector('dialog[open]'); if (!dlg) return null;
  let best = null, bestN = 1e9;
  for (const e of dlg.querySelectorAll('li, div, tr, section')) {
    const has = (e.textContent || '').includes(name) || [...e.querySelectorAll('input')].some((i) => i.value === name);
    const n = e.querySelectorAll('*').length;
    if (has && e.querySelectorAll('button').length >= 2 && n < bestN) { best = e; bestN = n; }
  }
  return best;
};
const byIcon = (row, d) => [...row.querySelectorAll('button')].find((b) => [...b.querySelectorAll('path')].some((p) => (p.getAttribute('d') || '').startsWith(d)));
const setText = (i, v) => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(i, v);
  i.dispatchEvent(new Event('input', {bubbles: true})); i.dispatchEvent(new Event('change', {bubbles: true})); };
"""

CREATE_JS = ROW_FN + """
const dlg = document.querySelector('dialog[open]'); if (!dlg) return 'no dialog';
const inp = dlg.querySelector('input[type=text], input:not([type])'); if (!inp) return 'no create field';
inp.focus(); setText(inp, %s);
// the button after the field in document order creates the category
const all = [...dlg.querySelectorAll('input[type=text], input:not([type]), button')];
const btn = all.slice(all.indexOf(inp) + 1).find((e) => e.tagName === 'BUTTON');
if (!btn) return 'no create button'; btn.click(); return 'ok';"""

RENAME_JS = ROW_FN + """
let row = rowOf(%s); if (!row) return 'no row';
let inp = [...row.querySelectorAll('input')].find((i) => i.value === %s);
if (!inp) { const pen = byIcon(row, %s); if (pen) { pen.click(); return 'opened'; } return 'no rename control'; }
inp.focus(); setText(inp, %s); return 'typed';"""

SAVE_JS = ROW_FN + """
const row = rowOf(%s); if (!row) return 'no row';
const save = [...row.querySelectorAll('button.btn')].find((x) => x.getClientRects().length > 0);
if (save) { save.click(); return 'saved'; } return 'no save button';"""

MOVE_JS = ROW_FN + """
const row = rowOf(%s); if (!row) return 'no row';
const b = byIcon(row, %s); if (!b) return 'no arrow'; if (b.disabled || b.getAttribute('aria-disabled') === 'true') return 'disabled'; b.click(); return 'ok';"""

DELETE_JS = ROW_FN + """
const row = rowOf(%s); if (!row) return 'no row';
const b = byIcon(row, %s); if (!b) return 'no delete control'; if (b.disabled) return 'disabled'; b.click(); return 'ok';"""

CONFIRM_JS = """
const c = [...document.querySelectorAll('dialog[open]')].pop();
const b = c && c.querySelector('.btn.danger.solid');
if (!b) return 'no confirm button'; b.click(); return 'ok';"""


def api_names(rc):
    return [c['name'] for c in cat_state(rc)[1]]


def wait_names(rc, ok, seconds=6.0):
    end = time.time() + seconds
    names = api_names(rc)
    while time.time() < end and not ok(names):
        time.sleep(0.4)
        names = api_names(rc)
    return names


def cat2_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    p = []
    try:
        restore_seed(rc)
        open_banks(d)
        if not d.js("!!document.querySelector('#bk-catmanage')"):
            p.append('no #bk-catmanage button for the administrator')
        else:
            d.tap_sel('#bk-catmanage', 0, 0.7)
            if not until(d, "document.querySelector('dialog[open]')", 5.0):
                p.append('#bk-catmanage did not open a dialog')
            else:
                dl = js(d, DLG_JS)
                if dl['l'] < -0.5 or dl['r'] > dl['vw'] + 0.5:
                    p.append('the dialog is %d..%d px wide in a %d px window' % (dl['l'], dl['r'], dl['vw']))
                if dl['sideways'] > 1:
                    p.append('the page is %d px wider than the viewport' % dl['sideways'])
                base_names = api_names(rc)
                for c in base_names:
                    if c not in dl['text']:
                        p.append('the manager does not list the category "%s"' % c)
                said = js(d, CREATE_JS % json.dumps(TEST_CAT))
                if said != 'ok':
                    p.append('create: ' + said)
                names = wait_names(rc, lambda n: TEST_CAT in n)
                if TEST_CAT not in names:
                    p.append('create: the server has no category "%s" (%s)' % (TEST_CAT, names))
                else:
                    time.sleep(0.5)
                    dl = js(d, DLG_JS)
                    if TEST_CAT not in dl['text'] and not d.js("[...document.querySelectorAll('dialog[open] input')].some((i) => i.value === %s)" % json.dumps(TEST_CAT)):
                        p.append('create: the list does not show the new category')
                    # rename: the inline field (a pencil may open it first), Enter or the row's save button commits
                    step = js(d, RENAME_JS % (json.dumps(TEST_CAT), json.dumps(TEST_CAT), json.dumps(ICON_PENCIL), json.dumps(TEST_CAT_2)))
                    if step == 'opened':
                        time.sleep(0.4)
                        step = js(d, RENAME_JS % (json.dumps(TEST_CAT), json.dumps(TEST_CAT), json.dumps(ICON_PENCIL), json.dumps(TEST_CAT_2)))
                    if step != 'typed':
                        p.append('rename: ' + step)
                    else:
                        d.js("(() => { const i = [...document.querySelectorAll('dialog[open] input')].find((x) => x.value === %s); i && i.focus(); return 1 })()" % json.dumps(TEST_CAT_2))
                        d.key('Enter', 'Enter', 13)
                        names = wait_names(rc, lambda n: TEST_CAT_2 in n, 2.5)
                        if TEST_CAT_2 not in names:
                            js(d, SAVE_JS % json.dumps(TEST_CAT_2))
                            names = wait_names(rc, lambda n: TEST_CAT_2 in n, 4.0)
                        if TEST_CAT_2 not in names:
                            p.append('rename: the server still has %s' % names)
                    cur = TEST_CAT_2 if TEST_CAT_2 in api_names(rc) else TEST_CAT
                    # move up: one place before the previous category
                    before = api_names(rc)
                    if len(before) >= 2:
                        said = js(d, MOVE_JS % (json.dumps(cur), json.dumps(ICON_UP)))
                        if said != 'ok':
                            p.append('move up: ' + said)
                        else:
                            after = wait_names(rc, lambda n: n.index(cur) < before.index(cur) if cur in n else False, 5.0)
                            if cur not in after or after.index(cur) != before.index(cur) - 1:
                                p.append('move up: the order is %s, expected "%s" one place earlier than %s' % (after, cur, before))
                    # delete: the confirm dialog, then the row is gone
                    said = 'disabled'
                    for _ in range(15):     # the trash button stays disabled while the move is still being saved
                        said = js(d, DELETE_JS % (json.dumps(cur), json.dumps(ICON_TRASH)))
                        if said != 'disabled':
                            break
                        time.sleep(0.4)
                    if said != 'ok':
                        p.append('delete: ' + said)
                    else:
                        until(d, 'document.querySelectorAll("dialog[open]").length >= 2', 5.0)
                        if js(d, CONFIRM_JS) != 'ok':
                            p.append('delete: no confirm dialog')
                        names = wait_names(rc, lambda n: cur not in n, 6.0)
                        if cur in names:
                            p.append('delete: the server still has "%s"' % cur)
                    if phone(size):
                        dl = js(d, DLG_JS)
                        if dl.get('open') and dl['small']:
                            p.append('controls under 44 px: %s' % '; '.join(dl['small'][:4]))
    finally:
        try:
            d.js("document.querySelectorAll('dialog[open]').forEach((x) => x.close()); 1")
        except Exception:
            pass
        restore_seed(rc)
    verdict(rc, 'W-V31-CAT-2', 'banks', 'manager', 'admin', lang, theme, size, d, p,
            'the manager creates, renames, moves up and deletes a category; the list and the server agree; controls >= 44 px on phones')


# ------------------------------------------------------------------------------------------------ W-V31-CAT-3

SELECT_JS = """
const names = %s; const out = [];
for (const card of document.querySelectorAll('.bk-bank')) {
  const nm = card.querySelector('.bk-bankname'); const box = card.querySelector('.bk-box');
  if (nm && box && names.some((n) => nm.textContent.includes(n)) && !box.checked) { box.scrollIntoView({block: 'center'}); box.click(); out.push(nm.textContent.trim()); }
}
return out;"""

GROUP_OF_JS = """
const name = %s; const card = [...document.querySelectorAll('.bk-bank')].find((c) => (c.querySelector('.bk-bankname') || {textContent: ''}).textContent.includes(name));
if (!card) return null; const sec = card.closest('.bk-cat'); const nm = sec && sec.querySelector('.bk-catname');
return nm ? nm.textContent.trim() : null;"""

PICK_JS = """
const dlg = document.querySelector('dialog[open]'); if (!dlg) return 'no dialog';
const target = %s;
const rows = [...dlg.querySelectorAll('label, li, [role=radio], [role=option]')];
const row = rows.find((r) => (r.textContent || '').trim().startsWith(target));
if (!row) return 'no entry for ' + target + ' (' + rows.map((r) => r.textContent.trim().slice(0, 20)).join(' | ') + ')';
const radio = row.querySelector('input[type=radio]') || row; radio.click();
const ok = dlg.querySelector('.acts .btn.primary, .btn.primary'); if (!ok) return 'no confirm button'; ok.click(); return 'ok';"""


def cat3_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    p = []
    moved = ['Kitchen basics', 'Long sample bank']
    try:
        restore_seed(rc)
        open_banks(d)
        if not d.js("!!document.querySelector('.bk-selbar')"):
            p.append('no selection toolbar (.bk-selbar)')
        else:
            got = d.js("(() => { %s })()" % (SELECT_JS % json.dumps(moved)))
            if len(got) != 2:
                p.append('selected %d of the two banks %s' % (len(got), moved))
            time.sleep(0.5)
            if not d.js("!!document.querySelector('#bk-movecat')"):
                p.append('no #bk-movecat button in the selection toolbar')
            else:
                if phone(size):
                    h = d.js("(() => { const b = document.querySelector('#bk-movecat'); const x = window.__walk.hitBox(b); return Math.min(x.r - x.l, x.b - x.t) })()")
                    if h < 43.5:
                        p.append('#bk-movecat is %d px (< 44)' % round(h))
                d.tap_sel('#bk-movecat', 0, 0.7)
                if not until(d, "document.querySelector('dialog[open]')", 5.0):
                    p.append('#bk-movecat did not open a dialog')
                else:
                    said = d.js('(() => { %s })()' % (PICK_JS % json.dumps('Science')))
                    if said != 'ok':
                        p.append('pick: ' + said)
                    else:
                        until(d, "(() => { const n = (n) => [...document.querySelectorAll('.bk-bank')].find((c) => (c.querySelector('.bk-bankname') || {textContent: ''}).textContent.includes(n)); "
                                 "const c = n('Long sample bank'); const s = c && c.closest('.bk-cat'); const h = s && s.querySelector('.bk-catname'); return h && h.textContent.trim() === 'Science' })()", 8.0)
                        for name in moved:
                            g = d.js('(() => { %s })()' % (GROUP_OF_JS % json.dumps(name)))
                            if g != 'Science':
                                p.append('"%s" sits under "%s" after the move, expected Science' % (name, g))
                        banks, cats = cat_state(rc)
                        sci = next((c['id'] for c in cats if c['name'] == 'Science'), None)
                        for name in moved:
                            if next((b['categoryId'] for b in banks if b['name'].startswith(name)), None) != sci:
                                p.append('the server does not have "%s" in Science' % name)
    finally:
        restore_seed(rc)
    banks, cats = cat_state(rc)
    ev = next((c['id'] for c in cats if c['name'] == 'Everyday life'), None)
    if next((b['categoryId'] for b in banks if b['name'].startswith('Kitchen basics')), None) != ev:
        p.append('the seed was not restored (Kitchen basics is not back in Everyday life)')
    verdict(rc, 'W-V31-CAT-3', 'banks', 'move', 'admin', lang, theme, size, d, p,
            'two selected banks moved to Science through #bk-movecat and appear under it; the seed is back')


# ------------------------------------------------------------------------------------------------ W-V31-CAT-4

CAT4_JS = """
return {manage: !!document.querySelector('#bk-catmanage'), move: !!document.querySelector('#bk-movecat'),
  secs: [...document.querySelectorAll('.bk-cat')].map((s) => (s.querySelector('.bk-catname') || {textContent: ''}).textContent.trim()),
  cards: document.querySelectorAll('.bk-bank').length, sideways: document.documentElement.scrollWidth - innerWidth};"""


def cat4_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    open_banks(d)
    # tick a bank so that a selection toolbar with its actions is on screen
    d.js("(() => { const b = document.querySelector('.bk-bank .bk-box'); if (b && !b.checked) b.click(); return 1 })()")
    time.sleep(0.5)
    r = js(d, CAT4_JS)
    p = []
    if r['manage']:
        p.append('a bank manager sees #bk-catmanage')
    if r['move']:
        p.append('a bank manager sees #bk-movecat')
    if not r['secs']:
        p.append('no category groups shown to the bank manager')
    if not r['cards']:
        p.append('no bank cards')
    if r['sideways'] > 1:
        p.append('the page is %d px wider than the viewport' % r['sideways'])
    verdict(rc, 'W-V31-CAT-4', 'banks', 'manager-role', 'sam', lang, theme, size, d, p,
            'no category management for a bank manager; %d group(s) still shown (%s)' % (len(r['secs']), ', '.join(r['secs'])))


# ------------------------------------------------------------------------------------------------ W-V31-CAT-5

CAT5_JS = HIT + """
const cats = [...document.querySelectorAll('.home-cat')];
return {cats: cats.map((c) => {
  const nm = c.querySelector('.home-cathead'); const box = c.querySelector('.home-catbox'); const fold = c.querySelector('.home-catfold');
  const banks = [...c.querySelectorAll('.home-bank')];
  const checks = banks.flatMap((b) => [...b.querySelectorAll('input[type=checkbox]')]);
  return {name: nm ? nm.textContent.replace(/\\s+/g, ' ').trim() : null, hasBox: !!box, fold: !!fold, foldBox: fold ? hit(fold) : null,
    banks: banks.length, checked: checks.filter((x) => x.checked).length, boxes: checks.length,
    catChecked: box ? box.checked : null, catInd: box ? box.indeterminate : null, expanded: fold ? fold.getAttribute('aria-expanded') : null,
    shown: banks.filter(vis).length};
}), sideways: document.documentElement.scrollWidth - innerWidth};"""

CAT5_TOGGLE_JS = """
const i = %d; const c = document.querySelectorAll('.home-cat')[i]; const box = c.querySelector('.home-catbox');
box.scrollIntoView({block: 'center'}); box.click(); return 1;"""

CAT5_PARTIAL_JS = """
const c = document.querySelectorAll('.home-cat')[%d];
const inner = [...c.querySelectorAll('.home-sheetrow input:checked')];
if (inner.length) { inner[0].click(); return inner.length; } return 0;"""

NO_UNCAT = "[...document.querySelectorAll('.home-cat')].every((c) => !/%s/i.test((c.querySelector('.home-cathead') || {textContent: ''}).textContent))"


def study_ready(d):
    goto(d, SETUP, 1.8)
    until(d, "document.querySelector('.home-cat') || document.querySelector('.home-bank')", 12.0)
    time.sleep(0.4)


def cat5_case(rc, lang, theme, size):
    d = rc.dev('lee', lang, theme, size)       # lee: an administrator who sees every bank
    study_ready(d)
    r = js(d, CAT5_JS)
    p = []
    cats = r['cats']
    if not cats:
        p.append('no .home-cat group on the Exam & practice page')
    else:
        if any(not c['hasBox'] or not c['fold'] or not c['name'] for c in cats):
            p.append('a group lacks its head, its tick box or its fold button')
        if phone(size) and any(c['foldBox'] and min(c['foldBox']['w'], c['foldBox']['h']) < 43.5 for c in cats):
            p.append('a group fold button is under 44 px')
        if r['sideways'] > 1:
            p.append('the page is %d px wider than the viewport' % r['sideways'])
        i = next((k for k, c in enumerate(cats) if c['boxes'] >= 1), 0)
        # tick the group: every box of its banks is ticked
        d.js('(() => { %s })()' % (CAT5_TOGGLE_JS % i))
        time.sleep(0.5)
        a = js(d, CAT5_JS)['cats'][i]
        if a['checked'] != a['boxes'] or not a['boxes']:
            p.append('ticking the category left %d of %d sheet boxes ticked' % (a['checked'], a['boxes']))
        if a['catChecked'] is not True:
            p.append('the category box is not checked after ticking it')
        # untick one sheet: indeterminate
        d.js('(() => { %s })()' % (CAT5_PARTIAL_JS % i))
        time.sleep(0.5)
        b = js(d, CAT5_JS)['cats'][i]
        if a['boxes'] > 1 and b['catInd'] is not True:
            p.append('with %d of %d sheets ticked the category box is not indeterminate' % (b['checked'], b['boxes']))
        # clear the group
        d.js("(() => { const box = document.querySelectorAll('.home-cat')[%d].querySelector('.home-catbox'); if (box.indeterminate || !box.checked) box.click(); if (box.checked) box.click(); return 1 })()" % i)
        time.sleep(0.5)
        c = js(d, CAT5_JS)['cats'][i]
        if c['checked'] != 0:
            p.append('clearing the category left %d sheet boxes ticked' % c['checked'])
        # fold the group
        d.tap_sel('.home-catfold', i, 0.5)
        f = js(d, CAT5_JS)['cats'][i]
        if f['shown'] != 0 or f['expanded'] != 'false':
            p.append('after folding the group shows %d banks (aria-expanded %s)' % (f['shown'], f['expanded']))
    # alex sees only Science, and no "Uncategorised" group
    d2 = rc.dev('alex', lang, theme, size)
    study_ready(d2)
    ra = js(d2, CAT5_JS)['cats']
    names = [c['name'] or '' for c in ra]
    if len(ra) != 1 or 'Science' not in names[0]:
        p.append('alex sees the groups %s, expected only Science' % names)
    if any(UNCAT_RE.search(n) for n in names):
        p.append('alex sees an "Uncategorised" group')
    verdict(rc, 'W-V31-CAT-5', 'study', 'groups', 'lee', lang, theme, size, d, p,
            '%d groups; the group box ticks, clears and shows the indeterminate state; folding works; alex sees only Science' % len(cats))


# ------------------------------------------------------------------------------------------------ W-V31-CAT-6

CAT6_JS = """
const dlg = document.querySelector('dialog[open]'); if (!dlg) return {open: false};
const later = dlg.querySelector('.us-later-row');
const rows = [...dlg.querySelectorAll('.us-cat-row')];
const kids = rows.map((r) => { const box = r.querySelector('.us-cat-box');
  let n = r.nextElementSibling; const ids = [];
  while (n && !n.matches('.us-cat-row')) { const c = n.querySelector('input[type=checkbox]'); if (c) ids.push(c); n = n.nextElementSibling; }
  return {box: box ? box.checked : null, ind: box ? box.indeterminate : null, banks: ids.length, ticked: ids.filter((c) => c.checked).length}; });
const after = !!later && rows.length > 0 && (later.compareDocumentPosition(rows[0]) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
return {open: true, later: !!later, cats: rows.length, kids, after,
  ticked: [...dlg.querySelectorAll('input[type=checkbox]')].filter((c) => c.checked).length};"""


def reveal_user_banks(d, uid):
    """Pages (or searches) the users list until the bank button of user `uid` is on the page (phones show 5 users per page)."""
    probe = "!!document.querySelector('#us-banks-%d')" % uid
    d.js("(() => { const i = document.querySelector('#us-search'); if (i) { i.value = 'riley'; i.dispatchEvent(new Event('input', {bubbles: true})); } return 1 })()")
    time.sleep(0.6)
    for _ in range(12):
        if d.js(probe) or not d.js("(() => { const n = document.querySelector('#us-next'); if (!n || n.getAttribute('aria-disabled') === 'true') return false; n.click(); return true })()"):
            return
        time.sleep(0.5)


def cat6_case(rc, lang, theme, size):
    d = rc.dev('admin', lang, theme, size)
    s = rc.session('admin')
    riley = s.user_id('riley')
    before = [u for u in s.call('/api/users')['users'] if u['id'] == riley][0]
    goto(d, '#/users?tab=users', 2.0)
    until(d, "document.querySelector('.us-row')", 15.0)
    p = []
    sel = '#us-banks-%d' % riley
    reveal_user_banks(d, riley)
    # riley sees every bank ('all'); the dialog starts from that state
    if not d.js("!!document.querySelector('%s')" % sel):
        p.append('no %s button' % sel)
    else:
        d.tap_sel(sel, 0, 0.8)
        if not until(d, "document.querySelector('dialog[open] .us-later-row')", 6.0):
            p.append('the bank dialog did not open (no .us-later-row)')
        else:
            r = js(d, CAT6_JS)
            if not r['cats']:
                p.append('no .us-cat-row rows in the bank dialog')
            elif not r['after']:
                p.append('the .us-cat-row rows do not come after .us-later-row')
            else:
                # untick the first category row, then tick it: its banks follow
                d.js("(() => { const b = document.querySelector('dialog[open] .us-cat-box'); b.scrollIntoView({block: 'center'}); if (b.checked || b.indeterminate) b.click(); return 1 })()")
                time.sleep(0.4)
                a = js(d, CAT6_JS)['kids'][0]
                if a['ticked'] != 0 and a['banks']:
                    p.append('clearing the category left %d of %d banks ticked' % (a['ticked'], a['banks']))
                d.js("(() => { const b = document.querySelector('dialog[open] .us-cat-box'); if (!b.checked) b.click(); return 1 })()")
                time.sleep(0.4)
                b = js(d, CAT6_JS)['kids'][0]
                if b['banks'] and b['ticked'] != b['banks']:
                    p.append('ticking the category ticked %d of %d banks' % (b['ticked'], b['banks']))
            # Cancel: the last non-primary button of the dialog actions
            d.js("(() => { const x = [...document.querySelectorAll('dialog[open] .acts .btn')].find((e) => !e.classList.contains('primary')); x && x.click(); return 1 })()")
            time.sleep(0.6)
            if d.js("!!document.querySelector('dialog[open]')"):
                d.js("document.querySelectorAll('dialog[open]').forEach((x) => x.close()); 1")
    after = [u for u in s.call('/api/users')['users'] if u['id'] == riley][0]
    if after.get('banks') != before.get('banks'):
        p.append('Cancel changed the user: banks %r -> %r' % (before.get('banks'), after.get('banks')))
    verdict(rc, 'W-V31-CAT-6', 'users', 'bank-dialog', 'admin', lang, theme, size, d, p,
            'the bank dialog lists category rows after "include later"; a category tick ticks its banks; Cancel changes nothing')


# ------------------------------------------------------------------------------------------------ W-V31-CAT-7

CAT7_JS = """
const sels = [...document.querySelectorAll('.rec-bank-sel')];
return sels.map((s) => ({groups: [...s.querySelectorAll('optgroup')].map((g) => g.label), plain: s.querySelectorAll(':scope > option').length,
  options: s.querySelectorAll('option').length}));"""


def cat7_case(rc, lang, theme, size):
    d = rc.dev('lee', lang, theme, size)
    goto(d, '#/records', 2.0)
    until(d, "document.querySelector('.rec-bank-sel')", 12.0)
    r = d.js("(() => { %s })()" % CAT7_JS)
    p = []
    names = [c['name'] for c in cat_state(rc)[1]]
    if not r:
        p.append('no bank filter .rec-bank-sel')
    else:
        groups = r[0]['groups']
        if not groups:
            p.append('the bank filter has no <optgroup>')
        else:
            known = [g for g in groups if g in names or UNCAT_RE.search(g or '')]
            if len(known) != len(groups):
                p.append('optgroup labels %s are not category names (%s)' % (groups, names))
            if not any(g in names for g in groups):
                p.append('no optgroup is named after a category (%s)' % names)
        if r[0]['plain'] < 1:
            p.append('the "all banks" option is gone')
    verdict(rc, 'W-V31-CAT-7', 'records', 'filter', 'lee', lang, theme, size, d, p,
            'the bank filter groups its banks in <optgroup>s named after the categories (%s)' % ', '.join(r[0]['groups'] if r else []))


def g_v31_categories(rc):
    cases(rc, 'W-V31-CAT-1', 'banks', cat1_case, PHONES + [DESKTOP])
    light_cases(rc, 'W-V31-CAT-2', 'banks', cat2_case, PHONES[:1] + [DESKTOP])
    light_cases(rc, 'W-V31-CAT-3', 'banks', cat3_case, PHONES[:1] + [DESKTOP])
    light_cases(rc, 'W-V31-CAT-4', 'banks', cat4_case, PHONES[:1] + [DESKTOP])
    cases(rc, 'W-V31-CAT-5', 'study', cat5_case, PHONES + [DESKTOP])
    light_cases(rc, 'W-V31-CAT-6', 'users', cat6_case, PHONES[:1] + [DESKTOP])
    light_cases(rc, 'W-V31-CAT-7', 'records', cat7_case, PHONES[:1] + [DESKTOP])


GROUPS_V31_CAT = [g_v31_categories]
