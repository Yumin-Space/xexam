# -*- coding: utf-8 -*-
"""Nginx 行为检查（不依赖后端实现）：用 Docker 起 nginx 镜像 + 一个「回声」假后端，验证：
  Host 原样透传、X-Forwarded-* 被清空、413 返回 JSON、安全头、静态文件、隐藏文件 404、未知方法被拒、
  后端不可达时返回 JSON 502、/healthz。

用法: python -I tests/integration/nginx_check.py <镜像，默认 xexam-web:test> [端口，默认 18080]
只用 Docker 和标准库；创建的容器 / 网络都带 xexam-nginxcheck 前缀，结束后清理。"""
import http.client
import json
import subprocess
import sys
import time

IMAGE = sys.argv[1] if len(sys.argv) > 1 else 'xexam-web:test'
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 18080
NET = 'xexam-nginxcheck-net'
BACK = 'xexam-nginxcheck-backend'
WEB = 'xexam-nginxcheck-web'

ECHO = r'''
import http.server, json
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def _do(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n) if n else b""
        out = json.dumps({"method": self.command, "path": self.path, "headers": {k.lower(): v for k, v in self.headers.items()}, "body_len": len(body)}).encode()
        self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(out))); self.end_headers(); self.wfile.write(out)
    do_GET = do_POST = do_HEAD = _do
    def log_message(self, *a): pass
http.server.ThreadingHTTPServer(("0.0.0.0", 8765), H).serve_forever()
'''

results = []


def ok(cond, msg):
    results.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(results))
        cleanup()
        sys.exit(1)


def sh(*a, check=True):
    r = subprocess.run(a, capture_output=True, text=True, encoding='utf-8')
    if check and r.returncode != 0:
        raise SystemExit('命令失败: %s\n%s' % (' '.join(a), r.stderr))
    return r


def cleanup():
    for c in (WEB, BACK):
        sh('docker', 'rm', '-f', c, check=False)
    sh('docker', 'network', 'rm', NET, check=False)


def req(method, path, headers=None, body=None, host=None):
    c = http.client.HTTPConnection('127.0.0.1', PORT, timeout=15)
    h = dict(headers or {})
    if host is not None:
        h['Host'] = host
    c.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
    for k, v in h.items():
        c.putheader(k, v)
    if body is not None:
        c.putheader('Content-Length', str(len(body)))
    c.endheaders(body)
    r = c.getresponse()
    data = r.read()
    hdrs = {k.lower(): v for k, v in r.getheaders()}
    c.close()
    return r.status, hdrs, data


cleanup()
sh('docker', 'network', 'create', NET)
try:
    # 假后端（只在内部网络）
    sh('docker', 'run', '-d', '--name', BACK, '--network', NET, '--network-alias', 'backend', 'python:3.12-slim', 'python', '-c', ECHO)
    # nginx：和 compose 里一样，只读、非 root、丢掉所有 capability
    sh('docker', 'run', '-d', '--name', WEB, '--network', NET, '-p', '127.0.0.1:%d:8080' % PORT,
       '--read-only', '--tmpfs', '/tmp', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', IMAGE)
    for _ in range(40):
        try:
            req('GET', '/healthz', host='localhost')
            break
        except Exception:
            time.sleep(0.5)
    time.sleep(1.5)

    st, h, b = req('GET', '/healthz', host='localhost:%d' % PORT)
    ok(st == 200 and b.strip() == b'ok', '/healthz 返回 200 ok')
    try:
        st0, _, _ = req('GET', '/healthz', host='')
    except Exception:
        st0 = 444           # 444 = Nginx 直接断开连接，客户端会看到连接被关闭
    ok(st0 in (444, 400), '没有 Host 的请求被 Nginx 直接拒绝（不会到后端）。实际 %r' % st0)

    # —— 静态文件 ——
    st, h, b = req('GET', '/', host='localhost:%d' % PORT)
    ok(st == 200 and b'<div id="app">' in b and b'Xexam' in b, '首页由 Nginx 直接提供（含 #app）')
    ok(h.get('cache-control') == 'no-cache', 'index.html: Cache-Control: no-cache。实际 %r' % h.get('cache-control'))
    import re
    asset = re.search(rb'/assets/[\w.\-]+\.js', b)
    ok(bool(asset), '页面引用了带哈希的 JS')
    st, h2, b2 = req('GET', asset.group(0).decode(), host='localhost:%d' % PORT)
    ok(st == 200 and 'immutable' in h2.get('cache-control', ''), '带哈希的资源可长期缓存。实际 %r' % h2.get('cache-control'))
    ok('javascript' in h2.get('content-type', ''), 'JS 的 Content-Type 正确。实际 %r' % h2.get('content-type'))
    for p, ct in (('/icon.svg', 'svg'), ('/apple-touch-icon.png', 'png')):
        st, h3, _ = req('GET', p, host='localhost:%d' % PORT)
        ok(st == 200 and ct in h3.get('content-type', ''), '%s 能取到' % p)

    # —— 安全头 ——
    for k, want in (('x-content-type-options', 'nosniff'), ('x-frame-options', 'DENY'), ('referrer-policy', 'no-referrer')):
        ok(h.get(k) == want, '首页安全头 %s: %s。实际 %r' % (k, want, h.get(k)))
    csp = h.get('content-security-policy', '')
    ok("default-src 'self'" in csp and "frame-ancestors 'none'" in csp and "style-src 'self'" in csp, '首页带严格的 CSP。实际 %r' % csp[:80])
    ok('server' in h and not re.search(r'\d+\.\d+', h['server']), 'Server 头不带版本号。实际 %r' % h.get('server'))

    # —— 路径与方法 ——
    st, _, _ = req('GET', '/nope.txt', host='localhost:%d' % PORT)
    ok(st == 404, '未知路径 404（这个应用没有按网址切换的页面，不需要 SPA 回退）。实际 %d' % st)
    st, _, _ = req('GET', '/.git/config', host='localhost:%d' % PORT)
    ok(st == 404, '点文件 404。实际 %d' % st)
    st, _, _ = req('GET', '/assets/', host='localhost:%d' % PORT)
    ok(st in (403, 404), '不列目录。实际 %d' % st)
    st, _, _ = req('DELETE', '/api/bank', host='localhost:%d' % PORT)
    ok(st == 403, 'DELETE /api/ 被 Nginx 拒绝。实际 %d' % st)
    st, _, _ = req('PUT', '/api/bank', host='localhost:%d' % PORT)
    ok(st == 403, 'PUT /api/ 被 Nginx 拒绝。实际 %d' % st)
    st, _, _ = req('GET', '/api/upload', host='localhost:%d' % PORT)
    ok(st == 403, 'GET /api/upload 被拒绝（只允许 POST）。实际 %d' % st)

    # —— 代理：Host 原样透传，转发类头被清空 ——
    st, _, b = req('GET', '/api/info', host='xexam.example.com', headers={
        'X-Forwarded-For': '6.6.6.6', 'X-Forwarded-Host': 'evil.example.com', 'X-Forwarded-Proto': 'https',
        'Forwarded': 'for=1.2.3.4', 'X-Real-IP': '7.7.7.7'})
    ok(st == 200, '代理到后端。实际 %d' % st)
    seen = json.loads(b)['headers']
    ok(seen.get('host') == 'xexam.example.com', '后端看到的 Host = 浏览器发的 Host。实际 %r' % seen.get('host'))
    for k in ('x-forwarded-for', 'x-forwarded-host', 'x-forwarded-proto', 'forwarded', 'x-real-ip'):
        ok(k not in seen, '后端收不到 %s（Nginx 清空了它）' % k)
    st, _, b = req('GET', '/api/info', host='192.168.1.20:8765')
    ok(json.loads(b)['headers'].get('host') == '192.168.1.20:8765', 'Host 带端口也原样透传')
    st, _, b = req('POST', '/api/attempt', host='localhost', headers={'Content-Type': 'application/json', 'Origin': 'https://xexam.example.com'}, body=b'{"a":1}')
    seen = json.loads(b)
    ok(seen['headers'].get('origin') == 'https://xexam.example.com' and seen['body_len'] == 7, 'Origin 与请求体原样到达后端')

    # —— 上传大小 ——
    st, hh, b = req('POST', '/api/upload', host='localhost', headers={'Content-Type': 'application/octet-stream', 'X-Filename': 'a.xlsx'}, body=b'x' * (5 * 1024 * 1024))
    ok(st == 200 and json.loads(b)['body_len'] == 5 * 1024 * 1024, '5 MB 上传通过（并完整到达后端）。实际 %d' % st)
    st, hh, b = req('POST', '/api/upload', host='localhost', headers={'Content-Type': 'application/octet-stream', 'X-Filename': 'a.xlsx'}, body=b'x' * (20 * 1024 * 1024 + 1))
    ok(st == 413, '超过 20 MB 被 Nginx 拒绝（413）。实际 %d' % st)
    ok('json' in hh.get('content-type', ''), '413 是 JSON 而不是 HTML。实际 %r' % hh.get('content-type'))
    ok('文件太大了' in json.loads(b)['error'], '413 带后端同样的中文文案')

    # —— 限流 ——
    codes = [req('POST', '/api/upload', host='localhost', headers={'Content-Type': 'application/octet-stream', 'X-Filename': 'a.xlsx'}, body=b'x')[0] for _ in range(12)]
    ok(429 in codes, '上传限流：连发 12 次出现 429。实际 %r' % sorted(set(codes)))
    time.sleep(6)

    # —— 后端挂了 ——
    sh('docker', 'stop', BACK)
    time.sleep(1)
    st, hh, b = req('GET', '/api/info', host='localhost')
    ok(st == 502 and 'json' in hh.get('content-type', ''), '后端不可达时返回 JSON 502。实际 %d %r' % (st, hh.get('content-type')))
    ok('后端暂时连不上' in json.loads(b)['error'], '502 带可读的中文说明')

    # —— 运行身份 ——
    r = sh('docker', 'exec', WEB, 'id', '-u')
    ok(r.stdout.strip() != '0', 'Nginx 不是 root 运行（uid %s）' % r.stdout.strip())
    r = sh('docker', 'inspect', '-f', '{{.State.Health.Status}}', WEB)
    ok(r.stdout.strip() in ('healthy', 'starting'), '容器健康检查状态：%s' % r.stdout.strip())
finally:
    cleanup()

print('\n'.join(results))
print('\n全部 %d 项通过' % len(results))
