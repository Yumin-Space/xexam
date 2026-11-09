# -*- coding: utf-8 -*-
"""Build sample.db with the REAL app.db.Store from the fictional demo workbooks (never touches the owner's data).
Sequence: import demo.xlsx -> 3 attempts (fixed clock) -> import demo_v2.xlsx (1 updated, 1 removed, 2 added)
so the file contains: active + inactive questions, stats incl. a wrong-answer notebook entry, attempts, imports, meta.
usage: python -I -B make_sample_db.py [out_db]
"""
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import paths  # noqa: E402
sys.path.insert(0, paths.LEGACY_ROOT)
from app import db, xlsx_parser  # noqa: E402

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(paths.FIXTURES, 'sample.db')
D1 = paths.DEMO
D2 = paths.DEMO2
for s in ('', '-journal'):
    if os.path.exists(OUT + s):
        os.remove(OUT + s)

clock = iter(['2026-01-05 09:00', '2026-01-05 09:10', '2026-01-06 20:15', '2026-01-06 20:30', '2026-01-07 08:00', '2026-01-07 08:05', '2026-01-07 08:06',
              '2026-01-07 08:07', '2026-01-07 08:08', '2026-01-07 08:09'])
db.now = lambda: next(clock)

st = db.Store(OUT)
s1, q1 = xlsx_parser.parse_workbook(D1)
st.apply_import('demo.xlsx', s1, q1)                                  # t0 09:00
ids = [q['id'] for q in q1]
A = lambda pairs, mode='all', sec=60: {'results': [{'id': ids[i], 'correct': c} for i, c in pairs], 'sheets': ['demo'], 'mode': mode, 'seconds': sec}
st.record_attempt(A([(0, True), (1, False), (2, False), (3, True), (4, True)], 'all', 95))   # 09:10  -> 3/5 = 60
st.record_attempt(A([(1, True), (2, False)], 'weak', 40))                                    # 20:15  -> 1/2 = 50
st.record_attempt(A([(1, True), (2, True), (3, False)], 'retry', 70))                        # 20:30  -> 2/3 = 67
s2, q2 = xlsx_parser.parse_workbook(D2)
st.apply_import('demo_v2.xlsx', s2, q2)                                                      # 08:00
st.close()
print('wrote', OUT, os.path.getsize(OUT), 'bytes; leftover journal:', os.path.exists(OUT + '-journal'))
import sqlite3
c = sqlite3.connect(OUT)
print('integrity_check', c.execute('PRAGMA integrity_check').fetchone()[0],
      '| counts', {t: c.execute('SELECT COUNT(*) FROM %s' % t).fetchone()[0] for t in ('questions', 'stats', 'attempts', 'imports', 'meta')},
      '| active', c.execute('SELECT COUNT(*) FROM questions WHERE active=1').fetchone()[0])
