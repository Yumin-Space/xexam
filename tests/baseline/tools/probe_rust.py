# -*- coding: utf-8 -*-
"""把安全探针对准 Rust 后端：default / hosts / anyhost 三套配置各起一个全新实例，探测，再与旧版基准比较。

用法: python -I probe_rust.py <后端可执行文件> <旧版基准 json> [--out 目录]
基准由:  python -I security_probe.py --spawn-python <含 server.py 的目录> --no-dos --out python.json  生成。
（旧版的 `shutdown` 配置是 Python 专有的 --allow-shutdown，这里单独由 capture.py 的 #170/#171 覆盖。）
退出码 0 = 没有未登记的差异。"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import paths  # noqa: E402

exe = os.path.abspath(sys.argv[1])
golden = os.path.abspath(sys.argv[2])
out = sys.argv[sys.argv.index('--out') + 1] if '--out' in sys.argv else tempfile.mkdtemp(prefix='xexam_probe_rust_')
os.makedirs(out, exist_ok=True)
ALLOW = os.path.join(paths.GOLDEN, 'allowed_probe_differences_rust.json')
PROFILES = {
    'default': {},
    'hosts': {'XEXAM_ALLOWED_HOSTS': 'xexam.example.com,192.168.1.20', 'XEXAM_ALLOWED_ORIGINS': 'https://pub.example.org'},
    'anyhost': {'XEXAM_ALLOW_ANY_HOST': '1'},
}
PORT = 8872
version = open(os.path.join(paths.REPO_ROOT, 'VERSION')).read().strip()

mine = []
for prof, extra in PROFILES.items():
    d = tempfile.mkdtemp(prefix='xexam_prs_')
    env = {k: v for k, v in os.environ.items() if not k.startswith('XEXAM_')}
    env.update(extra)
    env.update(XEXAM_DATA=d, XEXAM_HOST='127.0.0.1', XEXAM_PORT=str(PORT), XEXAM_VERSION=version)
    log = open(os.path.join(d, 'server.log'), 'wb')
    p = subprocess.Popen([exe, 'serve', '--no-browser'], env=env, cwd=d, stdout=log, stderr=subprocess.STDOUT)
    try:
        for _ in range(100):
            try:
                urllib.request.urlopen('http://127.0.0.1:%d/api/info' % PORT, timeout=1)
                break
            except Exception:
                time.sleep(0.2)
        dest = os.path.join(out, 'rust_%s.json' % prof)
        r = subprocess.run([sys.executable, '-I', '-B', os.path.join(HERE, 'security_probe.py'), '--base', 'http://127.0.0.1:%d' % PORT,
                            '--profile', prof, '--no-dos', '--out', dest], capture_output=True, text=True, encoding='utf-8', timeout=900)
        print('%-8s 探针退出码 %s' % (prof, r.returncode))
        mine += json.load(open(dest, encoding='utf-8'))['results']
    finally:
        p.terminate()
        try:
            p.wait(timeout=8)
        except Exception:
            p.kill()
        time.sleep(0.4)
        shutil.rmtree(d, ignore_errors=True)

gold = json.load(open(golden, encoding='utf-8'))['results']
gk = {'%s/%s' % (g['profile'], g['id']): g for g in gold}
allow = json.load(open(ALLOW, encoding='utf-8')) if os.path.exists(ALLOW) else {}
base = lambda c: (c or '').split(';')[0].strip()
diffs, allowed, same, missing = [], [], 0, 0
for r in mine:
    key = '%s/%s' % (r['profile'], r['id'])
    g = gk.get(key)
    if g is None:
        missing += 1
        continue
    if g['state'] == r['state'] and base(g.get('ctype')) == base(r.get('ctype')):
        same += 1
        continue
    item = (key, r['desc'][:60], '%s %s' % (g['state'], base(g.get('ctype'))), '%s %s' % (r['state'], base(r.get('ctype'))))
    (allowed if key in allow else diffs).append(item)

print('探针 %d 项（旧版基准里没有的 %d 项）：相同 %d，已登记的差异 %d，未解释的差异 %d' % (len(mine), missing, same, len(allowed), len(diffs)))
for k, d, a, b in allowed:
    print('  [已登记] %-18s %s' % (k, allow[k]))
for k, d, a, b in diffs:
    print('  [未解释] %-18s %s | 旧版=%s | Rust=%s' % (k, d, a, b))
json.dump({'diffs': diffs, 'allowed': allowed}, open(os.path.join(out, 'compare.json'), 'w', encoding='utf-8'), ensure_ascii=False, indent=1)
sys.exit(1 if diffs else 0)
