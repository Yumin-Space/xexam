# -*- coding: utf-8 -*-
"""The screenshot scenes: headless Chrome drives the real UI through DOM clicks, one browser per language.

shoot_language() writes, into one folder, the 17 images the READMEs and docs use (desktop 1280 px wide and iPhone
390 x 844, both at device scale 2):

  desktop-1-home        Home with the saved exam to continue (green Start exam, orange Start practice)
  desktop-2-quiz        an exam in progress, on a hotspot question with its first item answered
  desktop-3-result      the result of a whole exam: the score and the first answer card (an orange wrong option, the verdict)
  desktop-4-records     Study records
  desktop-5-banks       Question banks: the preview of an update ("Kitchen basics (v2)") and the guide unfolded, shortened
                        to its first two sections (basics and the six columns) so the whole guide card ends inside the image
  desktop-6-settings    Settings
  desktop-7-dark        the exam page in the dark theme
  desktop-8-about       About: the newest changelog entry shortened to the bullets that fit, so the whole What's new card
                        (with the folded earlier versions) ends inside the image
  desktop-9-signin      the sign-in page
  desktop-10-users      Users
  desktop-11-practice   practice: one question checked right, the next one checked wrong (orange) with the right ones green
  desktop-12-user-banks the "Question banks" dialog of a user
  desktop-13-logs       System log, the whole card (lead, filters, lines, note) in view
  iphone-1-home, iphone-2-quiz, iphone-3-menu, iphone-4-records

Every desktop image is cut where no text line, control or card is split (see FIT_JS) or measured to its card.
The page clock runs in UTC and the browser's language follows the UI language.
"""
import base64
import json
import os
import time

import seed
from browser import Chrome, strip_png

SCENES = ['desktop-%d-%s.png' % kv for kv in enumerate(
    ['home', 'quiz', 'result', 'records', 'banks', 'settings', 'dark', 'about', 'signin', 'users', 'practice',
     'user-banks', 'logs'], start=1)] + ['iphone-1-home.png', 'iphone-2-quiz.png', 'iphone-3-menu.png', 'iphone-4-records.png']

BROWSER_LANG = {'en': 'en-US', 'zh-CN': 'zh-CN', 'ja': 'ja'}
UI_LANG = {'en': 'en', 'zh-CN': 'zh', 'ja': 'ja'}          # the uiLang / questionLang setting
DESKTOP_UA = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36'
IPHONE_UA = ('Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) '
             'Version/17.5 Mobile/15E148 Safari/604.1')
WIDTH, PHONE_W, PHONE_H = 1280, 390, 844

# Installed in every page before its own scripts
PRELUDE = r"""
(() => {
  const real = Date.now.bind(Date);
  window.__skew = 0;                                  // demo only: pretend that time has passed (a script answers in a second)
  Date.now = () => real() + window.__skew;
  window.__wait = ms => new Promise(r => setTimeout(r, ms));
  window.__qa = s => [...document.querySelectorAll(s)];
  // the question on screen, found in /api/bank by its stem in any of the three languages (six-column questions: English in
  // stem, Chinese in zh, Japanese in ja); the key is the first line long enough to tell questions apart (not "HOTSPOT")
  window.__current = async () => {
    window.__bank = window.__bank || (await (await fetch('/api/bank')).json()).questions;
    const shown = document.querySelector('.stem').innerText.replace(/\s+/g, '');
    const key = s => {
      const ls = (s || '').split('\n').map(x => x.trim()).filter(Boolean);
      return (ls.find(l => l.length > 12) || ls[0] || '\u0000').replace(/\s+/g, '').slice(0, 40);
    };
    return window.__bank.find(q => [q.stem, q.en?.stem, q.zh?.stem, q.ja?.stem].some(s => s && shown.includes(key(s))));
  };
  window.__en = q => (q.lang === 'en' ? q.stem : q.en?.stem) || '';
  // answer the question on screen; bad = answer it wrongly
  window.__answer = async (bad) => {
    const q = await window.__current();
    if (q.kind === 'hotspot') {
      q.items.forEach((it, k) => window.__qa('.hs-item')[k].querySelectorAll('.chip')[bad && k === 0 ? (q.answer[k] + 1) % it.choices.length : q.answer[k]].click());
    } else {
      const keys = bad ? [q.options.find(o => !q.answer.includes(o.key)).key] : q.answer;
      keys.forEach(k => window.__qa('.opt').find(b => b.querySelector('.bubble').textContent === k).click());
    }
    return q.kind;
  };
  // Start exam / Start practice from Home in bank order (shuffle off), so every language shows the same questions
  window.__start = async (kind) => {
    const sh = document.querySelector('.home-shuffle input');
    if (sh && sh.checked) sh.click();
    document.querySelector(kind === 'practice' ? '.page-home .home-go-btns .btn.big:not(.primary)' : '.page-home .home-go-btns .btn.primary.big').click();
    await window.__wait(450);
  };
})();
"""

JS_SETUP = r"""(async (lang, theme) => {
  const post = (u, b) => fetch(u, {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(b || {})});
  const r = await post('/api/settings', {uiLang: lang, questionLang: lang, theme, autoAdvance: false});
  if (!r.ok) throw new Error('POST /api/settings: ' + r.status);
  localStorage.removeItem('xexam.settings.v1');       // only the first-paint cache; a saved exam stays
  return 'settings ' + lang + ' ' + theme;
})"""

# A whole exam through the UI: every question, two of them wrong (the fruit question and the first item of the sun hotspot)
JS_EXAM = r"""(async () => {
  const wait = window.__wait, qa = window.__qa, q1 = s => document.querySelector(s);
  await window.__start('exam');
  const n = qa('.tile').length;
  for (let i = 0; i < n; i++) {
    qa('.tile')[i].click(); await wait(60);
    const q = await window.__current();
    const bad = /fruits/i.test(window.__en(q)) || (q.kind === 'hotspot' && /sun rises/i.test(JSON.stringify(q.items || []) + window.__en(q)));
    await window.__answer(bad);
  }
  window.__skew = 394000;                              // the result shows about 6:34
  q1('.rail .btn.primary').click(); await wait(250);
  q1('dialog[open] .btn.primary').click(); await wait(1600);
  window.scrollTo(0, 0);
  return q1('.score .big')?.getAttribute('aria-label') + ' / ' + n + ' questions';
})()"""

# Start an exam, answer the first questions, open the first hotspot question and answer its first item
JS_QUIZ = r"""(async () => {
  const wait = window.__wait, qa = window.__qa;
  window.__skew = 0;
  await window.__start('exam');
  let hs = -1;
  for (let i = 0; i < qa('.tile').length; i++) {
    qa('.tile')[i].click(); await wait(60);
    if (document.querySelector('.hs-item') && hs < 0) { hs = i; continue; }
    if (i < 3 && !document.querySelector('.hs-item')) await window.__answer(false);
  }
  qa('.tile')[hs].click(); await wait(120);
  const q = await window.__current();
  qa('.hs-item')[0].querySelectorAll('.chip')[q.answer[0]].click();
  window.__skew = 83000;
  await wait(1200);
  document.activeElement?.blur?.();
  window.scrollTo(0, 0);
  return 'hotspot at ' + (hs + 1);
})()"""

# Practice: question 1 right (single choice, checked on pick), then the fruit question (multiple choice): A (right) and
# B (wrong) chosen, Check answer -> wrong: B orange, A and C green, the explanation and the memory aid
JS_PRACTICE = r"""(async () => {
  const wait = window.__wait, qa = window.__qa, q1 = s => document.querySelector(s);
  window.__skew = 0;
  await window.__start('practice');
  await window.__answer(false); await wait(300);
  let found = -1;
  for (let i = 0; i < qa('.tile').length; i++) {
    qa('.tile')[i].click(); await wait(80);
    const q = await window.__current();
    if (/fruits/i.test(window.__en(q))) { found = i; break; }
  }
  ['A', 'B'].forEach(k => qa('.opt').find(b => b.querySelector('.bubble').textContent === k).click());
  await wait(100);
  q1('[data-focus="q-check"]').click(); await wait(500);
  window.__skew = 71000;
  await wait(1200);
  document.activeElement?.blur?.();
  window.scrollTo(0, 0);
  return 'question ' + (found + 1) + ': ' + (q1('.pverdict')?.innerText.split('\n')[0] || 'no verdict');
})()"""

JS_SAVE_EXIT = r"""(async () => { document.querySelector('.qbar .btn.quiet').click(); await window.__wait(400); window.scrollTo(0, 0); return location.hash; })()"""

JS_CLEAR_SAVED = r"""Object.keys(localStorage).filter(k => /session\.v1$/.test(k)).forEach(k => localStorage.removeItem(k)); 'cleared'"""

JS_RESUME = r"""(async () => { document.querySelector('.resume .btn.primary')?.click(); await window.__wait(600); window.scrollTo(0, 0); return location.hash })()"""

JS_UPDATE = r"""(async () => {
  const card = window.__qa('.bk-bank').find(c => /Kitchen basics/.test(c.innerText));
  card.querySelector('.bk-bankacts .btn:not(.danger)').click(); await window.__wait(400);
  return document.querySelector('#bk-target') ? 'update target set' : 'no update target';
})()"""

# The height (from the current scroll position, at most `cap`) at which to cut a tall page so that no text line, control
# or card is split: the lowest point that no such box crosses, plus up to 24 px of the free space below the last one.
# Boxes clipped by a scrolling ancestor count only where they are visible; fixed elements, and cards that start too high
# to be kept whole (page-sized wrappers), are ignored.
FIT_JS = r"""((cap, minimum) => {
  const top = window.scrollY, docH = Math.ceil(document.documentElement.scrollHeight);
  if (docH - top <= cap) return Math.max(minimum, docH - top);
  const limit = top + cap, leaves = [], frames = [];
  const clipOf = el => {
    let lo = -Infinity, hi = Infinity;
    for (let a = el.parentElement; a && a !== document.body; a = a.parentElement) {
      if (getComputedStyle(a).overflowY !== 'visible') {
        const r = a.getBoundingClientRect();
        lo = Math.max(lo, r.top + scrollY); hi = Math.min(hi, r.bottom + scrollY);
      }
    }
    return [lo, hi];
  };
  for (const el of document.body.querySelectorAll('*')) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden' || cs.position === 'fixed' || el.closest('dialog:not([open])')) continue;
    const text = [...el.childNodes].some(n => n.nodeType === 3 && n.textContent.trim());
    const atomic = el.matches('img, svg, button, input, select, textarea, canvas, video, hr');
    const framed = !text && !atomic && ((parseFloat(cs.borderTopWidth) > 0 && cs.borderTopStyle !== 'none') ||
      !/^(rgba\(0, 0, 0, 0\)|transparent)$/.test(cs.backgroundColor) || cs.boxShadow !== 'none');
    if (!text && !atomic && !framed) continue;
    const [lo, hi] = clipOf(el);
    for (const r of el.getClientRects()) {
      const a = Math.max(r.top + scrollY, lo), b = Math.min(r.bottom + scrollY, hi);
      if (r.width < 1 || b - a < 1) continue;
      if (framed && a < top + minimum) continue;              // starts too high to be kept whole (a page-sized wrapper)
      (framed ? frames : leaves).push([a, b]);
    }
  }
  const cut = boxes => {
    let y = limit;
    for (;;) {
      const hit = boxes.filter(b => b[0] < y && b[1] > y);
      if (!hit.length) break;
      y = Math.min(...hit.map(b => b[0]));
    }
    if (y < top + minimum) return null;
    const above = boxes.filter(b => b[1] <= y).map(b => b[1]);
    return Math.min(y, (above.length ? Math.max(...above) : y) + 24) - top;
  };
  return Math.floor(cut(leaves.concat(frames)) ?? cut(leaves) ?? cap);
})"""

# The System log card: its bottom edge (page coordinates) and the text of its note
JS_LOG_CARD = r"""(() => {
  window.scrollTo(0, 0);
  const c = document.querySelector('.lg-note').closest('section, .panel, .card') || document.querySelector('.lg-note').parentElement;
  return [Math.ceil(c.getBoundingClientRect().bottom + window.scrollY), document.querySelectorAll('.lg-line').length];
})()"""

# Question banks: unfold the guide (a click on its summary, as a reader would) and hide everything of it after its first
# two sections (basics, the six columns; the whole guide is several thousand pixels tall), then the bottom of the two
# columns (page coordinates), whether the guide is open and how many of its parts were hidden
JS_BANKS_GUIDE = r"""(async () => {
  const d = document.querySelector('#bk-guide-d');
  if (d && !d.open) { document.querySelector('#bk-guide-s').click(); await window.__wait(500); }
  const third = document.querySelectorAll('#bk-guide-d .bk-guide-body > h3')[2];
  let hidden = 0;
  for (let el = third; el; el = el.nextElementSibling) { el.style.display = 'none'; hidden++; }
  window.scrollTo(0, 0);
  const grid = document.querySelector('.page-banks .bk-grid');
  return [Math.ceil(grid.getBoundingClientRect().bottom + window.scrollY), !!document.querySelector('#bk-guide-d[open]'),
          hidden];
})()"""

# About: hides the last top-level bullets of the newest changelog entry (and nothing else) until the bottom of the What's
# new card is at most `cap` px down the page, keeping at least one bullet; returns that bottom (page coordinates) and the
# number of bullets kept and hidden
JS_ABOUT = r"""((cap) => {
  window.scrollTo(0, 0);
  const card = document.querySelector('.ab-news');
  const items = [...document.querySelectorAll('.ab-first .ab-body > ul > li')];
  const bottom = () => Math.ceil(card.getBoundingClientRect().bottom + window.scrollY);
  let shown = items.length;
  while (shown > 1 && bottom() > cap) items[--shown].style.display = 'none';
  return [bottom(), shown, items.length - shown];
})"""


class Shooter:
    def __init__(self, cdp, base, out, only):
        self.cdp, self.base, self.out, self.only = cdp, base, out, only
        self.written = []

    def wanted(self, name):
        return not self.only or name in self.only

    def metrics(self, w, h, mobile):
        self.cdp.call('Emulation.setDeviceMetricsOverride', width=w, height=int(h), deviceScaleFactor=2, mobile=mobile,
                      screenWidth=w, screenHeight=int(h))

    def nav(self, route='#/', settle=1.6):
        # a URL that differs only in its hash would not reload the page (and not read the settings again)
        self.cdp.call('Page.navigate', url='about:blank')
        time.sleep(0.2)
        self.cdp.call('Page.navigate', url=self.base + '/' + route)
        time.sleep(settle)

    def js(self, code):
        return self.cdp.evaluate(code)

    def png(self, name, w, h, mobile):
        self.metrics(w, h, mobile)
        time.sleep(0.5)
        data = strip_png(base64.b64decode(self.cdp.call('Page.captureScreenshot', format='png')['data']))
        path = os.path.join(self.out, name)
        for attempt in range(8):                     # a viewer or a virus scanner may hold the file for a moment
            try:
                with open(path + '.tmp', 'wb') as fh:
                    fh.write(data)
                os.replace(path + '.tmp', path)
                break
            except OSError:
                time.sleep(0.5 * (attempt + 1))
        else:
            raise OSError('could not write ' + path)
        self.written.append(path)
        print('  %-26s %4d x %-4d %8d bytes' % (name, w * 2, int(h) * 2, len(data)))

    def desktop(self):
        self.cdp.call('Emulation.setTouchEmulationEnabled', enabled=False)
        self.cdp.call('Emulation.setUserAgentOverride', userAgent=DESKTOP_UA)
        self.metrics(WIDTH, 900, False)

    def iphone(self):
        self.cdp.call('Emulation.setTouchEmulationEnabled', enabled=True, maxTouchPoints=5)
        self.cdp.call('Emulation.setUserAgentOverride', userAgent=IPHONE_UA)
        self.metrics(PHONE_W, PHONE_H, True)

    def page(self, name, cap=1100, fixed=None, minimum=560):
        """A desktop image: `fixed` px tall, or as tall as the page up to `cap`, cut where nothing is split."""
        if not self.wanted(name):
            return
        if fixed is None:
            self.metrics(WIDTH, 900, False)
            time.sleep(0.3)
            fixed = self.js('(%s)(%d, %d)' % (FIT_JS, cap, minimum))
        self.png(name, WIDTH, fixed, False)

    def phone(self, name, scroll_to=None):
        if not self.wanted(name):
            return
        if scroll_to:
            self.js("document.querySelector(%s)?.scrollIntoView({block: 'start'}); window.scrollBy(0, -64); 'ok'"
                    % json.dumps(scroll_to))
        self.png(name, PHONE_W, PHONE_H, True)


def shoot_language(chrome_exe, base, cookie, lang, out, upload, only=(), with_log=True):
    """All scenes of one language into `out`. `upload`: the path of "Kitchen basics (v2).xlsx". Returns the files."""
    os.makedirs(out, exist_ok=True)
    code = UI_LANG[lang]
    before = seed.record_ids(base, cookie)
    with Chrome(chrome_exe, BROWSER_LANG[lang]) as chrome:
        cdp = chrome.page
        for domain in ('Page', 'Runtime', 'DOM', 'Network'):
            cdp.call(domain + '.enable')
        cdp.call('Page.addScriptToEvaluateOnNewDocument', source=PRELUDE)
        cdp.call('Emulation.setLocaleOverride', locale=BROWSER_LANG[lang])
        cdp.call('Emulation.setTimezoneOverride', timezoneId='UTC')
        cdp.call('Emulation.setEmulatedMedia', features=[{'name': 'prefers-color-scheme', 'value': 'light'}])
        s = Shooter(cdp, base, out, set(only))
        _desktop_scenes(s, base, cookie, code, upload, with_log)
        _phone_scenes(s)
        # leave nothing behind for the next language: the saved exam and the exam record of this run
        s.nav()
        s.js(JS_CLEAR_SAVED)
    added = seed.record_ids(base, cookie) - before
    if added:
        seed.delete_records(base, cookie, added)
    return s.written


def _desktop_scenes(s, base, cookie, code, upload, with_log):
    cdp = s.cdp
    s.desktop()
    cdp.call('Network.clearBrowserCookies')
    s.nav(settle=2.4)
    s.page('desktop-9-signin.png', cap=900)
    cdp.call('Network.setCookie', name=seed.COOKIE, value=cookie, url=base + '/', httpOnly=True, sameSite='Strict')
    s.nav()
    print('  ' + s.js(JS_SETUP + "(%s, 'light')" % json.dumps(code)))
    s.js(JS_CLEAR_SAVED)
    s.nav()
    if s.wanted('desktop-3-result.png'):
        print('  exam:', s.js(JS_EXAM))
        # the score and the first answer card, cut just below that card
        bottom = s.js("(() => { const c = document.querySelector('.page-result .stem').closest('article, li, section, .panel'); "
                      "return Math.ceil(c.getBoundingClientRect().bottom + window.scrollY) })()")
        s.page('desktop-3-result.png', fixed=bottom + 9)
    if s.wanted('desktop-4-records.png'):
        s.nav('#/records')
        s.page('desktop-4-records.png', cap=1320)
    if s.wanted('desktop-11-practice.png'):
        s.nav()
        print('  practice:', s.js(JS_PRACTICE))
        s.page('desktop-11-practice.png', cap=1250)
        s.js(JS_CLEAR_SAVED)
    s.nav()
    print('  quiz:', s.js(JS_QUIZ))
    s.page('desktop-2-quiz.png', cap=900)
    s.js(JS_SAVE_EXIT)
    s.page('desktop-1-home.png', cap=1100)
    if s.wanted('desktop-5-banks.png'):
        s.nav('#/banks')
        print('  banks:', s.js(JS_UPDATE))
        root = cdp.call('DOM.getDocument', depth=-1)['root']['nodeId']
        node = cdp.call('DOM.querySelector', nodeId=root, selector='#bk-file')['nodeId']
        cdp.call('DOM.setFileInputFiles', files=[os.path.abspath(upload)], nodeId=node)
        time.sleep(1.5)
        print('  preview:', s.js("[...document.querySelectorAll('.bk-chip')].map(p => p.innerText.replace(/\\s+/g, ' ')).join(' | ')"))
        bottom, opened, hidden = s.js(JS_BANKS_GUIDE)
        print('  guide: %s, %d later part(s) hidden, columns end at %d px' % ('open' if opened else 'NOT OPEN', hidden,
                                                                           bottom))
        s.page('desktop-5-banks.png', fixed=bottom + 24)
        s.js("fetch('/api/import/cancel', {method: 'POST', headers: {'Content-Type': 'application/json'}, body: '{}'}).then(r => r.status)")
    s.nav('#/settings')
    s.page('desktop-6-settings.png', cap=1800)
    s.nav('#/users', settle=2.2)
    s.page('desktop-10-users.png', cap=1100)
    if s.wanted('desktop-12-user-banks.png'):
        admin = seed.Session(base)
        admin.cookie[seed.COOKIE] = cookie
        alex = admin.user_id('alex')
        print('  banks dialog:', s.js("(async () => { document.querySelector('#us-banks-%d').click(); await window.__wait(900); "
                                      "return document.querySelector('dialog[open]') ? 'open' : 'no dialog' })()" % alex))
        # tall enough for the whole dialog with some backdrop above and below it
        need = s.js("Math.ceil(document.querySelector('dialog[open]').getBoundingClientRect().height)")
        s.page('desktop-12-user-banks.png', fixed=max(820, need + 120))
        s.js("document.querySelector('dialog[open]')?.close(); 'closed'")
    if with_log and s.wanted('desktop-13-logs.png'):
        s.nav('#/logs', settle=2.5)
        # the log box is min(64vh, 720px) tall: find the viewport height whose card ends 24 px above the bottom edge
        h = 1000
        for _ in range(30):
            s.metrics(WIDTH, h, False)
            time.sleep(0.3)
            bottom, lines = s.js(JS_LOG_CARD)
            if abs(bottom + 24 - h) <= 1:
                break
            h = bottom + 24
        print('  log: %d lines, card ends at %d px' % (lines, bottom))
        s.page('desktop-13-logs.png', fixed=h)
    # the dark theme: the exam page (the saved exam continued)
    s.js(JS_SETUP + "(%s, 'dark')" % json.dumps(code))
    s.nav()
    s.js(JS_RESUME)
    s.page('desktop-7-dark.png', cap=900)
    s.js(JS_SETUP + "(%s, 'light')" % json.dumps(code))
    s.nav('#/about', settle=2.4)
    bottom, kept, hidden = s.js('(%s)(%d)' % (JS_ABOUT, 1500))
    print('  about: %d bullet(s) shown, %d hidden, card ends at %d px' % (kept, hidden, bottom))
    s.page('desktop-8-about.png', fixed=bottom + 24)


def _phone_scenes(s):
    s.iphone()
    s.nav()
    s.phone('iphone-1-home.png')
    s.js(JS_RESUME)
    s.phone('iphone-2-quiz.png')
    s.nav('#/records')
    s.js("(async () => { document.querySelector('.menu-btn').click(); await window.__wait(450); return 'menu' })()")
    s.phone('iphone-3-menu.png')
    s.nav('#/records')
    s.phone('iphone-4-records.png', scroll_to='#rec-list-h')
