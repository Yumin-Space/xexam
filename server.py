#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Xexam - 服务端（只用 Python 标准库）。

题库和学习记录都保存在 SQLite（<data目录>/xexam.db）。更新题库：在管理页上传 Excel。

用法：
    python server.py                         # 数据放在 ./data，自动打开浏览器
    python server.py --data D:\\xexam-data   # 指定数据目录
    python server.py --import 题库.xlsx      # 不启动服务，直接把 Excel 导入数据库后退出
Docker 里用：python server.py --host 0.0.0.0 --data /data --no-browser
"""
import argparse
import io
import json
import os
import platform
import re
import secrets
import signal
import sys
import threading
import time
import webbrowser
import zipfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from types import SimpleNamespace
from urllib.parse import unquote, urlparse

APP_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, APP_DIR)
from app import db, xlsx_parser          # noqa: E402

WEB_DIR = os.path.join(APP_DIR, 'web')


def read_version():
    """版本号来自 VERSION 文件（镜像里也带着它）。读不到就显示 dev。"""
    try:
        with open(os.path.join(APP_DIR, 'VERSION'), encoding='utf-8') as f:
            v = f.read().strip()
    except OSError:
        return 'dev'
    return v if re.fullmatch(r'[0-9A-Za-z.+-]{1,20}', v) else 'dev'


VERSION = read_version()
MAX_JSON = 1_000_000
MAX_UPLOAD = 20 * 1024 * 1024           # 上传的 Excel 最大 20 MB（几千道题也只有 1-2 MB）
PENDING_TTL = 30 * 60                   # 上传后的预览 30 分钟内有效
SKIP_SHOWN = 30                         # 预览里每个工作表最多列出多少条「读不出来」的原因


def say(msg):
    try:
        print(msg, flush=True)
    except UnicodeEncodeError:
        print(msg.encode('ascii', 'replace').decode(), flush=True)


# ---------------------------------------------------------------- 上传文件检查

def check_xlsx(data):
    """上传内容必须像一个正常的 .xlsx。不合格就抛 ValueError（带中文说明）。"""
    if len(data) < 100:
        raise ValueError('文件是空的，或者没有上传完整。')
    if data[:2] != b'PK':
        if data[:8] == b'\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1':
            raise ValueError('这是旧版 .xls 格式，或者是设置了密码的 Excel 文件。请在 Excel 里去掉密码，并另存为「Excel 工作簿(.xlsx)」再上传。')
        raise ValueError('这个文件不是 .xlsx（可能是 .csv、.pdf 或别的格式）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。')
    try:
        z = zipfile.ZipFile(io.BytesIO(data))
    except zipfile.BadZipFile:
        raise ValueError('这个 .xlsx 文件已损坏，打不开。请用 Excel 重新保存一份再上传。')
    if 'xl/workbook.xml' not in set(z.namelist()):
        raise ValueError('这个文件里没有 Excel 工作簿的内容，不是有效的 .xlsx。')
    total = 0
    for info in z.infolist():
        total += info.file_size
        if info.file_size > 100 * 1024 * 1024 or total > 300 * 1024 * 1024:
            raise ValueError('文件解压后太大，已拒绝。')
        if info.filename.endswith(('.xml', '.rels')):
            blob = z.read(info)
            if b'<!ENTITY' in blob or b'<!DOCTYPE' in blob:
                raise ValueError('文件里含有不被允许的内容（XML 实体声明），已拒绝。正常的 Excel 文件不会有这个。')


def host_name(value):
    """从 Host 头（或 URL 的 netloc）里取出主机名：去掉端口，保留 IPv6 的方括号内容，转小写，去掉末尾的点。"""
    h = (value or '').strip().lower()
    if h.startswith('['):
        end = h.find(']')
        name = h[1:end] if end > 0 else h
    else:
        name = h.rsplit(':', 1)[0] if ':' in h else h
    return name.rstrip('.')


def origin_host(origin):
    """Origin 头形如 https://example.com:8443 ；返回主机名。不是 http/https 的（比如 null、file://）返回 None。"""
    o = (origin or '').strip()
    low = o.lower()
    for scheme in ('http://', 'https://'):
        if low.startswith(scheme):
            rest = o[len(scheme):].split('/', 1)[0]
            return host_name(rest) if rest and '@' not in rest else None
    return None


def parse_allowed_hosts(text):
    """XEXAM_ALLOWED_HOSTS：逗号分隔。每一项可以带端口或 http(s):// 前缀，统一成主机名。"""
    out = set()
    for item in re.split(r'[,\s;]+', text or ''):
        item = item.strip()
        if not item:
            continue
        item = item.split('://', 1)[-1].split('/', 1)[0]
        n = host_name(item)
        if n:
            out.add(n)
    return out


def clean_filename(raw):
    name = unquote(raw or '').replace('\\', '/').split('/')[-1]
    name = re.sub(r'[\x00-\x1f\x7f]', '', name).strip()
    return name[:120] or '未命名.xlsx'


# ---------------------------------------------------------------- 学习记录请求

def clean_attempt(d):
    results = []
    raw_results = d.get('results') or []
    if not isinstance(raw_results, list):
        raise ValueError('成绩的格式不对')
    for r in raw_results[:2000]:
        if isinstance(r, dict) and isinstance(r.get('id'), str) and isinstance(r.get('correct'), bool):
            results.append({'id': r['id'][:64], 'correct': r['correct']})
    if not results:
        raise ValueError('没有成绩可保存')
    raw_sheets = d.get('sheets') or []
    sheets = [s[:80] for s in (raw_sheets if isinstance(raw_sheets, list) else []) if isinstance(s, str)][:50]
    mode = d.get('mode') if d.get('mode') in ('all', 'weak', 'retry') else 'all'
    try:
        seconds = max(0, min(int(d.get('seconds') or 0), 7 * 86400))
    except (TypeError, ValueError, OverflowError):
        seconds = 0
    return {'results': results, 'sheets': sheets, 'mode': mode, 'seconds': seconds}


# ---------------------------------------------------------------- 上传 -> 预览 -> 确认

class Pending:
    """上传后、点「确认导入」前，把解析好的内容暂存在内存里。只保留最近一次。"""

    def __init__(self):
        self.lock = threading.Lock()
        self.item = None

    def put(self, filename, sheets, questions):
        token = secrets.token_urlsafe(16)
        with self.lock:
            self.item = {'token': token, 'filename': filename, 'sheets': sheets, 'questions': questions, 'expires': time.time() + PENDING_TTL}
        return token

    def take(self, token):
        """取出并清掉。token 不对或已过期返回 None。"""
        with self.lock:
            it = self.item
            if not it or not secrets.compare_digest(it['token'].encode('utf-8'), str(token or '').encode('utf-8')) or it['expires'] < time.time():
                return None
            self.item = None
            return it

    def drop(self):
        with self.lock:
            self.item = None


def preview_payload(plan, filename, token):
    sheets = []
    for s in plan['sheets']:
        extra = max(0, len(s['skipped']) - SKIP_SHOWN)
        sheets.append(dict(s, skipped=s['skipped'][:SKIP_SHOWN], skippedMore=extra, skippedCount=len(s['skipped'])))
    return dict(plan, sheets=sheets, filename=filename, token=token)


# ---------------------------------------------------------------- HTTP

MIME = {
    '.html': 'text/html; charset=utf-8', '.css': 'text/css; charset=utf-8',
    '.js': 'text/javascript; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon',
    '.json': 'application/json; charset=utf-8',
}


class Handler(BaseHTTPRequestHandler):
    server_version = 'Xexam/' + VERSION

    def log_message(self, fmt, *args):
        pass

    def _send(self, code, body, ctype):
        self.send_response(code)
        self.send_header('Content-Type', ctype)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.end_headers()
        if not getattr(self, '_head_only', False):
            self.wfile.write(body)

    def _json(self, obj, code=200):
        self._send(code, json.dumps(obj, ensure_ascii=False).encode('utf-8'), MIME['.json'])

    def _host_ok(self):
        """默认只接受发给本机或白名单域名的请求，挡住「DNS 重绑定」（恶意网页让浏览器把自己的域名解析到本机再来访问）。
        放在 Cloudflare Tunnel 等有身份验证的反向代理后面时：把你的域名写进 XEXAM_ALLOWED_HOSTS，
        或者设 XEXAM_ALLOW_ANY_HOST=1 不再检查 Host（这时安全完全靠前面的验证）。"""
        cfg = self.server.cfg
        if cfg.allow_any_host:
            return True
        return host_name(self.headers.get('Host')) in cfg.allowed_hosts

    def _origin_ok(self):
        """写请求的 Origin 必须和 Host 是同一个主机名（协议和端口不比较），或者在 XEXAM_ALLOWED_ORIGINS 里：
        经 HTTPS 反向代理时浏览器发的是 https://域名，而服务端看到的 Host 只有域名，所以不能要求 http:// 开头。
        如果反向代理把 Host 改写成了源站地址（例如 cloudflared 的 httpHostHeader），Origin 仍是公网域名，
        两者不一致，这时把公网域名写进 XEXAM_ALLOWED_ORIGINS。
        没有 Origin 头的请求（比如命令行工具）放行：它不是浏览器里的跨站请求。"""
        origin = self.headers.get('Origin')
        if origin is None:
            return True
        oh = origin_host(origin)
        if oh is None:
            return False
        return oh == host_name(self.headers.get('Host')) or oh in self.server.cfg.allowed_origins

    def do_HEAD(self):
        """反向代理和一些健康检查工具会用 HEAD。响应头和 GET 完全一样，只是不带正文。"""
        self._head_only = True
        try:
            self.do_GET()
        finally:
            self._head_only = False

    def do_GET(self):
        if not self._host_ok():
            seen = re.sub(r'[\x00-\x1f\x7f]', '', host_name(self.headers.get('Host')))[:80]
            say('拒绝了一个请求：Host 是「%s」，不在允许的列表里（要放行请设置 XEXAM_ALLOWED_HOSTS）' % seen)
            return self._send(403, ('Forbidden host: %s。通过别的地址访问（局域网 IP、你的域名）时，请把它加进环境变量 XEXAM_ALLOWED_HOSTS，'
                                    '或设 XEXAM_ALLOW_ANY_HOST=1（只在前面有身份验证时）。' % seen).encode('utf-8'), 'text/plain; charset=utf-8')
        path = urlparse(self.path).path
        cfg = self.server.cfg
        if path == '/api/info':
            return self._json({'version': VERSION, 'arch': platform.machine() or 'unknown',
                               'canShutdown': cfg.allow_shutdown, 'bankCount': cfg.store.count_active()})
        if path == '/api/bank':
            return self._json(cfg.store.bank())
        if path == '/api/progress':
            return self._json(cfg.store.progress())
        if path == '/api/imports':
            return self._json({'imports': cfg.store.import_history(12)})
        rel = 'index.html' if path in ('', '/') else unquote(path).lstrip('/')
        full = os.path.normpath(os.path.join(WEB_DIR, rel))
        try:
            inside = os.path.commonpath([full, WEB_DIR]) == WEB_DIR
        except ValueError:
            inside = False
        if not inside or not os.path.isfile(full):
            return self._send(404, b'not found', 'text/plain; charset=utf-8')
        with open(full, 'rb') as f:
            data = f.read()
        self._send(200, data, MIME.get(os.path.splitext(full)[1].lower(), 'application/octet-stream'))

    def _body(self, limit):
        """读请求体。超过 limit 返回 None —— 但会先把多余的数据读掉丢弃（有上限），
        否则服务端提前回应后连接被重置，浏览器看到的是网络错误而不是「文件太大」。"""
        try:
            n = int(self.headers.get('Content-Length') or 0)
        except ValueError:
            n = 0
        if n < 0:
            return None
        if n > limit:
            left = min(n, limit * 4)
            while left > 0:
                chunk = self.rfile.read(min(65536, left))
                if not chunk:
                    break
                left -= len(chunk)
            self.close_connection = True
            return None
        return self.rfile.read(n)

    def _drain_body(self):
        """提前拒绝（403/415）时，把客户端已经在发的请求体读掉并丢弃（有上限）；否则连接被重置，浏览器看到的是网络错误而不是原因。"""
        try:
            n = int(self.headers.get('Content-Length') or 0)
        except ValueError:
            n = 0
        left = min(max(n, 0), MAX_UPLOAD + 65536)
        while left > 0:
            chunk = self.rfile.read(min(65536, left))
            if not chunk:
                break
            left -= len(chunk)
        self.close_connection = True

    def _deny_reason(self):
        """POST 被拒绝时给出能照着做的原因。返回 None 表示没问题。"""
        def short(v):
            return re.sub(r'[\x00-\x1f\x7f]', '', str(v or ''))[:80]
        if not self._host_ok():
            return ('被拒绝：这个请求的 Host 是「%s」，不在允许的列表里。'
                    '请把它加进 XEXAM_ALLOWED_HOSTS（或设 XEXAM_ALLOW_ANY_HOST=1，只在前面有身份验证时）。' % short(self.headers.get('Host')))
        if not self._origin_ok():
            o = origin_host(self.headers.get('Origin'))
            if o is None:
                return '被拒绝：这个请求的 Origin 是「%s」，不是一个网址。' % short(self.headers.get('Origin'))
            return ('被拒绝：这个请求来自「%s」，和服务看到的 Host「%s」不是同一个主机。'
                    '如果你在 Cloudflare Tunnel 等反向代理后面且代理改写了 Host，请把「%s」加进 XEXAM_ALLOWED_ORIGINS。'
                    % (short(o), short(host_name(self.headers.get('Host'))), short(o)))
        return None

    def do_POST(self):
        why = self._deny_reason()
        if why:
            self._drain_body()
            say('拒绝了一个请求：%s %s | %s' % (self.command, re.sub(r'[\x00-\x1f\x7f]', '', self.path)[:80], why))
            return self._json({'error': why}, 403)
        path = urlparse(self.path).path
        cfg = self.server.cfg
        ctype = self.headers.get('Content-Type') or ''

        if path == '/api/upload':
            if 'application/octet-stream' not in ctype:
                self._drain_body()
                return self._json({'error': 'bad content type'}, 415)
            body = self._body(MAX_UPLOAD)
            if body is None:
                return self._json({'error': '文件太大了（最大 %d MB）。题库通常只有 1–2 MB，请确认选对了文件。' % (MAX_UPLOAD // 1024 // 1024)}, 413)
            filename = clean_filename(self.headers.get('X-Filename'))
            try:
                check_xlsx(body)
                sheets, questions = xlsx_parser.parse_workbook(io.BytesIO(body))
            except ValueError as e:
                return self._json({'error': str(e)}, 400)
            except Exception as e:       # 解析器没预料到的文件内容
                return self._json({'error': '读取这个 Excel 时出错：%s: %s' % (type(e).__name__, e)}, 400)
            plan = cfg.store.plan_import(sheets, questions)
            token = cfg.pending.put(filename, sheets, questions)
            return self._json(preview_payload(plan, filename, token))

        if ctype.split(';', 1)[0].strip().lower() != 'application/json':
            self._drain_body()
            return self._json({'error': 'bad content type'}, 415)
        raw = self._body(MAX_JSON)
        if raw is None:
            return self._json({'error': 'too large'}, 413)
        try:
            data = json.loads(raw or b'{}')
        except (ValueError, RecursionError):
            return self._json({'error': 'bad json'}, 400)
        if not isinstance(data, dict):
            return self._json({'error': 'bad json'}, 400)

        if path == '/api/import/confirm':
            it = cfg.pending.take(data.get('token'))
            if it is None:
                return self._json({'error': '这次上传的预览已经过期或已被处理，请重新上传。'}, 409)
            if not it['questions']:
                return self._json({'error': '这份文件里没有读到任何可用的题，没有导入。'}, 400)
            try:
                result = cfg.store.apply_import(it['filename'], it['sheets'], it['questions'])
            except Exception as e:
                return self._json({'error': '写入数据库失败，题库没有任何改动：%s' % e}, 500)
            return self._json({'result': result, 'bank': cfg.store.bank()})
        if path == '/api/import/cancel':
            cfg.pending.drop()
            return self._json({'ok': True})
        if path == '/api/attempt':
            try:
                return self._json(cfg.store.record_attempt(clean_attempt(data)))
            except ValueError as e:
                return self._json({'error': str(e)}, 400)
        if path == '/api/reset':
            return self._json(cfg.store.reset_progress())
        if path == '/api/shutdown':
            if not cfg.allow_shutdown:
                return self._json({'error': '在 Docker 里请用 docker compose stop 停止容器。'}, 403)
            self._json({'ok': True})
            threading.Thread(target=self.server.shutdown, daemon=True).start()
            return
        return self._json({'error': 'not found'}, 404)


# ---------------------------------------------------------------- 启动

def open_store(data_dir):
    os.makedirs(data_dir, exist_ok=True)
    path = os.path.join(data_dir, 'xexam.db')
    if not os.path.exists(path):
        # 早期版本的数据库文件名不同。新版只读 xexam.db，所以数据目录里有别的 .db 文件时先提醒，免得让人以为数据丢了
        others = sorted(f for f in os.listdir(data_dir) if f.lower().endswith('.db'))
        if others:
            say('注意：数据目录里没有 xexam.db，但有别的数据库文件：%s。如果那是以前版本的题库，请先停止程序，把它改名为 xexam.db 再启动；'
                '现在会新建一个空的 xexam.db。' % '、'.join(others))
    store = db.Store(path)
    # 旧版把学习记录放在 progress.json：第一次启动时迁移进数据库（只迁移一次，不会覆盖已有记录）
    for cand in (os.path.join(data_dir, 'progress.json'), os.path.join(APP_DIR, 'progress.json')):
        if os.path.isfile(cand):
            ok, msg = store.migrate_legacy_progress(cand)
            if ok:
                say('已把旧的学习记录迁移进数据库：%s（来自 %s）' % (msg, cand))
            break
    return store


def main():
    ap = argparse.ArgumentParser(description='Xexam')
    ap.add_argument('--data', default=os.environ.get('XEXAM_DATA') or os.path.join(APP_DIR, 'data'), help='数据目录（里面是 xexam.db）')
    ap.add_argument('--host', default=os.environ.get('XEXAM_HOST', '127.0.0.1'))
    ap.add_argument('--port', type=int, default=int(os.environ.get('XEXAM_PORT', '8765')))
    ap.add_argument('--no-browser', action='store_true', help='不自动打开浏览器')
    ap.add_argument('--allow-shutdown', action='store_true', help='允许在页面里点「关闭程序」（本机运行时用，Docker 里不用）')
    ap.add_argument('--import', dest='import_file', help='不启动服务，直接把这个 Excel 导入数据库后退出')
    args = ap.parse_args()

    store = open_store(args.data)

    if args.import_file:
        try:
            with open(args.import_file, 'rb') as f:
                data = f.read()
            check_xlsx(data)
            sheets, questions = xlsx_parser.parse_workbook(io.BytesIO(data))
        except (OSError, ValueError) as e:
            say('导入失败：%s' % e)
            sys.exit(1)
        if not questions:
            say('这份文件里没有读到任何可用的题，没有导入。')
            sys.exit(1)
        plan = store.apply_import(os.path.basename(args.import_file), sheets, questions)
        say('已导入 %s：新增 %d，更新 %d，恢复 %d，下架 %d，读不出来 %d；现在共 %d 题。' % (
            os.path.basename(args.import_file), plan['added'], plan['updated'], plan['restored'], plan['removed'], plan['skipped'], store.count_active()))
        for s in plan['sheets']:
            for k in s['skipped']:
                say('  没读入：%s 第 %d 行：%s' % (s['name'], k['row'], k['reason']))
        return

    httpd = None
    for port in range(args.port, args.port + 20):
        try:
            httpd = ThreadingHTTPServer((args.host, port), Handler)
            break
        except OSError:
            continue
    if httpd is None:
        say('端口 %d-%d 都被占用了，请关掉其他占用的程序，或用 --port 换一个端口。' % (args.port, args.port + 19))
        sys.exit(1)

    extra = parse_allowed_hosts(os.environ.get('XEXAM_ALLOWED_HOSTS'))
    any_host = (os.environ.get('XEXAM_ALLOW_ANY_HOST') or '').strip().lower() in ('1', 'true', 'yes', 'on')
    origins = parse_allowed_hosts(os.environ.get('XEXAM_ALLOWED_ORIGINS'))
    httpd.cfg = SimpleNamespace(
        store=store, pending=Pending(), allow_shutdown=args.allow_shutdown,
        allow_any_host=any_host, allowed_hosts={'localhost', '127.0.0.1', '::1'} | extra,
        allowed_origins=origins | extra)       # 白名单里的域名，既允许作为 Host，也允许作为 Origin
    url = 'http://localhost:%d/' % port
    say('Xexam v%s 已启动：%s' % (VERSION, url))
    say('数据目录：' + os.path.abspath(args.data))
    if any_host:
        say('注意：XEXAM_ALLOW_ANY_HOST 已打开，不再检查 Host。这个程序自己没有登录，请确保前面有身份验证（例如 Cloudflare Access）。')
    elif extra:
        say('允许的域名/地址：' + '、'.join(sorted(extra)))
    if origins:
        say('额外允许的 Origin：' + '、'.join(sorted(origins)))
    if store.count_active() == 0:
        say('题库还是空的：打开页面后在「题库管理」里上传 Excel。')
    if not args.no_browser:
        threading.Timer(0.6, webbrowser.open, [url]).start()
    # docker stop 发的是 SIGTERM。Python 作为容器里的 1 号进程时默认会忽略它，要等 10 秒被 SIGKILL 强杀；
    # 这里收到后让服务正常退出：先完成手头的请求，数据库连接关干净，再结束。
    def _stop(signum, frame):
        say('收到停止信号，正在退出……')
        threading.Thread(target=httpd.shutdown, daemon=True).start()
    for name in ('SIGTERM', 'SIGINT'):
        sig = getattr(signal, name, None)
        if sig is not None:
            try:
                signal.signal(sig, _stop)
            except (ValueError, OSError):
                pass
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        httpd.server_close()
        store.close()
        say('已退出。')


if __name__ == '__main__':
    main()
