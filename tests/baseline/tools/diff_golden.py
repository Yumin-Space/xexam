# -*- coding: utf-8 -*-
"""比较两组黄金响应（capture.py 的输出目录）。只比稳定字段：status、稳定响应头、normalized（易变值已替换）、静态文件的 body_md5。
用法: python -I diff_golden.py <目录A> <目录B> [--allow allow.json]
  allow.json: {"<序号前缀 如 087>": "为什么这个差异是有意的/可以接受的"}  —— 白名单里的差异不算失败，但会列出来。
退出码：0 = 除白名单外没有差异；1 = 有未解释的差异。"""
import json
import os
import sys

a_dir, b_dir = sys.argv[1], sys.argv[2]
allow = {}
if '--allow' in sys.argv:
    allow = json.load(open(sys.argv[sys.argv.index('--allow') + 1], encoding='utf-8'))


def load(d):
    out = {}
    for f in sorted(os.listdir(d)):
        if f[:3].isdigit() and f.endswith('.json'):
            out[f[:3]] = json.load(open(os.path.join(d, f), encoding='utf-8'))
    return out


A, B = load(a_dir), load(b_dir)
SKIP_HEADERS = {'date', 'server'}


def view(c):
    h = {k.lower(): v for k, v in (c.get('headers_stable') or {}).items() if k.lower() not in SKIP_HEADERS}
    # 只有静态文件（没有 normalized）才比 body_md5：JSON 响应体里有时间、令牌，已经在 normalized 里替换过，哈希本来就会变
    md5 = c.get('body_md5') if c.get('normalized') is None else None
    if 'content-length' in h and c.get('normalized') is not None:
        h = {k: v for k, v in h.items() if k != 'content-length'}      # 长度随易变值变化，由 normalized 负责
    return {'status': c.get('status'), 'headers': h, 'normalized': c.get('normalized'), 'body_md5': md5, 'no_response': c.get('no_response')}


diffs, allowed = [], []
for k in sorted(set(A) | set(B)):
    if k not in A or k not in B:
        diffs.append((k, '只有一边有这一项', None, None))
        continue
    va, vb = view(A[k]), view(B[k])
    if va != vb:
        name = A[k].get('name') or B[k].get('name') or ''
        what = [f for f in va if va[f] != vb[f]]
        item = (k, name, {f: va[f] for f in what}, {f: vb[f] for f in what})
        (allowed if k in allow else diffs).append(item)

print('比较 %d 项：相同 %d，白名单内的差异 %d，未解释的差异 %d' % (len(set(A) | set(B)), len(set(A) | set(B)) - len(diffs) - len(allowed), len(allowed), len(diffs)))
for k, name, x, y in allowed:
    print('  [允许] %s %s —— %s' % (k, name, allow[k]))
for k, name, x, y in diffs:
    print('  [差异] %s %s' % (k, name))
    print('     A:', json.dumps(x, ensure_ascii=False)[:400])
    print('     B:', json.dumps(y, ensure_ascii=False)[:400])
sys.exit(1 if diffs else 0)
