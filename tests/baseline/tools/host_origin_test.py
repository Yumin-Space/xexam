# -*- coding: utf-8 -*-
"""Host / Origin 校验：先测纯函数，再启动真实服务端，按「本机 / 局域网 / Cloudflare 隧道」三种场景逐一请求。"""
import sys, os, json, time, subprocess, tempfile, shutil, http.client

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import paths  # noqa: E402
ROOT = paths.LEGACY_ROOT
sys.path.insert(0, ROOT)
import server as S

out = []
def ok(cond, msg):
    out.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(out)); sys.exit(1)

# ---- 纯函数 ----
for h, want in [('localhost:8765', 'localhost'), ('LOCALHOST', 'localhost'), ('localhost.', 'localhost'), ('[::1]:8765', '::1'),
                ('xexam.example.com', 'xexam.example.com'), ('Xexam.Example.COM:443', 'xexam.example.com'),
                ('192.168.1.20:8765', '192.168.1.20'), ('', ''), (None, '')]:
    ok(S.host_name(h) == want, 'host_name(%r) = %r' % (h, S.host_name(h)))
for o, want in [('https://xexam.example.com', 'xexam.example.com'), ('http://localhost:8765', 'localhost'), ('HTTPS://X.Example.com:8443/', 'x.example.com'),
                ('null', None), ('file://', None), ('', None), ('https://user@evil.com', None), ('ftp://x', None), ('https://', None)]:
    ok(S.origin_host(o) == want, 'origin_host(%r) = %r' % (o, S.origin_host(o)))
ok(S.parse_allowed_hosts('xexam.example.com, https://b.example.org:8443/path ,192.168.1.20:8765,,') == {'xexam.example.com', 'b.example.org', '192.168.1.20'},
   'XEXAM_ALLOWED_HOSTS 能解析：域名 / 带协议带端口带路径 / IP:端口 / 空项')

# ---- 真实服务端 ----
def start(port, env_extra):
    d = tempfile.mkdtemp()
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', XEXAM_DATA=d, XEXAM_HOST='127.0.0.1', XEXAM_PORT=str(port))
    env.pop('XEXAM_ALLOWED_HOSTS', None); env.pop('XEXAM_ALLOW_ANY_HOST', None)
    env.update(env_extra)
    p = subprocess.Popen([sys.executable, 'server.py', '--no-browser'], cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8')
    for _ in range(60):
        try:
            c = http.client.HTTPConnection('127.0.0.1', port, timeout=2); c.request('GET', '/api/info'); c.getresponse().read(); c.close(); return p, d
        except Exception:
            time.sleep(0.2)
    p.terminate(); raise SystemExit('server did not start')

def req(port, method, path, host=None, origin=None, body=None, ctype='application/json'):
    c = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
    c.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
    if host is not None: c.putheader('Host', host)
    if origin is not None: c.putheader('Origin', origin)
    if body is not None:
        c.putheader('Content-Type', ctype); c.putheader('Content-Length', str(len(body)))
    c.endheaders(body)
    r = c.getresponse(); data = r.read(); c.close()
    return r.status, data

procs = []
try:
    # 场景 1：默认（只允许本机）
    p, d1 = start(8921, {}); procs.append((p, d1))
    ok(req(8921, 'GET', '/api/info', host='localhost:8765')[0] == 200, '默认：localhost 放行')
    ok(req(8921, 'GET', '/api/info', host='127.0.0.1:8765')[0] == 200, '默认：127.0.0.1 放行')
    ok(req(8921, 'GET', '/api/info', host='[::1]:8765')[0] == 200, '默认：[::1] 放行')
    ok(req(8921, 'GET', '/api/info', host='xexam.example.com')[0] == 403, '默认：公网域名被拒绝')
    st, b = req(8921, 'GET', '/api/info', host='xexam.example.com'); ok('XEXAM_ALLOWED_HOSTS' in b.decode('utf-8') and 'xexam.example.com' in b.decode('utf-8'), '默认：403 提示里说明了怎么放行，并写出被拒绝的主机名')
    ok(req(8921, 'GET', '/api/info', host='192.168.1.20:8765')[0] == 403, '默认：局域网 IP 被拒绝')
    ok(req(8921, 'POST', '/api/import/cancel', host='localhost:8765', origin='http://localhost:8765', body=b'{}')[0] == 200, '默认：本机同源 POST 放行')
    ok(req(8921, 'POST', '/api/import/cancel', host='localhost:8765', origin='http://evil.example.com', body=b'{}')[0] == 403, '默认：跨站 Origin 的 POST 被拒绝')
    ok(req(8921, 'POST', '/api/import/cancel', host='localhost:8765', origin='null', body=b'{}')[0] == 403, '默认：Origin: null 的 POST 被拒绝')
    ok(req(8921, 'POST', '/api/import/cancel', host='localhost:8765', body=b'{}')[0] == 200, '默认：没有 Origin 的 POST（命令行工具）放行')
    ok(req(8921, 'POST', '/api/import/cancel', host='localhost:8765', origin='http://localhost:9999', body=b'{}')[0] == 200, '默认：同主机不同端口的 Origin 放行（只比较主机名）')

    # 场景 2：Cloudflare 隧道 —— 把你的域名加进白名单
    p, d2 = start(8922, {'XEXAM_ALLOWED_HOSTS': 'xexam.example.com'}); procs.append((p, d2))
    ok(req(8922, 'GET', '/', host='xexam.example.com')[0] == 200, '隧道(白名单)：公网域名 GET 放行')
    ok(req(8922, 'GET', '/', host='XEXAM.Example.COM')[0] == 200, '隧道(白名单)：大小写不敏感')
    ok(req(8922, 'GET', '/api/bank', host='other.example.com')[0] == 403, '隧道(白名单)：别的域名仍被拒绝')
    ok(req(8922, 'POST', '/api/import/cancel', host='xexam.example.com', origin='https://xexam.example.com', body=b'{}')[0] == 200, '隧道(白名单)：浏览器经 HTTPS 域名发的 POST（Origin=https://域名）放行  <- 之前这里会 403')
    ok(req(8922, 'POST', '/api/import/cancel', host='xexam.example.com', origin='https://evil.example.com', body=b'{}')[0] == 403, '隧道(白名单)：别的网站发来的跨站 POST 仍被拒绝')
    ok(req(8922, 'POST', '/api/import/cancel', host='xexam.example.com', origin='https://xexam.example.com.evil.com', body=b'{}')[0] == 403, '隧道(白名单)：域名前缀欺骗（xexam.example.com.evil.com）被拒绝')
    ok(req(8922, 'POST', '/api/import/cancel', host='xexam.example.com', origin='https://user@xexam.example.com', body=b'{}')[0] == 403, '隧道(白名单)：Origin 里带 userinfo 被拒绝')
    ok(req(8922, 'GET', '/api/info', host='localhost:8765')[0] == 200, '隧道(白名单)：本机访问照常可用')

    # 场景 3：cloudflared 把 Host 改写成源站地址（httpHostHeader=局域网IP:8765）+ 浏览器 Origin 仍是 https://公网域名
    p, d3 = start(8923, {'XEXAM_ALLOWED_HOSTS': '192.168.1.20'}); procs.append((p, d3))
    ok(req(8923, 'GET', '/', host='192.168.1.20:8765')[0] == 200, '改写 Host：GET 放行')
    st = req(8923, 'POST', '/api/import/cancel', host='192.168.1.20:8765', origin='https://xexam.example.com', body=b'{}')[0]
    ok(st == 403, '改写 Host 且没配 XEXAM_ALLOWED_ORIGINS：Origin(https://域名) 和 Host(IP) 不一致 -> POST 403。实际 %d' % st)

    # 场景 3b：同样的改写，但把公网域名写进 XEXAM_ALLOWED_ORIGINS —— 这是你的部署（cloudflared 在局域网别处）要用的写法
    p, d3b = start(8926, {'XEXAM_ALLOWED_HOSTS': '192.168.1.20', 'XEXAM_ALLOWED_ORIGINS': 'https://xexam.example.com'}); procs.append((p, d3b))
    ok(req(8926, 'POST', '/api/import/cancel', host='192.168.1.20:8765', origin='https://xexam.example.com', body=b'{}')[0] == 200, '改写 Host + XEXAM_ALLOWED_ORIGINS：浏览器经域名发的 POST 放行')
    ok(req(8926, 'POST', '/api/import/cancel', host='192.168.1.20:8765', origin='https://evil.example.com', body=b'{}')[0] == 403, '改写 Host + XEXAM_ALLOWED_ORIGINS：别的网站的 POST 仍被拒绝')
    ok(req(8926, 'POST', '/api/import/cancel', host='192.168.1.20:8765', origin='https://xexam.example.com.evil.com', body=b'{}')[0] == 403, '改写 Host + XEXAM_ALLOWED_ORIGINS：域名前缀欺骗被拒绝')
    ok(req(8926, 'GET', '/', host='xexam.example.com')[0] == 403, 'XEXAM_ALLOWED_ORIGINS 只放行 Origin，不会把它当成允许的 Host')

    # 场景 4：XEXAM_ALLOW_ANY_HOST=1 —— 前面有 Cloudflare Access 时最省事的写法
    p, d4 = start(8924, {'XEXAM_ALLOW_ANY_HOST': '1'}); procs.append((p, d4))
    ok(req(8924, 'GET', '/', host='anything.example.net')[0] == 200, '不检查 Host：任意域名 GET 放行')
    ok(req(8924, 'GET', '/', host='192.168.1.20:8765')[0] == 200, '不检查 Host：局域网 IP 放行')
    ok(req(8924, 'POST', '/api/import/cancel', host='anything.example.net', origin='https://anything.example.net', body=b'{}')[0] == 200, '不检查 Host：同主机 Origin 的 POST 放行')
    ok(req(8924, 'POST', '/api/import/cancel', host='anything.example.net', origin='https://evil.example.com', body=b'{}')[0] == 403, '不检查 Host：跨站 Origin 的 POST 仍被拒绝（Origin 检查不受影响）')
    ok(req(8924, 'POST', '/api/import/cancel', host='192.168.1.20:8765', origin='https://xexam.example.com', body=b'{}')[0] == 403, '不检查 Host、没配 ALLOWED_ORIGINS：Host 被改写成 IP、Origin 是公网域名 -> POST 仍 403（要配 XEXAM_ALLOWED_ORIGINS）')
    p, d4b = start(8927, {'XEXAM_ALLOW_ANY_HOST': '1', 'XEXAM_ALLOWED_ORIGINS': 'xexam.example.com'}); procs.append((p, d4b))
    ok(req(8927, 'POST', '/api/import/cancel', host='192.168.1.20:8765', origin='https://xexam.example.com', body=b'{}')[0] == 200, '不检查 Host + XEXAM_ALLOWED_ORIGINS（裸域名写法）：POST 放行')

    # 场景 5：值写法
    p, d5 = start(8925, {'XEXAM_ALLOW_ANY_HOST': 'false', 'XEXAM_ALLOWED_HOSTS': 'https://xexam.example.com/'}); procs.append((p, d5))
    ok(req(8925, 'GET', '/', host='other.example.com')[0] == 403, 'XEXAM_ALLOW_ANY_HOST=false 不会放开')
    ok(req(8925, 'GET', '/', host='xexam.example.com')[0] == 200, 'XEXAM_ALLOWED_HOSTS 写成完整网址也能识别')

    # HEAD / 静态文件 / 其它保护不受影响
    ok(req(8922, 'GET', '/..%2f..%2fserver.py', host='xexam.example.com')[0] == 404, '放开域名后路径穿越仍被挡')
    ok(req(8922, 'GET', '/xexam.db', host='xexam.example.com')[0] == 404, '放开域名后数据库文件仍不可下载')
    ok(req(8922, 'POST', '/api/shutdown', host='xexam.example.com', origin='https://xexam.example.com', body=b'{}')[0] == 403, '放开域名后 /api/shutdown 仍被禁用')
    print('\n'.join(out)); print('\n全部 %d 项通过' % len(out))
finally:
    for p, d in procs:
        p.terminate()
        try: p.wait(timeout=5)
        except Exception: pass
        shutil.rmtree(d, ignore_errors=True)
