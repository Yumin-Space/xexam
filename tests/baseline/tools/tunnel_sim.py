# -*- coding: utf-8 -*-
"""用 HTTPS 反向代理模拟 Cloudflare Tunnel：
  浏览器 --https://xexam.test:8943--> 本脚本(TLS 终止) --http--> server.py(8941)
  xexam.test 通过 Chrome 的 --host-resolver-rules 指到 127.0.0.1，证书是临时自签的，只在临时目录里。
  --rewrite-host 时，代理把 Host 头改成 127.0.0.1:8941（对应 cloudflared 的 httpHostHeader 设置）。
然后让真实 Chrome 打开 https://xexam.test:8943，上传演示 Excel -> 预览 -> 确认导入，看浏览器真正发出的 Origin 头。"""
import os, sys, ssl, json, time, shutil, socket, subprocess, tempfile, threading, urllib.request, http.client, http.server, base64, struct
sys.argv = [sys.argv[0], tempfile.gettempdir(), 'http://x'] + sys.argv[1:]
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
REWRITE = '--rewrite-host' in sys.argv
ROOT = paths.LEGACY_ROOT
HERE = os.path.dirname(os.path.abspath(__file__))
SHOTS = os.environ.get('XEXAM_SHOTS_DIR', '')
sys.path.insert(0, SHOTS)
from shoot import WS, evaluate, CHROME

APP, PROXY, DBG = 8941, 8943, 9461
DOMAIN = 'xexam.test'
tmp = tempfile.mkdtemp()
seen = []   # 代理看到的每个请求的 Host / Origin

# ---- 自签证书 ----
conf = os.path.join(tmp, 'o.cnf')
open(conf, 'w').write('[req]\ndistinguished_name=dn\nx509_extensions=v3\nprompt=no\n[dn]\nCN=%s\n[v3]\nsubjectAltName=DNS:%s\n' % (DOMAIN, DOMAIN))
subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', os.path.join(tmp, 'k.pem'), '-out', os.path.join(tmp, 'c.pem'),
                '-days', '1', '-config', conf], check=True, capture_output=True)

class Proxy(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.0'
    def log_message(self, *a): pass
    def _go(self):
        n = int(self.headers.get('Content-Length') or 0)
        body = self.rfile.read(n) if n else None
        seen.append({'method': self.command, 'path': self.path.split('?')[0], 'host': self.headers.get('Host'), 'origin': self.headers.get('Origin')})
        c = http.client.HTTPConnection('127.0.0.1', APP, timeout=30)
        hdrs = {k: v for k, v in self.headers.items() if k.lower() not in ('connection', 'keep-alive', 'transfer-encoding', 'accept-encoding')}
        if REWRITE:
            hdrs['Host'] = '127.0.0.1:%d' % APP
        hdrs['X-Forwarded-Proto'] = 'https'
        c.request(self.command, self.path, body=body, headers=hdrs, skip_host=True) if False else None
        c.putrequest(self.command, self.path, skip_host=True, skip_accept_encoding=True)
        for k, v in hdrs.items(): c.putheader(k, v)
        c.endheaders(body)
        r = c.getresponse(); data = r.read()
        self.send_response(r.status)
        for k, v in r.getheaders():
            if k.lower() not in ('connection', 'transfer-encoding', 'server', 'date'): self.send_header(k, v)
        self.end_headers(); self.wfile.write(data)
    do_GET = do_POST = do_HEAD = _go

def serve_proxy():
    srv = http.server.ThreadingHTTPServer(('127.0.0.1', PROXY), Proxy)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(os.path.join(tmp, 'c.pem'), os.path.join(tmp, 'k.pem'))
    srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    srv.serve_forever()
threading.Thread(target=serve_proxy, daemon=True).start()

env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', XEXAM_DATA=os.path.join(tmp, 'data'), XEXAM_HOST='127.0.0.1', XEXAM_PORT=str(APP))
env.pop('XEXAM_ALLOWED_HOSTS', None); env.pop('XEXAM_ALLOWED_ORIGINS', None); env.pop('XEXAM_ALLOW_ANY_HOST', None)
if REWRITE:
    env['XEXAM_ALLOWED_HOSTS'] = '127.0.0.1'
    env['XEXAM_ALLOWED_ORIGINS'] = 'https://%s:%d' % (DOMAIN, PROXY)
else:
    env['XEXAM_ALLOWED_HOSTS'] = DOMAIN
app = subprocess.Popen([sys.executable, 'server.py', '--no-browser'], cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8')
chrome = None
try:
    for _ in range(60):
        try: urllib.request.urlopen('http://127.0.0.1:%d/api/info' % APP, timeout=1); break
        except Exception: time.sleep(0.2)
    udd = os.path.join(tmp, 'chrome')
    chrome = subprocess.Popen([CHROME, '--headless=new', '--disable-gpu', '--no-first-run', '--ignore-certificate-errors',
                               '--host-resolver-rules=MAP %s 127.0.0.1' % DOMAIN, '--remote-debugging-port=%d' % DBG,
                               '--user-data-dir=' + udd, 'about:blank'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(60):
        try: tabs = json.load(urllib.request.urlopen('http://127.0.0.1:%d/json' % DBG, timeout=1)); break
        except Exception: time.sleep(0.3)
    ws = WS([t for t in tabs if t.get('type') == 'page'][0]['webSocketDebuggerUrl'])
    ws.call('Page.enable'); ws.call('Runtime.enable')
    ws.call('Page.navigate', url='https://%s:%d/' % (DOMAIN, PROXY)); time.sleep(2.5)
    print('模式:', '代理改写 Host -> 127.0.0.1:%d（配了 XEXAM_ALLOWED_ORIGINS）' % APP if REWRITE else '代理保留 Host=域名（只配了 XEXAM_ALLOWED_HOSTS）')
    print('页面地址:', evaluate(ws, 'location.href'), '| 标题:', evaluate(ws, 'document.title'))
    # 演示 Excel 以 base64 注入，不依赖第二个服务器
    demo = base64.b64encode(open(os.path.join(SHOTS, 'files', 'demo.xlsx'), 'rb').read()).decode()
    js = """(async()=>{
      const wait=ms=>new Promise(r=>setTimeout(r,ms));
      const bin=Uint8Array.from(atob('%s'),c=>c.charCodeAt(0));
      await openAdmin(); await wait(300);
      await uploadFile(new File([bin],'demo.xlsx')); await wait(600);
      const err = S.admin.error, prev = S.admin.preview && {added:S.admin.preview.added, usable:S.admin.preview.usable};
      if(!prev) return JSON.stringify({step:'upload', err});
      await confirmImport(); await wait(800);
      const info = await (await fetch('/api/info')).json();
      return JSON.stringify({step:'done', preview:prev, importedOk: !!S.admin.done, err:S.admin.error, bankCount: info.bankCount});
    })()""" % demo
    print('浏览器里上传 -> 预览 -> 确认导入 ->', evaluate(ws, js))
    # 再交一场成绩（也是 POST）
    js2 = """(async()=>{
      const r = await fetch('/api/attempt',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({results:[{id:S.bank.questions[0].id,correct:true}],sheets:['demo'],mode:'all',seconds:3})});
      return r.status;
    })()"""
    print('浏览器里提交成绩 POST /api/attempt ->', evaluate(ws, js2))
    print()
    print('代理实际收到的请求（Host / Origin）:')
    for s in seen:
        if s['method'] == 'POST': print('  %-5s %-22s Host=%-26s Origin=%s' % (s['method'], s['path'], s['host'], s['origin']))
    print('GET 请求的 Host 样例:', sorted({s['host'] for s in seen if s['method'] == 'GET'}))
finally:
    for p in (chrome, app):
        if p:
            p.terminate()
            try: p.wait(timeout=5)
            except Exception: pass
    shutil.rmtree(tmp, ignore_errors=True)
