# -*- coding: utf-8 -*-
"""v1.2 walk checks: the two-line live token text of a running AI generation / import card (A-OWNER-84) and the import
cancel words. Same style as checks.py (it reuses its helpers). Register in checks.py:

    from checks_v12_tokens import GROUPS_V12_TOKENS      # next to the other imports
    GROUPS = [..., *GROUPS_V12_TOKENS]                   # at the end of the list

W-TK-2: on a running card the token text is one grid (calls | label | numbers); row 2 (cache write/read) sits under row 1's
label and numbers; when the columns do not fit the call count has its own line and the label rows sit under each other; no
cell wraps, nothing sticks out, the page does not scroll sideways; the import card's cancel button says "Cancel import".
"""
import time

from checks import FAKE_JS, cat, fill, goto, until

USAGE = {'calls': 12, 'in': 85400, 'out': 59200, 'read': 76300, 'write': 6900}

TK2_JS = """(() => { const u = document.querySelector('.gen-usage'); const c = document.querySelector('.gen-cancel'); if (!u || !c) return null;
  const cs = getComputedStyle(u); const cells = [...u.children]; const q = (s) => [...u.querySelectorAll(s)];
  const box = (e) => { const b = e.getBoundingClientRect(); return {l: b.left, r: b.right, t: Math.round(b.top), h: b.height, text: e.textContent} };
  const lh = parseFloat(cs.lineHeight) || 18;
  const wraps = cells.filter(e => e.textContent && e.getBoundingClientRect().height > lh * 1.6).map(e => e.textContent);
  const vis = (e) => getComputedStyle(e).display !== 'none';
  return {narrow: u.classList.contains('gu-narrow'), tiny: u.classList.contains('gu-tiny'), fit: u.scrollWidth <= u.clientWidth, wraps, fs: parseFloat(cs.fontSize), tab: cs.fontVariantNumeric,
    n: q('.gu-n:not(.gu-e)').filter(vis).map(box), l: q('.gu-l').map(box), v: q('.gu-v').map(box), right: u.getBoundingClientRect().right, vw: innerWidth,
    side: document.documentElement.scrollWidth <= innerWidth + 1, label: c.textContent.trim(), tip: (u.closest('.has-tip').querySelector('.tip') || {}).textContent} })()"""


def tk2_case(rc, lang, theme, size, source, cache):
    d = rc.dev('admin', lang, theme, size)
    now = 'Math.floor(Date.now() / 1000)'
    usage = "{calls: %d, tokensIn: %d, tokensOut: %d%s}" % (
        USAGE['calls'], USAGE['in'], USAGE['out'], ', tokensCached: %d, tokensCacheWrite: %d' % (USAGE['read'], USAGE['write']) if cache else '')
    job_js = """window.__fakeJob = {id: 987003, state: 'running', phase: 'writing', source: '%s', name: 'Made-up bank_admin_20261113120000', from: [], fromCount: 0,
      file: %s, mode: null, found: null, read: null, confirmBy: null, made: 4, goal: 20, calls: 12, createdAt: %s - 60, startedAt: %s - 50, bankId: null, error: null, charged: false,
      usage: %s}; 1""" % (source, "{name: 'Made-up notes.docx', type: 'docx', size: 2048}" if source == 'file' else 'null', now, now, usage)
    script = d.cdp.call('Page.addScriptToEvaluateOnNewDocument', source=FAKE_JS + ';' + job_js)
    try:
        goto(d, '#/banks', 1.0)
        d.reload(3.0)
        if not until(d, "!!document.querySelector('.gen-usage')", 12):
            rc.rec('W-TK-2', 'banks', source, 'admin', size, lang, theme, False, 'no token text on the running card', d)
            return
        time.sleep(0.6)
        r = d.js(TK2_JS)
    finally:
        try:
            d.cdp.call('Page.removeScriptToEvaluateOnNewDocument', identifier=(script or {}).get('identifier'))
        except Exception:
            pass
        d.js("window.__fakeJob = null; 1")
        d.reload(1.5)
    problems = []
    if r is None:
        problems.append('no token text or no cancel button')
    else:
        want_calls = fill(cat(lang, 'aigen.card.calls'), calls=12)
        sep = cat(lang, 'aigen.card.callsSep')
        if len(r['n']) != 1 or not r['n'][0]['text'].startswith(want_calls):
            problems.append('call text "%s", wanted "%s..."' % ([x['text'] for x in r['n']], want_calls))
        elif not r['narrow'] and not r['n'][0]['text'].endswith(sep):
            problems.append('wide call text lacks "%s": %s' % (sep, r['n'][0]['text']))
        if [x['text'] for x in r['l'][:1]] != [cat(lang, 'aigen.card.ioLabel')] or r['v'][0]['text'] != '85.4K/59.2K Token':
            problems.append('row 1 is "%s" "%s"' % (r['l'][:1], r['v'][:1]))
        if cache:
            if len(r['l']) != 2 or r['l'][1]['text'] != cat(lang, 'aigen.card.cacheLabel') or r['v'][1]['text'] != '6.9K/76.3K Token':
                problems.append('row 2 is "%s" "%s"' % (r['l'][1:], r['v'][1:]))
            else:
                if abs(r['l'][0]['l'] - r['l'][1]['l']) > 0.5 or abs(r['v'][0]['l'] - r['v'][1]['l']) > 0.5:
                    problems.append('row 2 is not aligned under row 1 (label %.1f / %.1f, numbers %.1f / %.1f)'
                                    % (r['l'][0]['l'], r['l'][1]['l'], r['v'][0]['l'], r['v'][1]['l']))
                if r['l'][1]['t'] <= r['l'][0]['t']:
                    problems.append('row 2 is not below row 1')
                if not r['narrow'] and r['n'][0]['t'] != r['l'][0]['t']:
                    problems.append('wide: the call text and the label are not on one row')
        elif len(r['l']) != 1:
            problems.append('row 2 shown without cache figures')
        if r['narrow'] and r['n'][0]['t'] >= r['l'][0]['t']:
            problems.append('narrow: the call text is not above the label rows')
        if not r['fit'] or r['wraps'] or r['right'] > r['vw'] + 0.5 or not r['side']:
            problems.append('does not fit: fit %s, wrapped %s, right %.1f of %s, no sideways scroll %s' % (r['fit'], r['wraps'], r['right'], r['vw'], r['side']))
        if r['fs'] < 11 or r['fs'] > 12.5 or 'tabular' not in r['tab']:
            problems.append('type %s / %s' % (r['fs'], r['tab']))
        if cache and not any(x in (r['tip'] or '') for x in ('6,900', '6.900', '6 900')):
            problems.append('the tip has no exact cache write: %s' % r['tip'])
        if r['label'] != cat(lang, 'aigen.imp.card.cancel' if source == 'file' else 'aigen.card.cancel'):
            problems.append('cancel label "%s"' % r['label'])
    detail = '; '.join(problems) or '%s layout, "%s"' % ('narrow' if r['narrow'] else 'wide', ' | '.join(x['text'] for x in r['n'] + r['l'] + r['v']))
    rc.rec('W-TK-2', 'banks', '%s%s' % (source, '-cache' if cache else ''), 'admin', size, lang, theme, not problems, detail, d if problems else None)


def g_tk2(rc):
    if not rc.want_any(['W-TK-2'], ['banks']):
        return
    for lang in rc.langs(['zh', 'en', 'ja']):
        for size in rc.sizes([(320, 640), (360, 740), (390, 844), (402, 874), (430, 932), (1280, 800)], lambda s: True):
            for source in ('file', 'bank'):
                rc.guard('W-TK-2', 'banks', tk2_case, rc, lang, rc.themes(['light'])[0], size, source, True)
            rc.guard('W-TK-2', 'banks', tk2_case, rc, lang, rc.themes(['light'])[0], size, 'bank', False)


GROUPS_V12_TOKENS = [g_tk2]
