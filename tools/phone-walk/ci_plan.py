#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Plans the CI phone walk: which runner (shard) runs which checks, so that every runner finishes at about the same time.

    python tools/phone-walk/ci_plan.py                         # print the plan as a table (N chosen automatically)
    python tools/phone-walk/ci_plan.py --n 12                  # force the runner count
    python tools/phone-walk/ci_plan.py --out plan.json --github-output "$GITHUB_OUTPUT" --summary "$GITHUB_STEP_SUMMARY"
    python tools/phone-walk/ci_plan.py --merge OUT_ZH OUT_EN OUT_JA --out MERGED_DIR     # merge the reports of one runner
    python tools/phone-walk/ci_plan.py --update-from reports --plan plan.json --write ci-timings.json   # refresh the timings

How the plan is made (all numbers come from ci-timings.json, seconds per check id and language):
  * every check id belongs to a function group (FUNCTIONS below: "AI chat", "quiz and answer sheet", ...); an id that no group
    lists is still planned ("other checks", a default estimate) and runs on the catch-all runner;
  * W-SH-* / W-MN-* (the checks that walk every page) are split by page (`walk.py --pages`) into "common checks" pieces;
  * a group that is longer than the target is cut into parts by check id; one check id (or one page) that is still longer is cut
    by theme (`--themes light` / `--themes dark`, two runners);
  * the pieces are packed (best fit, longest first, preferring a runner of the same function) into at most N runners, and N in
    8..19 (GitHub's free plan runs 20 jobs at once; one is the plan job) is the one with the shortest predicted wall time;
  * each runner starts three walk.py processes at once (--langs zh / en / ja), so its time is the longest language, not the sum;
  * the catch-all runner runs `--skip <every id of every other runner>`, so a check id that no piece lists is never lost, and
    validate() proves that every known check id (and every page) is covered exactly once before the plan is accepted.
The plan is deterministic. Output for GitHub Actions: a JSON matrix {"include": [{name, only, pages, skip, themes, est}]}.
"""
import argparse
import ast
import fnmatch
import glob
import json
import math
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TIMINGS = os.path.join(HERE, 'ci-timings.json')
LANGS = ['zh', 'en', 'ja']
# checks that hard-code the language zh even in --full (checks.py g_l54): only the zh process of a runner runs them
HARD_ZH = ['W-JC-1', 'W-AB-2', 'W-GB-2']
SHARED_PATTERNS = ['W-SH-*', 'W-MN-*']      # the checks that visit every page; split by --pages
MIN_RUNNERS, MAX_RUNNERS = 8, 19            # GitHub's free plan: 20 jobs at once, one of them is the plan job
PAGE_STEP = 5                               # seconds: the capacity search steps

# Function groups: (name shown in the job list, id patterns). First match wins; a pattern is an exact id or an fnmatch pattern.
FUNCTIONS = [
    ('AI chat · conversation', ['W-CH-*', 'W-V13-CV', 'W-V13-ED', 'W-V13-MM', 'W-V22-CS', 'W-V23-CMP', 'W-V23-CTR', 'W-V23-EN',
                                'W-V23-LT', 'W-V23-NT', 'W-V23-HD']),
    ('AI chat · history drawer', ['W-V23-*']),
    ('AI settings · connections and models', ['W-AS-1', 'W-AS-2', 'W-AS-3', 'W-V13-AC', 'W-V13-AT', 'W-V22-MS', 'W-FP-1', 'W-CAP-*']),
    ('AI settings · usage and tokens', ['W-UC-*', 'W-PT-*', 'W-TK-*']),
    ('banks and AI import', ['W-IM-*', 'W-GB-*', 'W-JC-1', 'W-V22-GJ', 'W-L26', 'W-L57', 'W-L73', 'W-BK-29', 'W-AB-2', 'W-V31-CAT-*']),
    ('AI badges and buttons', ['W-AB-1', 'W-AI-BTN', 'W-BT-1']),
    ('quiz and answer sheet', ['W-QZ-*', 'W-AS-1v', 'W-L60', 'W-V22-QT', 'W-V27-QD', 'W-V27-QT', 'W-FX-2', 'W-FX-3', 'W-FX-4']),
    ('home dashboard', ['W-HM-1', 'W-V25-DASH', 'W-V25-RS', 'W-V25-ST', 'W-V25-MN', 'W-V26-RANK', 'W-V26-POD', 'W-V26-CAP',
                        'W-V26-TL', 'W-V26-DSK', 'W-V31-GD-*']),
    ('result pages', ['W-RS-*', 'W-RC-*', 'W-V13-RS', 'W-V25-RB']),
    ('users, logs and records', ['W-US-*', 'W-V13-US', 'W-V13-LG', 'W-LG-*', 'W-V26-LOG', 'W-V27-PG', 'W-V27-UR', 'W-V31-DV-2']),
    ('sign-in, profile and settings', ['W-SI-1', 'W-V13-SI', 'W-V26-NAME', 'W-V26-PRF', 'W-V26-CTR', 'W-ST-1', 'W-FT-1', 'W-V22-FT',
                                       'W-V22-AB', 'W-FX-1', 'W-V31-NT-*', 'W-V31-IN-*', 'W-V31-DV-1', 'W-V31-TZ-*', 'W-V31-SW-*']),
]
OTHER = 'other checks'
COMMON = 'common checks'
ID_RE = re.compile(r'^W-[A-Z0-9]+(?:-[A-Za-z0-9]+)*$')


# ------------------------------------------------------------------------------------------------ known ids and pages

def known_ids():
    """Every check id the walk can report: the string literals of tools/phone-walk/checks*.py that are exactly an id."""
    found = set()
    for path in sorted(glob.glob(os.path.join(HERE, 'checks*.py'))):
        with open(path, encoding='utf-8') as fh:
            tree = ast.parse(fh.read(), path)
        for node in ast.walk(tree):
            if isinstance(node, ast.Constant) and isinstance(node.value, str) and ID_RE.match(node.value):
                found.add(node.value)
    found.discard('W-GROUP')                 # the id of a crashed group, not a check
    return sorted(found)


def known_pages():
    """PAGES of walk.py, read from its source (importing walk.py would pull in the whole harness)."""
    with open(os.path.join(HERE, 'walk.py'), encoding='utf-8') as fh:
        tree = ast.parse(fh.read())
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(getattr(t, 'id', '') == 'PAGES' for t in node.targets):
            return list(ast.literal_eval(node.value))
    raise SystemExit('PAGES not found in walk.py')


def is_shared(check_id):
    return any(fnmatch.fnmatchcase(check_id, p) for p in SHARED_PATTERNS)


def group_of(check_id):
    for name, patterns in FUNCTIONS:
        if any(fnmatch.fnmatchcase(check_id, p) for p in patterns):
            return name
    return OTHER


# ------------------------------------------------------------------------------------------------ timings

def load_timings(path=TIMINGS):
    with open(path, encoding='utf-8') as fh:
        return json.load(fh)


def vec(timings, table, key, default):
    """Seconds per language [zh, en, ja] of `key` ('<key>|<lang>' entries of timings[table]); `default` where unknown.
    A check the full run only saw in zh (it runs once, in zh) runs in the language of its process when the languages are split
    over three processes (--langs en / ja), so a missing en / ja entry takes the zh value."""
    t = timings.get(table, {})
    zh = float(t.get('%s|zh' % key, default))
    return [zh] + [float(t.get('%s|%s' % (key, lang), zh)) for lang in LANGS[1:]]


def add(a, b):
    return [x + y for x, y in zip(a, b)]


def scale(a, f):
    return [x * f for x in a]


def vmax(a):
    return max(a)


# ------------------------------------------------------------------------------------------------ pieces and packing

class Piece:
    """What one runner runs: check ids (plain) or pages of the shared checks; `themes` None = both themes."""

    def __init__(self, group, label, cost, ids=None, pages=None, themes=None):
        self.group, self.label, self.cost, self.ids, self.pages, self.themes = group, label, cost, ids or [], pages or [], themes

    @property
    def kind(self):
        return ('pages' if self.pages else 'plain', self.themes)


class Runner:
    def __init__(self, kind, base=None):
        self.kind, self.load, self.pieces, self.catch_all = kind, list(base or [0.0] * 3), [], False

    def put(self, piece):
        self.load = add(self.load, piece.cost)
        self.pieces.append(piece)


def pages_base(timings):
    """What every runner that uses --pages pays besides its own pages: the always-visited sign-in page and the common home visits."""
    return add(vec(timings, 'pageSeconds', '@common', 0), vec(timings, 'pageSeconds', '@signin', 0))


def split_group(name, members, costs, target):
    """One group as pieces of at most 0.6 x `target` (seconds, longest language): whole when small, else cut by check id into balanced parts;
    one check id that alone is longer than the target is cut by theme into two dedicated pieces (light / dark)."""
    total = [sum(costs[i][k] for i in members) for k in range(3)]
    if vmax(total) <= target * 0.6:
        return [Piece(name, name, total, ids=members)]
    pieces, rest = [], []
    for i in members:
        if vmax(costs[i]) > target * 1.05:
            for th in ('light', 'dark'):
                pieces.append(Piece(name, '%s · %s (%s)' % (name, i, th), scale(costs[i], 0.5), ids=[i], themes=th))
        else:
            rest.append(i)
    if rest:
        total = [sum(costs[i][k] for i in rest) for k in range(3)]
        parts = max(1, math.ceil(vmax(total) / (target * 0.6)))
        bins = [[[0.0] * 3, []] for _ in range(parts)]
        for i in sorted(rest, key=lambda x: (-vmax(costs[x]), x)):
            best = min(bins, key=lambda b: (vmax(add(b[0], costs[i])), len(b[1])))
            best[0] = add(best[0], costs[i])
            best[1].append(i)
        bins = sorted((b for b in bins if b[1]), key=lambda b: -vmax(b[0]))
        for n, (cost, ids_) in enumerate(bins, 1):
            pieces.append(Piece(name, name if len(bins) == 1 else '%s · part %d of %d' % (name, n, len(bins)), cost, ids=sorted(ids_)))
    return pieces


def lpt(pieces, runners, slack):
    """Longest piece first onto the least loaded runner of its kind; a runner that already holds the same function wins when its
    load is within `slack` seconds of the least loaded one, so that a runner's name stays meaningful."""
    for piece in sorted(pieces, key=lambda p: (-vmax(p.cost), p.label)):
        cands = [r for r in runners if r.kind == piece.kind]
        low = min(vmax(add(r.load, piece.cost)) for r in cands)
        near = [r for r in cands if vmax(add(r.load, piece.cost)) <= low + slack]
        same = [r for r in near if any(q.group == piece.group for q in r.pieces)]
        pick = min(same or near, key=lambda r: (vmax(add(r.load, piece.cost)), len(r.pieces)))
        pick.put(piece)


def improve(runners):
    """Hill climbing on the packing: move a piece, or swap two, between two runners of the same kind while that lowers the
    longer of the two (the longest language of each runner counts)."""
    def worst(a, b):
        return max(vmax(a), vmax(b))
    for _ in range(400):
        top = max(runners, key=lambda r: vmax(r.load))
        best = None
        for other in runners:
            if other is top or other.kind != top.kind:
                continue
            now = worst(top.load, other.load)
            for p in [q for q in top.pieces if not q.themes]:
                cand = worst([x - y for x, y in zip(top.load, p.cost)], add(other.load, p.cost))
                if cand < now - 1 and (best is None or cand < best[0]):
                    best = (cand, other, p, None)
                for q in [q for q in other.pieces if not q.themes]:
                    cand = worst(add([x - y for x, y in zip(top.load, p.cost)], q.cost), add([x - y for x, y in zip(other.load, q.cost)], p.cost))
                    if cand < now - 1 and (best is None or cand < best[0]):
                        best = (cand, other, p, q)
        if best is None:
            return
        _, other, p, q = best
        top.pieces.remove(p)
        top.load = [x - y for x, y in zip(top.load, p.cost)]
        other.put(p)
        if q is not None:
            other.pieces.remove(q)
            other.load = [x - y for x, y in zip(other.load, q.cost)]
            top.put(q)


def layout(ids, pages, timings, count, page_runners):
    """The runners for exactly `count` runners, `page_runners` of them for the shared checks; None when it cannot be done."""
    default = float(timings.get('defaultCheckSeconds', 40))
    base = pages_base(timings)
    by_group = {}
    for i in ids:
        if not is_shared(i):
            by_group.setdefault(group_of(i), []).append(i)
    costs = {i: vec(timings, 'seconds', i, default) for g in by_group.values() for i in g}
    page_cost = {p: vec(timings, 'pageSeconds', p, default) for p in pages if p != 'signin'}
    total = [sum(c[k] for c in costs.values()) + sum(c[k] for c in page_cost.values()) + page_runners * base[k] for k in range(3)]
    floor = vmax(total) / count
    target = floor
    runners = None
    for _ in range(30):          # the dedicated (theme-cut) runners change the share of the others: raise the target until it fits
        plain, shared = [], []
        for name in [n for n, _ in FUNCTIONS] + [OTHER]:
            members = sorted(by_group.get(name, []))
            if not members:
                continue
            if name == OTHER:
                plain.append(Piece(name, name, [sum(costs[i][k] for i in members) for k in range(3)], ids=members))
            else:
                plain += split_group(name, members, costs, target)
        for p, cost in page_cost.items():
            if vmax(add(cost, base)) > target:
                for th in ('light', 'dark'):
                    shared.append(Piece(COMMON, '%s · %s (%s)' % (COMMON, p, th), scale(cost, 0.5), pages=[p], themes=th))
            else:
                shared.append(Piece(COMMON, '%s · %s' % (COMMON, p), cost, pages=[p]))
        if 'signin' in pages:
            shared.append(Piece(COMMON, '%s · signin' % COMMON, vec(timings, 'pageSeconds', 'signin', 0), pages=['signin']))
        dedicated = [p for p in plain + shared if p.themes]
        runners = [Runner(p.kind, base if p.pages else None) for p in dedicated]
        free_shared = [p for p in shared if not p.themes]
        free_plain = [p for p in plain if not p.themes]
        n_shared = page_runners if free_shared else 0
        n_plain = count - len(runners) - n_shared
        if n_plain < 1 or (free_shared and n_shared < 1):
            target *= 1.05
            runners = None
            continue
        runners += [Runner(('pages', None), base) for _ in range(n_shared)] + [Runner(('plain', None)) for _ in range(n_plain)]
        lpt(dedicated + free_shared + free_plain, runners, 0.04 * target)
        improve(runners)
        if max(vmax(r.load) for r in runners) <= target * 1.1:
            return runners
        target *= 1.05
    return runners


def runner_name(r):
    weight = {}
    for p in r.pieces:
        weight[p.group] = weight.get(p.group, 0) + vmax(p.cost)
    groups = sorted(weight, key=lambda g: (-weight[g], g))
    th = ' (%s)' % r.kind[1] if r.kind[1] else ''
    if r.kind[0] == 'pages':
        pages = [x for p in r.pieces for x in p.pages]
        return '%s · %s%s' % (COMMON, ', '.join(pages), th)
    if len(r.pieces) == 1:
        return r.pieces[0].label
    parts = [x for x in groups]
    first = ' + '.join(parts[:2]) if len(parts) <= 2 else '%s + %d more' % (parts[0], len(parts) - 1)
    return first + th


def plan(timings, n=None, ids=None, pages=None):
    """The best layout for N in MIN_RUNNERS..MAX_RUNNERS (or `n`): the shortest predicted wall time wins, a larger N must be
    2 % faster to be worth another runner."""
    ids = ids if ids is not None else known_ids()
    pages = pages if pages is not None else known_pages()
    setup = float(timings.get('setupSeconds', 45)) + float(timings.get('jobOverheadSeconds', 35))
    factor = float(timings.get('parallelFactor', 1.3))
    best = None
    for count in ([n] if n else range(MIN_RUNNERS, MAX_RUNNERS + 1)):
        top = None
        for page_runners in range(1, min(8, count - 1)):
            runners = layout(ids, pages, timings, count, page_runners)
            if not runners:
                continue
            slowest = max(vmax(r.load) for r in runners)
            if top is None or slowest < top[0] - 1:
                top = (slowest, runners)
        if top is None:
            continue
        wall = setup + factor * top[0]
        if best is None or wall < best[0] * 0.98:
            best = (wall, count, top[1], top[0])
    wall, count, runners, slowest = best
    # the catch-all runner: the one with the "other checks" piece, else the lightest plain runner with both themes
    plain = [r for r in runners if r.kind == ('plain', None)]
    holder = [r for r in plain if any(p.group == OTHER for p in r.pieces)]
    catch = holder[0] if holder else min(plain, key=lambda r: vmax(r.load))
    catch.catch_all = True
    return {'runners': runners, 'count': count, 'wall': wall, 'slowest': slowest, 'setup': setup, 'factor': factor}


def matrix(result):
    """The GitHub Actions matrix: one entry per runner."""
    runners = result['runners']
    claimed = sorted({i for r in runners if not r.catch_all for p in r.pieces for i in p.ids})
    names, include = {}, []
    for r in sorted(runners, key=lambda r: (r.kind[0] == 'pages', runner_name(r))):
        name = runner_name(r)
        names[name] = names.get(name, 0) + 1
        if names[name] > 1:
            name += ' %d' % names[name]
        item = {'name': name, 'slug': '', 'nozh': ','.join(sorted(HARD_ZH)), 'only': '', 'pages': '', 'skip': '', 'themes': r.kind[1] or '',
                'est': ' / '.join('%.0f' % (x / 60) for x in r.load)}
        if r.kind[0] == 'pages':
            item['only'] = ','.join(SHARED_PATTERNS)
            item['pages'] = ','.join(x for p in r.pieces for x in p.pages)
        elif r.catch_all:
            item['skip'] = ','.join(claimed + SHARED_PATTERNS)
        else:
            item['only'] = ','.join(i for p in r.pieces for i in p.ids)
        include.append(item)
    used = set()
    for item in include:                                # ASCII artifact name: upload-artifact refuses / \ : < > | * ? " and line breaks
        base = re.sub(r'[^a-z0-9]+', '-', item['name'].lower()).strip('-') or 'shard'
        slug, k = base, 1
        while slug in used:
            k += 1
            slug = '%s-%d' % (base, k)
        used.add(slug)
        item['slug'] = slug
    return {'include': include}


def selects(item, check_id):
    """Would walk.py with this matrix item run `check_id` (walk.wants_check)?"""
    only = [x for x in item['only'].split(',') if x]
    skip = [x for x in item['skip'].split(',') if x]
    if any(fnmatch.fnmatchcase(check_id, p) for p in skip):
        return False
    return not only or any(fnmatch.fnmatchcase(check_id, p) for p in only)


def validate(mx, ids, pages):
    """Every known check id runs in exactly one runner (both themes), every page of the shared checks in exactly one."""
    problems = []
    for i in ids:
        if is_shared(i):
            continue
        modes = [it['themes'] or 'both' for it in mx['include'] if selects(it, i) and not it['pages']]
        if sorted(modes) not in (['both'], ['dark', 'light']):
            problems.append('check %s runs %s' % (i, modes or 'nowhere'))
    for page in pages:
        modes = [it['themes'] or 'both' for it in mx['include'] if it['pages'] and page in it['pages'].split(',')]
        if page != 'signin' and sorted(modes) not in (['both'], ['dark', 'light']):
            problems.append('page %s runs %s' % (page, modes or 'nowhere'))
        if page == 'signin' and not modes:
            problems.append('page signin runs nowhere')
    for fam in ('W-SH-1', 'W-MN-1'):                     # a shared check never runs on a plain runner
        if any(selects(it, fam) and not it['pages'] for it in mx['include']):
            problems.append('%s runs on a runner without --pages' % fam)
    names = [it['name'] for it in mx['include']]
    if len(set(names)) != len(names):
        problems.append('duplicate runner names')
    slugs = [it.get('slug', '') for it in mx['include']]
    if len(set(slugs)) != len(slugs) or not all(re.fullmatch(r'[a-z0-9][a-z0-9-]*', x) for x in slugs):
        problems.append('runner slugs must be unique ASCII words (they name the artifacts)')
    for it in mx['include']:
        if any(c in it['name'] for c in '\\"/:<>|*?\r\n'):
            problems.append('runner name %r contains a character an artifact name refuses' % it['name'])
    return problems


def table(result, mx, ids):
    lines = []
    lines.append('| # | runner | checks | zh / en / ja min (sequential) | predicted wall min |')
    lines.append('|--:|---|---|---|--:|')
    byname = {}
    runners = sorted(result['runners'], key=lambda r: (r.kind[0] == 'pages', runner_name(r)))
    for n, (r, it) in enumerate(zip(runners, mx['include']), 1):
        what = []
        for p in r.pieces:
            what.append(p.label if not p.ids else '%s [%d]' % (p.label, len(p.ids)))
        if r.catch_all:
            what.append('+ catch-all (--skip of every other id)')
        wall = (result['setup'] + result['factor'] * vmax(r.load)) / 60
        lines.append('| %d | %s | %s | %s | %.1f |' % (n, it['name'], '; '.join(what), it['est'], wall))
    lines.append('')
    lines.append('%d runners, slowest runner %.1f min of checks (longest language), predicted wall time %.1f min '
                 '(setup %.0f s + parallel factor %.2f); %d check ids planned.'
                 % (result['count'], result['slowest'] / 60, result['wall'] / 60, result['setup'], result['factor'], len(ids)))
    return '\n'.join(lines)


# ------------------------------------------------------------------------------------------------ reports: merge and timings

def merge_reports(dirs, out):
    """Merges the report.json of the per-language walk processes of one runner into one report.json (+ their screenshots) in `out`."""
    import shutil
    results, notes, seconds, full = [], [], 0, False
    os.makedirs(out, exist_ok=True)
    seen = set()
    for d in dirs:
        proc = os.path.basename(d.rstrip('/\\')).rsplit('-', 1)[-1]      # the language of the process (walk-zh -> zh)
        path = os.path.join(d, 'report.json')
        if not os.path.isfile(path):
            notes.append('no report.json in %s (the walk process died before it wrote one)' % os.path.basename(d.rstrip('/\\')))
            continue
        with open(path, encoding='utf-8') as fh:
            doc = json.load(fh)
        for r in doc.get('results', []):
            r['proc'] = proc
            key = (r['id'], r.get('page'), r.get('state'), r.get('user'), r.get('size'), r.get('lang'), r.get('theme'))
            if proc != 'zh' and key in seen:            # a check that hard-codes a language runs in every process: keep the first
                continue
            seen.add(key)
            results.append(r)
        notes += doc.get('notes', [])
        seconds = max(seconds, doc.get('seconds', 0))
        full = full or doc.get('full', False)
        for name in os.listdir(d):
            if name != 'report.json':
                src = os.path.join(d, name)
                if os.path.isfile(src):
                    shutil.copy2(src, os.path.join(out, name))
    for r in results:                                   # screenshot paths now point into the merged folder
        if r.get('screenshot'):
            r['screenshot'] = os.path.join(out, os.path.basename(r['screenshot']))
    failed = [r for r in results if r['pass'] is False and r.get('priority', 1) == 1]
    summary = {'total': len(results), 'passed': sum(1 for r in results if r['pass'] is True),
               'failed': sum(1 for r in results if r['pass'] is False), 'p1Failed': len(failed),
               'skipped': sum(1 for r in results if r['pass'] is None), 'seconds': seconds}
    if len(notes) and any(n.startswith('no report.json') for n in notes):
        summary['p1Failed'] += 1                        # a missing report is a failure, never a silent pass
        results.append({'id': 'W-CI-MERGE', 'page': 'ci', 'state': 'missing-report', 'user': None, 'size': '0x0', 'lang': '-',
                        'theme': '-', 'pass': False, 'detail': '; '.join(n for n in notes if n.startswith('no report.json')),
                        'screenshot': None, 'priority': 1, 'ms': 0})
        summary['total'] += 1
        summary['failed'] += 1
    doc = {'seconds': seconds, 'full': full, 'summary': summary, 'notes': notes, 'results': results}
    with open(os.path.join(out, 'report.json'), 'w', encoding='utf-8', newline='\n') as fh:
        json.dump(doc, fh, indent=1, ensure_ascii=False)
        fh.write('\n')
    print('merged %d report(s): %d entries, %d passed, %d failed (%d P1), %d skipped'
          % (len(dirs), summary['total'], summary['passed'], summary['failed'], summary['p1Failed'], summary['skipped']))
    return 1 if summary['p1Failed'] else 0


def attribute(entries_by_shard, pages_by_shard, themes_by_shard=None):
    """Seconds per ('<id>|<lang>') and per ('<page>|<lang>') from walk entries [{id, page, lang, ms}] of every shard.
    The shared checks (W-SH-*, W-MN-*) are measured per page of their shard; '@common' is what every pages shard visits
    anyway (the home page), '@signin' the sign-in visit. A shard that ran one theme only counts twice."""
    themes_by_shard = themes_by_shard or {}
    seconds, pages = {}, {}
    common = {}
    sh_pages = {}                                      # shard -> {label owner: {lang: s}}
    for shard, entries in entries_by_shard.items():
        mine = [p for p in pages_by_shard.get(shard, []) if p]
        mult = 2.0 if themes_by_shard.get(shard) else 1.0
        per = {}
        for e in entries:
            lang = e.get('lang')
            if lang not in LANGS or e.get('ms') is None or e.get('proc', lang) != lang:
                continue                                # an entry of a language the process did not own is a hard-coded repeat
            s = e['ms'] / 1000.0 * mult
            if is_shared(e['id']):
                label = e.get('page', '').split(':')[0]
                owner = label if label in mine else next((p for p in mine if label.startswith(p + '-')), None)
                if owner is None:
                    owner = '@signin' if label == 'signin' else '@common'
                per.setdefault(owner, {}).setdefault(lang, 0.0)
                per[owner][lang] += s
            else:
                key = '%s|%s' % (e['id'], lang)
                cur = seconds.setdefault(shard, {})
                cur[key] = cur.get(key, 0.0) + s
        sh_pages[shard] = per
    ids_best = {}
    for shard, cur in seconds.items():                 # an id seen by several shards (side products): the biggest one is the real run
        for k, v in cur.items():
            ids_best[k] = max(ids_best.get(k, 0.0), v)
    for lang in LANGS:
        c = [sp['@common'][lang] for sp in sh_pages.values() if '@common' in sp and lang in sp['@common']]
        s = [sp['@signin'][lang] for sp in sh_pages.values() if '@signin' in sp and lang in sp['@signin']]
        common[lang] = sum(c) / len(c) if c else 0.0
        if c:
            pages['@common|%s' % lang] = common[lang]
        if s:
            pages['@signin|%s' % lang] = sum(s) / len(s)
    for shard, sp in sh_pages.items():
        for owner, per_lang in sp.items():
            if owner.startswith('@'):
                continue
            for lang, v in per_lang.items():
                if owner == 'home':
                    v = max(v - common[lang], 0.0)       # the home visits every pages shard makes are in '@common'
                if owner == 'signin':
                    v = max(v - pages.get('@signin|%s' % lang, 0.0), 0.0)
                pages['%s|%s' % (owner, lang)] = pages.get('%s|%s' % (owner, lang), 0.0) + v
    return ids_best, pages


def update_from(reports_dir, plan_path, write):
    pages_by_shard, themes_by_shard = {}, {}
    if plan_path and os.path.isfile(plan_path):
        with open(plan_path, encoding='utf-8') as fh:
            for it in json.load(fh)['matrix']['include']:
                key = it.get('slug') or it['name']
                pages_by_shard[key] = [x for x in it['pages'].split(',') if x]
                themes_by_shard[key] = it['themes']
    entries = {}
    for path in sorted(glob.glob(os.path.join(reports_dir, '*', 'report.json')) + glob.glob(os.path.join(reports_dir, 'report.json'))):
        shard = os.path.basename(os.path.dirname(path))
        shard = shard[len('walkreport-'):] if shard.startswith('walkreport-') else shard
        with open(path, encoding='utf-8') as fh:
            doc = json.load(fh)
        entries[shard] = [e for e in doc.get('results', []) if 'ms' in e]
    if not any(entries.values()):
        print('no report with "ms" fields under %s: timings not updated' % reports_dir)
        return 1
    seconds, pages = attribute(entries, pages_by_shard, themes_by_shard)
    try:
        old = load_timings()
    except OSError:
        old = {}
    new = dict(old)
    new['seconds'] = dict(old.get('seconds', {}), **{k: round(v, 1) for k, v in seconds.items()})
    if pages:
        new['pageSeconds'] = dict(old.get('pageSeconds', {}), **{k: round(v, 1) for k, v in pages.items()})
    new['parallelFactor'] = 1.0                 # reports of the parallel (three languages at once) runs already contain the slowdown
    new['source'] = 'reports of a CI walk run (the "ms" field of every entry); refreshed by ci_plan.py --update-from'
    with open(write, 'w', encoding='utf-8', newline='\n') as fh:
        json.dump(new, fh, indent=1, sort_keys=True, ensure_ascii=False)
        fh.write('\n')
    print('timings written to %s (%d check/language entries, %d page entries)' % (write, len(seconds), len(pages)))
    return 0


# ------------------------------------------------------------------------------------------------ main

def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    p.add_argument('--n', type=int, help='force the runner count (default: the best in %d..%d)' % (MIN_RUNNERS, MAX_RUNNERS))
    p.add_argument('--timings', default=TIMINGS, help='ci-timings.json (default: next to this script)')
    p.add_argument('--out', help='plan: write the plan JSON ({matrix, count, ...}) here; --merge: the merged output folder')
    p.add_argument('--github-output', help='append matrix= and count= to this file ($GITHUB_OUTPUT)')
    p.add_argument('--summary', help='append the plan table to this file ($GITHUB_STEP_SUMMARY)')
    p.add_argument('--merge', nargs='+', metavar='DIR', help='merge the report.json of these walk output folders into --out')
    p.add_argument('--update-from', metavar='DIR', help='refresh the timings from the report.json files below DIR (walkreport-*/)')
    p.add_argument('--plan', help='--update-from: the plan JSON of that run (to know the pages of every shard)')
    p.add_argument('--write', help='--update-from: where to write the new timings (default: the --timings file)')
    args = p.parse_args(argv)
    if args.merge:
        if not args.out:
            p.error('--merge needs --out')
        return merge_reports(args.merge, args.out)
    if args.update_from:
        return update_from(args.update_from, args.plan, args.write or args.timings)
    timings = load_timings(args.timings)
    ids, pages = known_ids(), known_pages()
    result = plan(timings, args.n, ids, pages)
    mx = matrix(result)
    problems = validate(mx, ids, pages)
    text = table(result, mx, ids)
    print(text)
    if problems:
        print('\nPLAN INVALID:\n  ' + '\n  '.join(problems), file=sys.stderr)
        return 1
    if args.summary:
        with open(args.summary, 'a', encoding='utf-8') as fh:
            fh.write('## Phone walk plan\n\n' + text + '\n')
    if args.out:
        with open(args.out, 'w', encoding='utf-8', newline='\n') as fh:
            json.dump({'matrix': mx, 'count': len(mx['include']), 'predictedMinutes': round(result['wall'] / 60, 1)}, fh, indent=1)
            fh.write('\n')
    if args.github_output:
        with open(args.github_output, 'a', encoding='utf-8', newline='\n') as fh:
            fh.write('matrix=%s\ncount=%d\n' % (json.dumps(mx, separators=(',', ':')), len(mx['include'])))
    return 0


if __name__ == '__main__':
    sys.exit(main())
