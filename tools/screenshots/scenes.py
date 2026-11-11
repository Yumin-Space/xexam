# -*- coding: utf-8 -*-
"""The screenshot scenes: headless Chrome drives the real UI through DOM clicks, one browser per language.

shoot_language() writes, into one folder, the 24 images the READMEs and docs use (19 desktop images 1280 px wide and 5
iPhone images 390 x 844, both at device scale 2):

  desktop-1-home        Home with the saved exam to continue (green Start exam, orange Start practice)
  desktop-2-quiz        an exam in progress, on a hotspot question with its first item answered
  desktop-3-result      the result of a whole exam: the score and the first answer card (an orange wrong option, the verdict)
  desktop-4-records     Study records
  desktop-5-banks       Question banks: the preview of an update ("Kitchen basics (v2)") and the guide unfolded, shortened
                        to its first two sections (basics and the six columns) so the whole guide card ends inside the image
  desktop-6-settings    Settings, as the administrator: with the AI chat mode section (Normal selected; administrators only)
  desktop-7-dark        the exam page in the dark theme
  desktop-8-about       About: the newest changelog entry shortened to the bullets that fit, so the whole What's new card
                        (with the folded earlier versions) ends inside the image
  desktop-9-signin      the sign-in page
  desktop-10-users      Users: the AI+ feature line of each user (shown while AI+ is available), the bright AI+ badge
                        of the administrators and sam, the dark one of alex (AI+ feature paused), none for kim
  desktop-11-practice   practice: one question checked right, the next one checked wrong (orange) with the right ones green
  desktop-12-user-banks the "Question banks" dialog of a user
  desktop-13-logs       System log, the whole card (lead, filters, lines, note) in view
  desktop-14-ai-chat    AI Chat as the administrator: a chat started through the API opened, a second question typed
                        into its composer and sent through the UI, its answer finished; the administrator notice, the
                        green "Free mode" tag right of the title (the scene turns the administrator's free mode on through
                        POST /api/ai/free first and off again afterwards; AI Chat itself has no mode switch, the choice
                        is in Settings), the who line (You / AI, model, time) above and
                        Copy (with Remember this / Retry) under every message are visible. The window is made just tall
                        enough for the whole conversation box and the image ends below it. The chat is deleted again
                        through the API, so every language starts without chats
  desktop-15-ai-settings AI settings after a click on Refresh list and on Test: the four status chips and the note that
                        ai-service is connected, the connection with the made-up address and the masked made-up key, the
                        model with the number of demo models found, the test result and the users with the AI+ feature
                        (badge, AI+ feature: alex Paused with Resume, sam Active with Pause; administrators always)
  desktop-16-ai-usage   the rest of that page, scrolled into view: the usage statistics with the cached-tokens column (they grow
                        by a few calls per language), the chat retention and the safety limits
  desktop-17-ai-hint    a practice question of the made-up demo bank with the AI hint window open (the screenshot user has the
                        AI+ feature, so the AI hint / AI explanation controls are bright); the stub's hint, in the UI language
  desktop-18-ai-result  the result of the exam: the first answer card with the AI analysis (AI 解析) window open
  desktop-19-ai-summary Study records with the AI summary window open: the "based on" line and the stub's four headings
  iphone-1-home, iphone-2-quiz, iphone-3-menu, iphone-4-records
  iphone-5-ai-hint      the practice page at 390 x 844 with the AI hint window open

Layout checks (assertions, not images) run in the same pass: at 390 px the practice action bar has at most two rows of
buttons (en / zh-CN / ja), and at 800 px the Study records list (en and ja) is in the card layout with no horizontal
scroll and its Delete buttons on one line. The AI asks of the new scenes are saved in AI Chat like any other; the run
deletes the administrator's AI chats before the AI Chat scenes and at the end.

The made-up AI provider (tests/integration/ai_stub.py) answers in the reply language the page sends (the Settings
language), so each language's AI Chat image shows questions and answers in that language. Every value on the AI images
is made up (the stub's address, the key hint ...9c21, the demo-* models); real addresses, keys or models never appear.

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
     'user-banks', 'logs', 'ai-chat', 'ai-settings', 'ai-usage', 'ai-hint', 'ai-result', 'ai-summary'], start=1)] + [
    'iphone-1-home.png', 'iphone-2-quiz.png', 'iphone-3-menu.png', 'iphone-4-records.png', 'iphone-5-ai-hint.png']

BROWSER_LANG = {'en': 'en-US', 'zh-CN': 'zh-CN', 'ja': 'ja'}
UI_LANG = {'en': 'en', 'zh-CN': 'zh', 'ja': 'ja'}          # the uiLang / questionLang setting
DESKTOP_UA = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36'
IPHONE_UA = ('Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) '
             'Version/17.5 Mobile/15E148 Safari/604.1')
WIDTH, PHONE_W, PHONE_H = 1280, 390, 844
# Self-made study questions of the AI Chat scene, per UI language: the earlier chat, then the one typed into the composer
AI_QUESTIONS = {
    'en': ('How can I remember new vocabulary for my exam?', 'Can you give me a short plan for one week?'),
    'zh': ('怎样才能更好地记住考试要用的新词汇？', '能给我一个一周的简短复习计划吗？'),
    'ja': ('試験のために新しい単語をうまく覚えるにはどうすればいいですか？', '一週間の短い復習プランを作ってもらえますか？'),
}

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
  // wait until the text of `sel` (a hint, an explanation, a summary) has appeared and stopped growing; returns its length
  window.__settle = async (sel) => {
    let last = -1, still = 0;
    for (let i = 0; i < 120; i++) {
      await window.__wait(150);
      const n = (document.querySelector(sel)?.innerText || '').length;
      still = n > 30 && n === last ? still + 1 : 0;
      last = n;
      if (still >= 6) return n;
    }
    return -1;
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


# Users: what the AI+ parts of the page show (for the log of the run)
JS_USERS_STATE = r"""(() => {
  const n = s => document.querySelectorAll(s).length;
  return n('.ai-badge:not(.off)') + ' bright badge(s), ' + n('.ai-badge.off') + ' dark, ' + n('.us-ailine') + ' AI+ line(s)';
})()"""

# AI settings: the AI+ feature cells of the users with the AI+ feature (for the log of the run)
JS_AI_FUNCTIONS = r"""(() => [...document.querySelectorAll('[id^="ais-fn-"]')].map(b => b.closest('tr')?.innerText
  .replace(/\s+/g, ' ').trim().slice(0, 60)).join(' | ') || 'no Pause / Resume button')()"""

# AI Chat: open the chat whose title starts like `title` in the chat list
JS_AI_OPEN = r"""(async (title) => {
  const list = document.querySelector('.ai-list');
  if (!list) return 'no chat list';
  const key = title.replace(/\s+/g, ' ').slice(0, 16);
  const item = [...list.querySelectorAll('button, a, [role="button"]')].find(el => (el.innerText || '').includes(key));
  if (!item) return 'chat not found';
  item.click(); await window.__wait(900);
  return document.querySelectorAll('.ai-msg').length + ' messages shown';
})"""

# AI Chat: focus the composer (the text itself is typed through the DevTools protocol)
JS_AI_FOCUS = r"""(() => { const t = document.querySelector('#ai-input'); if (!t) return 'no composer'; t.focus(); return 'focused'; })()"""

# AI Chat: press Send and wait until the answer has finished (no streaming message left, two answers shown)
JS_AI_SEND = r"""(async () => {
  document.querySelector('#ai-send').click();
  let n = 0;
  for (let i = 0; i < 150; i++) {
    await window.__wait(200);
    n = document.querySelectorAll('.ai-msg.assistant').length;
    if (i > 3 && n >= 2 && !document.querySelector('.ai-msgs [aria-busy="true"]')) break;
  }
  await window.__wait(600);
  document.activeElement?.blur?.();
  window.scrollTo(0, 0);
  return n + ' answers; notice ' + (document.querySelector('.ai-notice') ? 'shown' : 'MISSING') +
    '; free-mode tag ' + (document.querySelector('.ai-mode-tag') ? 'shown' : 'MISSING') +
    '; mode switch ' + (document.querySelector('#ai-free') ? 'STILL THERE' : 'none');
})()"""

# AI Chat: scroll the conversation box to its end; returns how many px of the conversation do not fit in the box
JS_AI_OVER = r"""(() => {
  const sc = document.getElementById('ai-scroll');
  if (!sc) return 0;
  sc.scrollTop = sc.scrollHeight;
  return Math.ceil(sc.scrollHeight - sc.clientHeight);
})()"""

# AI Chat: the bottom (page coordinates) of the chat list and the conversation
JS_AI_PANES_BOTTOM = r"""(() => {
  window.scrollTo(0, 0);
  const p = document.getElementById('ai-panes');
  return p ? Math.ceil(p.getBoundingClientRect().bottom + window.scrollY) : null;
})()"""

# AI settings: press Refresh list (the line "N models found" appears under the model), then Test, and wait for each
# result (the page reads the usage again after both, so the Usage card counts these calls too)
JS_AI_TEST = r"""(async () => {
  const wait = window.__wait, q = s => document.querySelector(s);
  const inCard = (heading, s) => document.getElementById(heading)?.closest('section')?.querySelector(s);
  const until = async (ok, ms) => {
    for (let t = 0; t < ms; t += 100) { await wait(100); if (ok()) return true; }
    return false;
  };
  if (!q('#ai-models-btn') || !q('#ai-test-btn')) return 'no Refresh list / Test button';
  q('#ai-models-btn').click();
  const listed = await until(() => !q('#ai-models-btn')?.disabled && inCard('ais-model-h', '.ais-note[role="status"]'), 8000);
  const note = inCard('ais-model-h', '.ais-note[role="status"]')?.innerText || '';
  q('#ai-test-btn').click();
  const tested = await until(() => !q('#ai-test-btn')?.disabled && inCard('ais-test-h', '.ais-ok, .ais-fail, .ais-ferr'), 15000);
  const result = inCard('ais-test-h', '.ais-ok')?.innerText || 'TEST FAILED';
  await wait(1200);                                    // the usage tables are read again after the test
  document.activeElement?.blur?.();
  window.scrollTo(0, 0);
  return (listed ? note : 'NO MODEL LIST') + '; ' + (tested ? result : 'NO TEST RESULT');
})()"""

# AI settings: [top, bottom] (page coordinates) of the card whose heading has the id `first`, through the card whose
# heading has the id `last`
JS_AI_CARDS = r"""((first, last) => {
  window.scrollTo(0, 0);
  const card = id => document.getElementById(id)?.closest('section');
  const a = card(first), b = card(last);
  if (!a || !b) return null;
  const y = window.scrollY;
  return [Math.floor(a.getBoundingClientRect().top + y), Math.ceil(b.getBoundingClientRect().bottom + y)];
})"""


# AI scenes (v0.9). Practice: start a practice from Home, open the AI hint window with the button of the action bar and
# wait until the stub's hint is complete; returns what happened (for the log of the run)
JS_AI_HINT = r"""(async (phone) => {
  window.__skew = 0;
  await window.__start('practice');
  const b = document.querySelector('[data-focus="q-aihint"]');
  if (!b) return 'NO AI HINT BUTTON';
  if (b.getAttribute('aria-disabled') === 'true') return 'AI HINT BUTTON GREYED OUT';
  if (phone) b.scrollIntoView({block: 'center'});
  b.click();
  const n = await window.__settle('#aip-pop-body');
  document.activeElement?.blur?.();
  return n < 0 ? 'HINT DID NOT FINISH' : 'hint of ' + n + ' characters; window ' + (document.querySelector('#aip-pop') ? 'open' : 'MISSING');
})"""

# The number of rows of buttons in the action bar of the practice page (buttons whose tops are within 6 px share a row)
JS_BAR_ROWS = r"""(() => {
  const tops = [];
  for (const el of document.querySelectorAll('.pfoot button')) {
    if (!el.offsetParent) continue;
    const t = Math.round(el.getBoundingClientRect().top);
    if (!tops.some(x => Math.abs(x - t) < 6)) tops.push(t);
  }
  return tops.length;
})()"""

# Result page of the exam: open the AI analysis window of the first answer card; returns [text length or a message, the
# lowest bottom (page coordinates) of the card and the window]
JS_AI_RESULT = r"""(async () => {
  const b = document.querySelector('[data-focus^="rv-ai-"]:not(.aip-ico)');
  if (!b) return ['NO AI ANALYSIS BUTTON (data-focus on the page: ' + window.__qa('[data-focus]').map(x => x.dataset.focus).slice(0, 25).join(' ') + ')', 0];
  if (b.getAttribute('aria-disabled') === 'true') return ['AI ANALYSIS BUTTON GREYED OUT', 0];
  b.click();
  const n = await window.__settle('#aip-pop-body');
  document.activeElement?.blur?.();
  const card = document.querySelector('.page-result .stem').closest('article, li, section, .panel');
  const pop = document.querySelector('#aip-pop');
  const bottom = Math.max(card.getBoundingClientRect().bottom, pop ? pop.getBoundingClientRect().bottom : 0) + window.scrollY;
  return [n, Math.ceil(bottom)];
})()"""

# Study records: open the AI summary window of the first record that has a usable summary button; returns the text length
# and the "based on" line
JS_AI_SUMMARY = r"""(async () => {
  const b = window.__qa('.rec-table .rec-ai button').find(x => x.getAttribute('aria-disabled') !== 'true');
  if (!b) return 'NO AI SUMMARY BUTTON';
  b.click();
  const n = await window.__settle('#aip-sum-body');
  const meta = document.querySelector('#aip-sum-meta')?.innerText.replace(/\s+/g, ' ') || '';
  return (n < 0 ? 'SUMMARY DID NOT FINISH' : 'summary of ' + n + ' characters') + '; ' + (meta || 'NO "based on" LINE');
})()"""

# Study records at 800 px: the card layout (rows are grids), no horizontal scroll, every Delete button on one line;
# returns a list of the problems found (empty = fine)
JS_RECORDS_LAYOUT = r"""(() => {
  const bad = [];
  const table = document.querySelector('.rec-table');
  if (!table) return ['no records table'];
  if (!table.classList.contains('has-ai')) bad.push('the table has no has-ai class');
  const row = table.querySelector('tbody tr');
  if (!row || getComputedStyle(row).display !== 'grid') bad.push('the rows are not in the card layout');
  const de = document.documentElement;
  if (de.scrollWidth > de.clientWidth + 1) bad.push('the page scrolls horizontally');
  const sc = document.querySelector('.rec-scroll');
  if (sc && sc.scrollWidth > sc.clientWidth + 1) bad.push('the list scrolls horizontally');
  for (const el of document.querySelectorAll('.rec-del')) {
    if (!el.offsetParent) continue;
    const r = document.createRange();
    r.selectNodeContents(el);
    const tops = new Set([...r.getClientRects()].map(x => Math.round(x.top / 4)));
    if (tops.size > 1) { bad.push('a Delete button wraps onto two lines'); break; }
  }
  return bad;
})()"""


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

    def png(self, name, w, h, mobile, clip=None, scroll_y=None):
        """The w x h window as `name`; with `clip`, only its top `clip` px (for a page whose layout follows the window
        height, so the window cannot simply be made as tall as the image); with `scroll_y`, scrolled down that far."""
        self.metrics(w, h, mobile)
        time.sleep(0.5)
        if scroll_y is not None:
            self.js('window.scrollTo(0, %d); window.scrollY' % scroll_y)
            time.sleep(0.3)
        shot = {'format': 'png'}
        if clip:
            shot['clip'] = {'x': 0, 'y': 0, 'width': w, 'height': int(clip), 'scale': 1}
            h = int(clip)
        data = strip_png(base64.b64decode(self.cdp.call('Page.captureScreenshot', **shot)['data']))
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
        _ai_scenes(s, base, cookie, code)
        _phone_scenes(s)
        _check_paused_phone(s, base, cookie, code)
        seed.delete_ai_chats(base, cookie)
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
    if s.wanted('desktop-3-result.png') or s.wanted('desktop-18-ai-result.png'):
        print('  exam:', s.js(JS_EXAM))
        # the score and the first answer card, cut just below that card
        bottom = s.js("(() => { const c = document.querySelector('.page-result .stem').closest('article, li, section, .panel'); "
                      "return Math.ceil(c.getBoundingClientRect().bottom + window.scrollY) })()")
        s.page('desktop-3-result.png', fixed=bottom + 9)
        if s.wanted('desktop-18-ai-result.png'):
            # the AI analysis window of that first card (the exam is saved, so the button is bright)
            s.metrics(WIDTH, 900, False)
            time.sleep(0.4)
            text, low = s.js(JS_AI_RESULT)
            print('  ai result:', text if isinstance(text, str) else ('analysis of %d characters' % text if text > 0 else 'ANALYSIS DID NOT FINISH'))
            s.page('desktop-18-ai-result.png', fixed=min(1100, max(560, low + 24)))
    if s.wanted('desktop-4-records.png') or s.wanted('desktop-19-ai-summary.png'):
        s.nav('#/records')
        s.page('desktop-4-records.png', cap=1320)
        if s.wanted('desktop-4-records.png') and code in ('en', 'ja'):
            _check_records_layout(s, code)
        if s.wanted('desktop-19-ai-summary.png'):
            s.desktop()
            s.nav('#/records')
            print('  ai summary:', s.js(JS_AI_SUMMARY))
            s.png('desktop-19-ai-summary.png', WIDTH, 900, False)
    if s.wanted('desktop-17-ai-hint.png'):
        s.desktop()
        s.js(JS_CLEAR_SAVED)
        s.nav()
        print('  ai hint:', s.js('(%s)(false)' % JS_AI_HINT))
        low = s.js("Math.ceil(Math.max(document.querySelector('.pfoot').getBoundingClientRect().bottom, "
                   "document.querySelector('#aip-pop')?.getBoundingClientRect().bottom || 0) + window.scrollY)")
        s.page('desktop-17-ai-hint.png', fixed=min(1100, max(560, low + 40)))
        s.js(JS_CLEAR_SAVED)
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
    print('  users:', s.js(JS_USERS_STATE))
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


def _check_records_layout(s, code):
    """At 800 px the Study records list is in the card layout with no horizontal scroll and Delete on one line (en, ja)."""
    s.metrics(800, 900, False)
    time.sleep(0.6)
    bad = s.js(JS_RECORDS_LAYOUT)
    s.metrics(WIDTH, 900, False)
    if bad:
        raise AssertionError('Study records at 800 px (%s): %s' % (code, '; '.join(bad)))
    print('  records at 800 px: card layout, no horizontal scroll, Delete on one line')


def _ai_scenes(s, base, cookie, code):
    """desktop-14-ai-chat, desktop-15-ai-settings and desktop-16-ai-usage (after the System log scene, so the AI calls of
    one language do not change the log image of the next one more than necessary)."""
    # the AI asks of the scenes before are saved as chats of the administrator: start the AI Chat scenes without them
    seed.delete_ai_chats(base, cookie)
    if s.wanted('desktop-14-ai-chat.png'):
        first, second = AI_QUESTIONS[code]
        seed.ai_chat(base, cookie, first, code)
        print('  free mode on:', seed.set_free(base, cookie, True))   # the green tag shows while it is on
        s.nav('#/ai', settle=2.4)
        print('  ai chat:', s.js('(%s)(%s)' % (JS_AI_OPEN, json.dumps(first))))
        print('  composer:', s.js(JS_AI_FOCUS))
        s.cdp.call('Input.insertText', text=second)
        print('  ai answer:', s.js(JS_AI_SEND))
        # The conversation is a box as tall as the window allows (like a chat app; at most 900 px), with the page footer
        # always just below the window's bottom edge: make the window just tall enough for the whole conversation, then
        # keep only the part down to 24 px below the panes (the footer stays out; nothing in the box is cut)
        h = 900
        for _ in range(8):
            s.metrics(WIDTH, h, False)
            time.sleep(0.4)
            over = s.js(JS_AI_OVER)
            if over <= 0 or h >= 1400:
                break
            h = min(1400, h + over)
        bottom = s.js(JS_AI_PANES_BOTTOM)
        if over > 0:
            print('  warning: the conversation is %d px taller than its box; it is shown scrolled to its end' % over)
        s.png('desktop-14-ai-chat.png', WIDTH, h, False, clip=min(h, bottom + 24) if bottom else None)
        print('  free mode off:', seed.set_free(base, cookie, False))
        print('  ai chats deleted:', seed.delete_ai_chats(base, cookie))
    if s.wanted('desktop-15-ai-settings.png') or s.wanted('desktop-16-ai-usage.png'):
        s.desktop()
        s.nav('#/ai-settings', settle=2.6)
        print('  ai settings:', s.js(JS_AI_TEST))
        print('  AI+ feature:', s.js(JS_AI_FUNCTIONS))
        # desktop-15: the page from the top through the users with AI+, 12 px below that card (the Usage card starts 16 px
        # below it and stays out of the image)
        span = s.js('(%s)("ais-conn-h", "ais-users-h")' % JS_AI_CARDS)
        if span:
            s.page('desktop-15-ai-settings.png', fixed=span[1] + 12)
        else:
            print('  warning: the AI settings cards were not found; the image is cut by the usual rule')
            s.page('desktop-15-ai-settings.png', cap=1800)
        # desktop-16: the rest of the page scrolled into view, Usage through Safety limits (12 px above Usage, so the
        # users card above it stays out, and 24 px below Safety limits)
        span = s.js('(%s)("ais-usage-h", "ais-lim-h")' % JS_AI_CARDS)
        if s.wanted('desktop-16-ai-usage.png'):
            if span:
                s.png('desktop-16-ai-usage.png', WIDTH, span[1] - span[0] + 36, False, scroll_y=span[0] - 12)
            else:
                print('  warning: the Usage and Safety limits cards were not found; desktop-16-ai-usage.png is skipped')


# The practice page as a user whose AI+ feature is paused (greyed controls with the "unavailable" tag): the action bar rows,
# the horizontal overflow, the AI controls greyed; returns a list of the problems found (empty = fine)
JS_PAUSED_PRACTICE = r"""(async () => {
  await window.__start('practice');
  await window.__wait(300);
  const bad = [];
  const tops = [];
  for (const el of document.querySelectorAll('.pfoot button')) {
    if (!el.offsetParent) continue;
    const t = Math.round(el.getBoundingClientRect().top);
    if (!tops.some(x => Math.abs(x - t) < 6)) tops.push(t);
  }
  if (tops.length > 2) bad.push('the action bar has ' + tops.length + ' rows');
  const de = document.documentElement;
  if (de.scrollWidth > de.clientWidth + 1) bad.push('the page scrolls horizontally by ' + (de.scrollWidth - de.clientWidth) + ' px');
  const b = document.querySelector('[data-focus="q-aihint"]');
  if (!b) bad.push('no AI hint button');
  else if (b.getAttribute('aria-disabled') !== 'true') bad.push('the AI hint button is not greyed out');
  return bad;
})()"""


def _check_paused_phone(s, base, cookie, code):
    """A user whose AI+ feature is paused (alex) on the practice page at 390 and 360 px: at most two rows of buttons in the
    action bar, no horizontal scroll, the AI controls greyed. Signs the page back in as the administrator afterwards."""
    if not s.wanted('iphone-5-ai-hint.png'):
        return
    alex = seed.Session(base).sign_in('alex', seed.DEMO_PASSWORD).cookie[seed.COOKIE]
    cdp = s.cdp
    try:
        cdp.call('Network.setCookie', name=seed.COOKIE, value=alex, url=base + '/', httpOnly=True, sameSite='Strict')
        s.iphone()
        s.nav()
        s.js(JS_SETUP + "(%s, 'light')" % json.dumps(code))
        for width in (390, 360):
            s.metrics(width, PHONE_H, True)
            s.nav()
            s.js(JS_CLEAR_SAVED)
            s.nav()
            problems = s.js(JS_PAUSED_PRACTICE)
            if problems:
                raise AssertionError('practice page at %d px, AI+ feature paused: %s' % (width, '; '.join(problems)))
            print('  practice page at %d px, AI+ feature paused: two rows at most, no horizontal scroll, controls greyed' % width)
            s.js(JS_CLEAR_SAVED)
    finally:
        cdp.call('Network.setCookie', name=seed.COOKIE, value=cookie, url=base + '/', httpOnly=True, sameSite='Strict')
        s.nav()


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
    if s.wanted('iphone-5-ai-hint.png'):
        s.js(JS_CLEAR_SAVED)
        s.nav()
        print('  ai hint (phone):', s.js('(%s)(true)' % JS_AI_HINT))
        rows = s.js(JS_BAR_ROWS)
        if rows > 2:
            raise AssertionError('the practice action bar at 390 px has %d rows of buttons (at most 2)' % rows)
        print('  practice action bar at 390 px: %d row(s)' % rows)
        s.png('iphone-5-ai-hint.png', PHONE_W, PHONE_H, True)
