# -*- coding: utf-8 -*-
"""验证这次审计修复的行为（真实启动 server.py，端口 8960-8969，临时数据目录；不碰用户数据）。"""
import http.client
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import paths  # noqa: E402
ROOT = paths.LEGACY_ROOT
sys.path.insert(0, ROOT)
out = []


def ok(cond, msg):
    out.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(out))
        sys.exit(1)


def start(port, env_extra=None, data=None, cwd=None):
    d = data or tempfile.mkdtemp()
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', XEXAM_DATA=d, XEXAM_HOST='127.0.0.1', XEXAM_PORT=str(port))
    for k in ('XEXAM_ALLOWED_HOSTS', 'XEXAM_ALLOWED_ORIGINS', 'XEXAM_ALLOW_ANY_HOST'):
        env.pop(k, None)
    env.update(env_extra or {})
    log = open(os.path.join(d, 'log.txt'), 'wb')
    p = subprocess.Popen([sys.executable, '-I', os.path.join(ROOT, 'server.py'), '--no-browser'], cwd=cwd or tempfile.gettempdir(), env=env, stdout=log, stderr=subprocess.STDOUT)
    for _ in range(60):
        try:
            c = http.client.HTTPConnection('127.0.0.1', port, timeout=2)
            c.request('GET', '/api/info')
            c.getresponse().read()
            c.close()
            return p, d
        except Exception:
            time.sleep(0.2)
    p.terminate()
    raise SystemExit('server did not start')


def req(port, method, path, body=None, headers=None):
    c = http.client.HTTPConnection('127.0.0.1', port, timeout=8)
    c.request(method, path, body=body, headers=headers or {})
    r = c.getresponse()
    data = r.read()
    c.close()
    return r.status, data


def log_of(d):
    time.sleep(0.3)
    return open(os.path.join(d, 'log.txt'), 'rb').read().decode('utf-8', 'replace')


procs = []
try:
    # ---- 1. 大请求体被提前 403 / 415 时，客户端要能读到原因，而不是连接被重置
    p, d = start(8961)
    procs.append(p)
    big = b'x' * 2_000_000
    st, body = req(8961, 'POST', '/api/upload', big, {'Content-Type': 'application/octet-stream', 'Origin': 'https://evil.example.com', 'X-Filename': 'a.xlsx'})
    ok(st == 403 and b'error' in body, '2 MB 上传带陌生 Origin：收到 403 和原因（以前是连接被重置）。实际 %s' % st)
    st, body = req(8961, 'POST', '/api/upload', big, {'Content-Type': 'text/plain', 'X-Filename': 'a.xlsx'})
    ok(st == 415, '2 MB 上传但 Content-Type 不对：收到 415。实际 %s' % st)
    st, body = req(8961, 'POST', '/api/attempt', big, {'Content-Type': 'text/plain;x=application/json'})
    ok(st == 415, 'Content-Type 里夹带 application/json 的 text/plain：不再被当成 JSON（415）。实际 %s' % st)
    ok('拒绝了一个请求' in log_of(d), '被拒绝的请求在日志里留了一行')
    st, body = req(8961, 'GET', '/api/info', headers={'Host': 'evil.example.com'})
    ok(st == 403 and '拒绝了一个请求：Host 是「evil.example.com」' in log_of(d), '陌生 Host 的 GET：403，且日志里写明了 Host')

    # ---- 2. 畸形 JSON 不再让连接断掉
    H = {'Content-Type': 'application/json'}
    cases = [
        ('results 是字典', b'{"results": {"a": 1}}'),
        ('seconds 是 Infinity', b'{"results":[{"id":"a","correct":true}],"seconds": Infinity}'),
        ('sheets 是数字', b'{"results":[{"id":"a","correct":true}],"sheets": 5}'),
        ('嵌套 10 万层', b'[' * 100000 + b']' * 100000),
        ('确认码含非 ASCII', json.dumps({'token': '题库é'}).encode('utf-8')),
    ]
    for name, raw in cases:
        path = '/api/import/confirm' if '确认码' in name else '/api/attempt'
        try:
            st, body = req(8961, 'POST', path, raw, H)
        except Exception as e:
            ok(False, '%s：连接被断开 %r' % (name, e))
        ok(st in (200, 400, 409), '%s：有正常的 HTTP 回复（%s）' % (name, st))
    st, body = req(8961, 'POST', '/api/attempt', b'{"results":[{"id":"a","correct":true}],"seconds": Infinity}', H)
    ok(st == 200, 'seconds=Infinity 当 0 秒处理，成绩照常保存。实际 %s' % st)
    st, body = req(8961, 'POST', '/api/attempt', b'{"results":[{"id":"a","correct":true}],"sheets":"abc"}', H)
    ok(st == 200 and json.loads(body)['history'][0]['sheets'] == [], 'sheets 是字符串：不再被拆成字符。')

    # ---- 3. XEXAM_ALLOWED_HOSTS 用空格 / 分号分隔也能识别
    import server as S
    ok(S.parse_allowed_hosts('a.example.com b.example.com;c.example.com,d.example.com') == {'a.example.com', 'b.example.com', 'c.example.com', 'd.example.com'},
       'XEXAM_ALLOWED_HOSTS：逗号、空格、分号分隔都能识别')

    # ---- 4. 数据目录里只有别的 .db 文件：启动时提醒，不改动那个文件
    d2 = tempfile.mkdtemp()
    old = os.path.join(d2, 'legacy_named.db')
    open(old, 'wb').write(b'not really a database, only a stand-in file')
    before = open(old, 'rb').read()
    p2, _ = start(8962, data=d2)
    procs.append(p2)
    lg = log_of(d2)
    ok('legacy_named.db' in lg and '改名为 xexam.db' in lg, '数据目录里有别的 .db：启动日志里提醒了')
    ok(open(old, 'rb').read() == before, '那个文件原封不动')
    ok(os.path.exists(os.path.join(d2, 'xexam.db')), '新建了空的 xexam.db')

    # ---- 5. 旧 progress.json 格式不对：程序照样能启动
    import app.db as D
    st_dir = tempfile.mkdtemp()
    s = D.Store(os.path.join(st_dir, 'xexam.db'))
    for i, content in enumerate(['[]', 'null', '{"questions": 5, "history": []}', '{"questions": {}, "history": ["x"]}',
                                 '{"questions": {}, "history": [{"at": "t"}]}', '\ufeff{"questions": {}, "history": []}']):
        pth = os.path.join(st_dir, 'p%d.json' % i)
        open(pth, 'w', encoding='utf-8', newline='').write(content)
        s2 = D.Store(os.path.join(tempfile.mkdtemp(), 'x.db'))
        try:
            r = s2.migrate_legacy_progress(pth)
        except Exception as e:
            ok(False, '旧 progress.json 样例 %d 让程序崩了：%r' % (i, e))
        ok(isinstance(r, tuple), '旧 progress.json 样例 %d：返回（%s）而不是崩溃' % (i, r[1][:30]))
        s2.close()
    s.close()

    # ---- 6. COMMIT 失败要回滚，连接不能卡死
    import sqlite3
    wd = tempfile.mkdtemp()
    s3 = D.Store(os.path.join(wd, 'xexam.db'))
    s3.conn.execute('PRAGMA busy_timeout=300')
    blocker = sqlite3.connect(os.path.join(wd, 'xexam.db'), isolation_level=None)
    blocker.execute('BEGIN')
    blocker.execute('SELECT * FROM questions').fetchall()
    try:
        s3.record_attempt({'results': [{'id': 'q', 'correct': True}], 'sheets': [], 'mode': 'all', 'seconds': 1})
        failed = False
    except sqlite3.OperationalError:
        failed = True
    ok(failed, '别的连接占着读锁：写入失败（预期）')
    ok(not s3.conn.in_transaction, '失败之后连接不在事务里（以前会一直卡住）')
    blocker.execute('ROLLBACK')
    blocker.close()
    s3.record_attempt({'results': [{'id': 'q', 'correct': True}], 'sheets': [], 'mode': 'all', 'seconds': 1})
    ok(s3.progress()['history'][0]['total'] == 1, '锁释放后，同一个连接可以继续写入')
    s3.close()

    # ---- 7. 百分比取整：和 JS 的 roundHalfEven 对齐（Python round 的结果）
    import math
    def half_even(x):
        f = math.floor(x); d_ = x - f
        if abs(d_ - 0.5) < 1e-9:
            return f if f % 2 == 0 else f + 1
        return round(x)
    bad = [(g, t) for t in range(1, 201) for g in range(0, t + 1) if half_even(100 * g / t) != round(100 * g / t)]
    ok(not bad, 'JS 版取整规则与 Python round() 在 1..200 题所有组合上一致（不一致 %d 处）' % len(bad))
finally:
    for p in procs:
        p.terminate()
        try:
            p.wait(timeout=5)
        except Exception:
            p.kill()

print('\n'.join(out))
print('\n全部 %d 项通过' % len(out))
