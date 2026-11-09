# -*- coding: utf-8 -*-
"""Golden-response capture for Xexam server.py (Phase 0, read-only on the repo).

Usage:
  python -I -B capture.py                       # starts server.py from the repo on 127.0.0.1:8701 (+8703 with --allow-shutdown)
  python -I -B capture.py --base-url http://127.0.0.1:9000 [--shutdown-url http://127.0.0.1:9001]
        # rerun against another implementation (e.g. the Rust backend). It MUST be a FRESH, EMPTY database
        # started with default config (no extra allowed hosts/origins, shutdown disabled).
Output: one JSON file per capture in --out (default: directory of this script), plus index.json (md5 of the 3 python
sources, env, list of captures). Volatile values are replaced in the "normalized" field by "<VOLATILE>"; "raw_body" keeps
the originals. Diff tools should compare "status", "headers_stable", "normalized" (and "body_md5" for static files).
Safety: when it starts the server itself it pre-seeds meta.legacy_migrated in the temp DB so the repo's progress.json
(owner's personal data) is NOT migrated into the captures, and asserts that /api/progress starts empty.
"""
import argparse, hashlib, http.client, io, json, os, shutil, sqlite3, subprocess, sys, tempfile, time, zipfile
from urllib.parse import urlparse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import paths  # noqa: E402
REPO = paths.LEGACY_ROOT
DEMO = paths.DEMO
DEMO2 = paths.DEMO2
HERE = os.path.dirname(os.path.abspath(__file__))
VOLATILE_KEYS = {'token', 'mtime', 'at', 'version', 'arch'}
VOLATILE_HEADERS = {'date', 'server'}
STABLE_HEADERS = ['content-type', 'content-length', 'cache-control', 'x-content-type-options', 'server', 'date', 'connection', 'allow']

ap = argparse.ArgumentParser()
ap.add_argument('--base-url')
ap.add_argument('--shutdown-url', help='base URL of a second instance started with shutdown enabled')
ap.add_argument('--out', default=HERE)
ap.add_argument('--port', type=int, default=8701)
ap.add_argument('--demo', default=DEMO)
ap.add_argument('--demo2', default=DEMO2)
args = ap.parse_args()
OUT = args.out
os.makedirs(OUT, exist_ok=True)
for f in os.listdir(OUT):
    if f[:3].isdigit() and f.endswith('.json'):
        os.remove(os.path.join(OUT, f))

procs, tmpdirs = [], []


def start_server(port, extra=()):
    d = tempfile.mkdtemp(prefix='xexam_cap_')
    tmpdirs.append(d)
    c = sqlite3.connect(os.path.join(d, 'xexam.db'))
    c.execute('CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL)')
    c.execute("INSERT INTO meta(key,value) VALUES('legacy_migrated','capture-seed')")
    c.commit(); c.close()
    env = {k: v for k, v in os.environ.items() if not k.startswith('XEXAM_')}
    env['PYTHONDONTWRITEBYTECODE'] = '1'
    log = open(os.path.join(d, 'server.log'), 'w', encoding='utf-8')
    p = subprocess.Popen([sys.executable, '-I', '-B', 'server.py', '--no-browser', '--port', str(port), '--data', d] + list(extra),
                         cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT)
    procs.append(p)
    for _ in range(60):
        try:
            cc = http.client.HTTPConnection('127.0.0.1', port, timeout=2); cc.request('GET', '/api/info'); cc.getresponse().read(); cc.close()
            return 'http://127.0.0.1:%d' % port, p
        except Exception:
            time.sleep(0.2)
    raise SystemExit('server did not start on %d' % port)


counter = [0]


def norm(o):
    if isinstance(o, dict):
        return {k: ('<VOLATILE>' if k in VOLATILE_KEYS and isinstance(v, str) and v else norm(v)) for k, v in o.items()}
    if isinstance(o, list):
        return [norm(x) for x in o]
    return o


def call(base, name, method, path, headers=None, body=None, host='__default__', static=False, note=''):
    """Raw request: Host is only sent when given (default: the real host:port), so Host violations can be tested."""
    u = urlparse(base)
    headers = dict(headers or {})
    entry = {'name': name, 'note': note, 'request': {'method': method, 'path': path, 'headers': dict(headers),
             'host_header': (u.netloc if host == '__default__' else host),
             'body': (None if body is None else ('<%d bytes>' % len(body)) if len(body) > 300 else body.decode('utf-8', 'replace'))}}
    c = http.client.HTTPConnection(u.hostname, u.port, timeout=30)
    try:
        c.putrequest(method, path, skip_host=True, skip_accept_encoding=True)
        h = u.netloc if host == '__default__' else host
        if h is not None:
            c.putheader('Host', h)
        for k, v in headers.items():
            c.putheader(k, v)
        if body is not None and 'Content-Length' not in headers:
            c.putheader('Content-Length', str(len(body)))
        c.endheaders(body if body is not None else None)
        r = c.getresponse()
        data = r.read()
        entry['status'] = r.status
        entry['http_version'] = {10: 'HTTP/1.0', 11: 'HTTP/1.1'}.get(r.version, str(r.version))
        hs = {k.lower(): v for k, v in r.getheaders()}
        entry['headers'] = hs
        entry['headers_stable'] = {k: v for k, v in hs.items() if k not in VOLATILE_HEADERS and k in STABLE_HEADERS}
        entry['volatile_headers'] = sorted(k for k in hs if k in VOLATILE_HEADERS)
        entry['body_len'] = len(data)
        entry['body_md5'] = hashlib.md5(data).hexdigest()
        if not static:
            txt = data.decode('utf-8', 'replace')
            entry['raw_body'] = txt
            try:
                entry['json'] = json.loads(txt)
                entry['normalized'] = norm(entry['json'])
            except ValueError:
                entry['normalized'] = txt
    except Exception as e:      # no response at all (server dropped the connection)
        entry['status'] = None
        entry['no_response'] = '%s: %s' % (type(e).__name__, e)
        entry['normalized'] = None
    finally:
        c.close()
    counter[0] += 1
    fn = '%03d_%s.json' % (counter[0], name)
    with open(os.path.join(OUT, fn), 'w', encoding='utf-8') as f:
        json.dump(entry, f, ensure_ascii=False, indent=1, sort_keys=False)
    print('%-3d %-44s -> %s' % (counter[0], name, entry['status'] if entry['status'] is not None else 'NO RESPONSE ' + entry['no_response'][:60]))
    return entry


def jpost(base, name, path, obj=None, raw=None, ctype='application/json', headers=None, **kw):
    body = raw if raw is not None else json.dumps(obj, ensure_ascii=False).encode('utf-8')
    hh = {'Content-Type': ctype} if ctype is not None else {}
    hh.update(headers or {})
    return call(base, name, 'POST', path, headers=hh, body=body, **kw)


def upload(base, name, body, fname='demo.xlsx', ctype='application/octet-stream', headers=None, **kw):
    hh = {'Content-Type': ctype} if ctype is not None else {}
    if fname is not None:
        hh['X-Filename'] = fname
    hh.update(headers or {})
    return call(base, name, 'POST', '/api/upload', headers=hh, body=body, **kw)


def mkzip(files):
    b = io.BytesIO()
    with zipfile.ZipFile(b, 'w', zipfile.ZIP_DEFLATED) as z:
        for n, d in files.items():
            z.writestr(n, d)
    return b.getvalue()


def main():
    if args.base_url:
        base, shut = args.base_url.rstrip('/'), (args.shutdown_url or '').rstrip('/')
    else:
        base, _ = start_server(args.port)
        shut = start_server(args.port + 2, ['--allow-shutdown'])[0]
    demo = open(args.demo, 'rb').read()
    demo2 = open(args.demo2, 'rb').read() if os.path.exists(args.demo2) else None
    HN = urlparse(base).hostname
    ORIG = 'http://%s:%d' % (HN, urlparse(base).port)

    # ---- 1. empty state
    for n, p in [('empty_info', '/api/info'), ('empty_bank', '/api/bank'), ('empty_progress', '/api/progress'), ('empty_imports', '/api/imports')]:
        call(base, n, 'GET', p)
    e = call(base, 'empty_progress_check', 'GET', '/api/progress')
    assert e['json']['questions'] == {} and e['json']['history'] == [], 'database is not empty; aborting (would capture personal data)'
    call(base, 'head_info', 'HEAD', '/api/info')
    call(base, 'head_bank', 'HEAD', '/api/bank')
    call(base, 'get_query_string', 'GET', '/api/info?x=1&y=%E2%82%AC')
    call(base, 'get_trailing_slash_api_bank', 'GET', '/api/bank/')
    call(base, 'get_api_unknown', 'GET', '/api/nope')
    call(base, 'get_api_upload_is_static_404', 'GET', '/api/upload')
    call(base, 'get_api_shutdown_is_static_404', 'GET', '/api/shutdown')
    # ---- 2. static files
    for n, p in [('static_root', '/'), ('static_index', '/index.html'), ('static_index_q', '/?a=1'), ('static_app_js', '/app.js'), ('static_style', '/style.css'),
                 ('static_icon_svg', '/icon.svg'), ('static_apple_icon', '/apple-touch-icon.png'), ('static_pct_encoded', '/%61pp.js'),
                 ('static_double_slash', '//app.js'), ('static_dot_segments', '/./app.js'), ('static_dir_web', '/web'),
                 ('static_traversal_dotdot', '/../server.py'), ('static_traversal_pct', '/%2e%2e/server.py'), ('static_traversal_pct_slash', '/..%2fserver.py'),
                 ('static_traversal_backslash', '/..\\server.py'), ('static_missing', '/missing.js'), ('static_upper_ext', '/APP.JS'), ('static_nul_byte', '/app.js%00.png'),
                 ('static_fragment', '/app.js#frag')]:
        call(base, n, 'GET', p, static=True)
    call(base, 'static_head_app_js', 'HEAD', '/app.js', static=True)
    # ---- 3. unsupported methods
    for m in ('PUT', 'DELETE', 'OPTIONS', 'PATCH', 'TRACE', 'CONNECT'):
        call(base, 'method_' + m.lower() + '_api_info', m, '/api/info', body=b'' if m in ('PUT', 'PATCH') else None)
    call(base, 'method_options_origin_cors_preflight', 'OPTIONS', '/api/attempt', headers={'Origin': 'https://evil.example', 'Access-Control-Request-Method': 'POST'})
    # ---- 4. Host / Origin guards
    call(base, 'host_evil_get_api', 'GET', '/api/bank', host='evil.example.com')
    call(base, 'host_evil_get_static', 'GET', '/', host='evil.example.com')
    call(base, 'host_evil_head', 'HEAD', '/api/info', host='evil.example.com')
    call(base, 'host_missing_get', 'GET', '/api/info', host=None)
    call(base, 'host_empty_get', 'GET', '/api/info', host='')
    for n, h in [('localhost_port', 'localhost:1'), ('localhost_upper_dot', 'LOCALHOST.'), ('ipv4', '127.0.0.1:9'), ('ipv6', '[::1]:9'), ('lan_ip', '192.168.1.20:8765'),
                 ('port_suffix_trick', 'localhost:1:2'), ('evil_with_localhost_prefix', 'localhost.evil.com')]:
        call(base, 'host_variant_' + n, 'GET', '/api/info', host=h)
    call(base, 'host_evil_ctrl_chars_in_403', 'GET', '/api/info', host='evil\t.com')
    call(base, 'host_long_403_truncation', 'GET', '/api/info', host='a' * 200 + '.com')
    call(base, 'post_host_evil', 'POST', '/api/import/cancel', headers={'Content-Type': 'application/json'}, body=b'{}', host='evil.example.com')
    call(base, 'post_host_evil_and_origin_evil_host_wins', 'POST', '/api/import/cancel', headers={'Content-Type': 'application/json', 'Origin': 'https://evil.example'}, body=b'{}', host='evil.example.com')
    for n, o in [('evil', 'https://evil.example.com'), ('null', 'null'), ('file', 'file://'), ('ftp', 'ftp://localhost'), ('userinfo', 'http://user@localhost'),
                 ('empty', ''), ('scheme_only', 'https://'), ('trailing_path', 'http://%s/path' % HN), ('other_port_scheme_ok', 'https://%s:1234' % HN),
                 ('upper', 'HTTP://%s:5' % HN.upper()), ('sameorigin', ORIG), ('long_ctrl', 'https://' + 'e' * 120 + '.com')]:
        jpost(base, 'post_origin_' + n, '/api/import/cancel', {}, headers={'Origin': o})
    jpost(base, 'post_origin_absent_ok', '/api/import/cancel', {})
    jpost(base, 'post_origin_evil_on_upload_checked_before_ctype', '/api/upload', raw=b'', ctype='text/plain', headers={'Origin': 'https://evil.example.com'})
    # ---- 5. content-type / body / JSON guards (before any import)
    jpost(base, 'json_ctype_text_plain', '/api/reset', raw=b'{}', ctype='text/plain')
    jpost(base, 'json_ctype_missing', '/api/reset', raw=b'{}', ctype=None)
    jpost(base, 'json_ctype_charset_ok', '/api/import/cancel', raw=b'{}', ctype='application/json; charset=utf-8')
    jpost(base, 'json_ctype_substring_match', '/api/import/cancel', raw=b'{}', ctype='text/plain; x=application/json')
    jpost(base, 'json_unknown_path_ctype_checked_first', '/api/nope', raw=b'{}', ctype='text/plain')
    jpost(base, 'json_unknown_path', '/api/nope', {})
    jpost(base, 'json_post_to_info_is_404', '/api/info', {})
    jpost(base, 'json_bad_json', '/api/attempt', raw=b'{not json')
    jpost(base, 'json_array_not_object', '/api/attempt', raw=b'[1,2]')
    jpost(base, 'json_string_not_object', '/api/attempt', raw=b'"x"')
    jpost(base, 'json_null_not_object', '/api/attempt', raw=b'null')
    jpost(base, 'json_empty_body_is_empty_object', '/api/import/cancel', raw=b'')
    jpost(base, 'json_empty_body_attempt', '/api/attempt', raw=b'')
    jpost(base, 'json_bom_body', '/api/import/cancel', raw=b'\xef\xbb\xbf{}')
    jpost(base, 'json_invalid_utf8', '/api/attempt', raw=b'{"a":"\xff"}')
    jpost(base, 'json_nan_accepted', '/api/import/cancel', raw=b'{"x": NaN}')
    jpost(base, 'json_too_large_1000001', '/api/attempt', raw=b'{"pad":"' + b'a' * 999990 + b'"}' + b' ' * 10)
    jpost(base, 'json_deeply_nested', '/api/import/cancel', raw=b'[' * 100000 + b']' * 100000)
    call(base, 'json_content_length_not_number', 'POST', '/api/import/cancel', headers={'Content-Type': 'application/json', 'Content-Length': 'abc'}, body=b'{}')
    call(base, 'json_content_length_negative', 'POST', '/api/attempt', headers={'Content-Type': 'application/json', 'Content-Length': '-5'}, body=b'')
    jpost(base, 'attempt_empty_object', '/api/attempt', {})
    jpost(base, 'attempt_empty_results', '/api/attempt', {'results': []})
    jpost(base, 'confirm_nothing_pending', '/api/import/confirm', {'token': 'x'})
    jpost(base, 'confirm_no_token', '/api/import/confirm', {})
    jpost(base, 'cancel_nothing_pending', '/api/import/cancel', {})
    jpost(base, 'shutdown_disabled', '/api/shutdown', {})
    # ---- 6. upload error cases
    upload(base, 'upload_empty_body', b'')
    upload(base, 'upload_tiny_99_bytes', b'PK' + b'x' * 97)
    upload(base, 'upload_wrong_ctype_json', demo, ctype='application/json')
    upload(base, 'upload_ctype_missing', demo, ctype=None)
    upload(base, 'upload_ctype_substring_match_empty', b'', ctype='multipart/form-data; x=application/octet-stream')
    upload(base, 'upload_xls_signature', b'\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1' + b'\x00' * 600)
    upload(base, 'upload_not_zip_text', b'a,b,c\n1,2,3\n' * 20)
    upload(base, 'upload_pk_but_corrupt', b'PK\x03\x04' + b'garbage' * 50)
    upload(base, 'upload_zip_without_workbook', mkzip({'hello.txt': 'x' * 300}))
    upload(base, 'upload_zip_workbook_doctype', mkzip({'xl/workbook.xml': '<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "b">]><workbook/>' + ' ' * 100}))
    upload(base, 'upload_zip_workbook_not_xml', mkzip({'xl/workbook.xml': 'this is not xml ' * 20}))
    upload(base, 'upload_zip_workbook_no_rels', mkzip({'xl/workbook.xml': '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheets/></workbook>' + ' ' * 100}))
    upload(base, 'upload_valid_zip_no_questions', mkzip({
        'xl/workbook.xml': '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="s" sheetId="1" r:id="rId1"/></sheets></workbook>',
        'xl/_rels/workbook.xml.rels': '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="x" Target="worksheets/sheet1.xml"/></Relationships>',
        'xl/worksheets/sheet1.xml': '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>'}))
    upload(base, 'upload_zip_bomb_declared_big', mkzip({'xl/workbook.xml': '<workbook/>', 'xl/big.bin': b'\x00' * (101 * 1024 * 1024)}))
    upload(base, 'upload_21mb', b'\x00' * (21 * 1024 * 1024))
    upload(base, 'upload_exactly_20mb_plus_1', b'\x00' * (20 * 1024 * 1024 + 1))
    # ---- 7. successful import flow (demo.xlsx)
    up = upload(base, 'upload_demo_ok_missing_xfilename', demo, fname=None)
    jpost(base, 'cancel_after_missing_filename_upload', '/api/import/cancel', {})
    up = upload(base, 'upload_demo_ok', demo, fname='demo.xlsx')
    tok = up.get('json', {}).get('token')
    call(base, 'bank_after_preview_still_empty', 'GET', '/api/bank')
    jpost(base, 'confirm_wrong_token_keeps_pending', '/api/import/confirm', {'token': 'wrong-token'})
    jpost(base, 'confirm_token_is_int', '/api/import/confirm', {'token': 123})
    jpost(base, 'confirm_token_non_ascii_compare_digest', '/api/import/confirm', {'token': '令牌'})
    jpost(base, 'confirm_token_null', '/api/import/confirm', {'token': None})
    jpost(base, 'confirm_ok', '/api/import/confirm', {'token': tok})
    jpost(base, 'confirm_reused_token', '/api/import/confirm', {'token': tok})
    for n, p in [('after_import_info', '/api/info'), ('after_import_bank', '/api/bank'), ('after_import_progress', '/api/progress'), ('after_import_imports', '/api/imports')]:
        call(base, n, 'GET', p)
    # same file again -> all unchanged; then cancel; then token invalid
    up2 = upload(base, 'upload_demo_again_unchanged', demo, fname='%E3%83%86%E3%82%B9%E3%83%88%2Fdir%5Cevil%20name.xlsx', note='percent-encoded Japanese name with / and \\ path parts')
    jpost(base, 'cancel_ok', '/api/import/cancel', {})
    jpost(base, 'confirm_after_cancel', '/api/import/confirm', {'token': up2.get('json', {}).get('token')})
    up3 = upload(base, 'upload_second_replaces_first_pending', demo, fname='first.xlsx')
    up4 = upload(base, 'upload_second_replaces_first_pending_b', demo, fname='second.xlsx')
    jpost(base, 'confirm_first_token_after_second_upload', '/api/import/confirm', {'token': up3.get('json', {}).get('token')})
    jpost(base, 'confirm_second_token_ok_unchanged_import', '/api/import/confirm', {'token': up4.get('json', {}).get('token')})
    call(base, 'imports_after_two_confirms', 'GET', '/api/imports')
    # ---- 8. attempts
    bank = call(base, 'bank_for_ids', 'GET', '/api/bank')['json']
    ids = [q['id'] for q in bank['questions']]
    A = lambda n, obj=None, **kw: jpost(base, n, '/api/attempt', obj, **kw)
    A('attempt_all_correct', {'results': [{'id': i, 'correct': True} for i in ids], 'sheets': ['demo'], 'mode': 'all', 'seconds': 125})
    A('attempt_mixed', {'results': [{'id': ids[0], 'correct': False}, {'id': ids[1], 'correct': True}, {'id': ids[2], 'correct': False}], 'sheets': ['demo'], 'mode': 'retry', 'seconds': 40})
    A('attempt_weak_mode_streak_reset', {'results': [{'id': ids[0], 'correct': True}], 'sheets': ['demo'], 'mode': 'weak', 'seconds': 5})
    A('attempt_unknown_ids_accepted', {'results': [{'id': 'does-not-exist', 'correct': False}, {'id': 'x' * 100, 'correct': True}], 'sheets': [], 'mode': 'all', 'seconds': 1})
    A('attempt_bad_mode_becomes_all', {'results': [{'id': ids[1], 'correct': True}], 'sheets': ['demo'], 'mode': 'bogus', 'seconds': 3})
    A('attempt_huge_seconds_clamped', {'results': [{'id': ids[1], 'correct': True}], 'mode': 'all', 'seconds': 10 ** 12})
    A('attempt_negative_seconds', {'results': [{'id': ids[1], 'correct': True}], 'mode': 'all', 'seconds': -50})
    A('attempt_float_seconds', {'results': [{'id': ids[1], 'correct': True}], 'mode': 'all', 'seconds': 12.7})
    A('attempt_string_seconds', {'results': [{'id': ids[1], 'correct': True}], 'mode': 'all', 'seconds': '77'})
    A('attempt_garbage_seconds', {'results': [{'id': ids[1], 'correct': True}], 'mode': 'all', 'seconds': 'abc'})
    A('attempt_seconds_inf_literal', raw=b'{"results":[{"id":"a","correct":true}],"seconds":Infinity}')
    A('attempt_seconds_nan_literal', raw=b'{"results":[{"id":"a","correct":true}],"seconds":NaN}')
    A('attempt_invalid_entries_filtered', {'results': [{'id': ids[2], 'correct': 1}, {'id': 5, 'correct': True}, 'x', None, {'id': ids[2], 'correct': True}], 'mode': 'all'})
    A('attempt_all_entries_invalid', {'results': [{'id': ids[2], 'correct': 1}, 'x']})
    A('attempt_results_is_string', {'results': 'abc'})
    A('attempt_results_is_int_crash', {'results': 5})
    A('attempt_results_is_dict', {'results': {'a': 1}})
    A('attempt_sheets_filter_truncate', {'results': [{'id': ids[2], 'correct': True}], 'sheets': ['s' * 100, 5, None, 'ok'], 'mode': 'all'})
    A('attempt_sheets_is_string', {'results': [{'id': ids[2], 'correct': True}], 'sheets': 'abc'})
    A('attempt_sheets_is_int_crash', {'results': [{'id': ids[2], 'correct': True}], 'sheets': 5})
    A('attempt_duplicate_ids_in_one_request', {'results': [{'id': ids[3], 'correct': True}, {'id': ids[3], 'correct': False}], 'mode': 'all'})
    A('attempt_extra_fields_ignored', {'results': [{'id': ids[3], 'correct': True, 'zzz': 1}], 'zzz': 2})
    call(base, 'progress_after_attempts', 'GET', '/api/progress')
    # history cap: 60 attempts kept
    # fill quietly (not recorded one by one), then record the capped /api/progress
    for k in range(1, 61):
        c = http.client.HTTPConnection(urlparse(base).hostname, urlparse(base).port, timeout=10)
        c.request('POST', '/api/attempt', json.dumps({'results': [{'id': ids[4], 'correct': True}], 'mode': 'all', 'seconds': k}), {'Content-Type': 'application/json'})
        c.getresponse().read(); c.close()
    call(base, 'progress_history_capped_at_60', 'GET', '/api/progress')
    # ---- 9. v2 import (added/updated/removed with history)
    if demo2:
        upload(base, 'upload_demo_v2_preview', demo2, fname='demo_v2.xlsx')
        t2 = call(base, 'imports_before_v2_confirm', 'GET', '/api/imports')
        tok2 = json.loads(open(os.path.join(OUT, '%03d_upload_demo_v2_preview.json' % (counter[0] - 1)), encoding='utf-8').read())['json']['token']
        jpost(base, 'confirm_v2_ok', '/api/import/confirm', {'token': tok2})
        call(base, 'bank_after_v2_has_inactive', 'GET', '/api/bank')
        call(base, 'info_after_v2', 'GET', '/api/info')
        call(base, 'imports_after_v2', 'GET', '/api/imports')
        jpost(base, 'confirm_empty_questions_not_testable_placeholder', '/api/import/cancel', {})
    # ---- 10. reset
    jpost(base, 'reset_ok', '/api/reset', {})
    jpost(base, 'reset_again', '/api/reset', {})
    call(base, 'bank_after_reset_unchanged', 'GET', '/api/bank')
    call(base, 'progress_after_reset', 'GET', '/api/progress')
    # ---- 11. shutdown (separate instance started with --allow-shutdown)
    if shut:
        call(shut, 'shutdown_instance_info', 'GET', '/api/info')
        jpost(shut, 'shutdown_enabled_post', '/api/shutdown', {})
        time.sleep(1.0)
        call(shut, 'shutdown_instance_after_exit', 'GET', '/api/info')
    # ---- index
    srcs = {}
    for f in ('server.py', 'app/db.py', 'app/xlsx_parser.py'):
        srcs[f] = hashlib.md5(open(os.path.join(REPO, f), 'rb').read()).hexdigest()
    idx = {'captured_at_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), 'base_url': base, 'python': sys.version, 'platform': sys.platform,
           'md5': srcs, 'demo_md5': hashlib.md5(demo).hexdigest(), 'demo2_md5': hashlib.md5(demo2).hexdigest() if demo2 else None,
           'volatile_json_keys': sorted(VOLATILE_KEYS), 'volatile_headers': sorted(VOLATILE_HEADERS), 'captures': counter[0]}
    json.dump(idx, open(os.path.join(OUT, 'index.json'), 'w', encoding='utf-8'), indent=1)
    print('done:', counter[0], 'captures; md5', srcs)


try:
    main()
finally:
    for p in procs:
        if p.poll() is None:
            p.terminate()
    for p in procs:
        try:
            p.wait(timeout=5)
        except Exception:
            p.kill()
    for d in tmpdirs:
        shutil.rmtree(d, ignore_errors=True)
