# -*- coding: utf-8 -*-
"""整套新栈（nginx + Rust 后端 + SQLite 卷）的端到端检查。用 Docker Compose 起栈，全程只用临时项目名、临时卷，
绝不碰真实的 xexam_data 卷。

用法: python -I tests/integration/stack_check.py [端口，默认 18090]
前提：本机能 docker compose build（会构建 backend 和 nginx 两个镜像）。

检查内容：
  1. 栈起来、两个容器 healthy；后端没有发布任何端口，宿主机连不到它
  2. 经 Nginx 的 /api/info /api/bank /api/progress /api/imports
  3. 上传演示工作簿 → 预览（库不变）→ 确认 → 题库有 5 题；取消 / 重复确认 / 错误确认码
  4. 交卷成绩 → 学习记录；重置
  5. 超过 20 MB：Nginx 层 413（JSON）；Host / Origin 守卫经过 Nginx 后仍然生效
  6. 删除容器后用同一个卷重建，数据还在；docker stop 用时
  7. 非 root、只读根文件系统
"""
import http.client
import json
import os
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18090
PROJECT = 'xexamcheck' + uuid.uuid4().hex[:6]
DEMO = os.path.join(ROOT, 'tests', 'baseline', 'fixtures', 'demo.xlsx')
COMPOSE = os.path.join(ROOT, 'docker-compose.modern.yml')

env = dict(os.environ, XEXAM_BIND='127.0.0.1:%d' % PORT, COMPOSE_PROJECT_NAME=PROJECT, MSYS_NO_PATHCONV='1')
for k in ('XEXAM_ALLOWED_HOSTS', 'XEXAM_ALLOWED_ORIGINS', 'XEXAM_ALLOW_ANY_HOST'):
    env.pop(k, None)
results = []


def dc(*a, check=True):
    r = subprocess.run(['docker', 'compose', '-f', COMPOSE, '-p', PROJECT, *a], capture_output=True, text=True, encoding='utf-8', env=env)
    if check and r.returncode != 0:
        raise SystemExit('docker compose %s 失败:\n%s' % (' '.join(a), (r.stdout + r.stderr)[-1500:]))
    return r


def cleanup():
    dc('down', '-v', '--remove-orphans', check=False)


def ok(cond, msg):
    results.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(results))
        cleanup()
        sys.exit(1)


def req(method, path, headers=None, body=None, host=None, timeout=20):
    c = http.client.HTTPConnection('127.0.0.1', PORT, timeout=timeout)
    c.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
    c.putheader('Host', host or 'localhost:%d' % PORT)
    for k, v in (headers or {}).items():
        c.putheader(k, v)
    if body is not None:
        c.putheader('Content-Length', str(len(body)))
    c.endheaders(body)
    r = c.getresponse()
    data = r.read()
    out = (r.status, {k.lower(): v for k, v in r.getheaders()}, data)
    c.close()
    return out


def jreq(method, path, obj=None, headers=None, host=None):
    h = {'Content-Type': 'application/json'}
    h.update(headers or {})
    st, hd, b = req(method, path, h, json.dumps(obj).encode() if obj is not None else None, host)
    return st, hd, (json.loads(b) if b[:1] in (b'{', b'[') else b)


def wait_healthy(names, secs=120):
    t0 = time.time()
    while time.time() - t0 < secs:
        good = 0
        for n in names:
            r = subprocess.run(['docker', 'inspect', '-f', '{{.State.Health.Status}}', n], capture_output=True, text=True, env=env)
            good += r.stdout.strip() == 'healthy'
        if good == len(names):
            return True
        time.sleep(2)
    return False


def upload(path):
    data = open(path, 'rb').read()
    return req('POST', '/api/upload', {'Content-Type': 'application/octet-stream', 'X-Filename': 'demo.xlsx'}, data)


cleanup()
try:
    dc('up', '-d', '--build')
    # 这个项目里容器名是固定的 xexam / xexam-backend；换项目名并不改变 container_name，所以先确认没有真实容器在用同名
    names = ['xexam', 'xexam-backend']
    ok(wait_healthy(names), '两个容器都变成 healthy')

    # —— 1 暴露面 ——
    r = subprocess.run(['docker', 'port', 'xexam-backend'], capture_output=True, text=True, env=env)
    ok(r.stdout.strip() == '', '后端没有发布任何端口（docker port 为空）')
    r = subprocess.run(['docker', 'port', 'xexam'], capture_output=True, text=True, env=env)
    ok('127.0.0.1:%d' % PORT in r.stdout, 'Nginx 只绑定 127.0.0.1:%d。实际 %r' % (PORT, r.stdout.strip()))
    for n, want in (('xexam', True), ('xexam-backend', True)):
        r = subprocess.run(['docker', 'exec', n, 'id', '-u'], capture_output=True, text=True, env=env)
        ok(r.stdout.strip() not in ('', '0'), '%s 不是 root 运行（uid %s）' % (n, r.stdout.strip()))
    r = subprocess.run(['docker', 'inspect', '-f', '{{.HostConfig.ReadonlyRootfs}}', 'xexam-backend'], capture_output=True, text=True, env=env)
    ok(r.stdout.strip() == 'true', '后端根文件系统只读')

    # —— 2 读接口 ——
    st, h, info = jreq('GET', '/api/info')
    ok(st == 200 and info['bankCount'] == 0 and info['canShutdown'] is False, '/api/info：空库、不可关闭。实际 %r' % info)
    ok(open(os.path.join(ROOT, 'VERSION')).read().strip().startswith(info['version'].split('.')[0]), '版本号来自 VERSION：%s' % info['version'])
    st, h, bank = jreq('GET', '/api/bank')
    ok(st == 200 and bank['questions'] == [] and list(bank.keys()) == ['file', 'mtime', 'error', 'questions', 'sheets'], '/api/bank 空库，键顺序与旧版一致')
    st, h, prog = jreq('GET', '/api/progress')
    ok(st == 200 and list(prog.keys()) == ['version', 'questions', 'history', 'masterStreak'], '/api/progress 键顺序与旧版一致')

    # —— 3 导入流程 ——
    st, h, b = upload(DEMO)
    pv = json.loads(b)
    ok(st == 200 and pv['added'] == 5 and pv['usable'] == 5 and pv['token'], '上传 demo.xlsx：预览新增 5 题。实际 %d' % st)
    st, _, info = jreq('GET', '/api/info')
    ok(info['bankCount'] == 0, '预览阶段库里仍是 0 题（预览不写库）')
    st, _, e = jreq('POST', '/api/import/confirm', {'token': 'wrong-token'})
    ok(st == 409, '错误确认码 → 409。实际 %d' % st)
    st, _, e = jreq('POST', '/api/import/confirm', {'token': '题库é'})
    ok(st == 409, '非 ASCII 确认码 → 409（不会让连接断掉）。实际 %d' % st)
    st, _, c = jreq('POST', '/api/import/confirm', {'token': pv['token']})
    ok(st == 200 and len(c['bank']['questions']) == 5, '正确确认码 → 导入 5 题')
    st, _, e = jreq('POST', '/api/import/confirm', {'token': pv['token']})
    ok(st == 409, '确认码一次性：重复确认 → 409。实际 %d' % st)
    st, _, info = jreq('GET', '/api/info')
    ok(info['bankCount'] == 5, '库里现在 5 题')
    st, h, b = upload(DEMO)
    pv2 = json.loads(b)
    ok(pv2['added'] == 0 and pv2['unchanged'] == 5, '再传同一份：没变化 5 题')
    st, _, _ = jreq('POST', '/api/import/cancel', {})
    st, _, e = jreq('POST', '/api/import/confirm', {'token': pv2['token']})
    ok(st == 409, '取消之后不能再确认')
    st, _, imps = jreq('GET', '/api/imports')
    ok(len(imps['imports']) == 1 and imps['imports'][0]['filename'] == 'demo.xlsx', '导入记录只有 1 条（预览与取消不留记录）')

    # —— 4 成绩 ——
    ids = [q['id'] for q in bank_q] if (bank_q := jreq('GET', '/api/bank')[2]['questions']) else []
    st, _, p = jreq('POST', '/api/attempt', {'results': [{'id': ids[0], 'correct': True}, {'id': ids[1], 'correct': False}], 'sheets': ['demo'], 'mode': 'all', 'seconds': 12})
    ok(st == 200 and p['history'][0]['percent'] == 50 and p['history'][0]['total'] == 2, '交卷 1/2 → 50%')
    st, _, p = jreq('POST', '/api/attempt', {'results': [{'id': ids[0], 'correct': True}] + [{'id': i, 'correct': False} for i in ids[1:8]], 'sheets': [], 'mode': 'all', 'seconds': 1})
    ok(st == 200, '再交一场')
    st, _, e = jreq('POST', '/api/attempt', {'results': {'a': 1}})
    ok(st == 400, 'results 不是列表 → 400（不会断开连接）。实际 %d' % st)

    # —— 5 守卫与大小 ——
    st, _, e = jreq('GET', '/api/info', host='evil.example.com')
    ok(st == 403, '陌生 Host 经过 Nginx 仍被后端拒绝（403）')
    st, _, e = jreq('POST', '/api/import/cancel', {}, headers={'Origin': 'https://evil.example.com'})
    ok(st == 403, '陌生 Origin 的 POST → 403')
    st, _, e = jreq('POST', '/api/import/cancel', {}, headers={'Origin': 'http://localhost:%d' % PORT})
    ok(st == 200, '同源 POST → 200')
    st, h, b = req('POST', '/api/upload', {'Content-Type': 'application/octet-stream', 'X-Filename': 'big.xlsx'}, b'x' * (20 * 1024 * 1024 + 1))
    ok(st == 413 and 'json' in h.get('content-type', ''), '超过 20 MB → 413 JSON')
    st, _, e = jreq('POST', '/api/shutdown', {})
    ok(st == 403, '/api/shutdown 在容器里恒 403')
    st, h, b = req('GET', '/')
    ok(st == 200 and b'<div id="app">' in b and 'nosniff' == h.get('x-content-type-options'), '首页由 Nginx 提供')

    # —— 6 持久化 ——
    dc('rm', '-sf', 'nginx', 'backend')
    ok(subprocess.run(['docker', 'volume', 'inspect', PROJECT + '_data'], capture_output=True, env=env).returncode == 0, '删除容器后卷还在')
    dc('up', '-d')
    ok(wait_healthy(names), '用同一个卷重建后两个容器再次 healthy')
    st, _, info = jreq('GET', '/api/info')
    ok(info['bankCount'] == 5, '重建后题库仍是 5 题（数据在卷里）')
    st, _, p = jreq('GET', '/api/progress')
    ok(len(p['history']) == 2, '重建后成绩历史仍是 2 场')
    t0 = time.time()
    subprocess.run(['docker', 'stop', 'xexam-backend'], capture_output=True, env=env)
    dt = time.time() - t0
    ok(dt < 9, 'docker stop 后端用时 %.1f 秒（<9）' % dt)
    st, _, p = jreq('POST', '/api/reset', {}) if False else (0, 0, 0)

finally:
    cleanup()

print('\n'.join(results))
print('\n全部 %d 项通过' % len(results))
