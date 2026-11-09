# -*- coding: utf-8 -*-
"""Run run_parser.py on every fixture + demo; write golden/<name>.json and golden/_index.json (summary)."""
import glob, json, os, subprocess, sys
H = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, H)
import paths  # noqa: E402
files = [('demo', paths.DEMO)] + [(os.path.splitext(os.path.basename(p))[0], p) for p in sorted(glob.glob(os.path.join(H, 'fixtures', '*.xlsx')))]
idx = {}
for name, p in files:
    r = subprocess.run([sys.executable, '-B', '-I', os.path.join(H, 'run_parser.py'), p, '--meta'], capture_output=True)
    if r.returncode != 0:
        idx[name] = {'runner_failed': r.stderr.decode('utf-8', 'replace')[-400:]}
        continue
    open(os.path.join(H, 'golden', name + '.json'), 'wb').write(r.stdout)
    d = json.loads(r.stdout.decode('utf-8'))
    so = d['server_outcome']
    idx[name] = {'status': so['status'], 'error': so.get('error'), 'nq': len(so.get('questions', [])),
                 'sheets': [(s['name'], s['total'], s['usable'], len(s['skipped'])) for s in so.get('sheets', [])]}
json.dump(idx, open(os.path.join(H, 'golden', '_index.json'), 'w', encoding='utf-8'), ensure_ascii=False, indent=1)
for k, v in idx.items():
    print(k, json.dumps(v, ensure_ascii=False))
