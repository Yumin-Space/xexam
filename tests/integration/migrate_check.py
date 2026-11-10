# -*- coding: utf-8 -*-
"""旧 progress.json 的迁移：文档里写的流程（先启动一次建好数据目录 → 再把 progress.json 放进去 → 重启）必须真的有效，
并且只迁一次、不覆盖已有的学习记录。只用合成数据，全程在临时目录里。

用法: python -I tests/integration/migrate_check.py <Rust 后端可执行文件>
步骤：
  A. 全新数据目录启动一次（建出 xexam.db），此时没有 progress.json。学习记录是空的。
  B. 停掉后端，放进一个合成的 progress.json，重启 → 学习记录里有了迁移来的内容（评审发现的缺陷：以前这一步什么也不做）。
  C. 再重启一次 → 不会重复迁移（成绩历史条数不变）。
  D. 在数据库里已经有学习记录的目录里放 progress.json → 不覆盖。
  E. 格式不对的 progress.json → 程序照样启动，日志里写明原因和 migrate-progress 的提示；修好后重启即可迁移。
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

exe = os.path.abspath(sys.argv[1])
results = []


def ok(cond, msg):
    results.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(results))
        sys.exit(1)


def free_port():
    with socket.socket() as sk:
        sk.bind(('127.0.0.1', 0))
        return sk.getsockname()[1]


PORT = free_port()
QID = 'a1b2c3d4e5f6'
SAMPLE = {
    'version': 1,
    'questions': {QID: {'attempts': 3, 'correct': 1, 'wrong': 2, 'streak': 0, 'last': False, 'at': '2026-01-02 03:04'}},
    'history': [{'at': '2026-01-02 03:04', 'sheets': ['demo'], 'mode': 'all', 'total': 3, 'correct': 1, 'percent': 33, 'seconds': 50}],
}


def run_once(data, log_name, seconds_up=1.0):
    """启动后端，等它起来，读 /api/progress，再停掉。返回 (progress, 日志文本)。"""
    env = {k: v for k, v in os.environ.items() if not k.startswith('XEXAM_')}
    env.update(XEXAM_DATA=data, XEXAM_HOST='127.0.0.1', XEXAM_PORT=str(PORT), XEXAM_VERSION='0.3')
    logp = os.path.join(data, '..', log_name)
    with open(logp, 'wb') as log:
        p = subprocess.Popen([exe, 'serve', '--no-browser'], env=env, cwd=data, stdout=log, stderr=subprocess.STDOUT)
        try:
            prog = None
            for _ in range(120):
                if p.poll() is not None:
                    break
                try:
                    with urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:%d/api/progress' % PORT, headers={'Host': 'localhost:%d' % PORT}), timeout=1) as r:
                        prog = json.loads(r.read().decode('utf-8'))
                        break
                except Exception:
                    time.sleep(0.2)
        finally:
            p.terminate()
            try:
                p.wait(timeout=10)
            except Exception:
                p.kill()
            time.sleep(0.3)
    return prog, open(logp, encoding='utf-8', errors='replace').read()


root = tempfile.mkdtemp(prefix='xexam_migrate_')
try:
    # A
    d = os.path.join(root, 'a')
    os.makedirs(d)
    prog, _ = run_once(d, 'a1.log')
    ok(prog is not None and prog['history'] == [] and prog['questions'] == {}, 'A. 全新目录启动：学习记录是空的')
    ok(os.path.exists(os.path.join(d, 'xexam.db')), 'A. 已经建出 xexam.db')

    # B
    with open(os.path.join(d, 'progress.json'), 'w', encoding='utf-8') as f:
        json.dump(SAMPLE, f, ensure_ascii=False)
    prog, log = run_once(d, 'a2.log')
    ok(prog is not None and len(prog['history']) == 1 and QID in prog['questions'], 'B. 数据库已存在、之后才放进 progress.json，重启后被迁移（1 场成绩、1 道题的统计）')
    ok('已把旧的学习记录迁移进数据库' in log, 'B. 日志里写了「已把旧的学习记录迁移进数据库」')

    # C
    prog, log = run_once(d, 'a3.log')
    ok(prog is not None and len(prog['history']) == 1, 'C. 再重启一次：不会重复迁移（仍是 1 场成绩）')
    ok('已把旧的学习记录迁移进数据库' not in log and '没有迁移' not in log, 'C. 日志里没有多余的迁移提示')

    # D：库里已经有学习记录（上面迁移来的），换一份不同的 progress.json —— 不覆盖
    other = dict(SAMPLE, history=[dict(SAMPLE['history'][0], percent=99)] * 2)
    shutil.copy(os.path.join(d, 'xexam.db'), os.path.join(root, 'keep.db'))
    with open(os.path.join(d, 'progress.json'), 'w', encoding='utf-8') as f:
        json.dump(other, f, ensure_ascii=False)
    prog, _ = run_once(d, 'a4.log')
    ok(prog is not None and len(prog['history']) == 1 and prog['history'][0]['percent'] == 33, 'D. 已经迁移过的库：不被新的 progress.json 覆盖')

    # E
    e = os.path.join(root, 'e')
    os.makedirs(e)
    run_once(e, 'e1.log')
    with open(os.path.join(e, 'progress.json'), 'w', encoding='utf-8') as f:
        f.write('{ this is not json')
    prog, log = run_once(e, 'e2.log')
    ok(prog is not None and prog['history'] == [], 'E. 格式不对的 progress.json：程序照样启动，学习记录仍为空')
    ok('migrate-progress' in log, 'E. 日志里提示了 xexam-backend migrate-progress')
    with open(os.path.join(e, 'progress.json'), 'w', encoding='utf-8') as f:
        json.dump(SAMPLE, f, ensure_ascii=False)
    prog, _ = run_once(e, 'e3.log')
    ok(prog is not None and len(prog['history']) == 1, 'E. 修好 progress.json 后重启即可迁移')
finally:
    shutil.rmtree(root, ignore_errors=True)

print('\n'.join(results))
print('\n全部 %d 项通过' % len(results))
