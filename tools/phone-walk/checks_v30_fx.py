# -*- coding: utf-8 -*-
"""v3.0 walk checks: the answer effects of a practice (confetti for a right answer, two claw marks for a wrong one) and their
Settings switch. Made-up accounts and data only. The overlay is a fixed element with class fx-layer appended to body.

  W-FX-1  Settings (sam): the "Answer effects" switch exists next to the auto-advance switch, is ON by default (the server reset
          gives answerFx true), shows its hint, is >= 44 px high on phones, fits the width (no sideways scroll), and a tap turns
          it off and on again (the server value follows). zh / en / ja, light and dark.
  W-FX-2  practice (sam), switch on: checking a right answer creates a fixed, pointer-events none fx-layer with a canvas that does
          not block the next tap and is gone within about 8 s; checking a wrong answer creates the claw overlay (a canvas that is
          painted as a wipe); the Next tap during the effect still advances and ends it.
  W-FX-3  practice, switch off: neither a right nor a wrong answer creates an overlay.
  W-FX-4  practice, prefers-reduced-motion emulated: no overlay for either verdict.

The walk's devices save answerFx = false (device.save_settings) so that the effects never disturb the other checks; only the
W-FX checks turn it on, and each puts the default of the walk back when it is done.

Registered in checks.py (GROUPS_V30); run alone with --only 'W-FX-*'.
"""
import json
import time

from checks import goto, phone, start_quiz, until
from checks_v25 import DESKTOP, cases
from checks_v13 import verdict
from checks_v22 import js

FX_SIZES = [(360, 740), (393, 852), DESKTOP]
PRACTICE_SIZES = [(393, 852), (360, 740)]
LABEL = {'zh': '对错特效', 'en': 'Answer effects', 'ja': '正誤エフェクト'}
HINT = {'zh': '练习中核对答案时', 'en': 'In practice, when you check an answer', 'ja': '練習で答えを確認したとき'}

SW_JS = """
const label = %s, hint = %s;
const sws = [...document.querySelectorAll('.set-switch')];
const sw = sws.find((e) => (e.textContent || '').includes(label));
if (!sw) return {found: false, n: sws.length};
sw.scrollIntoView({block: 'center'});
const b = box(sw), auto = sws.find((e) => e !== sw);
return {found: true, checked: sw.getAttribute('aria-checked'), h: b.h, w: b.w, l: b.l, r: b.r, hint: (sw.textContent || '').includes(hint),
  role: sw.getAttribute('role'), n: sws.length, autoNear: !!auto && Math.abs(box(auto).t - b.t) < 400,
  sideways: document.documentElement.scrollWidth - innerWidth, vw: innerWidth};"""

API_JS = """(async () => { const r = await fetch('/api/settings'); const j = await r.json(); return j.answerFx })()"""
RESET_JS = """(async () => { const r = await fetch('/api/settings/reset', {method: 'POST', headers: {'Content-Type': 'application/json'}, body: '{}'});
  const g = await fetch('/api/settings'); const j = await g.json(); return {status: r.status, fx: j.answerFx, keys: Object.keys(j)} })()"""

# the overlay and the state of the verdict on the page, in one go
FX_JS = """
const l = [...document.querySelectorAll('.fx-layer:not(.fx-out)')].pop(); const cs = l ? getComputedStyle(l) : null;
const cv = l ? [...l.querySelectorAll('canvas')] : [];
const v = document.querySelector('.pverdict');
return {layer: !!l, pe: cs ? cs.pointerEvents : null, pos: cs ? cs.position : null, canvases: cv.length,
  wipes: cv.filter((c) => c.classList.contains('fx-wipe')).length, plain: cv.filter((c) => !c.classList.contains('fx-wipe')).length,
  cls: l ? l.className : '', verdict: v ? (v.classList.contains('ok') ? 'ok' : v.classList.contains('ng') ? 'ng' : '?') : null};"""

STEM = "(document.querySelector('.stem') || {textContent: ''}).textContent.trim().slice(0, 60)"


def fx(d):
    return js(d, FX_JS)


# a layer that is fading out (fx-out) belongs to an effect that already ended: never measured
LIVE = "document.querySelector('.fx-layer:not(.fx-out)')"
# the effect arms its tap-to-end listener 250 ms after it starts; a tap meant to end it waits until the effect is this old
ARMED = 0.45


def wait_layer(d, seconds=1.5):
    return until(d, LIVE, seconds, 0.05)


def wait_armed(s):
    """Sleeps until the effect of the check at s['t0'] listens for taps (so a tap ends it)."""
    time.sleep(max(0.0, ARMED - (time.time() - s['t0'])))


def wait_no_layer(d, seconds):
    return until(d, "!document.querySelector('.fx-layer')", seconds, 0.2)


def pick_and_check(d, idx):
    """Picks option `idx` (modulo the options) of the open question and taps Check; True when a verdict appeared."""
    n = d.js("document.querySelectorAll('.opt').length") or 0
    if not n:
        return False
    d.tap_sel('.opt', idx % n)
    time.sleep(0.3)
    if not d.js("!!document.querySelector('.pfoot [data-act=\"check\"]:not([disabled])')"):
        return False
    d.tap_sel('.pfoot [data-act="check"]', 0, 0.2)
    return until(d, "document.querySelector('.pverdict')", 3.0, 0.1)


def next_question(d):
    """Taps Next; True when the stem changed (the question advanced)."""
    before = d.js(STEM)
    if not d.js("!!document.querySelector('.pfoot [data-act=\"next\"]:not([disabled])')"):
        return False
    d.tap_sel('.pfoot [data-act="next"]', 0, 0.3)
    return until(d, "%s !== %s" % (STEM, repr(before)), 3.0, 0.1)


def reach(d, want):
    """Answers questions until one verdict ('ok' or 'ng') is reached; returns the state right after the check, with the time the
    check was tapped. The effect of an unwanted verdict is ended by the Next tap (a tap anywhere ends it)."""
    for attempt in range(10):
        if not pick_and_check(d, attempt if want == 'ng' else attempt + 1):
            if not next_question(d):
                return None
            continue
        t0 = time.time()
        wait_layer(d, 1.5)
        s = fx(d)
        s['t0'] = t0
        if s['verdict'] == want:
            return s
        wait_armed(s)
        if not next_question(d):
            return None
        wait_no_layer(d, 2.0)
    return None


# ---------------------------------------------------------------------------------------------------- W-FX-1

def fx1_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    p = []
    try:
        goto(d, '#/', 1.0)
        r = d.js(RESET_JS)
        if not r or r['fx'] is not True:
            p.append('after the reset the server answerFx is %r (status %r), expected true' % (r and r['fx'], r and r['status']))
        elif 'questionLang' not in r['keys'] or 'answerFx' not in r['keys']:
            p.append('the /api/settings keys are %s, expected questionLang and answerFx among them' % r['keys'])
        d.save_settings(answerFx=True)       # the reset also reset the language and theme of this device: put them back
        goto(d, '#/settings', 1.8)
        until(d, "document.querySelector('.set-switch')", 8.0)
        time.sleep(0.4)
        s = js(d, SW_JS % (json.dumps(LABEL[lang]), json.dumps(HINT[lang])))
        if not s['found']:
            p.append('no switch labelled "%s" (%d switches on the page)' % (LABEL[lang], s['n']))
        else:
            if s['role'] != 'switch':
                p.append('the control has role %r, not switch' % s['role'])
            if s['checked'] != 'true':
                p.append('the switch is not on (aria-checked %r)' % s['checked'])
            if not s['hint']:
                p.append('the one-line hint "%s ..." is missing' % HINT[lang])
            if phone(size) and s['h'] < 43.5:
                p.append('the switch is %d px high (< 44)' % round(s['h']))
            if s['l'] < -0.5 or s['r'] > s['vw'] + 0.5:
                p.append('the switch sticks out sideways (%d..%d of %d px)' % (round(s['l']), round(s['r']), s['vw']))
            if s['sideways'] > 1:
                p.append('the page is %d px wider than the viewport' % s['sideways'])
            if s['n'] < 2 or not s['autoNear']:
                p.append('the switch is not next to the auto-advance switch (%d switches)' % s['n'])
            if not p:
                idx = d.js("[...document.querySelectorAll('.set-switch')].findIndex((e) => (e.textContent || '').includes(%s))" % json.dumps(LABEL[lang]))
                d.tap_sel('.set-switch', idx)
                time.sleep(0.6)
                if d.js(API_JS) is not False:
                    p.append('a tap did not turn the server value off')
                d.tap_sel('.set-switch', idx)
                time.sleep(0.6)
                if d.js(API_JS) is not True:
                    p.append('a second tap did not turn it back on')
    finally:
        d.save_settings()       # the walk default (answerFx off), also after an error
    verdict(rc, 'W-FX-1', 'settings', 'switch', 'sam', lang, theme, size, d, p,
            'the "%s" switch is on by default, next to auto-advance, >= 44 px on phones, fits the width, toggles' % LABEL[lang])


# ---------------------------------------------------------------------------------------------------- W-FX-2

def fx2_case(rc, lang, theme, size):
    d = rc.dev('sam', lang, theme, size)
    p = []
    try:
        d.save_settings(answerFx=True)
        start_quiz(d, 'practice')
        ok = reach(d, 'ok')
        if not ok:
            p.append('no right answer reached in 10 questions (no verdict)')
        else:
            if not ok['layer']:
                p.append('right answer: no .fx-layer after the check')
            else:
                if ok['pe'] != 'none' or ok['pos'] != 'fixed':
                    p.append('right answer: the layer is %s / pointer-events %s (want fixed / none)' % (ok['pos'], ok['pe']))
                if not ok['plain']:
                    p.append('right answer: no canvas for the confetti in the layer')
                if ok['wipes']:
                    p.append('right answer: the layer holds claw wipes')
                # the next tap still works while the effect runs (it also ends the effect: a tap anywhere does)
                wait_armed(ok)
                if not next_question(d):
                    p.append('right answer: Next did not advance while the effect was on')
                elif not wait_no_layer(d, 2.0):
                    p.append('right answer: the tap on Next did not end the effect')
            # the effect left alone is gone within about 8 s: a second right answer, no tap
            again = reach(d, 'ok')
            if again and again['layer'] and not wait_no_layer(d, max(0.5, 9.0 - (time.time() - again['t0']))):
                p.append('right answer: the layer is still there %d s after the check' % round(time.time() - again['t0']))
        ng = reach(d, 'ng')
        if not ng:
            p.append('no wrong answer reached in 10 questions (no verdict)')
        else:
            if not ng['layer']:
                p.append('wrong answer: no .fx-layer after the check')
            else:
                if ng['pe'] != 'none' or ng['pos'] != 'fixed':
                    p.append('wrong answer: the layer is %s / pointer-events %s (want fixed / none)' % (ng['pos'], ng['pe']))
                if not ng['wipes']:
                    p.append('wrong answer: no claw canvas (fx-wipe) in the layer')
                if ng['plain']:
                    p.append('wrong answer: the layer holds confetti canvases')
                wait_armed(ng)
                if not next_question(d):
                    p.append('wrong answer: Next did not advance while the effect was on')
                elif not wait_no_layer(d, 2.0):
                    p.append('wrong answer: the tap on Next did not end the effect')
    finally:
        d.save_settings()
    verdict(rc, 'W-FX-2', 'quiz', 'effects', 'sam', lang, theme, size, d, p,
            'right answer: fixed click-through canvas layer, Next still works, gone within 8 s; wrong answer: claw overlay')


# ---------------------------------------------------------------------------------------------------- W-FX-3 / W-FX-4

def none_case(rc, cid, lang, theme, size, setup, teardown, what):
    d = rc.dev('sam', lang, theme, size)
    p = []
    seen = set()
    try:
        setup(d)
        start_quiz(d, 'practice')
        for attempt in range(10):
            if not pick_and_check(d, attempt):
                if not next_question(d):
                    break
                continue
            time.sleep(0.5)
            s = fx(d)
            seen.add(s['verdict'])
            if s['layer']:
                p.append('an effect layer appeared after a %s verdict %s' % ('right' if s['verdict'] == 'ok' else 'wrong', what))
                break
            if {'ok', 'ng'} <= seen:
                break
            if not next_question(d):
                break
        if not seen:
            p.append('no verdict was reached, nothing was measured')
    finally:
        teardown(d)
    verdict(rc, cid, 'quiz', 'no-effects', 'sam', lang, theme, size, d, p,
            'no effect layer %s (verdicts seen: %s)' % (what, ', '.join(sorted(seen)) or 'none'))


def _off_on(d):
    d.save_settings(answerFx=False)


def _rm_on(d):
    d.save_settings(answerFx=True)
    d.set_reduced_motion(True)


def _rm_off(d):
    d.set_reduced_motion(False)
    d.save_settings()


def fx3_case(rc, lang, theme, size):
    none_case(rc, 'W-FX-3', lang, theme, size, _off_on, lambda d: d.save_settings(), 'with the switch off')


def fx4_case(rc, lang, theme, size):
    none_case(rc, 'W-FX-4', lang, theme, size, _rm_on, _rm_off, 'with prefers-reduced-motion')


def g_v30_fx(rc):
    cases(rc, 'W-FX-1', 'settings', fx1_case, FX_SIZES)
    for cid, fn in (('W-FX-2', fx2_case), ('W-FX-3', fx3_case), ('W-FX-4', fx4_case)):
        if not rc.want(cid) or not rc.walk.wants_page('quiz'):
            continue
        for size in rc.sizes(PRACTICE_SIZES, phone):
            rc.guard(cid, 'quiz', fn, rc, rc.langs(['zh'])[0], rc.themes(['light'])[0], size)


GROUPS_V30 = [g_v30_fx]
