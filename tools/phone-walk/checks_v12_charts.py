# -*- coding: utf-8 -*-
"""v1.2 walk checks: the AI usage charts have no slanted label (A-OWNER-86). Same style as checks.py (it reuses its helpers).
Register in checks.py:

    from checks_v12_charts import GROUPS_V12_CHARTS      # next to the other imports
    GROUPS = [..., *GROUPS_V12_CHARTS]                   # at the end of the list

W-UC-3: the usage answer is replaced in the page by made-up rows (1 to 30 models with long ids like
example-model-long-name-5-5, 1 to 30 days). Wanted at every size: no text turned by a transform; on a phone (and wherever the
column chart would not show every label flat) the by-feature / by-model bars are horizontal rows with the name on one line at the
left (an ellipsis only when the name is longer than 40 percent of the card, the full name in the tip), the bar growing right and
the value at the bar's end; the dates of the line are short (MM-DD), horizontal, never touching (at least 6 px apart), the newest
always named; nothing leaves the card; no sideways scroll; the calls / Token switch keeps all of it; a tap shows the tip.
"""
import time

from checks import goto, until

# Replaces byModel / byPeriod of the usage answer: `n` models, `days` days (the newest is today, UTC); short=true: short names
FAKE_USAGE_JS = """(() => { const real = window.fetch.bind(window); const P = %s;
  window.fetch = async (u, init) => { const r = await real(u, init);
    if (!String(u).includes('/api/ai/admin/usage')) return r;
    const j = await r.clone().json(); const day = 86400000; const to = new Date(); to.setUTCHours(0, 0, 0, 0);
    const from = new Date(to.getTime() - (P.days - 1) * day);
    const iso = (t) => new Date(t).toISOString().slice(0, 10);
    const names = ['example-model-long-name-5-5', 'm-1', 'example-haiku-5-5', 'x', 'example-opus-5-5-20261101-preview'];
    j.byModel = Array.from({length: P.n}, (_, k) => ({model: P.short ? ['Chat', 'Hint', 'Notes', 'Quiz', 'Misc', 'Plan', 'Tags'][k %% 7] + (k > 6 ? k : '') : names[k %% 5] + (k >= 5 ? '-' + k : ''),
      calls: 90 - k * 2 + 1, tokensIn: 50000 - k * 900, tokensOut: 20000 - k * 300, errors: k %% 4 === 0 ? 1 : 0, tokensCached: 1000, tokensCacheWrite: 300}));
    const sum = (key) => j.byModel.reduce((a, r) => a + r[key], 0);
    j.totals = Object.assign({}, j.totals, {calls: sum('calls'), tokensIn: sum('tokensIn'), tokensOut: sum('tokensOut')});
    j.unit = 'day'; j.from = iso(from.getTime()); j.to = iso(to.getTime());
    j.byPeriod = Array.from({length: P.days}, (_, k) => ({period: iso(from.getTime() + k * day), calls: (k * 7) %% 13 + 1, tokensIn: 900 * k + 100, tokensOut: 400 * k + 50, errors: 0, tokensCached: 10, tokensCacheWrite: 5}));
    return new Response(JSON.stringify(j), {status: r.status, headers: {'Content-Type': 'application/json'}}); }; return 1 })()"""

UC3_JS = r"""(async () => {
  const wait = (ms) => new Promise(r => setTimeout(r, ms));
  const rect = (e) => { const b = e.getBoundingClientRect(); return {l: b.left, r: b.right, t: b.top, b: b.bottom, w: b.width, text: e.textContent} };
  const read = (host) => {
    const svg = host.querySelector('svg'); const box = host.getBoundingClientRect();
    const turned = [...svg.querySelectorAll('text')].filter(e => e.hasAttribute('transform') || /rotate/.test(getComputedStyle(e).transform)).length;
    const h = svg.classList.contains('uc-hsvg');
    const names = [...svg.querySelectorAll('.uc-name')]; const dates = [...svg.querySelectorAll('.uc-date')];
    const labels = [...svg.querySelectorAll('.uc-label')].map(rect);
    let overlaps = 0;
    for (let i = 0; i < labels.length; i++) for (let j = i + 1; j < labels.length; j++) { const a = labels[i], b = labels[j];
      if (a.l < b.r - 0.5 && b.l < a.r - 0.5 && a.t < b.b - 0.5 && b.t < a.b - 0.5) overlaps++; }
    const out = labels.filter(l => l.l < box.left - 0.5 || l.r > box.right + 0.5).length;
    const o = {h, hl: box.left, turned, overlaps, out, w: box.width, items: svg.querySelectorAll('.uc-item').length, vals: [], dateGaps: [], dates: dates.map(e => e.textContent),
      label: svg.getAttribute('aria-label')};
    if (h) {
      const items = [...svg.querySelectorAll('.uc-item')];
      o.rows = items.map(g => { const n = g.querySelector('.uc-name'); const v = g.querySelector('.uc-val'); const bars = [...g.querySelectorAll('.uc-bar')].map(rect);
        const full = (g.getAttribute('aria-label') || '').split(', ')[0]; const nr = rect(n), vr = rect(v);
        const barR = Math.max(...bars.map(b => b.r)); return {full, shown: n.textContent, nameR: nr.r, nameW: nr.w, lines: n.querySelectorAll('tspan').length, valL: vr.l, valR: vr.r, barR, barL: Math.min(...bars.map(b => b.l)),
          rowH: g.querySelector('.uc-hit').getBoundingClientRect().height, tall: Math.abs((nr.t + nr.b) / 2 - (vr.t + vr.b) / 2) < 3} });
      o.axisL = svg.querySelector('.uc-axis').getBoundingClientRect().left;
    }
    if (dates.length) { const rs = dates.map(rect).sort((a, b) => a.l - b.l); for (let i = 1; i < rs.length; i++) o.dateGaps.push(rs[i].l - rs[i - 1].r);
      o.lastDate = rs[rs.length - 1].text; o.firstL = rs[0].l - box.left; }
    return o; };
  const res = {side: document.documentElement.scrollWidth <= innerWidth + 1};
  const bars = [...document.querySelectorAll('.uc-bars')].map(read); res.bars = bars;
  const line = document.querySelector('.uc-line'); res.line = line ? read(line) : null;
  // a tap on the first row / column shows the tip with the full name
  const first = document.querySelector('.uc-bars .uc-item'); if (first) { first.dispatchEvent(new Event('click')); await wait(80);
    const tip = document.querySelector('.uc-bars .uc-tip'); res.tip = tip && !tip.hidden ? tip.textContent : ''; res.tipIn = tip && !tip.hidden && tip.getBoundingClientRect().right <= innerWidth + 0.5; }
  // tokens mode: the same checks again
  document.querySelectorAll('.uc-sw-btn')[1].click(); await wait(300);
  res.tokBars = [...document.querySelectorAll('.uc-bars')].map(read);
  const lineBtn = line && line.closest('.uc-card').querySelectorAll('.uc-sw-btn')[1]; if (lineBtn) { lineBtn.click(); await wait(300); res.tokLine = read(document.querySelector('.uc-line')); }
  return res })()"""


def uc3_problems(r, size):
    """What is wrong in the page's answer `r` of UC3_JS (a list of texts; empty = fine)"""
    problems = []
    if not r:
        problems.append('no answer from the page')
    else:
        if not r['side']:
            problems.append('sideways scroll')
        phone = size[0] < 520
        for where, charts in (('calls', r['bars']), ('tokens', r['tokBars'])):
            if len(charts) != 2:
                problems.append('%s: %d bar charts' % (where, len(charts)))
            for c in charts:
                if c['turned']:
                    problems.append('%s %s: %d slanted labels' % (where, c['label'], c['turned']))
                if c['overlaps'] or c['out']:
                    problems.append('%s %s: %d overlapping, %d outside' % (where, c['label'], c['overlaps'], c['out']))
                if c['items'] == 0:
                    problems.append('%s %s: no bars' % (where, c['label']))
                if phone and not c['h']:
                    problems.append('%s %s: columns on a phone' % (where, c['label']))
                if c['h']:
                    for row in c['rows']:
                        if row['lines'] > 0:
                            problems.append('name on several lines: %s' % row['full'])
                        if row['shown'] != row['full'] and (not row['shown'].endswith('…') or row['nameW'] > 0.4 * c['w'] + 1):
                            problems.append('name "%s" cut wrongly (full "%s", %.0f of %.0f px)' % (row['shown'], row['full'], row['nameW'], c['w']))
                        if row['valL'] < row['barR'] - 0.5 or row['valR'] > c['hl'] + c['w'] + 0.5:
                            problems.append('value outside / before the bar end: %s' % row['full'])
                        if row['nameR'] > row['barL'] + 0.5:
                            problems.append('name touches the bar: %s' % row['full'])
                        if not row['tall']:
                            problems.append('name and value not on one line: %s' % row['full'])
                    if len(c['rows']) != c['items']:
                        problems.append('%d rows for %d items' % (len(c['rows']), c['items']))
        if r.get('tip') is not None and not r.get('tip'):
            problems.append('a tap on a bar shows no tip')
        if r.get('tip') and not r.get('tipIn'):
            problems.append('the tip leaves the screen')
        for where, c in (('calls', r['line']), ('tokens', r.get('tokLine'))):
            if c is None:
                problems.append('%s: no line chart' % where)
                continue
            if c['turned']:
                problems.append('%s line: %d slanted labels' % (where, c['turned']))
            if c['overlaps'] or c['out']:
                problems.append('%s line: %d overlapping, %d outside' % (where, c['overlaps'], c['out']))
            if not c['dates'] or any(len(x) != 5 or x[2] != '-' for x in c['dates']):
                problems.append('%s line: dates not MM-DD: %s' % (where, c['dates'][:4]))
            if any(g < 6 for g in c['dateGaps']):
                problems.append('%s line: dates only %.1f px apart' % (where, min(c['dateGaps'])))
            if c['dates'] and c['lastDate'] != time.strftime('%m-%d', time.gmtime()):
                problems.append('%s line: the newest date is not named (%s)' % (where, c['lastDate']))
    return problems


def uc3_case(rc, lang, theme, size, n, days, short):
    d = rc.dev('admin', lang, theme, size)
    js = FAKE_USAGE_JS % ('{n: %d, days: %d, short: %s}' % (n, days, 'true' if short else 'false'))
    script = d.cdp.call('Page.addScriptToEvaluateOnNewDocument', source=js)
    tag = '%d models%s, %d days' % (n, ' (short names)' if short else '', days)
    try:
        goto(d, '#/ai-settings?tab=usage', 1.0)
        d.reload(3.0)
        if not until(d, "document.querySelectorAll('.uc-chart svg').length >= 3", 12):
            rc.rec('W-UC-3', 'ai-settings', tag, 'admin', size, lang, theme, False, 'no charts', d)
            return
        time.sleep(0.6)
        r = d.js(UC3_JS)
    finally:
        try:
            d.cdp.call('Page.removeScriptToEvaluateOnNewDocument', identifier=(script or {}).get('identifier'))
        except Exception:
            pass
        d.reload(1.5)
    problems = uc3_problems(r, size)
    detail = '; '.join(sorted(set(problems))[:3]) or '%s, no slanted label, %s bars, dates %s' % (
        tag, 'horizontal' if r['bars'][0]['h'] else 'column', ' '.join(r['line']['dates'][-3:]))
    rc.rec('W-UC-3', 'ai-settings', tag, 'admin', size, lang, theme, not problems, detail, d if problems else None)


def g_uc3(rc):
    if not rc.want_any(['W-UC-3'], ['ai-settings']):
        return
    variants = [(30, 30, False), (1, 1, False), (5, 7, False), (6, 14, True)]
    for lang in rc.langs(['zh', 'en', 'ja']):
        # the language changes the feature names only: every language at five sizes, the other phone sizes in zh
        sizes = [(320, 640), (393, 852), (430, 932), (768, 1024), (1280, 800)]
        if lang == 'zh':
            sizes = [(320, 640), (360, 740), (375, 667), (390, 844), (393, 852), (402, 874), (430, 932), (768, 1024), (1280, 800)]
        for size in rc.sizes(sizes, lambda s: True):
            for n, days, short in variants:
                rc.guard('W-UC-3', 'ai-settings', uc3_case, rc, lang, rc.themes(['light'])[0], size, n, days, short)


GROUPS_V12_CHARTS = [g_uc3]
