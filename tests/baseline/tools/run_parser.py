# -*- coding: utf-8 -*-
"""Run the ORIGINAL Python parser (app/xlsx_parser.py + server.check_xlsx) on one .xlsx and print canonical JSON.

usage:  python -B -I run_parser.py <file.xlsx> [--repo <含 app/ 的目录，默认旧版 Python 代码>] [--meta] [--no-reader]  > out.json

The repo is only READ: modules are loaded from source with bytecode writing disabled, so nothing is created in the repo.

Output (UTF-8, sort_keys, indent=1) has these top-level keys:
  input            {name, size}
  upload_check     result of server.check_xlsx(data)        -> {"ok":true} | {"ok":false,"exc":..,"error":..}
  parse            result of xlsx_parser.parse_workbook()   -> {"ok":true,"sheets":[..],"questions":[..]} | {"ok":false,"exc":..,"error":..}
                   (run even when upload_check failed, so the parser's own behaviour is visible)
  server_outcome   what POST /api/upload would answer for this body: {"status":400,"error":..} or {"status":200,"sheets":..,"questions":..}
                   (replicates server.py do_POST: check_xlsx first, then parse_workbook; ValueError -> message, any other exception ->
                    '读取这个 Excel 时出错：<ExcType>: <msg>')
  reader           (unless --no-reader) raw output of read_workbook(): [{name, rows:[[rowNumber, {colIndex: text}]]}]  (cell layer, before parse_question)
  meta             (only with --meta) python/expat versions and md5 of the source files

A later phase can diff this against the Rust parser's JSON for the same file (compare 'server_outcome' for status + question/sheet data;
error TEXT produced by Python library exceptions (expat / KeyError / zipfile) is Python-specific: compare the class of failure, not the string).
"""
import hashlib
import importlib.util
import io
import json
import os
import sys

sys.dont_write_bytecode = True

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import paths  # noqa: E402
DEFAULT_REPO = paths.LEGACY_ROOT


def load(repo):
    # server.py inserts APP_DIR in sys.path and imports `app.db`, `app.xlsx_parser`
    spec = importlib.util.spec_from_file_location('xexam_server_under_test', os.path.join(repo, 'server.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def md5(path):
    h = hashlib.md5()
    with open(path, 'rb') as f:
        h.update(f.read())
    return h.hexdigest()


def exc_info(e):
    return {'ok': False, 'exc': type(e).__name__, 'error': str(e)}


def main():
    args = sys.argv[1:]
    repo = DEFAULT_REPO
    meta = False
    reader = True
    files = []
    i = 0
    while i < len(args):
        a = args[i]
        if a == '--repo':
            repo = args[i + 1]
            i += 2
            continue
        if a == '--meta':
            meta = True
        elif a == '--no-reader':
            reader = False
        else:
            files.append(a)
        i += 1
    if len(files) != 1:
        sys.stderr.write(__doc__)
        sys.exit(2)
    path = files[0]
    with open(path, 'rb') as f:
        data = f.read()

    server = load(repo)
    xp = server.xlsx_parser
    out = {'input': {'name': os.path.basename(path), 'size': len(data)}}

    # 1. server.check_xlsx
    try:
        server.check_xlsx(data)
        out['upload_check'] = {'ok': True}
        check_exc = None
    except Exception as e:  # noqa: BLE001 - we want every escape
        out['upload_check'] = exc_info(e)
        check_exc = e

    # 2. parser on its own
    parse_exc = None
    sheets = questions = None
    try:
        sheets, questions = xp.parse_workbook(io.BytesIO(data))
        out['parse'] = {'ok': True, 'sheets': sheets, 'questions': questions}
    except Exception as e:  # noqa: BLE001
        out['parse'] = exc_info(e)
        parse_exc = e

    # 3. what /api/upload would answer
    if check_exc is not None:
        if isinstance(check_exc, ValueError):
            out['server_outcome'] = {'status': 400, 'error': str(check_exc)}
        else:
            out['server_outcome'] = {'status': 400, 'error': '读取这个 Excel 时出错：%s: %s' % (type(check_exc).__name__, check_exc)}
    elif parse_exc is not None:
        if isinstance(parse_exc, ValueError):
            out['server_outcome'] = {'status': 400, 'error': str(parse_exc)}
        else:
            out['server_outcome'] = {'status': 400, 'error': '读取这个 Excel 时出错：%s: %s' % (type(parse_exc).__name__, parse_exc)}
    else:
        out['server_outcome'] = {'status': 200, 'sheets': sheets, 'questions': questions}

    # 4. cell layer
    if reader:
        try:
            raw = xp.read_workbook(io.BytesIO(data))
            out['reader'] = [{'name': n, 'rows': [[rn, {str(k): v for k, v in sorted(cells.items())}] for rn, cells in rows]} for n, rows in raw]
        except Exception as e:  # noqa: BLE001
            out['reader'] = exc_info(e)

    if meta:
        import pyexpat
        out['meta'] = {
            'python': sys.version.split()[0], 'expat': pyexpat.EXPAT_VERSION, 'platform': sys.platform,
            'xlsx_parser_md5': md5(os.path.join(repo, 'app', 'xlsx_parser.py')),
            'server_md5': md5(os.path.join(repo, 'server.py')),
        }

    sys.stdout.buffer.write((json.dumps(out, ensure_ascii=False, sort_keys=True, indent=1) + '\n').encode('utf-8'))


if __name__ == '__main__':
    main()
