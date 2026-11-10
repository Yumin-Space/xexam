# -*- coding: utf-8 -*-
"""Migration of the old progress.json: the flow described in the docs (start once to create the data directory → drop progress.json in → restart) must really work,
migrate only once, and never overwrite existing progress records. Uses synthetic data only, entirely inside a temp directory.

usage: python -I tests/integration/migrate_check.py <Rust backend executable>
Steps:
  A. Start once with a fresh data directory (creates xexam.db); there is no progress.json yet. The progress records are empty.
  B. Stop the backend, drop in a synthetic progress.json, restart → the progress records now contain the migrated data (a defect found in review: this step used to do nothing).
  C. Restart once more → no repeated migration (the number of score-history entries is unchanged).
  D. Put a progress.json into a directory whose database already has progress records → it is not overwritten.
  E. A malformed progress.json → the app still starts, and the log states the reason plus the migrate-progress hint; after fixing the file, a restart migrates it.
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

exe = os.path.abspath(sys.argv[1])
results = []


def ok(cond, msg):
    results.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(results))
        sys.exit(1)


def free_port():
    with socket.socket() as sk:
        sk.bind(('127.0.0.1', 0))
        return sk.getsockname()[1]


PORT = free_port()
QID = 'a1b2c3d4e5f6'
SAMPLE = {
    'version': 1,
    'questions': {QID: {'attempts': 3, 'correct': 1, 'wrong': 2, 'streak': 0, 'last': False, 'at': '2026-01-02 03:04'}},
    'history': [{'at': '2026-01-02 03:04', 'sheets': ['demo'], 'mode': 'all', 'total': 3, 'correct': 1, 'percent': 33, 'seconds': 50}],
}


def run_once(data, log_name, seconds_up=1.0):
    """Start the backend, wait until it is up, read /api/progress, then stop it. Returns (progress, log text)."""
    env = {k: v for k, v in os.environ.items() if not k.startswith('XEXAM_')}
    env.update(XEXAM_DATA=data, XEXAM_HOST='127.0.0.1', XEXAM_PORT=str(PORT), XEXAM_VERSION='0.3')
    logp = os.path.join(data, '..', log_name)
    with open(logp, 'wb') as log:
        p = subprocess.Popen([exe, 'serve', '--no-browser'], env=env, cwd=data, stdout=log, stderr=subprocess.STDOUT)
        try:
            prog = None
            for _ in range(120):
                if p.poll() is not None:
                    break
                try:
                    with urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:%d/api/progress' % PORT, headers={'Host': 'localhost:%d' % PORT}), timeout=1) as r:
                        prog = json.loads(r.read().decode('utf-8'))
                        break
                except Exception:
                    time.sleep(0.2)
        finally:
            p.terminate()
            try:
                p.wait(timeout=10)
            except Exception:
                p.kill()
            time.sleep(0.3)
    return prog, open(logp, encoding='utf-8', errors='replace').read()


root = tempfile.mkdtemp(prefix='xexam_migrate_')
try:
    # A
    d = os.path.join(root, 'a')
    os.makedirs(d)
    prog, _ = run_once(d, 'a1.log')
    ok(prog is not None and prog['history'] == [] and prog['questions'] == {}, 'A. start with a fresh directory: the progress records are empty')
    ok(os.path.exists(os.path.join(d, 'xexam.db')), 'A. xexam.db has been created')

    # B
    with open(os.path.join(d, 'progress.json'), 'w', encoding='utf-8') as f:
        json.dump(SAMPLE, f, ensure_ascii=False)
    prog, log = run_once(d, 'a2.log')
    ok(prog is not None and len(prog['history']) == 1 and QID in prog['questions'], 'B. the database already existed and progress.json was dropped in afterwards; after a restart it is migrated (1 score entry, stats for 1 question)')
    ok('Migrated the old study records into the database' in log, 'B. the log reports that the old progress records were migrated into the database')

    # C
    prog, log = run_once(d, 'a3.log')
    ok(prog is not None and len(prog['history']) == 1, 'C. restart once more: no repeated migration (still 1 score entry)')
    ok('Migrated the old study records into the database' not in log and 'Did not migrate' not in log, 'C. the log has no extra migration messages')

    # D: the database already has progress records (migrated above); swap in a different progress.json — not overwritten
    other = dict(SAMPLE, history=[dict(SAMPLE['history'][0], percent=99)] * 2)
    shutil.copy(os.path.join(d, 'xexam.db'), os.path.join(root, 'keep.db'))
    with open(os.path.join(d, 'progress.json'), 'w', encoding='utf-8') as f:
        json.dump(other, f, ensure_ascii=False)
    prog, _ = run_once(d, 'a4.log')
    ok(prog is not None and len(prog['history']) == 1 and prog['history'][0]['percent'] == 33, 'D. a database that was already migrated: not overwritten by the new progress.json')

    # E
    e = os.path.join(root, 'e')
    os.makedirs(e)
    run_once(e, 'e1.log')
    with open(os.path.join(e, 'progress.json'), 'w', encoding='utf-8') as f:
        f.write('{ this is not json')
    prog, log = run_once(e, 'e2.log')
    ok(prog is not None and prog['history'] == [], 'E. malformed progress.json: the app still starts and the progress records stay empty')
    ok('migrate-progress' in log, 'E. the log hints at xexam-backend migrate-progress')
    with open(os.path.join(e, 'progress.json'), 'w', encoding='utf-8') as f:
        json.dump(SAMPLE, f, ensure_ascii=False)
    prog, _ = run_once(e, 'e3.log')
    ok(prog is not None and len(prog['history']) == 1, 'E. after fixing progress.json, a restart migrates it')
finally:
    shutil.rmtree(root, ignore_errors=True)

print('\n'.join(results))
print('\nAll %d checks passed' % len(results))
