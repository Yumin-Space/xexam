# -*- coding: utf-8 -*-
"""起两个全新的 Rust 后端实例（一个普通、一个带 --allow-shutdown），用 capture.py 抓一遍，再和旧版的黄金响应比较。

用法: python -I run_rust_capture.py <后端可执行文件> [--out 目录] [--allow allowed_differences_rust.json]
- 每个实例用全新的临时数据目录；不碰任何真实数据，也不读 progress.json。
- 退出码 0 = 除白名单外没有差异。"""
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
out = sys.argv[sys.argv.index('--out') + 1] if '--out' in sys.argv else os.path.join(tempfile.gettempdir(), 'xexam_golden_rust')
allow = sys.argv[sys.argv.index('--allow') + 1] if '--allow' in sys.argv else os.path.join(paths.GOLDEN, 'allowed_differences_rust.json')
P1, P2 = 8861, 8863

env = dict(os.environ)
for k in list(env):
    if k.startswith('XEXAM_'):
        env.pop(k)
env['XEXAM_VERSION'] = open(os.path.join(paths.REPO_ROOT, 'VERSION')).read().strip()


def start(port, data, *extra):
    # 日志写文件，不用管道：管道没人读会被填满，后端就卡在写日志上
    log = open(os.path.join(data, 'server.log'), 'wb')
    p = subprocess.Popen([exe, 'serve', '--data', data, '--host', '127.0.0.1', '--port', str(port), '--no-browser', *extra],
                         env=env, stdout=log, stderr=subprocess.STDOUT)
    for _ in range(80):
        try:
            urllib.request.urlopen('http://127.0.0.1:%d/api/info' % port, timeout=1)
            return p
        except Exception:
            time.sleep(0.25)
    p.terminate()
    raise SystemExit('后端没有起来（端口 %d）' % port)


d1, d2 = tempfile.mkdtemp(prefix='xexam_rs1_'), tempfile.mkdtemp(prefix='xexam_rs2_')
procs = []
try:
    procs.append(start(P1, d1))
    procs.append(start(P2, d2, '--allow-shutdown'))
    r = subprocess.run([sys.executable, '-I', '-B', os.path.join(HERE, 'capture.py'),
                        '--base-url', 'http://127.0.0.1:%d' % P1, '--shutdown-url', 'http://127.0.0.1:%d' % P2, '--out', out],
                       capture_output=True, text=True, encoding='utf-8')
    print((r.stdout or '')[-600:])
    if r.returncode != 0:
        print(r.stderr[-1500:])
        sys.exit(2)
finally:
    for p in procs:
        if p.poll() is None:
            p.terminate()
            try:
                p.wait(timeout=8)
            except Exception:
                p.kill()
    shutil.rmtree(d1, ignore_errors=True)
    shutil.rmtree(d2, ignore_errors=True)

cmd = [sys.executable, '-I', os.path.join(HERE, 'diff_golden.py'), paths.GOLDEN, out]
if os.path.exists(allow):
    cmd += ['--allow', allow]
sys.exit(subprocess.run(cmd).returncode)
