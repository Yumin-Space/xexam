#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Xexam security / trust-model probe (black box, stdlib only).

Purpose: record how a Xexam backend behaves on every security control, so that the same probes can be
replayed against the Rust backend (and against Nginx -> backend) and the results diffed.

Two ways to run:

  1. Spawn the legacy Python server for every profile (ports 8710-8719 by default) and write results:
        python probe.py --spawn-python <dir containing server.py> --out results_python.json

  2. Probe an already running server (Rust backend, or Nginx in front of it). You must start it with the
     environment that matches --profile (see PROFILES below), on a THROW-AWAY database copy:
        python probe.py --base http://127.0.0.1:8710 --profile default --out results_rust_default.json
        python probe.py --base http://127.0.0.1:8710 --profile default --golden results_python.json

  --golden FILE   compare status code (and content type) of every probe id with an earlier results file.
  --only a,b      run only probe groups whose name starts with one of the prefixes (host,origin,method,ctype,
                  body,upload,entity,pending,attempt,static,shutdown,dos,headers).
  --no-dos        skip the groups that deliberately leave sockets open / need long waits (dos).

Profiles (what the server under test must be configured with):
  default   no XEXAM_* host variables; shutdown not allowed
  hosts     XEXAM_ALLOWED_HOSTS=xexam.example.com,192.168.1.20  XEXAM_ALLOWED_ORIGINS=https://pub.example.org
  anyhost   XEXAM_ALLOW_ANY_HOST=1
  shutdown  Python only: --allow-shutdown flag (a Rust equivalent has to be defined)

Every probe sends "Connection: close" (unless the probe is about connections) so that responses can be read
until EOF, whatever the HTTP version of the server. Probes never touch anything but the server under test;
the server must use a temporary data directory (the spawn mode does this). Nothing here reads real user data.

Result states: an int HTTP status, or EMPTY (connection closed without any response), RESET, TIMEOUT.
"want" is what a reading of the legacy Python source predicts; SURPRISE marks a probe whose actual result
differs from that prediction (so the reading, not only the server, gets checked).
"""
import argparse
import hashlib
import io
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import zipfile
import zlib

sys.dont_write_bytecode = True

PROFILES = {
    'default': {'env': {}, 'args': []},
    'hosts': {'env': {'XEXAM_ALLOWED_HOSTS': 'xexam.example.com,192.168.1.20',
                      'XEXAM_ALLOWED_ORIGINS': 'https://pub.example.org'}, 'args': []},
    'anyhost': {'env': {'XEXAM_ALLOW_ANY_HOST': '1'}, 'args': []},
    'shutdown': {'env': {}, 'args': ['--allow-shutdown']},
}
MIB = 1024 * 1024


# --------------------------------------------------------------------------- raw HTTP client

class Resp:
    def __init__(self, status=None, headers=None, body=b'', err=None):
        self.status, self.headers, self.body, self.err = status, headers or [], body, err

    def h(self, name):
        n = name.lower()
        for k, v in self.headers:
            if k.lower() == n:
                return v
        return None

    def hs(self, name):
        n = name.lower()
        return [v for k, v in self.headers if k.lower() == n]

    @property
    def state(self):
        return self.status if self.status is not None else self.err

    def text(self):
        return self.body.decode('utf-8', 'replace')

    def json(self):
        try:
            return json.loads(self.body.decode('utf-8'))
        except Exception:
            return None


def parse_response(buf):
    if not buf:
        return Resp(err='EMPTY')
    head, sep, body = buf.partition(b'\r\n\r\n')
    lines = head.split(b'\r\n')
    try:
        status = int(lines[0].split(b' ', 2)[1])
    except Exception:
        return Resp(err='GARBAGE', body=buf[:200])
    headers = []
    for ln in lines[1:]:
        k, _, v = ln.partition(b':')
        headers.append((k.decode('latin-1').strip(), v.decode('latin-1').strip()))
    return Resp(status, headers, body)


def raw(port, chunks, timeout=8.0, half_close=False, host='127.0.0.1'):
    """Send the byte chunks, read until EOF. Returns Resp."""
    if isinstance(chunks, (bytes, bytearray)):
        chunks = [bytes(chunks)]
    s = socket.create_connection((host, port), timeout=timeout)
    s.settimeout(timeout)
    sent_err = None
    try:
        for c in chunks:
            s.sendall(c)
        if half_close:
            try:
                s.shutdown(socket.SHUT_WR)
            except OSError:
                pass
    except (ConnectionResetError, ConnectionAbortedError, BrokenPipeError) as e:
        sent_err = type(e).__name__
    except socket.timeout:
        sent_err = 'SEND_TIMEOUT'
    buf = b''
    err = None
    try:
        while True:
            d = s.recv(65536)
            if not d:
                break
            buf += d
    except socket.timeout:
        err = 'TIMEOUT'
    except (ConnectionResetError, ConnectionAbortedError):
        err = 'RESET'
    finally:
        s.close()
    r = parse_response(buf) if buf else Resp(err=err or ('RESET' if sent_err else 'EMPTY'))
    if buf and err and r.status is None:
        r.err = err
    r.sent_err = sent_err
    return r


def build(method, path, headers, body=None, version='HTTP/1.1', close=True):
    lines = ['%s %s %s' % (method, path, version)]
    for k, v in headers:
        lines.append('%s: %s' % (k, v))
    if close and not any(k.lower() == 'connection' for k, _ in headers):
        lines.append('Connection: close')
    head = ('\r\n'.join(lines) + '\r\n\r\n').encode('latin-1', 'replace')
    return head + (body or b'')


class Ctx:
    def __init__(self, port, profile, results, quiet=False):
        self.port, self.profile, self.results, self.quiet = port, profile, results, quiet
        self.group = ''

    @property
    def host(self):
        return 'localhost:%d' % self.port

    def req(self, method, path, headers=None, body=None, host='default', origin=None, ctype=None,
            cl='auto', version='HTTP/1.1', timeout=8.0, half_close=False, extra_chunks=None):
        hs = []
        if host == 'default':
            hs.append(('Host', self.host))
        elif host is not None:
            hs.append(('Host', host))
        if origin is not None:
            hs.append(('Origin', origin))
        if ctype is not None:
            hs.append(('Content-Type', ctype))
        for k, v in (headers or []):
            hs.append((k, v))
        if body is not None and cl == 'auto':
            hs.append(('Content-Length', str(len(body))))
        elif cl not in ('auto', None):
            hs.append(('Content-Length', str(cl)))
        data = build(method, path, hs, body, version)
        return raw(self.port, [data] + (extra_chunks or []), timeout=timeout, half_close=half_close)

    def get(self, path, **kw):
        return self.req('GET', path, **kw)

    def post_json(self, path, obj=None, rawbody=None, **kw):
        body = rawbody if rawbody is not None else json.dumps(obj if obj is not None else {}).encode('utf-8')
        kw.setdefault('ctype', 'application/json')
        return self.req('POST', path, body=body, **kw)

    def upload(self, data, fname='probe.xlsx', ctype='application/octet-stream', cl='auto', **kw):
        hs = [('X-Filename', urllib.parse.quote(fname))]
        return self.req('POST', '/api/upload', headers=hs, body=data, ctype=ctype, cl=cl, **kw)

    def rec(self, pid, desc, r, want=None, note=''):
        state = r.state if isinstance(r, Resp) else r
        ok = want is None or str(state) == str(want)
        row = {'id': pid, 'group': self.group, 'desc': desc, 'state': str(state), 'want': None if want is None else str(want),
               'surprise': not ok, 'note': note}
        if isinstance(r, Resp):
            row['ctype'] = r.h('Content-Type')
            row['body'] = r.text()[:140].replace('\n', ' ')
        self.results.append(row)
        if not self.quiet:
            print('%-6s %-62s %-8s %s%s' % (pid, desc[:62], state, '' if ok else 'SURPRISE(want %s) ' % want, (note or '')[:60]))
        return r


# --------------------------------------------------------------------------- xlsx builders

NS_M = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
NS_R = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
NS_P = 'http://schemas.openxmlformats.org/package/2006/relationships'
CT_XML = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
          '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
          '<Default Extension="xml" ContentType="application/xml"/></Types>')
ROOT_RELS = ('<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="%s"><Relationship Id="rId1" Type="%s/officeDocument" '
             'Target="xl/workbook.xml"/></Relationships>' % (NS_P, NS_R))
WORKBOOK = ('<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="%s" xmlns:r="%s"><sheets>'
            '<sheet name="S1" sheetId="1" r:id="rId1"/></sheets></workbook>' % (NS_M, NS_R))


def wb_rels(target='worksheets/sheet1.xml'):
    return ('<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="%s"><Relationship Id="rId1" Type="%s/worksheet" Target="%s"/>'
            '</Relationships>' % (NS_P, NS_R, target))


def esc(s):
    return s.replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;')


def qtext(tag):
    return 'Probe question %s: which option is right?\nA. alpha\nB. beta\nC. gamma\nD. delta' % tag


def sheet_xml(tag, raw_stem=None, prolog='<?xml version="1.0" encoding="UTF-8"?>', doctype=''):
    stem = raw_stem if raw_stem is not None else esc(qtext(tag))
    return ('%s%s<worksheet xmlns="%s"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>%s</t></is></c>'
            '<c r="C1" t="inlineStr"><is><t>\u3010\u7b54\u6848\u3011B</t></is></c></row></sheetData></worksheet>'
            % (prolog, doctype, NS_M, stem))


def zip_bytes(parts, method=zipfile.ZIP_DEFLATED):
    bio = io.BytesIO()
    with zipfile.ZipFile(bio, 'w', method) as z:
        for name, data in parts:
            z.writestr(name, data if isinstance(data, bytes) else data.encode('utf-8'))
    return bio.getvalue()


def std_parts(sheet, sheet_name='xl/worksheets/sheet1.xml', target='worksheets/sheet1.xml', workbook=WORKBOOK, rels=None):
    return [('[Content_Types].xml', CT_XML), ('_rels/.rels', ROOT_RELS), ('xl/workbook.xml', workbook),
            ('xl/_rels/workbook.xml.rels', rels if rels is not None else wb_rels(target)), (sheet_name, sheet)]


def good_xlsx(tag='t'):
    return zip_bytes(std_parts(sheet_xml(tag)))


def patch_central(zb, name, usize=None, crc=None, csize=None, flags=None, method=None):
    """Rewrite fields of the central-directory record of `name` (declared sizes / CRC / flags / method)."""
    b = bytearray(zb)
    nb = name.encode()
    pos = 0
    while True:
        pos = b.find(b'PK\x01\x02', pos)
        if pos < 0:
            raise KeyError(name)
        fl = struct.unpack_from('<H', b, pos + 28)[0]
        if bytes(b[pos + 46:pos + 46 + fl]) == nb:
            break
        pos += 4
    if flags is not None:
        struct.pack_into('<H', b, pos + 8, flags)
    if method is not None:
        struct.pack_into('<H', b, pos + 10, method)
    if crc is not None:
        struct.pack_into('<I', b, pos + 16, crc)
    if csize is not None:
        struct.pack_into('<I', b, pos + 20, csize)
    if usize is not None:
        struct.pack_into('<I', b, pos + 24, usize)
    return bytes(b)


def big_sheet(total_bytes, tag='big'):
    """A syntactically valid worksheet whose XML is ~total_bytes long (highly compressible)."""
    head = ('<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="%s"><sheetData>' % NS_M)
    tail = '</sheetData></worksheet>'
    unit = '<row r="%d"/>'
    body = []
    n, size = 1, 0
    while size < total_bytes - len(head) - len(tail):
        s = unit % n
        body.append(s)
        size += len(s)
        n += 1
        if n > 5_000_000:
            break
    return head + ''.join(body) + tail


# --------------------------------------------------------------------------- probe groups

def g_host(c):
    c.group = 'host'
    P = c.port
    prof = c.profile
    allowed_local = prof in ('default', 'hosts', 'anyhost', 'shutdown')
    c.rec('H01', 'Host: localhost:PORT', c.get('/api/info'), 200)
    c.rec('H02', 'Host: 127.0.0.1:PORT', c.get('/api/info', host='127.0.0.1:%d' % P), 200)
    c.rec('H03', 'Host: [::1]:PORT', c.get('/api/info', host='[::1]:%d' % P), 200)
    c.rec('H04', 'Host: LOCALHOST. (upper case + trailing dot)', c.get('/api/info', host='LOCALHOST.'), 200)
    c.rec('H05', 'Host: localhost (no port)', c.get('/api/info', host='localhost'), 200)
    c.rec('H06', 'Host: localhost:99999999 (nonsense port)', c.get('/api/info', host='localhost:99999999'), 200)
    c.rec('H07', 'Host: localhost:abc', c.get('/api/info', host='localhost:abc'), 200)
    ev = 200 if prof == 'anyhost' else 403
    c.rec('H10', 'Host: evil.example', c.get('/api/info', host='evil.example'), ev)
    c.rec('H11', 'Host: evil.example:80', c.get('/api/info', host='evil.example:80'), ev)
    c.rec('H12', 'no Host header (HTTP/1.0 request)', c.get('/api/info', host=None, version='HTTP/1.0'), ev)
    c.rec('H13', 'empty Host header', c.get('/api/info', host=''), ev)
    c.rec('H14', 'Host: user@localhost', c.get('/api/info', host='user@localhost'), ev)
    c.rec('H15', 'Host: localhost@evil.com', c.get('/api/info', host='localhost@evil.com'), ev)
    c.rec('H16', 'Host: ::1 (no brackets)', c.get('/api/info', host='::1'), ev)
    c.rec('H17', 'Host: [::1 (unclosed bracket)', c.get('/api/info', host='[::1'), ev)
    c.rec('H18', 'Host: localhost.evil.com', c.get('/api/info', host='localhost.evil.com'), ev)
    c.rec('H19', 'Host: "localhost, evil.com"', c.get('/api/info', host='localhost, evil.com'), ev)
    c.rec('H20', 'Host: 127.0.0.1. (trailing dot IP)', c.get('/api/info', host='127.0.0.1.'), 200)
    c.rec('H21', 'two Host headers: localhost first, evil second',
          c.get('/api/info', host=None, headers=[('Host', c.host), ('Host', 'evil.example')]), 200,
          note='Python uses the first Host header')
    c.rec('H22', 'two Host headers: evil first, localhost second',
          c.get('/api/info', host=None, headers=[('Host', 'evil.example'), ('Host', c.host)]), ev)
    c.rec('H23', 'absolute-form request line (http://evil/..) + Host: localhost',
          c.get('http://evil.example/api/info', host=c.host), 200, note='only the Host header is checked')
    c.rec('H24', 'HEAD /api/info, Host: evil.example', c.req('HEAD', '/api/info', host='evil.example'), ev)
    r = c.req('HEAD', '/api/info')
    c.rec('H25', 'HEAD /api/info, Host: localhost (no body, Content-Length present)', r, 200,
          note='CL=%s body=%d' % (r.h('Content-Length'), len(r.body)))
    c.rec('H26', 'static file / with evil Host (Host check covers static too)', c.get('/', host='evil.example'), ev)
    r = c.get('/api/info', host='evil.example')
    c.rec('H27', '403 text names the rejected host and the env var', r, ev,
          note='mentions XEXAM_ALLOWED_HOSTS=%s' % ('XEXAM_ALLOWED_HOSTS' in r.text()))
    if prof == 'hosts':
        c.rec('H30', 'Host: xexam.example.com (allow-listed)', c.get('/api/info', host='xexam.example.com'), 200)
        c.rec('H31', 'Host: XEXAM.Example.COM:8443', c.get('/api/info', host='XEXAM.Example.COM:8443'), 200)
        c.rec('H32', 'Host: 192.168.1.20:8080 (allow-listed LAN IP)', c.get('/api/info', host='192.168.1.20:8080'), 200)
        c.rec('H33', 'Host: pub.example.org (only in ALLOWED_ORIGINS -> not a Host)', c.get('/api/info', host='pub.example.org'), 403)
        c.rec('H34', 'Host: other.example.com', c.get('/api/info', host='other.example.com'), 403)
        c.rec('H35', 'Host: xexam.example.com.evil.com', c.get('/api/info', host='xexam.example.com.evil.com'), 403)
        c.rec('H36', 'Host: localhost still allowed when list is extended', c.get('/api/info', host='localhost:1'), 200)


def g_origin(c):
    c.group = 'origin'
    prof = c.profile
    ep = '/api/import/cancel'
    P = c.port

    def post(origin, **kw):
        return c.post_json(ep, {}, origin=origin, **kw)

    c.rec('O01', 'POST, no Origin header', post(None), 200)
    c.rec('O02', 'Origin: http://localhost:PORT (same origin)', post('http://localhost:%d' % P), 200)
    c.rec('O03', 'Origin: https://localhost (scheme ignored)', post('https://localhost'), 200)
    c.rec('O04', 'Origin: http://localhost:9999 (port ignored)', post('http://localhost:9999'), 200)
    c.rec('O05', 'Origin: HTTP://LOCALHOST (case)', post('HTTP://LOCALHOST'), 200)
    c.rec('O06', 'Origin: http://localhost/ (trailing slash)', post('http://localhost/'), 200)
    c.rec('O07', 'Origin: http://localhost/x?y (path ignored)', post('http://localhost/x?y'), 200)
    c.rec('O08', 'Origin: https://localhost. (trailing dot)', post('https://localhost.'), 200)
    c.rec('O10', 'Origin: http://127.0.0.1:PORT vs Host localhost', post('http://127.0.0.1:%d' % P), 403)
    c.rec('O11', 'Origin: localhost:PORT (no scheme)', post('localhost:%d' % P), 403)
    c.rec('O12', 'Origin: null', post('null'), 403)
    c.rec('O13', 'Origin: (empty value)', post(''), 403, note='present-but-empty is rejected, absent is allowed')
    c.rec('O14', 'Origin: file://', post('file://'), 403)
    c.rec('O15', 'Origin: http://localhost@evil.com (userinfo)', post('http://localhost@evil.com'), 403)
    c.rec('O16', 'Origin: http://localhost.evil.com', post('http://localhost.evil.com'), 403)
    c.rec('O17', 'Origin: http://evil.com', post('http://evil.com'), 403)
    c.rec('O18', 'Origin: ftp://localhost', post('ftp://localhost'), 403)
    c.rec('O19', 'Origin: http:// (no host)', post('http://'), 403)
    c.rec('O20', 'two Origin headers: localhost then evil',
          c.post_json(ep, {}, headers=[('Origin', 'http://localhost'), ('Origin', 'http://evil.com')]), 200, note='first wins')
    c.rec('O21', 'two Origin headers: evil then localhost',
          c.post_json(ep, {}, headers=[('Origin', 'http://evil.com'), ('Origin', 'http://localhost')]), 403)
    c.rec('O22', 'Origin http://[::1]:PORT with Host [::1]:PORT', post('http://[::1]:%d' % P, host='[::1]:%d' % P), 200)
    c.rec('O23', 'GET with Origin: http://evil.com (Origin only checked on POST)', c.get('/api/info', origin='http://evil.com'), 200)
    r = post('http://evil.com')
    c.rec('O24', '403 body is JSON {"error": ...}', r, 403, note='json=%s' % (isinstance(r.json(), dict) and 'error' in r.json()))
    ev = 200 if prof == 'anyhost' else 403
    r = c.post_json(ep, {}, host='evil.example', origin='http://evil.example')
    c.rec('O25', 'Host evil AND Origin evil: Host rule fires first', r, ev,
          note='msg mentions Host=%s' % ('Host' in r.text() and '\u8de8' not in r.text()))
    r = c.post_json(ep, {}, host='evil.example', origin='http://other.example')
    c.rec('O26', 'Host evil, Origin other: which message?', r, 403 if prof != 'anyhost' else 403, note=r.text()[:50])
    c.rec('O27', 'CORS preflight OPTIONS with Origin', c.req('OPTIONS', '/api/reset', origin='http://evil.com',
          headers=[('Access-Control-Request-Method', 'POST')]), 501, note='no OPTIONS handler')
    r = c.get('/api/info', origin='http://evil.com')
    c.rec('O28', 'no Access-Control-Allow-Origin on a cross-origin GET', r, 200,
          note='ACAO=%s' % r.h('Access-Control-Allow-Origin'))
    if prof == 'hosts':
        c.rec('O30', 'Host xexam.example.com + Origin https://xexam.example.com', post('https://xexam.example.com', host='xexam.example.com'), 200)
        c.rec('O31', 'Host 192.168.1.20:8080 + Origin https://pub.example.org (ALLOWED_ORIGINS)', post('https://pub.example.org', host='192.168.1.20:8080'), 200)
        c.rec('O32', 'Host 192.168.1.20 + Origin https://xexam.example.com (ALLOWED_HOSTS also count as origins)', post('https://xexam.example.com', host='192.168.1.20'), 200)
        c.rec('O33', 'Host 192.168.1.20 + Origin https://evil.com', post('https://evil.com', host='192.168.1.20'), 403)
        c.rec('O34', 'Host xexam.example.com + Origin https://xexam.example.com.evil.com', post('https://xexam.example.com.evil.com', host='xexam.example.com'), 403)
    if prof == 'anyhost':
        c.rec('O35', 'anyhost: Host anything + Origin same host', post('https://anything.example', host='anything.example'), 200)
        c.rec('O36', 'anyhost: Host anything + Origin other (Origin check still on)', post('https://evil.example', host='anything.example'), 403)
        c.rec('O37', 'anyhost: Host 10.0.0.5:8080 + Origin https://pub (no ALLOWED_ORIGINS)', post('https://pub.example', host='10.0.0.5:8080'), 403)


def g_method(c):
    c.group = 'method'
    ev = 200 if c.profile == 'anyhost' else 403
    for m in ('PUT', 'DELETE', 'PATCH', 'OPTIONS', 'TRACE'):
        r = c.req(m, '/api/bank')
        c.rec('M-' + m, '%s /api/bank' % m, r, 501, note='allow=%s ctype=%s' % (r.h('Allow'), r.h('Content-Type')))
    r = c.req('PUT', '/api/bank', host='evil.example')
    c.rec('M-PUTevil', 'PUT with evil Host (no host check: 501 before any rule)', r, 501)
    r = c.req('PUT', '/api/bank')
    c.rec('M-hdr', '501 page: nosniff/no-store present?', r, 501,
          note='nosniff=%s no-store=%s server=%s' % (r.h('X-Content-Type-Options'), r.h('Cache-Control'), r.h('Server')))
    c.rec('M-POSTinfo', 'POST /api/info (GET-only path)', c.post_json('/api/info', {}), 404)
    c.rec('M-GETupload', 'GET /api/upload', c.get('/api/upload'), 404)
    c.rec('M-GETshutdown', 'GET /api/shutdown', c.get('/api/shutdown'), 404)
    c.rec('M-GETconfirm', 'GET /api/import/confirm', c.get('/api/import/confirm'), 404)
    c.rec('M-GETunknown', 'GET /api/nope (unknown api path -> static 404, text/plain)', c.get('/api/nope'), 404)
    r = c.post_json('/api/nope', {})
    c.rec('M-POSTunknown', 'POST /api/nope with JSON -> 404 JSON', r, 404, note=r.text()[:40])
    c.rec('M-POSTunknownCT', 'POST /api/nope without JSON content type -> 415 first', c.req('POST', '/api/nope', body=b'x', ctype='text/plain'), 415)
    c.rec('M-info/', 'GET /api/info/ (trailing slash)', c.get('/api/info/'), 404)
    c.rec('M-info?', 'GET /api/info?x=1 (query ignored)', c.get('/api/info?x=1'), 200)
    c.rec('M-//info', 'GET //api/info (double slash)', c.get('//api/info'), 404)
    c.rec('M-INFO', 'GET /API/INFO (case)', c.get('/API/INFO'), 404)


def g_ctype(c):
    c.group = 'ctype'
    c.rec('C01', 'POST /api/reset without Content-Type', c.req('POST', '/api/reset', body=b'{}'), 415)
    c.rec('C02', 'Content-Type: text/plain', c.post_json('/api/reset', {}, ctype='text/plain'), 415)
    c.rec('C03', 'Content-Type: text/plain;application/json  (CORS-safelisted by browsers!)',
          c.post_json('/api/reset', {}, ctype='text/plain;application/json'), 200, note='substring match -> accepted')
    c.rec('C04', 'Content-Type: Application/JSON (case)', c.post_json('/api/reset', {}, ctype='Application/JSON'), 415)
    c.rec('C05', 'Content-Type: application/json; charset=utf-8', c.post_json('/api/reset', {}, ctype='application/json; charset=utf-8'), 200)
    c.rec('C06', 'Content-Type: application/jsonp', c.post_json('/api/reset', {}, ctype='application/jsonp'), 200, note='substring')
    c.rec('C07', 'upload with Content-Type: application/json', c.upload(good_xlsx('c7'), ctype='application/json'), 415)
    c.rec('C08', 'upload with multipart/form-data', c.upload(good_xlsx('c8'), ctype='multipart/form-data; boundary=x'), 415)
    c.rec('C09', 'upload with text/plain;application/octet-stream', c.upload(good_xlsx('c9'), ctype='text/plain;application/octet-stream'), 200, note='substring')
    c.rec('C10', 'upload without Content-Type', c.req('POST', '/api/upload', body=good_xlsx('c10')), 415)


def g_body(c):
    c.group = 'body'
    ep = '/api/import/cancel'
    pad = lambda n: json.dumps({'a': 'x' * (n - 9)}).encode()   # exactly n bytes: {"a": "xxx"} = 9 bytes of framing
    b = pad(1_000_000)
    assert len(b) == 1_000_000
    c.rec('B01', 'JSON body of exactly 1,000,000 bytes', c.post_json(ep, rawbody=b), 200)
    b = pad(1_000_001)
    c.rec('B02', 'JSON body of 1,000,001 bytes', c.post_json(ep, rawbody=b), 413)
    c.rec('B03', 'invalid JSON', c.post_json(ep, rawbody=b'{nope'), 400)
    c.rec('B04', 'JSON array (not an object)', c.post_json(ep, rawbody=b'[1,2]'), 400)
    c.rec('B05', 'empty body with Content-Length: 0 (treated as {})', c.post_json(ep, rawbody=b''), 200)
    c.rec('B06', 'Content-Length: -1', c.req('POST', ep, ctype='application/json', body=b'{}', cl='-1'), 413)
    c.rec('B07', 'Content-Length: abc (parsed as 0)', c.req('POST', ep, ctype='application/json', body=b'{}', cl='abc'), 200)
    r = c.req('POST', ep, ctype='application/json', body=b'{"a":"bc"}', cl='1_0')
    c.rec('B08', 'Content-Length: 1_0 (Python int() accepts underscores)', r, None, note='RFC-strict servers: 400')
    r = c.req('POST', ep, ctype='application/json', body=b'{}', headers=[('Content-Length', '2'), ('Content-Length', '9999999')], cl=None)
    c.rec('B09', 'two Content-Length headers (2 then 9999999)', r, None, note='Python uses the first')
    r = c.req('POST', ep, ctype='application/json', body=None, headers=[('Transfer-Encoding', 'chunked')], cl=None,
              extra_chunks=[b'2\r\n{}\r\n0\r\n\r\n'])
    c.rec('B10', 'chunked JSON body, no Content-Length (Python ignores TE, sees empty body)', r, 200)
    r = c.req('POST', '/api/reset', ctype='application/json', body=None, headers=[('Transfer-Encoding', 'chunked')], cl=None,
              extra_chunks=[b'2\r\n{}\r\n0\r\n\r\n'])
    c.rec('B11', 'chunked body to /api/reset', r, 200)
    deep = b'{"a":' + b'[' * 200000 + b']' * 200000 + b'}'
    c.rec('B12', 'JSON nested 200000 deep (RecursionError in Python)', c.post_json(ep, rawbody=deep), None,
          note='expect EMPTY on Python (unhandled), 400 on serde_json')
    # upload size limits
    n = 20 * MIB
    c.rec('B20', 'upload exactly 20 MiB of junk (CL) -> size ok, content rejected', c.upload(b'A' * n, timeout=30), 400)
    c.rec('B21', 'upload 20 MiB + 1 byte (CL)', c.upload(b'A' * (n + 1), timeout=30), 413)
    big = b'A' * (21 * MIB)
    c.rec('B22', 'upload 21 MiB with Content-Length', c.upload(big, timeout=30), 413)
    hs = [('X-Filename', 'big.xlsx')]
    r = c.req('POST', '/api/upload', headers=hs, ctype='application/octet-stream', body=None, cl=None, timeout=30,
              extra_chunks=[big], half_close=True)
    c.rec('B23', 'upload 21 MiB WITHOUT Content-Length (close-delimited)', r, None,
          note='Python answers 400 after 0 bytes, then closes with data unread -> client often sees RESET instead of the 400')
    small = good_xlsx('b23s')
    r = c.req('POST', '/api/upload', headers=hs, ctype='application/octet-stream', body=None, cl=None, timeout=15, extra_chunks=[small], half_close=True)
    c.rec('B23s', 'valid 1.5 KiB xlsx WITHOUT Content-Length (close-delimited) -> body ignored', r, 400, note=r.text()[:50])
    chunks = []
    for i in range(0, len(big), MIB):
        piece = big[i:i + MIB]
        chunks.append(b'%x\r\n' % len(piece) + piece + b'\r\n')
    chunks.append(b'0\r\n\r\n')
    r = c.req('POST', '/api/upload', headers=hs + [('Transfer-Encoding', 'chunked')], ctype='application/octet-stream', body=None,
              cl=None, timeout=30, extra_chunks=[b''.join(chunks)])
    c.rec('B24', 'upload 21 MiB with Transfer-Encoding: chunked', r, None, note='Python ignores chunked framing; same RESET artifact possible')
    one = b'%x' % len(small) + b'\r\n' + small + b'\r\n0\r\n\r\n'
    r = c.req('POST', '/api/upload', headers=hs + [('Transfer-Encoding', 'chunked')], ctype='application/octet-stream', body=None, cl=None, timeout=15, extra_chunks=[one])
    c.rec('B24s', 'valid 1.5 KiB xlsx sent chunked -> body ignored (empty file)', r, 400, note=r.text()[:50])
    r = c.upload(b'', timeout=10)
    c.rec('B25', 'upload with empty body', r, 400, note=r.text()[:50])
    r = c.upload(b'PK' + b'\0' * 50)
    c.rec('B26', 'upload of 52 bytes (< 100)', r, 400)
    # header-level limits of the HTTP stack
    big_hdr = 'x' * 70000
    r = c.req('GET', '/api/info', headers=[('X-Big', big_hdr)])
    c.rec('B30', 'single header line of 70,000 bytes', r, None, note='http.server: 431')
    r = c.req('GET', '/api/info', headers=[('X-H%d' % i, 'v') for i in range(150)])
    c.rec('B31', '150 header lines', r, None, note='http.server: 431 above 100')
    r = c.req('GET', '/' + 'a' * 70000)
    c.rec('B32', 'request line with 70,000-char path', r, None, note='http.server: 414')


def zip_with_junk_bytes(n):
    return b'PK\x03\x04' + os.urandom(n)


def g_upload(c, tmp):
    c.group = 'upload'
    r = c.upload(good_xlsx('u01'))
    j = r.json() or {}
    c.rec('U01', 'valid minimal xlsx -> preview with token', r, 200,
          note='token len=%s added=%s usable=%s' % (len(j.get('token', '')), j.get('added'), j.get('usable')))
    c.rec('U02', 'not a zip (text, 300 bytes)', c.upload(b'hello world ' * 25), 400)
    c.rec('U03', 'legacy .xls (OLE magic)', c.upload(b'\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1' + b'\0' * 200), 400)
    c.rec('U04', 'PK + random bytes (corrupt zip)', c.upload(zip_with_junk_bytes(500)), 400)
    c.rec('U05', 'zip without xl/workbook.xml', c.upload(zip_bytes([('a.txt', 'hello' * 40)])), 400)
    dt = '<!DOCTYPE worksheet [<!ENTITY probe "x">]>'
    c.rec('U10', 'UTF-8 DOCTYPE+ENTITY in sheet1.xml', c.upload(zip_bytes(std_parts(sheet_xml('u10', doctype=dt)))), 400)
    c.rec('U11', 'UTF-8 DOCTYPE only (no ENTITY) in sheet1.xml', c.upload(zip_bytes(std_parts(sheet_xml('u11', doctype='<!DOCTYPE worksheet>')))), 400)
    wb_dt = WORKBOOK.replace('<workbook', dt + '<workbook', 1)
    c.rec('U12', 'DOCTYPE in xl/workbook.xml', c.upload(zip_bytes(std_parts(sheet_xml('u12'), workbook=wb_dt))), 400)
    rels_dt = wb_rels().replace('<Relationships', dt + '<Relationships', 1)
    c.rec('U13', 'DOCTYPE in xl/_rels/workbook.xml.rels', c.upload(zip_bytes(std_parts(sheet_xml('u13'), rels=rels_dt))), 400)
    p = std_parts(sheet_xml('u14')) + [('docProps/app.xml', '<?xml version="1.0"?>' + dt + '<Properties/>')]
    c.rec('U14', 'DOCTYPE in an unrelated .xml part (docProps/app.xml)', c.upload(zip_bytes(p)), 400, note='all .xml/.rels members are scanned')
    p = std_parts(sheet_xml('u15')) + [('docProps/note.txt', dt)]
    c.rec('U15', 'DOCTYPE text in a non-xml part (note.txt)', c.upload(zip_bytes(p)), 200, note='only .xml/.rels scanned')
    # encodings that hide the ASCII marker
    for tag, enc, pid in (('u16', 'utf-16-le', 'U16'), ('u17', 'utf-16-be', 'U17'), ('u18', 'utf-16', 'U18')):
        text = sheet_xml(tag, raw_stem=esc('Probe question %s &probe; which option is right?\nA. alpha\nB. beta\nC. gamma\nD. delta' % tag).replace('&amp;probe;', '&probe;'),
                         prolog='<?xml version="1.0" encoding="UTF-16"?>', doctype='<!DOCTYPE worksheet [<!ENTITY probe "ENTITY-EXPANDED-OK">]>')
        data = text.encode(enc)
        if enc != 'utf-16':
            data = (b'\xff\xfe' if enc == 'utf-16-le' else b'\xfe\xff') + data
        r = c.upload(zip_bytes(std_parts(data)))
        c.rec(pid, 'DOCTYPE+ENTITY inside a %s encoded sheet1.xml' % enc, r, None,
              note='marker hidden from byte scan; accepted=%s' % (r.status == 200))
    # marker in a part that the parser reads but check_xlsx does not scan (name not ending in .xml/.rels)
    text = sheet_xml('u19', raw_stem=esc('Probe question u19 &probe; which option is right?\nA. alpha\nB. beta\nC. gamma\nD. delta').replace('&amp;probe;', '&probe;'),
                     doctype='<!DOCTYPE worksheet [<!ENTITY probe "ENTITY-EXPANDED-OK">]>')
    r = c.upload(zip_bytes(std_parts(text, sheet_name='xl/worksheets/sheet1.dat', target='worksheets/sheet1.dat')))
    c.rec('U19', 'UTF-8 DOCTYPE+ENTITY in sheet named sheet1.dat (relationship target)', r, None, note='accepted=%s' % (r.status == 200))
    r = c.upload(zip_bytes(std_parts(text, sheet_name='xl/worksheets/sheet1.XML', target='worksheets/sheet1.XML')))
    c.rec('U20', 'same, sheet named sheet1.XML (upper-case extension)', r, None, note='accepted=%s' % (r.status == 200))
    # billion laughs hidden in UTF-16 (so the byte scan does not see it)
    ents = ['<!ENTITY a0 "0123456789">'] + ['<!ENTITY a%d "%s">' % (i, '&a%d;' % (i - 1) * 10) for i in range(1, 9)]
    bl = ('<?xml version="1.0" encoding="UTF-16"?><!DOCTYPE worksheet [%s]><worksheet xmlns="%s"><sheetData><row r="1">'
          '<c r="A1" t="inlineStr"><is><t>&a8;</t></is></c></row></sheetData></worksheet>' % (''.join(ents), NS_M))
    t0 = time.time()
    r = c.upload(zip_bytes(std_parts(b'\xff\xfe' + bl.encode('utf-16-le'))), timeout=60)
    c.rec('U21', 'billion-laughs (10^9) in UTF-16 sheet (expat amplification guard?)', r, 400, note='%.2fs %s' % (time.time() - t0, r.text()[:70]))
    # XXE with an external file
    marker = os.path.join(tmp, 'xxe_marker.txt')
    with open(marker, 'w', encoding='utf-8') as f:
        f.write('XXE-SECRET-CONTENT')
    uri = 'file:///' + marker.replace('\\', '/').lstrip('/')
    xx = sheet_xml('u22', raw_stem='Probe question u22 &x; which option is right?\nA. alpha\nB. beta\nC. gamma\nD. delta',
                   prolog='<?xml version="1.0" encoding="UTF-16"?>', doctype='<!DOCTYPE worksheet [<!ENTITY x SYSTEM "%s">]>' % uri)
    r = c.upload(zip_bytes(std_parts(b'\xff\xfe' + xx.encode('utf-16-le'))))
    c.rec('U22', 'XXE: external SYSTEM entity (file) in UTF-16 sheet', r, None, note='resp=%s' % r.text()[:70])
    # forged / hostile zip headers
    base = zip_bytes(std_parts(sheet_xml('f')))
    r = c.upload(patch_central(base, 'xl/worksheets/sheet1.xml', usize=150 * MIB))
    c.rec('F01', 'central dir declares sheet1.xml = 150 MiB (> 100 MiB cap)', r, 400, note=r.text()[:50])
    parts = [('[Content_Types].xml', CT_XML), ('_rels/.rels', ROOT_RELS), ('xl/workbook.xml', WORKBOOK), ('xl/_rels/workbook.xml.rels', wb_rels()),
             ('xl/worksheets/sheet1.xml', sheet_xml('f2'))]
    z4 = zip_bytes(parts + [('x%d.bin' % i, b'z') for i in range(4)])
    for i in range(4):
        z4 = patch_central(z4, 'x%d.bin' % i, usize=90 * MIB)
    c.rec('F02', '4 entries declaring 90 MiB each (total 360 MiB > 300 MiB cap)', c.upload(z4), 400)
    r = c.upload(patch_central(base, 'xl/worksheets/sheet1.xml', usize=10))
    c.rec('F03', 'declared size shrunk to 10 bytes, CRC untouched', r, 400, note=r.text()[:70])
    # forged small size AND matching CRC while real data is large (what does the reader return?)
    real = big_sheet(30 * MIB)
    zb = zip_bytes(std_parts(real))
    prefix = real.encode()[:200]
    forged = patch_central(zb, 'xl/worksheets/sheet1.xml', usize=200, crc=zlib.crc32(prefix) & 0xffffffff)
    t0 = time.time()
    r = c.upload(forged, timeout=60)
    c.rec('F04', '30 MiB real data, header says 200 bytes + matching CRC (bounded read?)', r, 400, note='%.2fs %s' % (time.time() - t0, r.text()[:60]))
    t0 = time.time()
    r = c.upload(zb, timeout=120)
    c.rec('F05', 'honest 30 MiB highly compressible sheet (%d KiB zip)' % (len(zb) // 1024), r, None, note='%.2fs %s' % (time.time() - t0, r.text()[:60]))
    pe = std_parts(sheet_xml('f6'))
    enc = zip_bytes(pe)
    c.rec('F06', 'sheet entry flagged encrypted (bit 0)', c.upload(patch_central(enc, 'xl/worksheets/sheet1.xml', flags=1)), 400)
    c.rec('F07', 'sheet entry with unknown compression method 77', c.upload(patch_central(enc, 'xl/worksheets/sheet1.xml', method=77)), 400)
    many = zip_bytes(std_parts(sheet_xml('f8')) + [('pad/%d.bin' % i, b'') for i in range(30000)])
    t0 = time.time()
    r = c.upload(many, timeout=60)
    c.rec('F08', '30,000 empty entries (no entry-count cap in check_xlsx)', r, None, note='%.2fs zip=%dKiB' % (time.time() - t0, len(many) // 1024))
    # file name handling
    r = c.upload(good_xlsx('n1'), fname='evil.exe')
    c.rec('N01', 'X-Filename: evil.exe with valid xlsx bytes (extension not checked server-side)', r, 200)
    r = c.req('POST', '/api/upload', ctype='application/octet-stream', body=good_xlsx('n2'),
              headers=[('X-Filename', '..%2F..%2Fetc%2Fpasswd%00%0d%0a' + 'a' * 300)])
    c.rec('N02', 'X-Filename traversal / CRLF / 300 chars (stored name is cleaned)', r, 200, note='filename=%r' % ((r.json() or {}).get('filename')))
    r = c.req('POST', '/api/upload', ctype='application/octet-stream', body=good_xlsx('n3'))
    c.rec('N03', 'no X-Filename header', r, 200, note='filename=%r' % ((r.json() or {}).get('filename')))


def g_entity(c):
    """Confirm an upload whose DOCTYPE was hidden from the scan and look at the stored question text."""
    c.group = 'entity'
    text = sheet_xml('e1', raw_stem=esc('Probe question e1 &probe; which option is right?\nA. alpha\nB. beta\nC. gamma\nD. delta').replace('&amp;probe;', '&probe;'),
                     prolog='<?xml version="1.0" encoding="UTF-16"?>', doctype='<!DOCTYPE worksheet [<!ENTITY probe "ENTITY-EXPANDED-OK">]>')
    r = c.upload(zip_bytes(std_parts(b'\xff\xfe' + text.encode('utf-16-le'))))
    tok = (r.json() or {}).get('token')
    if r.status != 200 or not tok:
        c.rec('E01', 'UTF-16 entity file accepted at upload', r, 200)
        return
    r2 = c.post_json('/api/import/confirm', {'token': tok})
    bank = c.get('/api/bank').json() or {}
    stems = [q.get('stem', '') for q in bank.get('questions', [])]
    hit = any('ENTITY-EXPANDED-OK' in s for s in stems)
    c.rec('E01', 'entity declared in UTF-16 DOCTYPE expanded into stored question text?', r2, 200, note='expanded_in_bank=%s' % hit)


def g_pending(c, tmp):
    c.group = 'pending'

    def new(tag):
        r = c.upload(good_xlsx(tag))
        return (r.json() or {}).get('token')

    t = new('p1')
    import re
    c.rec('P01', 'token format (token_urlsafe(16): 22 chars [A-Za-z0-9_-])', Resp(200), 200, note='token=%s... ok=%s' % ((t or '')[:4], bool(t and re.fullmatch(r'[A-Za-z0-9_-]{22}', t))))
    c.rec('P02', 'confirm with wrong token', c.post_json('/api/import/confirm', {'token': 'x' * 22}), 409)
    c.rec('P03', 'confirm with no token field', c.post_json('/api/import/confirm', {}), 409)
    c.rec('P04', 'confirm with token = 5 (number)', c.post_json('/api/import/confirm', {'token': 5}), 409)
    c.rec('P05', 'confirm with token = null', c.post_json('/api/import/confirm', {'token': None}), 409)
    c.rec('P06', 'confirm with token = list', c.post_json('/api/import/confirm', {'token': [t]}), 409)
    r = c.post_json('/api/import/confirm', {'token': 'caf\u00e9-non-ascii'})
    c.rec('P07', 'confirm with a NON-ASCII token while a pending item exists (compare_digest TypeError?)', r, None,
          note='Python: unhandled TypeError -> EMPTY')
    r = c.post_json('/api/import/confirm', {'token': t})
    c.rec('P08', 'confirm with the right token after wrong attempts (wrong tries do not consume)', r, 200)
    c.rec('P09', 'replay: confirm the same token again', c.post_json('/api/import/confirm', {'token': t}), 409)
    # single slot: second upload replaces the first
    ta, tb = new('p2a'), new('p2b')
    c.rec('P10', 'two uploads: confirm the FIRST token (slot was replaced)', c.post_json('/api/import/confirm', {'token': ta}), 409)
    c.rec('P11', 'two uploads: confirm the SECOND token', c.post_json('/api/import/confirm', {'token': tb}), 200)
    # cancel
    t = new('p3')
    c.rec('P12', 'cancel (no token needed)', c.post_json('/api/import/cancel', {}), 200)
    c.rec('P13', 'confirm after cancel', c.post_json('/api/import/confirm', {'token': t}), 409)
    c.rec('P14', 'cancel with nothing pending', c.post_json('/api/import/cancel', {}), 200)
    # two tabs: B's cancel kills A's pending
    ta = new('p4a')
    c.post_json('/api/import/cancel', {})
    c.rec('P15', 'tab A uploaded, tab B calls cancel -> A confirm', c.post_json('/api/import/confirm', {'token': ta}), 409)
    # empty workbook: parses to zero questions
    empty = zip_bytes(std_parts('<?xml version="1.0"?><worksheet xmlns="%s"><sheetData/></worksheet>' % NS_M))
    r = c.upload(empty)
    tk = (r.json() or {}).get('token')
    c.rec('P16', 'upload with zero usable questions still issues a token', r, 200, note='usable=%s' % (r.json() or {}).get('usable'))
    c.rec('P17', 'confirm of an empty plan -> 400 and the pending item is consumed', c.post_json('/api/import/confirm', {'token': tk}), 400)
    c.rec('P18', 'confirm again after that 400', c.post_json('/api/import/confirm', {'token': tk}), 409)
    # concurrency
    imp0 = (c.get('/api/imports').json() or {}).get('imports', [])
    tk = new('p5')
    out = []

    def go():
        out.append(c.post_json('/api/import/confirm', {'token': tk}, timeout=30).state)
    ths = [threading.Thread(target=go) for _ in range(16)]
    [x.start() for x in ths]
    [x.join() for x in ths]
    imp1 = (c.get('/api/imports').json() or {}).get('imports', [])
    c.rec('P19', '16 parallel confirms with the same token', Resp(200 if out.count(200) == 1 and out.count(409) == 15 else 500), 200,
          note='results: 200 x%d, 409 x%d, other=%s; imports rows +%d' % (out.count(200), out.count(409), [o for o in out if o not in (200, 409)], len(imp1) - len(imp0) if len(imp1) < 12 else -1))
    # upload while a confirm of the previous one is not done: just replacement semantics already shown (P10)
    c.rec('P20', 'GET /api/imports history after the above', c.get('/api/imports'), 200)


def g_attempt(c):
    c.group = 'attempt'
    ok = lambda n=1, **kw: dict({'results': [{'id': 'q%d' % i, 'correct': True} for i in range(n)], 'sheets': ['S1'], 'mode': 'all', 'seconds': 5}, **kw)
    c.post_json('/api/reset', {})
    c.rec('A01', 'valid attempt', c.post_json('/api/attempt', ok(2)), 200)
    c.rec('A02', 'empty results', c.post_json('/api/attempt', {'results': []}), 400)
    c.rec('A03', 'results missing', c.post_json('/api/attempt', {}), 400)
    c.rec('A04', 'results = dict (TypeError in slice)', c.post_json('/api/attempt', {'results': {'a': 1}}), None, note='Python: unhandled -> EMPTY')
    c.rec('A05', 'results = 5 (int)', c.post_json('/api/attempt', {'results': 5}), None, note='Python: unhandled -> EMPTY')
    c.rec('A06', 'results = "abc" (string)', c.post_json('/api/attempt', {'results': 'abc'}), 400)
    c.rec('A07', 'ids not str / correct not bool are dropped (all dropped -> 400)',
          c.post_json('/api/attempt', {'results': [{'id': 1, 'correct': True}, {'id': 'a', 'correct': 1}, {'id': 'b', 'correct': 'true'}]}), 400)
    r = c.post_json('/api/attempt', ok(2001))
    j = r.json() or {}
    c.rec('A08', '2001 results: truncated to 2000', r, 200, note='history[0].total=%s' % ((j.get('history') or [{}])[0].get('total')))
    r = c.post_json('/api/attempt', {'results': [{'id': 'Z' * 200, 'correct': False}], 'sheets': ['s' * 200] * 60, 'mode': 'evil', 'seconds': 10 ** 12})
    j = r.json() or {}
    h0 = (j.get('history') or [{}])[0]
    id_lens = [len(k) for k in (j.get('questions') or {})] or [0]
    c.rec('A09', 'id 200 chars, 60 sheets x 200 chars, mode evil, seconds 10^12', r, 200,
          note='mode=%s seconds=%s sheets=%d x %d; longest id kept=%d chars' % (h0.get('mode'), h0.get('seconds'), len(h0.get('sheets', [])),
                                                                              len((h0.get('sheets') or [''])[0]), max(id_lens)))
    for pid, val in (('A10', -5), ('A11', 'abc'), ('A12', 1.9), ('A13', True), ('A14', [1])):
        r = c.post_json('/api/attempt', ok(1, seconds=val))
        c.rec(pid, 'seconds = %r' % (val,), r, 200, note='stored=%s' % (((r.json() or {}).get('history') or [{}])[0].get('seconds')))
    c.rec('A15', 'seconds = Infinity (JSON extension) -> OverflowError', c.post_json('/api/attempt', rawbody=b'{"results":[{"id":"a","correct":true}],"seconds":Infinity}'), None,
          note='Python: unhandled -> EMPTY; serde_json rejects')
    c.rec('A16', 'seconds = NaN', c.post_json('/api/attempt', rawbody=b'{"results":[{"id":"a","correct":true}],"seconds":NaN}'), None, note='int(nan) ValueError -> 0')
    r = c.post_json('/api/attempt', ok(1, results=[{'id': 'dup', 'correct': True}, {'id': 'dup', 'correct': False}]))
    c.rec('A17', 'same id twice in one attempt', r, 200, note='stats[dup]=%s' % ((r.json() or {}).get('questions', {}).get('dup')))
    for i in range(65):
        c.post_json('/api/attempt', ok(1))
    r = c.get('/api/progress')
    c.rec('A18', 'history is capped at 60 after 65+ attempts', r, 200, note='len=%d' % len((r.json() or {}).get('history', [])))
    r = c.post_json('/api/attempt', {'results': [{'id': 'not-a-real-question-%d' % i, 'correct': True} for i in range(2000)]})
    c.rec('A19', '2000 ids that match no question are all stored in stats (no FK, no existence check)', r, 200,
          note='stats rows=%d' % len((r.json() or {}).get('questions', {})))
    r = c.post_json('/api/reset', {})
    c.rec('A20', 'reset clears stats+attempts', r, 200, note='stats=%d history=%d' % (len((r.json() or {}).get('questions', {})), len((r.json() or {}).get('history', []))))


def g_static(c):
    c.group = 'static'
    for pid, p, ct in (('S01', '/', 'text/html'), ('S02', '/index.html', 'text/html'), ('S03', '/app.js', 'javascript'),
                       ('S04', '/style.css', 'text/css'), ('S05', '/icon.svg', 'svg'), ('S06', '/apple-touch-icon.png', 'png')):
        r = c.get(p)
        c.rec(pid, 'GET %s' % p, r, 200, note='ctype=%s %s' % (r.h('Content-Type'), '' if ct in (r.h('Content-Type') or '') else 'UNEXPECTED CTYPE'))
    trav = [
        ('T01', '/../server.py'), ('T02', '/..%2fserver.py'), ('T03', '/%2e%2e/server.py'), ('T04', '/..%5cserver.py'),
        ('T05', '/....//server.py'), ('T06', '//server.py'), ('T07', '/%2e%2e%2f%2e%2e%2fdata/xexam.db'), ('T08', '/%00'),
        ('T09', '/app.js%00.png'), ('T10', '/%252e%252e/server.py'), ('T11', '/app/db.py'), ('T12', '/server.py'), ('T13', '/VERSION'),
        ('T14', '/xexam.db'), ('T15', '/.git/config'), ('T16', '/..\\server.py'), ('T17', '/C:/Windows/win.ini'), ('T18', '/%c0%ae%c0%ae/server.py'),
        ('T19', '/web/'), ('T20', '/./'), ('T21', '/api'), ('T22', '/a/b/c'),
    ]
    for pid, p in trav:
        c.rec(pid, 'GET %s' % p.replace('\x00', '\\0'), c.get(p), 404)
    c.rec('T30', 'GET /./app.js (dot segment resolved inside web/)', c.get('/./app.js'), 200)
    c.rec('T31', 'GET /sub/../app.js (resolves inside web/)', c.get('/sub/../app.js'), 200)
    c.rec('T32', 'GET /APP.JS (case)', c.get('/APP.JS'), None, note='Windows: file systems are case-insensitive; Linux 404')
    r = c.req('HEAD', '/')
    c.rec('T33', 'HEAD / has no body', r, 200, note='body=%d' % len(r.body))
    c.rec('T34', 'Range request is ignored (200, full body)', c.get('/app.js', headers=[('Range', 'bytes=0-9')]), 200)


def g_shutdown(c):
    c.group = 'shutdown'
    if c.profile == 'shutdown':
        c.rec('X01', 'Origin evil + shutdown allowed -> Origin rule still blocks', c.post_json('/api/shutdown', {}, origin='http://evil.com'), 403)
        r = c.post_json('/api/shutdown', {})
        c.rec('X02', 'POST /api/shutdown with --allow-shutdown (server should exit afterwards)', r, 200)
        time.sleep(1.5)
        try:
            socket.create_connection(('127.0.0.1', c.port), timeout=1).close()
            alive = True
        except OSError:
            alive = False
        c.rec('X03', 'server still listening after shutdown?', Resp(200 if not alive else 500), 200, note='alive=%s' % alive)
        return
    r = c.post_json('/api/shutdown', {})
    c.rec('X01', 'POST /api/shutdown (not allowed)', r, 403, note=r.text()[:50])
    c.rec('X02', 'POST /api/shutdown, Origin evil -> blocked earlier by Origin rule', c.post_json('/api/shutdown', {}, origin='http://evil.com'), 403)
    r = c.get('/api/info')
    c.rec('X03', 'GET /api/info reports canShutdown=false', r, 200, note='canShutdown=%s' % (r.json() or {}).get('canShutdown'))
    c.rec('X04', 'server still alive', c.get('/api/info'), 200)


def g_headers(c):
    c.group = 'headers'
    want = ['Content-Type', 'Content-Length', 'Cache-Control', 'X-Content-Type-Options', 'Server', 'Date', 'Connection']
    absent = ['Content-Security-Policy', 'X-Frame-Options', 'Referrer-Policy', 'Strict-Transport-Security', 'Permissions-Policy',
              'Access-Control-Allow-Origin', 'Set-Cookie', 'ETag', 'Last-Modified', 'Allow']
    for pid, path in (('R01', '/api/info'), ('R02', '/'), ('R03', '/app.js'), ('R04', '/nope')):
        r = c.get(path)
        c.rec(pid, 'response headers of GET %s' % path, r, None,
              note='present=%s absent=%s' % ([h for h in want if r.h(h) is not None], [h for h in absent if r.h(h) is None]))
    r = c.get('/api/info')
    c.rec('R05', 'Server header value (version disclosure)', r, 200, note=r.h('Server'))
    r = c.get('/api/info')
    c.rec('R06', 'API JSON Content-Type', r, 200, note=r.h('Content-Type'))
    r = c.post_json('/api/import/cancel', {})
    c.rec('R07', 'POST response headers', r, 200, note='%s' % [(k, v) for k, v in r.headers if k.lower() in ('cache-control', 'x-content-type-options', 'content-type')])


def g_dos(c):
    """Sockets that stay open. Needs a few seconds; the server must stay responsive for other clients."""
    c.group = 'dos'
    held = []
    try:
        s1 = socket.create_connection(('127.0.0.1', c.port), timeout=5)
        s1.sendall(b'GET /api/info HTTP/1.1\r\nHost: localhost\r\nX-Slow: a')    # headers never finished
        held.append(s1)
        s2 = socket.create_connection(('127.0.0.1', c.port), timeout=5)
        s2.sendall(build('POST', '/api/upload', [('Host', c.host), ('Content-Type', 'application/octet-stream'), ('Content-Length', str(10 * MIB))], b'PK' + b'0' * 1000))
        held.append(s2)
        s3 = socket.create_connection(('127.0.0.1', c.port), timeout=5)
        s3.sendall(build('POST', '/api/upload', [('Host', c.host), ('Content-Type', 'application/octet-stream'), ('Content-Length', str(30 * MIB))], b'PK' + b'0' * 1000))
        held.append(s3)
        time.sleep(4)
        for i, s in enumerate(held, 1):
            s.settimeout(0.5)
            try:
                d = s.recv(100)
                state = 'closed-by-server' if d == b'' else 'got-bytes'
            except socket.timeout:
                state = 'still-open (no timeout in the server)'
            c.rec('D0%d' % i, ['partial header held 4s', 'CL=10MiB, 1KiB sent, held 4s', 'CL=30MiB(>limit), 1KiB sent, held 4s'][i - 1],
                  Resp(200), 200, note=state)
        c.rec('D04', 'other clients are still served while 3 sockets are stuck', c.get('/api/info'), 200)
    finally:
        for s in held:
            try:
                s.close()
            except OSError:
                pass
    time.sleep(0.5)
    c.rec('D05', 'server alive after the stuck sockets were closed', c.get('/api/info'), 200)


GROUPS = [('host', g_host, False), ('origin', g_origin, False), ('method', g_method, False), ('ctype', g_ctype, False),
          ('body', g_body, False), ('upload', g_upload, True), ('entity', g_entity, False), ('pending', g_pending, True),
          ('attempt', g_attempt, False), ('static', g_static, False), ('headers', g_headers, False), ('shutdown', g_shutdown, False),
          ('dos', g_dos, False)]
PROFILE_GROUPS = {
    'default': None,
    'hosts': ['host', 'origin'],
    'anyhost': ['host', 'origin'],
    'shutdown': ['shutdown'],
}


def run_profile(port, profile, only, no_dos, tmp):
    results = []
    c = Ctx(port, profile, results)
    allowed = PROFILE_GROUPS[profile]
    print('\n===== profile %s on port %d =====' % (profile, port))
    for name, fn, needs_tmp in GROUPS:
        if allowed is not None and name not in allowed:
            continue
        if only and not any(name.startswith(o) for o in only):
            continue
        if no_dos and name == 'dos':
            continue
        print('--- group %s' % name)
        fn(c, tmp) if needs_tmp else fn(c)
    return results


def wait_up(port, tries=60):
    for _ in range(tries):
        try:
            socket.create_connection(('127.0.0.1', port), timeout=0.5).close()
            return True
        except OSError:
            time.sleep(0.25)
    return False


def spawn_python(srvdir, profile, port, workdir):
    data = os.path.join(workdir, 'data_%s' % profile)
    shutil.rmtree(data, ignore_errors=True)
    os.makedirs(data)
    env = dict(os.environ)
    for k in list(env):
        if k.startswith('XEXAM_'):
            env.pop(k)
    env.update(PROFILES[profile]['env'])
    log = open(os.path.join(workdir, 'server_%s.log' % profile), 'wb')
    p = subprocess.Popen([sys.executable, '-B', '-I', os.path.join(srvdir, 'server.py'), '--no-browser', '--data', data, '--port', str(port),
                          '--host', '127.0.0.1'] + PROFILES[profile]['args'], stdout=log, stderr=subprocess.STDOUT, env=env)
    if not wait_up(port):
        p.kill()
        raise SystemExit('server did not start for profile %s' % profile)
    return p, log


def summarize_log(path):
    try:
        txt = open(path, 'r', encoding='utf-8', errors='replace').read()
    except OSError:
        return []
    out = []
    lines = txt.splitlines()
    for i, ln in enumerate(lines):
        if ln.startswith('Traceback'):
            j = i + 1
            while j < len(lines) and (lines[j].startswith(' ') or lines[j].startswith('\t') or lines[j].startswith('Traceback')):
                j += 1
            out.append(lines[j] if j < len(lines) else '?')
    return out


def compare(results, golden):
    g = {r['id']: r for r in golden}
    diffs = []
    for r in results:
        o = g.get(r['id'])
        if o is None:
            diffs.append((r['id'], 'NEW', '-', r['state']))
        elif o['state'] != r['state'] or (o.get('ctype') or '').split(';')[0] != (r.get('ctype') or '').split(';')[0]:
            diffs.append((r['id'], r['desc'][:60], '%s %s' % (o['state'], (o.get('ctype') or '').split(';')[0]), '%s %s' % (r['state'], (r.get('ctype') or '').split(';')[0])))
    return diffs


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--spawn-python', metavar='DIR', help='directory containing the legacy server.py; probe all profiles')
    ap.add_argument('--base', metavar='URL', help='probe a running server, e.g. http://127.0.0.1:8710')
    ap.add_argument('--profile', default='default', choices=sorted(PROFILES))
    ap.add_argument('--port-base', type=int, default=8710)
    ap.add_argument('--out', help='write results JSON here')
    ap.add_argument('--golden', help='results JSON of an earlier run to diff against')
    ap.add_argument('--only', default='')
    ap.add_argument('--no-dos', action='store_true')
    ap.add_argument('--workdir', default=None)
    a = ap.parse_args()
    only = [x for x in a.only.split(',') if x]
    work = a.workdir or tempfile.mkdtemp(prefix='xexam_probe_')
    os.makedirs(work, exist_ok=True)
    all_results = {}
    logs = {}
    if a.spawn_python:
        port = a.port_base
        for prof in ('default', 'hosts', 'anyhost', 'shutdown'):
            p, log = spawn_python(a.spawn_python, prof, port, work)
            try:
                all_results[prof] = run_profile(port, prof, only, a.no_dos, work)
            finally:
                if p.poll() is None:
                    p.terminate()
                    try:
                        p.wait(timeout=5)
                    except Exception:
                        p.kill()
                log.close()
            logs[prof] = summarize_log(os.path.join(work, 'server_%s.log' % prof))
            port += 1
    elif a.base:
        u = urllib.parse.urlparse(a.base)
        all_results[a.profile] = run_profile(u.port or 80, a.profile, only, a.no_dos, work)
    else:
        ap.error('give --spawn-python DIR or --base URL')
    flat = [dict(r, profile=p) for p, rs in all_results.items() for r in rs]
    for r in flat:
        r['key'] = '%s/%s' % (r['profile'], r['id'])
    sur = [r for r in flat if r['surprise']]
    print('\n===== summary =====')
    print('probes: %d, SURPRISE (differs from reading of the code): %d' % (len(flat), len(sur)))
    for r in sur:
        print('  SURPRISE %s: %s -> %s (want %s) %s' % (r['key'], r['desc'], r['state'], r['want'], r['note']))
    if logs:
        for prof, tb in logs.items():
            if tb:
                print('server stderr tracebacks (%s): %d -> %s' % (prof, len(tb), sorted(set(tb))))
    if a.golden:
        gold = json.load(open(a.golden, encoding='utf-8'))['results']
        gold = [dict(g, key=g.get('key') or '%s/%s' % (g.get('profile'), g['id'])) for g in gold]
        gk = {g['key']: g for g in gold}
        print('\n===== diff against golden =====')
        n = 0
        for r in flat:
            o = gk.get(r['key'])
            if o is None:
                print('  NEW   %-14s %s' % (r['key'], r['state']))
                n += 1
            elif o['state'] != r['state'] or (o.get('ctype') or '').split(';')[0] != (r.get('ctype') or '').split(';')[0]:
                print('  DIFF  %-14s golden=%s %s | now=%s %s | %s' % (r['key'], o['state'], (o.get('ctype') or '').split(';')[0], r['state'], (r.get('ctype') or '').split(';')[0], r['desc'][:55]))
                n += 1
        print('differences: %d' % n)
    if a.out:
        with open(a.out, 'w', encoding='utf-8') as f:
            json.dump({'when': time.strftime('%Y-%m-%d %H:%M:%S'), 'tracebacks': logs, 'results': flat}, f, ensure_ascii=False, indent=1)
        print('wrote', a.out)
    if a.spawn_python and not a.workdir:
        pass


if __name__ == '__main__':
    main()
