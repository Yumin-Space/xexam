# -*- coding: utf-8 -*-
"""Test-only oracle: drives the ORIGINAL Python implementation (app/db.py, app/xlsx_parser.py).

Usage: python -I -B py_oracle.py <repo_root> <command> [args...]
Commands (all print UTF-8 JSON to stdout):
  parse <xlsx>                      -> {"sheets": [...], "questions": [...]}
  build <db> <now>                  stdin: {"imports": [{"filename","sheets","questions"}], "attempts": [...]}
                                    builds a database with the Python Store using a fixed clock
  dump <db>                         -> {"bank","progress","imports"} read through the Python Store
  schema <db>                       -> sqlite_master rows in rowid order
  migrate <db> <now> <progress.json> -> {"result": [bool, msg], "dump": {...}}
  percent <max_total>               -> [[round(100*g/t) for g in 0..=t] for t in 1..=max_total]
"""
import json
import sqlite3
import sys

sys.dont_write_bytecode = True
repo, cmd, args = sys.argv[1], sys.argv[2], sys.argv[3:]
sys.path.insert(0, repo)
from app import db as D  # noqa: E402


def out(obj):
    sys.stdout.buffer.write(json.dumps(obj, ensure_ascii=False).encode('utf-8'))


def dump(store):
    return {'bank': store.bank(), 'progress': store.progress(), 'imports': store.import_history(100)}


if cmd == 'parse':
    from app import xlsx_parser as P
    sheets, questions = P.parse_workbook(args[0])
    out({'sheets': sheets, 'questions': questions})
elif cmd == 'build':
    D.now = lambda: args[1]
    st = D.Store(args[0])
    spec = json.loads(sys.stdin.buffer.read().decode('utf-8'))
    plans = []
    for imp in spec.get('imports', []):
        plans.append(st.apply_import(imp['filename'], imp['sheets'], imp['questions']))
    for att in spec.get('attempts', []):
        st.record_attempt(att)
    st.close()
    out({'ok': True, 'plans': plans})
elif cmd == 'dump':
    st = D.Store(args[0])
    d = dump(st)
    st.close()
    out(d)
elif cmd == 'schema':
    con = sqlite3.connect(args[0])
    rows = con.execute('SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY rowid').fetchall()
    meta = con.execute('SELECT key, value FROM meta ORDER BY key').fetchall()
    pragmas = [con.execute('PRAGMA journal_mode').fetchone()[0], con.execute('PRAGMA user_version').fetchone()[0]]
    con.close()
    out({'master': rows, 'meta': meta, 'pragmas': pragmas})
elif cmd == 'migrate':
    D.now = lambda: args[1]
    st = D.Store(args[0])
    res = st.migrate_legacy_progress(args[2])
    d = dump(st)
    st.close()
    out({'result': list(res), 'dump': d})
elif cmd == 'percent':
    n = int(args[0])
    out([[round(100 * g / t) for g in range(t + 1)] for t in range(1, n + 1)])
else:
    sys.exit('unknown command ' + cmd)
