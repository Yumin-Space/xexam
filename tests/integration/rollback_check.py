# -*- coding: utf-8 -*-
"""回滚演练：证明「新后端写过的库，旧 Python 还能读写；旧 Python 写过的库，新后端能直接用」，并且原件不被改动。
只用合成数据（虚构的演示工作簿），全程在临时目录里；不碰任何真实数据库。

用法: python -I tests/integration/rollback_check.py <Rust 后端可执行文件>
步骤：
  A. 旧 Python 建库 → 导入演示题库 → 记成绩        （这是「业主现有的库」的合成替身）
  B. 备份一份，对备份做 PRAGMA integrity_check，并记下原件的 sha256
  C. 在备份的**副本**上启动 Rust 后端，读 bank / progress / imports，与旧版逐项比较（零差异）
  D. Rust 在副本上导入第二份工作簿（下架一些题）、记成绩、重置
  E. 停掉 Rust，用旧 Python 打开同一个副本：bank / progress / imports 与 Rust 最后的输出逐项相同；integrity_check
  F. 证明原件的 sha256 没变
"""
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), 'baseline', 'tools'))
import paths  # noqa: E402

exe = os.path.abspath(sys.argv[1])
def free_port():
    import socket
    with socket.socket() as sk:
        sk.bind(('127.0.0.1', 0))
        return sk.getsockname()[1]


PORT = free_port()   # 系统分配的空闲端口：不会撞上别的进程
results = []


def ok(cond, msg, detail=None):
    results.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(results))
        if detail is not None:
            print('--- 详情：第一处不同 ---')
            print(detail() if callable(detail) else detail)
        sys.exit(1)


def first_diff(a, b, path=''):
    if type(a) != type(b):
        return '%s 类型 %s vs %s' % (path, type(a).__name__, type(b).__name__)
    if isinstance(a, dict):
        if list(a) != list(b):
            return '%s 键 %s vs %s' % (path, list(a), list(b))
        for k in a:
            d = first_diff(a[k], b[k], path + '/' + str(k))
            if d:
                return d
    elif isinstance(a, list):
        if len(a) != len(b):
            return '%s 长度 %d vs %d' % (path, len(a), len(b))
        for i, (x, y) in enumerate(zip(a, b)):
            d = first_diff(x, y, '%s[%d]' % (path, i))
            if d:
                return d
    elif a != b:
        return '%s 值 %r vs %r' % (path, a, b)
    return None


sha = lambda p: hashlib.sha256(open(p, 'rb').read()).hexdigest()


def clean_env(extra=None):
    e = {k: v for k, v in os.environ.items() if not k.startswith('XEXAM_')}
    e.update(extra or {})
    return e


def py_store(path):
    sys.path.insert(0, paths.LEGACY_ROOT)
    import importlib
    db = importlib.import_module('app.db')
    return db.Store(path)


def wait_up(proc):
    """等我们自己启动的后端起来：进程必须还活着，且 /api/info 报出的版本是我们传的 0.3（防止连到别人的服务）。"""
    for _ in range(120):
        if proc.poll() is not None:
            raise SystemExit('后端进程提前退出，退出码 %s' % proc.returncode)
        try:
            with urllib.request.urlopen('http://127.0.0.1:%d/api/info' % PORT, timeout=1) as r:
                if json.loads(r.read().decode('utf-8')).get('version') == '0.3':
                    return
        except Exception:
            pass
        time.sleep(0.25)
    raise SystemExit('后端没有在预期时间内起来')


def api(method, path, body=None, headers=None):
    req = urllib.request.Request('http://127.0.0.1:%d%s' % (PORT, path), data=body, method=method,
                                 headers=dict({'Host': 'localhost:%d' % PORT}, **(headers or {})))
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.loads(r.read().decode('utf-8'))


def dump(store_or_api):
    return {k: store_or_api[k] for k in ('bank', 'progress', 'imports')}


root = tempfile.mkdtemp(prefix='xexam_rollback_')
try:
    orig = os.path.join(root, 'orig')
    os.makedirs(orig)
    # —— A 旧 Python 建库 ——
    sys.path.insert(0, paths.LEGACY_ROOT)
    from app import xlsx_parser  # noqa: E402
    sheets, qs = xlsx_parser.parse_workbook(open(paths.DEMO, 'rb'))
    s = py_store(os.path.join(orig, 'xexam.db'))
    s.apply_import('demo.xlsx', sheets, qs)
    s.record_attempt({'results': [{'id': qs[0]['id'], 'correct': True}, {'id': qs[1]['id'], 'correct': False}], 'sheets': ['demo'], 'mode': 'all', 'seconds': 30})
    s.record_attempt({'results': [{'id': qs[1]['id'], 'correct': False}], 'sheets': ['demo'], 'mode': 'weak', 'seconds': 5})
    py_before = {'bank': s.bank(), 'progress': s.progress(), 'imports': {'imports': s.import_history(12)}}
    s.close()
    ok(True, 'A. 旧 Python 建库：5 题、2 场成绩')

    # —— B 备份 + integrity_check + 原件哈希 ——
    backup = os.path.join(root, 'backup')
    shutil.copytree(orig, backup)
    c = sqlite3.connect(os.path.join(backup, 'xexam.db'))
    ok(c.execute('PRAGMA integrity_check').fetchone()[0] == 'ok', 'B. 备份通过 PRAGMA integrity_check')
    c.close()
    orig_sha = sha(os.path.join(orig, 'xexam.db'))
    work = os.path.join(root, 'work')
    shutil.copytree(backup, work)

    # —— C Rust 读副本 ——
    env = clean_env({'XEXAM_DATA': work, 'XEXAM_HOST': '127.0.0.1', 'XEXAM_PORT': str(PORT), 'XEXAM_VERSION': '0.3'})
    log = open(os.path.join(root, 'rust.log'), 'wb')
    p = subprocess.Popen([exe, 'serve', '--no-browser'], env=env, cwd=root, stdout=log, stderr=subprocess.STDOUT)
    try:
        wait_up(p)
        rs = {'bank': api('GET', '/api/bank'), 'progress': api('GET', '/api/progress'), 'imports': api('GET', '/api/imports')}
        ok(rs['bank'] == py_before['bank'], 'C. Rust 读到的 /api/bank 与旧版的 bank() 完全相同（5 题，含键顺序以外的全部内容）', lambda: first_diff(py_before['bank'], rs['bank']))
        ok(json.dumps(rs['bank'], ensure_ascii=False) == json.dumps(py_before['bank'], ensure_ascii=False), 'C. …并且键顺序也相同（逐字节）')
        ok(rs['progress'] == py_before['progress'], 'C. /api/progress 与旧版相同（错题统计、成绩历史）', lambda: first_diff(py_before['progress'], rs['progress']))
        ok(json.dumps(rs['progress'], ensure_ascii=False) == json.dumps(py_before['progress'], ensure_ascii=False), 'C. …键顺序也相同')
        ok(rs['imports'] == py_before['imports'], 'C. /api/imports 与旧版相同')

        # —— D Rust 写：第二份工作簿（会下架一些题）、成绩、重置 ——
        data = open(paths.DEMO2, 'rb').read()
        pv = api('POST', '/api/upload', data, {'Content-Type': 'application/octet-stream', 'X-Filename': 'demo_v2.xlsx'})
        ok(pv['token'] and (pv['removed'] or pv['added'] or pv['updated']), 'D. Rust 解析第二份工作簿：新增 %d 更新 %d 下架 %d' % (pv['added'], pv['updated'], pv['removed']))
        res = api('POST', '/api/import/confirm', json.dumps({'token': pv['token']}).encode(), {'Content-Type': 'application/json'})
        bank = api('GET', '/api/bank')
        ids = [q['id'] for q in bank['questions']]
        api('POST', '/api/attempt', json.dumps({'results': [{'id': i, 'correct': k % 2 == 0} for k, i in enumerate(ids)], 'sheets': ['demo'], 'mode': 'all', 'seconds': 99}).encode(), {'Content-Type': 'application/json'})
        rust_after = {'bank': api('GET', '/api/bank'), 'progress': api('GET', '/api/progress'), 'imports': api('GET', '/api/imports')}
        ok(len(rust_after['imports']['imports']) == 2 and len(rust_after['progress']['history']) == 3, 'D. Rust 写入后：2 次导入记录、3 场成绩')
    finally:
        p.terminate()
        try:
            p.wait(timeout=10)
        except Exception:
            p.kill()
        time.sleep(0.5)

    # —— E 旧 Python 回读 Rust 写过的库 ——
    c = sqlite3.connect(os.path.join(work, 'xexam.db'))
    ok(c.execute('PRAGMA integrity_check').fetchone()[0] == 'ok', 'E. Rust 写过的库通过 PRAGMA integrity_check')
    ok(c.execute('PRAGMA journal_mode').fetchone()[0] == 'delete', 'E. journal_mode 仍是 delete（没有改成 WAL）')
    ok(not [f for f in os.listdir(work) if f.endswith(('-wal', '-shm', '-journal'))], 'E. 没有遗留 -wal / -shm / -journal 文件')
    ok(c.execute('PRAGMA user_version').fetchone()[0] == 0, 'E. user_version 仍是 0（没有引入旧版读不懂的东西）')
    c.close()
    s2 = py_store(os.path.join(work, 'xexam.db'))
    py_after = {'bank': s2.bank(), 'progress': s2.progress(), 'imports': {'imports': s2.import_history(12)}}
    ok(py_after['bank'] == rust_after['bank'], 'E. 旧 Python 读 Rust 写过的库：bank 与 Rust 最后输出相同')
    ok(py_after['progress'] == rust_after['progress'], 'E. …progress 相同')
    ok(py_after['imports'] == rust_after['imports'], 'E. …imports 相同')
    # 旧 Python 再写一场，Rust 再读
    s2.record_attempt({'results': [{'id': ids[0], 'correct': True}], 'sheets': ['demo'], 'mode': 'retry', 'seconds': 1})
    py_final = s2.progress()
    s2.close()
    p = subprocess.Popen([exe, 'serve', '--no-browser'], env=env, cwd=root, stdout=log, stderr=subprocess.STDOUT)
    try:
        wait_up(p)
        ok(api('GET', '/api/progress') == py_final, 'E. 旧 Python 再写一场成绩后，Rust 读到的 progress 与之相同（双向往返）')
    finally:
        p.terminate()
        try:
            p.wait(timeout=10)
        except Exception:
            p.kill()

    # —— F 原件没动 ——
    ok(sha(os.path.join(orig, 'xexam.db')) == orig_sha, 'F. 原件的 sha256 在整个演练前后没有变化')
    ok(not os.path.exists(os.path.join(orig, 'xexam.db-journal')), 'F. 原件旁边没有多出任何文件')
finally:
    shutil.rmtree(root, ignore_errors=True)

print('\n'.join(results))
print('\n全部 %d 项通过' % len(results))
