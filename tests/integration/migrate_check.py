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
  F. SQLite journal mode (v0.7): the database runs in WAL mode and the log says so once; XEXAM_SQLITE_JOURNAL=delete switches it back to
     DELETE (what an older version does on a rollback) with every record kept, and the next start without it is in WAL mode again.
  G. Only the six-column layout (v0.7): a schema-3 database (as v0.6 wrote it) whose bank holds questions of the old 3-column
     layout is upgraded (now straight to schema 8) with one backup, and its old-layout questions are retired (records,
     statistics and the bank stay; the log names the bank); the next start retires nothing more. The `import` command refuses
     a 3-column workbook (English message, exit code 1, nothing written) and imports a six-column one into that bank. The
     workbooks are made-up and written by this script (a minimal .xlsx built with zipfile).
  H. Schema 4 to 8 in one step (v0.8 + v0.9 + v1.1 + v1.3): a schema-4 database (as v0.7.5 wrote it: the G database with a second
     user, the two v0.8, the two v0.9 and the four v1.1 columns dropped again and meta.schema set back to 4) is upgraded to schema 8
     with a backup named xexam-schema4-*; every user then has ai_enabled = 0 and its own 32-hex uid; the next start
     writes nothing (no second backup, the same uids).
  I. The AI link (v0.8): `serve` creates <data>/link/internal.token (43 base64url characters; mode 600 and a folder of
     mode 700 on POSIX systems) and reads the same token again on the next start; with XEXAM_INTERNAL_TOKEN set no file
     is created; a token shorter than 32 characters turns the link off with a log line and the app still runs.
  J. Schema 5 to 8 (v0.9 + v1.1 + v1.3): a v0.8 database with study records (the H database with the two v0.9 attempts columns and
     the four v1.1 columns dropped again and meta.schema set back to 5) is upgraded to schema 8 with exactly one backup named
     xexam-schema5-*; attempts.session_key, attempts.started_at, attempts.time_limit and attempts.ended exist and are NULL
     for the old records, every record is unchanged, and
     the next start writes nothing (no new backup, no upgrade line, the same rows).
  K. Schema 7 to 8 (v1.3): the J database turned back into what v1.2 wrote (no v1.3 columns, tables or index, a session row with
     only the v1.2 columns) is upgraded with exactly one backup named xexam-schema7-*; users.last_active_at is backfilled with
     seconds, sessions.started_at is filled, the first activity row is system.migrate {"from": 7, "to": 8}, the sign-in switch
     stays off, the next start writes nothing; a fresh database has the same columns and index names as the migrated one; a
     v1.2-style session row (no started_at) still works on v1.3.
  L. Sign-in info matrix (v1.3, one switch, the eye): closed (default) -> NOTHING is recorded (no IP or location in users,
     sessions or activity_log), every API answers null, the device (User-Agent) is stored; open -> recording starts and is shown
     (the sign-in made while closed has none); a malformed address falls back to the connecting address, a malformed country is
     ignored; closed again -> hidden, nothing is deleted, a new sign-in neither records nor overwrites the stored values;
     POST /api/users/signin-info/clear (needs an administrator's session, works while closed) -> no user, session or earlier log
     row holds an IP or location, one activity entry users.signinInfoClear with a row count; open again -> recording resumes.
Since v0.6 every route except /api/info needs a session: each run signs in as the default administrator (admin / admin the first
time, then the password this script sets) and reads the administrator's /api/progress, which is where the old records go.
"""
import contextlib
import json
import os
import re
import shutil
import socket
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import zipfile

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


NEW_PASSWORD = 'migrate-check-pass'


def api(method, path, body=None, cookie=None):
    """One request; returns (status, headers, JSON body)."""
    headers = {'Host': 'localhost:%d' % PORT}
    data = None
    if body is not None:
        data = json.dumps(body).encode('utf-8')
        headers.update({'Content-Type': 'application/json', 'Origin': 'http://localhost:%d' % PORT})
    if cookie:
        headers['Cookie'] = cookie
    r = urllib.request.Request('http://127.0.0.1:%d%s' % (PORT, path), data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(r, timeout=5) as resp:
            return resp.status, resp.headers, json.loads(resp.read().decode('utf-8'))
    except urllib.error.HTTPError as e:
        return e.code, e.headers, json.loads(e.read().decode('utf-8') or 'null')


def sign_in():
    """Signs in as the administrator (choosing the new password on the first sign-in). Returns the cookie."""
    for pw in ('admin', NEW_PASSWORD):
        st, h, j = api('POST', '/api/auth/login', {'username': 'admin', 'password': pw})
        if st == 200:
            cookie = h.get('Set-Cookie', '').split(';')[0]
            if j['user']['mustChangePassword']:
                st, h, _ = api('POST', '/api/auth/password', {'password': NEW_PASSWORD}, cookie)
                ok(st == 200, 'the administrator chose a new password')
                # The password change rotates the session (v0.6.5): continue with the new session cookie
                cookie = next((c for c in h.get_all('Set-Cookie') or [] if c.startswith('xexam_session=')), '').split(';')[0]
            return cookie
    ok(False, 'could not sign in as the administrator')


def journal_mode(data):
    """The journal mode stored in <data>/xexam.db (read while the backend is stopped)."""
    with contextlib.closing(sqlite3.connect(os.path.join(data, 'xexam.db'))) as c:
        return c.execute('PRAGMA journal_mode').fetchone()[0].lower()


def copy_database(data, target):
    """Copies xexam.db with its -wal / -shm files, if any (a copy of xexam.db alone can miss the latest commits in WAL mode)."""
    for suffix in ('', '-wal', '-shm'):
        src = os.path.join(data, 'xexam.db' + suffix)
        if os.path.exists(src):
            shutil.copy(src, target + suffix)


def run_once(data, log_name, seconds_up=1.0, extra_env=None, check=None):
    """Start the backend, wait until it is up, sign in, read /api/progress (and run `check(cookie)` if given), then stop it.
    Returns (progress, log text)."""
    env = {k: v for k, v in os.environ.items() if not k.startswith('XEXAM_')}
    env.update(XEXAM_DATA=data, XEXAM_HOST='127.0.0.1', XEXAM_PORT=str(PORT), XEXAM_VERSION='0.3')
    env.update(extra_env or {})
    logp = os.path.join(data, '..', log_name)
    with open(logp, 'wb') as log:
        p = subprocess.Popen([exe, 'serve', '--no-browser'], env=env, cwd=data, stdout=log, stderr=subprocess.STDOUT)
        try:
            prog = None
            for _ in range(120):
                if p.poll() is not None:
                    break
                try:
                    with urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:%d/api/info' % PORT, headers={'Host': 'localhost:%d' % PORT}), timeout=1):
                        pass
                except Exception:
                    time.sleep(0.2)
                    continue
                cookie = sign_in()
                st, _, prog = api('GET', '/api/progress', cookie=cookie)
                ok(st == 200, 'signed in, /api/progress answers')
                if check:
                    check(cookie)
                break
        finally:
            p.terminate()
            try:
                p.wait(timeout=10)
            except Exception:
                p.kill()
            time.sleep(0.3)
    return prog, open(logp, encoding='utf-8', errors='replace').read()


def xlsx(path, rows):
    """Writes a minimal one-sheet .xlsx: `rows` are lists of cell texts from column A on (inline strings)."""
    def esc(t):
        return t.replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;')
    cells = []
    for r, row in enumerate(rows, 1):
        cs = ''.join('<c r="%s%d" t="inlineStr"><is><t xml:space="preserve">%s</t></is></c>' % (chr(65 + k), r, esc(t))
                     for k, t in enumerate(row))
        cells.append('<row r="%d">%s</row>' % (r, cs))
    ns = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
    rel = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
    pkg = 'http://schemas.openxmlformats.org/package/2006/relationships'
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as z:
        z.writestr('[Content_Types].xml',
                   '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
                   '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
                   '<Default Extension="xml" ContentType="application/xml"/>'
                   '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
                   '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
                   '</Types>')
        z.writestr('_rels/.rels', '<Relationships xmlns="%s"><Relationship Id="rId1" Type="%s/officeDocument" Target="xl/workbook.xml"/></Relationships>' % (pkg, rel))
        z.writestr('xl/workbook.xml', '<workbook xmlns="%s" xmlns:r="%s"><sheets><sheet name="Garden" sheetId="1" r:id="rId1"/></sheets></workbook>' % (ns, rel))
        z.writestr('xl/_rels/workbook.xml.rels', '<Relationships xmlns="%s"><Relationship Id="rId1" Type="%s/worksheet" Target="worksheets/sheet1.xml"/></Relationships>' % (pkg, rel))
        z.writestr('xl/worksheets/sheet1.xml', '<worksheet xmlns="%s"><sheetData>%s</sheetData></worksheet>' % (ns, ''.join(cells)))


def six_column_rows(n):
    """`n` made-up six-column questions: A-C the question in English, Chinese, Japanese; D-F the answer in the same languages."""
    return [[
        'A gardener waters the herbs every morning (question %d). Which can is used?\n\nA. The small can\n\nB. The big can\n\nC. The hose' % i,
        '【中文翻译】\n园丁每天早上给香草浇水（第 %d 题）。用哪个水壶？\nA. 小水壶\nB. 大水壶\nC. 水管' % i,
        '庭師が毎朝ハーブに水をやります（問題 %d）。どのじょうろを使いますか。\n\nA. 小さいじょうろ\n\nB. 大きいじょうろ\n\nC. ホース' % i,
        '【Answer】A\n\n【Explanation】\nA small can is enough for herbs.',
        '【答案】A\n\n【解析·怎么选】\n浇香草用小水壶就够了。',
        '【正解】A\n\n【解説】\nハーブには小さいじょうろで十分です。',
    ] for i in range(1, n + 1)]


# The old three-column layout (A the Japanese question, B the Chinese translation, C the answer): refused since v0.7
THREE_COLUMN_ROWS = [[
    'これは庭についての十分に長い日本語の問題文です。どれが正しいですか。\n\nA. 甲です\n\nB. 乙です\n\nC. 丙です',
    '【中文翻译】\n这是一个关于花园的足够长的中文题目。哪个正确？\nA. 甲\nB. 乙\nC. 丙',
    '【答案】B\n\n【解析·怎么选】因为乙。',
]]

# Schema 3 (v0.6), as its migrations created it: the tables the v4 migration starts from
V3_DDL = """
CREATE TABLE meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
CREATE TABLE questions (id TEXT PRIMARY KEY NOT NULL, sheet TEXT NOT NULL, row_no INTEGER NOT NULL, seq INTEGER NOT NULL,
  data TEXT NOT NULL, active INTEGER NOT NULL DEFAULT 1, added_at TEXT NOT NULL, updated_at TEXT NOT NULL, bank_id INTEGER);
CREATE INDEX idx_questions_sheet ON questions(sheet, active);
CREATE TABLE attempts (id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, sheets TEXT NOT NULL, mode TEXT NOT NULL,
  total INTEGER NOT NULL, correct INTEGER NOT NULL, percent INTEGER NOT NULL, seconds INTEGER NOT NULL,
  banks TEXT NOT NULL DEFAULT '[]', user_id INTEGER);
CREATE TABLE stats (question_id TEXT PRIMARY KEY NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, correct INTEGER NOT NULL DEFAULT 0,
  wrong INTEGER NOT NULL DEFAULT 0, streak INTEGER NOT NULL DEFAULT 0, last INTEGER, at TEXT);
CREATE TABLE imports (id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, filename TEXT NOT NULL, added INTEGER NOT NULL,
  updated INTEGER NOT NULL, removed INTEGER NOT NULL, restored INTEGER NOT NULL, skipped INTEGER NOT NULL, total INTEGER NOT NULL,
  bank_id INTEGER);
CREATE TABLE banks (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, filename TEXT NOT NULL,
  legacy_ids INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE TABLE attempt_items (attempt_id INTEGER NOT NULL, seq INTEGER NOT NULL, question_id TEXT NOT NULL, answer TEXT,
  correct INTEGER NOT NULL, PRIMARY KEY (attempt_id, seq));
CREATE INDEX idx_attempt_items_question ON attempt_items(question_id);
CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
CREATE INDEX idx_questions_bank ON questions(bank_id, active);
CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT NOT NULL UNIQUE COLLATE NOCASE, password_hash TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('admin', 'user')), disabled INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL,
  last_login_at TEXT, password_changed_at TEXT NOT NULL, must_change_password INTEGER NOT NULL DEFAULT 0);
CREATE TABLE sessions (token_hash TEXT PRIMARY KEY NOT NULL, user_id INTEGER NOT NULL, created_at TEXT NOT NULL,
  last_seen_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE TABLE user_stats (user_id INTEGER NOT NULL, question_id TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
  correct INTEGER NOT NULL DEFAULT 0, wrong INTEGER NOT NULL DEFAULT 0, streak INTEGER NOT NULL DEFAULT 0, last INTEGER, at TEXT,
  PRIMARY KEY (user_id, question_id));
CREATE TABLE user_settings (user_id INTEGER NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (user_id, key));
CREATE INDEX idx_attempts_user ON attempts(user_id);
"""


def write_v3_database(path, admin_hash):
    """A schema-3 database with the administrator `admin` (password hash `admin_hash`) and the made-up bank "Herbs", whose two
    questions are of the old 3-column layout (no "lang" in their data), with statistics and one exam record."""
    with contextlib.closing(sqlite3.connect(path)) as c:
        c.executescript(V3_DDL)
        c.execute("INSERT INTO users(id, username, password_hash, role, disabled, created_at, password_changed_at, must_change_password) "
                  "VALUES (1, 'admin', ?, 'admin', 0, '2026-01-01 10:00', '2026-01-01 10:00', 0)", (admin_hash,))
        c.executescript("""
            INSERT INTO banks(id, name, filename, legacy_ids, created_at, updated_at)
              VALUES (1, 'Herbs', 'Herbs.xlsx', 1, '2026-01-01 10:00', '2026-01-01 10:00');
            INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) VALUES
              ('h1', 'Garden', 1, 0, '{"kind": "single", "stem": "Basil or mint?"}', 1, '2026-01-01 10:00', '2026-01-01 10:00', 1),
              ('h2', 'Garden', 2, 1, '{"kind": "single", "stem": "Thyme or sage?"}', 1, '2026-01-01 10:00', '2026-01-01 10:00', 1);
            INSERT INTO user_stats(user_id, question_id, attempts, correct, wrong, streak, last, at)
              VALUES (1, 'h1', 2, 1, 1, 0, 0, '2026-01-03 10:00');
            INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds, banks, user_id)
              VALUES ('2026-01-03 10:00', '["Garden"]', 'all', 1, 0, 0, 20, '[{"id": 1, "name": "Herbs"}]', 1);
            INSERT INTO attempt_items(attempt_id, seq, question_id, answer, correct) VALUES (1, 0, 'h1', '["A"]', 0);
            INSERT INTO meta(key, value) VALUES ('schema', '3');
        """)
        c.commit()


def run_cli(data, *args):
    """Runs a command of the backend on `data`; returns (exit code, stdout + stderr)."""
    env = {k: v for k, v in os.environ.items() if not k.startswith('XEXAM_')}
    r = subprocess.run([exe, *args, '--data', data], env=env, capture_output=True, text=True, encoding='utf-8', errors='replace')
    return r.returncode, r.stdout + r.stderr


def herbs(cookie):
    """The bank "Herbs" in GET /api/banks."""
    st, _, banks = api('GET', '/api/banks', cookie=cookie)
    ok(st == 200, 'GET /api/banks answers')
    return next(b for b in banks['banks'] if b['name'] == 'Herbs')


root = tempfile.mkdtemp(prefix='xexam_migrate_')
try:
    # A
    d = os.path.join(root, 'a')
    os.makedirs(d)
    prog, log = run_once(d, 'a1.log')
    ok(prog is not None and prog['history'] == [] and prog['questions'] == {}, 'A. start with a fresh directory: the progress records are empty')
    ok(os.path.exists(os.path.join(d, 'xexam.db')), 'A. xexam.db has been created')
    ok(log.count('SQLite journal mode: wal') == 1, 'A. the log states the journal mode once: wal')
    ok(journal_mode(d) == 'wal', 'A. xexam.db is in WAL mode')
    token_file = os.path.join(d, 'link', 'internal.token')
    ok(os.path.isfile(token_file), 'I. serve created link/internal.token in the data directory')
    ok('AI link: ' in log and 'internal.token' in log, 'I. the log names the token file (AI link: <path>)')
    with open(token_file, encoding='ascii') as fh:
        first_token = fh.read()
    ok(re.fullmatch(r'[A-Za-z0-9_-]{43}\n?', first_token) is not None, 'I. the token is 43 base64url characters')
    if os.name == 'posix':
        ok(stat.S_IMODE(os.stat(token_file).st_mode) == 0o600, 'I. internal.token has mode 600')
        ok(stat.S_IMODE(os.stat(os.path.dirname(token_file)).st_mode) == 0o700, 'I. the link folder has mode 700')

    # B
    with open(os.path.join(d, 'progress.json'), 'w', encoding='utf-8') as f:
        json.dump(SAMPLE, f, ensure_ascii=False)
    prog, log = run_once(d, 'a2.log')
    ok(prog is not None and len(prog['history']) == 1 and QID in prog['questions'], 'B. the database already existed and progress.json was dropped in afterwards; after a restart it is migrated (1 score entry, stats for 1 question)')
    ok('Migrated the old study records into the database' in log, 'B. the log reports that the old progress records were migrated into the database')

    # C
    prog, log = run_once(d, 'a3.log')
    ok(prog is not None and len(prog['history']) == 1, 'C. restart once more: no repeated migration (still 1 score entry)')
    with open(token_file, encoding='ascii') as fh:
        ok(fh.read() == first_token, 'I. later starts read the same token again (it is created only once)')
    ok('Migrated the old study records into the database' not in log and 'Did not migrate' not in log, 'C. the log has no extra migration messages')

    # D: the database already has progress records (migrated above); swap in a different progress.json — not overwritten
    other = dict(SAMPLE, history=[dict(SAMPLE['history'][0], percent=99)] * 2)
    copy_database(d, os.path.join(root, 'keep.db'))
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

    # F: the journal mode opt-out, and back
    prog, log = run_once(d, 'f1.log', extra_env={'XEXAM_SQLITE_JOURNAL': 'delete'})
    ok('SQLite journal mode: delete (XEXAM_SQLITE_JOURNAL=delete)' in log, 'F. XEXAM_SQLITE_JOURNAL=delete: the log states the DELETE mode')
    ok(journal_mode(d) == 'delete', 'F. xexam.db is back in DELETE mode')
    ok(prog is not None and len(prog['history']) == 1 and prog['history'][0]['percent'] == 33, 'F. every record is still there in DELETE mode')
    ok(not os.path.exists(os.path.join(d, 'xexam.db-wal')), 'F. no -wal file in DELETE mode')
    prog, log = run_once(d, 'f2.log')
    ok('SQLite journal mode: wal' in log and journal_mode(d) == 'wal', 'F. the next start without the variable is in WAL mode again')
    ok(prog is not None and len(prog['history']) == 1, 'F. every record is still there in WAL mode')

    # G: the old layouts' questions are retired at the upgrade; only six-column workbooks can be imported
    with contextlib.closing(sqlite3.connect(os.path.join(d, 'xexam.db'))) as c:
        admin_hash = c.execute("SELECT password_hash FROM users WHERE username='admin'").fetchone()[0]
    g = os.path.join(root, 'g')
    os.makedirs(g)
    write_v3_database(os.path.join(g, 'xexam.db'), admin_hash)
    prog, log = run_once(g, 'g1.log')
    ok('Upgraded the database to schema 8' in log, 'G. the schema-3 database is upgraded to schema 8')
    names = [n for n in os.listdir(os.path.join(g, 'backups')) if n.startswith('xexam-schema')]
    ok(len(names) == 1 and names[0].startswith('xexam-schema3-'),
       'G. one backup of the schema-3 database was written first (xexam-schema3-*). Actual %r' % names)
    ok('retired 2 questions of bank "Herbs": they use the old 3- / 4-column layout; import the six-column version of the workbook' in log,
       'G. the log names the bank whose old-layout questions were retired')
    ok(prog is not None and 'h1' in prog['questions'] and len(prog['history']) == 1, 'G. the statistics and the exam record are kept')
    with contextlib.closing(sqlite3.connect(os.path.join(g, 'xexam.db'))) as c:
        rows = c.execute('SELECT id, active FROM questions ORDER BY id').fetchall()
        kept = c.execute('SELECT (SELECT COUNT(*) FROM banks), (SELECT COUNT(*) FROM attempts), (SELECT COUNT(*) FROM user_stats)').fetchone()
    ok(rows == [('h1', 0), ('h2', 0)], 'G. both old-layout questions are retired (active = 0), none is deleted. Actual %r' % rows)
    ok(kept == (1, 1, 1), 'G. the bank, the record and the statistics stay. Actual %r' % (kept,))
    run_cli(g, 'migrate-progress')
    prog, log = run_once(g, 'g2.log')
    ok('retired' not in log, 'G. the next start retires nothing more')
    files = os.path.join(root, 'files')
    os.makedirs(files)
    three = os.path.join(files, 'three.xlsx')
    xlsx(three, THREE_COLUMN_ROWS)
    code, out = run_cli(g, 'import', three)
    ok(code == 1 and 'the workbook uses the old 3- or 4-column layout; only the six-column layout can be imported' in out,
       'G. the import command refuses a 3-column workbook in English (exit 1). Actual %d %r' % (code, out[-300:]))
    with contextlib.closing(sqlite3.connect(os.path.join(g, 'xexam.db'))) as c:
        n = c.execute('SELECT (SELECT COUNT(*) FROM questions), (SELECT COUNT(*) FROM imports)').fetchone()
    ok(n == (2, 0), 'G. nothing was written by the refused import. Actual %r' % (n,))
    six = os.path.join(files, 'Herbs (six columns).xlsx')
    xlsx(six, six_column_rows(3))
    code, out = run_cli(g, 'import', six)
    ok(code == 0 and '3 added' in out, 'G. the import command imports a six-column workbook. Actual %d %r' % (code, out[-300:]))
    seen = {}
    prog, log = run_once(g, 'g3.log', check=lambda cookie: seen.update(herbs=herbs(cookie)))
    b = seen.get('herbs')
    ok(b is not None and b['total'] == 3 and b['inactive'] == 2, 'G. the bank now has the 3 six-column questions in use and the 2 retired ones. Actual %r' % b)
    ok('retired' not in log, 'G. six-column questions are never retired')

    # H: schema 4 -> 8. A second user first, then the database is turned back into what v0.7.5 wrote
    def add_user(cookie):
        st, _, j = api('POST', '/api/users/create', {'username': 'robin', 'password': 'robin-initial-2026', 'role': 'user',
                                                     'banks': 'all'}, cookie)
        ok(st == 200, 'H. a second user was created. Actual %d %r' % (st, j))
    run_once(g, 'h1.log', check=add_user)
    gdb, backups = os.path.join(g, 'xexam.db'), os.path.join(g, 'backups')
    with contextlib.closing(sqlite3.connect(gdb)) as c:
        cols = [r[1] for r in c.execute('PRAGMA table_info(users)')]
        ok('ai_enabled' in cols and 'uid' in cols, 'H. a v0.8 database has users.ai_enabled and users.uid. Actual %r' % cols)
        ok(c.execute("SELECT value FROM meta WHERE key = 'schema'").fetchone()[0] == '8', 'H. a v1.3 database is schema 8')
        c.execute('ALTER TABLE users DROP COLUMN ai_enabled')
        c.execute('ALTER TABLE users DROP COLUMN uid')
        c.execute('DROP INDEX IF EXISTS idx_attempts_session')  # v1.3 index on session_key
        c.execute('ALTER TABLE attempts DROP COLUMN session_key')
        c.execute('ALTER TABLE attempts DROP COLUMN started_at')
        # the v1.1 (schema 7) columns and indexes go too (in the order SQLite allows: CHECKs refer to the earlier column)
        c.execute('DROP INDEX IF EXISTS idx_banks_ai_job')
        c.execute('DROP INDEX IF EXISTS idx_questions_layout')
        for t, col in (('attempts', 'ended'), ('attempts', 'time_limit'), ('banks', 'ai_meta'), ('banks', 'ai_job')):
            c.execute('ALTER TABLE %s DROP COLUMN %s' % (t, col))
        c.execute("UPDATE meta SET value = '4' WHERE key = 'schema'")
        c.commit()
    before = set(os.listdir(backups))
    prog, log = run_once(g, 'h2.log')
    ok('Upgraded the database to schema 8' in log, 'H. the schema-4 database is upgraded to schema 8')
    added = set(os.listdir(backups)) - before
    ok(len(added) == 1 and next(iter(added)).startswith('xexam-schema4-'), 'H. one backup named xexam-schema4-* was written first. Actual %r' % sorted(added))
    with contextlib.closing(sqlite3.connect(gdb)) as c:
        users = c.execute('SELECT username, ai_enabled, uid FROM users ORDER BY id').fetchall()
        schema = c.execute("SELECT value FROM meta WHERE key = 'schema'").fetchone()[0]
    ok(schema == '8' and len(users) == 2, 'H. schema 8 with both users. Actual %r, %d users' % (schema, len(users)))
    ok(all(u[1] == 0 for u in users), 'H. every user starts with AI+ off (ai_enabled = 0)')
    ok(all(re.fullmatch(r'[0-9a-f]{32}', u[2] or '') for u in users) and len({u[2] for u in users}) == len(users),
       'H. every user has its own 32-hex uid')
    ok(prog is not None and len(prog['history']) == 1, 'H. the records are kept')
    prog, log = run_once(g, 'h3.log')
    with contextlib.closing(sqlite3.connect(gdb)) as c:
        again = c.execute('SELECT username, ai_enabled, uid FROM users ORDER BY id').fetchall()
    ok('Upgraded the database' not in log and set(os.listdir(backups)) == before | added and again == users,
       'H. the next start writes nothing: no new backup, the same uids')

    # J: schema 5 -> 8. The H database is turned back into what v0.8 wrote (no session columns), then upgraded
    def attempts_state():
        with contextlib.closing(sqlite3.connect(gdb)) as c:
            return (c.execute('SELECT id, user_id, at, kind, total, correct FROM attempts ORDER BY id').fetchall(),
                    c.execute("SELECT value FROM meta WHERE key = 'schema'").fetchone()[0],
                    [r[1] for r in c.execute('PRAGMA table_info(attempts)')])

    rows_before, schema, cols = attempts_state()
    ok(schema == '8' and 'session_key' in cols and 'started_at' in cols and len(rows_before) >= 1,
       'J. the upgraded database is schema 8 with attempts.session_key and attempts.started_at and holds records. Actual %r %r' % (schema, cols))
    with contextlib.closing(sqlite3.connect(gdb)) as c:
        c.execute('DROP INDEX IF EXISTS idx_attempts_session')  # v1.3 index on session_key
        c.execute('ALTER TABLE attempts DROP COLUMN session_key')
        c.execute('ALTER TABLE attempts DROP COLUMN started_at')
        # the v1.1 (schema 7) columns and indexes go too (in the order SQLite allows: CHECKs refer to the earlier column)
        c.execute('DROP INDEX IF EXISTS idx_banks_ai_job')
        c.execute('DROP INDEX IF EXISTS idx_questions_layout')
        for t, col in (('attempts', 'ended'), ('attempts', 'time_limit'), ('banks', 'ai_meta'), ('banks', 'ai_job')):
            c.execute('ALTER TABLE %s DROP COLUMN %s' % (t, col))
        c.execute("UPDATE meta SET value = '5' WHERE key = 'schema'")
        c.commit()
    before5 = set(os.listdir(backups))
    prog, log = run_once(g, 'j1.log')
    ok('Upgraded the database to schema 8' in log, 'J. the schema-5 database is upgraded to schema 8')
    added5 = set(os.listdir(backups)) - before5
    ok(len(added5) == 1 and next(iter(added5)).startswith('xexam-schema5-'), 'J. exactly one backup named xexam-schema5-* was written first. Actual %r' % sorted(added5))
    rows_after, schema, cols = attempts_state()
    ok(schema == '8' and all(k in cols for k in ('session_key', 'started_at', 'time_limit', 'ended')),
       'J. meta.schema is 8 and the v0.9 and v1.1 attempts columns exist. Actual %r %r' % (schema, cols))
    ok(rows_after == rows_before, 'J. every record is unchanged by the upgrade')
    with contextlib.closing(sqlite3.connect(gdb)) as c:
        tags = c.execute('SELECT COUNT(*), COUNT(session_key), COUNT(started_at), COUNT(time_limit), COUNT(ended) FROM attempts').fetchone()
    ok(tags[0] >= 1 and tags[1:] == (0, 0, 0, 0), 'J. session_key, started_at, time_limit and ended are NULL for the old records. Actual %r' % (tags,))
    ok(prog is not None and len(prog['history']) == 1, 'J. the records are still served')
    prog, log = run_once(g, 'j2.log')
    ok('Upgraded the database' not in log and set(os.listdir(backups)) == before5 | added5 and attempts_state() == (rows_after, '8', cols),
       'J. the next start writes nothing: no upgrade line, no new backup, the same rows')

    # K: schema 7 -> 8. The J database is turned back into what v1.2 wrote, then upgraded
    V8_USER_COLS = ('last_active_at', 'last_login_ip', 'last_login_country', 'last_login_region', 'last_login_city')
    V8_SESSION_COLS = ('started_at', 'remember', 'ip', 'ua')
    with contextlib.closing(sqlite3.connect(gdb)) as c:
        c.execute("UPDATE users SET last_login_at = '2026-11-01 08:00' WHERE username = 'robin'")
        c.execute('DROP INDEX IF EXISTS idx_attempts_session')
        c.execute('DROP TABLE IF EXISTS activity_log')
        c.execute('DROP TABLE IF EXISTS system_settings')
        for col in V8_USER_COLS:
            c.execute('ALTER TABLE users DROP COLUMN %s' % col)
        for col in V8_SESSION_COLS:
            c.execute('ALTER TABLE sessions DROP COLUMN %s' % col)
        c.execute('DELETE FROM sessions')
        c.execute("INSERT INTO sessions(token_hash, user_id, created_at, last_seen_at, expires_at) "
                  "VALUES (?, 1, '2026-11-01 08:00', ?, ?)", ('b' * 64, int(time.time()), int(time.time()) + 3600))
        c.execute("UPDATE meta SET value = '7' WHERE key = 'schema'")
        c.commit()
    before7 = set(os.listdir(backups))
    prog, log = run_once(g, 'k1.log')
    ok('Upgraded the database to schema 8' in log, 'K. the schema-7 database is upgraded to schema 8')
    added7 = set(os.listdir(backups)) - before7
    ok(len(added7) == 1 and next(iter(added7)).startswith('xexam-schema7-'), 'K. exactly one backup named xexam-schema7-* was written first. Actual %r' % sorted(added7))
    with contextlib.closing(sqlite3.connect(os.path.join(backups, next(iter(added7))))) as c:
        ok(c.execute("SELECT value FROM meta WHERE key = 'schema'").fetchone()[0] == '7'
           and 'last_active_at' not in [r[1] for r in c.execute('PRAGMA table_info(users)')],
           'K. the backup copy is the schema-7 database (opens under v1.2 code: no v1.3 column)')
    with contextlib.closing(sqlite3.connect(gdb)) as c:
        ucols = [r[1] for r in c.execute('PRAGMA table_info(users)')]
        scols = [r[1] for r in c.execute('PRAGMA table_info(sessions)')]
        ok(all(k in ucols for k in V8_USER_COLS) and all(k in scols for k in V8_SESSION_COLS), 'K. the v1.3 columns exist. Actual %r %r' % (ucols, scols))
        first = c.execute("SELECT last_active_at FROM users WHERE username = 'robin'").fetchone()[0]
        ok(first == '2026-11-01 08:00:00', 'K. last_active_at is backfilled from last_login_at with seconds. Actual %r' % (first,))
        ok(c.execute('SELECT COUNT(*) FROM sessions WHERE started_at IS NULL').fetchone()[0] == 0, 'K. sessions.started_at is filled by the backfill')
        ok(c.execute('SELECT remember FROM sessions').fetchone()[0] == 1, 'K. an old session row reads remember = 1')
        mig = c.execute("SELECT actor, result, detail FROM activity_log WHERE action = 'system.migrate'").fetchall()
        ok(len(mig) == 1 and mig[0][0] == 'system' and json.loads(mig[0][2]) == {'from': 7, 'to': 8}, 'K. the first log row is system.migrate from 7 to 8. Actual %r' % (mig,))
        ok(c.execute("SELECT COUNT(*) FROM system_settings WHERE key = 'users.signin_info' AND value <> '0'").fetchone()[0] == 0, 'K. the sign-in info switch is off after the upgrade')
        ok(c.execute("SELECT 1 FROM sqlite_master WHERE name = 'idx_attempts_session'").fetchone() is not None, 'K. the idempotent-save index exists')
    prog, log = run_once(g, 'k2.log')
    ok('Upgraded the database' not in log and set(os.listdir(backups)) == before7 | added7, 'K. the next start writes no upgrade line and no new backup')
    # the migrated file and a fresh one have the same columns and index names
    kf = os.path.join(root, 'kf')
    os.makedirs(kf)
    run_once(kf, 'kf.log')

    def shape(path):
        with contextlib.closing(sqlite3.connect(path)) as c:
            tabs = {t: [r[1] for r in c.execute('PRAGMA table_info(%s)' % t)]
                    for (t,) in c.execute("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")}
            idx = sorted(n for (n,) in c.execute("SELECT name FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_%'"))
        return tabs, idx
    tm, im = shape(gdb)
    tf, i_f = shape(os.path.join(kf, 'xexam.db'))
    diff = [t for t, cols_ in tf.items() if sorted(tm.get(t, [])) != sorted(cols_)]
    ok(not diff, 'K. every table of a fresh database has the same columns in the migrated one. Different: %r' % diff)
    ok(set(i_f) <= set(im), 'K. every index of a fresh database exists in the migrated one. Missing %r' % sorted(set(i_f) - set(im)))

    # forward / back: a v1.2-style row (no started_at, no remember) on a v1.3 file is still a working session
    def old_row_works(cookie):
        with contextlib.closing(sqlite3.connect(gdb, timeout=5)) as c:
            c.execute('UPDATE sessions SET started_at = NULL, remember = 1, ip = NULL, ua = NULL')
            c.commit()
        st, _, j = api('GET', '/api/auth/state', cookie=cookie)
        ok(st == 200 and 'user' in j, 'K. a v1.2-style session row (no started_at) is still signed in on v1.3. Actual %d %r' % (st, j))
    run_once(g, 'k3.log', check=old_row_works)

    # L: the sign-in info matrix on a fresh database
    l = os.path.join(root, 'l')
    os.makedirs(l)
    ldb = os.path.join(l, 'xexam.db')
    UA = 'Mozilla/5.0 (X11; Linux x86_64) MigrateCheck/1.0'
    PLACE = {'X-Real-IP': '203.0.113.7', 'X-Geo-Country': 'ZZ', 'X-Geo-Region': 'Example Region', 'X-Geo-City': 'Example City'}

    def hapi(method, path, body=None, cookie=None, extra=None):
        """api() with extra request headers (the ones nginx sets); returns (status, headers, body)."""
        headers = {'Host': 'localhost:%d' % PORT, 'User-Agent': UA}
        headers.update(extra or {})
        data = None
        if body is not None:
            data = json.dumps(body).encode('utf-8')
            headers.update({'Content-Type': 'application/json', 'Origin': 'http://localhost:%d' % PORT})
        if cookie:
            headers['Cookie'] = cookie
        r = urllib.request.Request('http://127.0.0.1:%d%s' % (PORT, path), data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(r, timeout=5) as resp:
                return resp.status, resp.headers, json.loads(resp.read().decode('utf-8'))
        except urllib.error.HTTPError as e:
            return e.code, e.headers, json.loads(e.read().decode('utf-8') or 'null')

    def login(extra):
        st, h, j = hapi('POST', '/api/auth/login', {'username': 'admin', 'password': NEW_PASSWORD}, extra=extra)
        ok(st == 200, 'L. admin signs in. Actual %d %r' % (st, j))
        return h.get('Set-Cookie', '').split(';')[0]

    def wait_for(fn, secs=8):
        end = time.time() + secs
        while time.time() < end:
            if fn():
                return True
            time.sleep(0.25)
        return fn()

    def dbq(sql):
        with contextlib.closing(sqlite3.connect(ldb, timeout=5)) as c:
            return c.execute(sql).fetchall()

    def stored_places():
        return (dbq('SELECT COUNT(*) FROM users WHERE last_login_ip IS NOT NULL OR last_login_country IS NOT NULL OR last_login_region IS NOT NULL OR last_login_city IS NOT NULL')[0][0],
                dbq('SELECT COUNT(*) FROM sessions WHERE ip IS NOT NULL')[0][0],
                dbq('SELECT COUNT(*) FROM activity_log WHERE ip IS NOT NULL OR country IS NOT NULL OR region IS NOT NULL OR city IS NOT NULL')[0][0])

    def matrix(cookie):
        # the first sign-in of this database (run_once) chose the password; below, fresh sign-ins carry the headers nginx sets
        c1 = login(PLACE)
        ok(wait_for(lambda: dbq("SELECT COUNT(*) FROM activity_log WHERE action = 'auth.signin' AND ua = '%s'" % UA)[0][0] >= 1), 'L. the sign-in reaches the activity log with its device')
        time.sleep(1.0)
        ok(stored_places() == (0, 0, 0),
           'L. eye closed (default): no IP and no location is recorded (user, session, log). Actual %r' % (stored_places(),))
        ok(dbq("SELECT last_login_ip, last_login_country, last_login_region, last_login_city FROM users WHERE username = 'admin'")[0]
           == (None, None, None, None), 'L. eye closed: the user row stores no IP and no location')
        ok(dbq("SELECT COUNT(*) FROM sessions WHERE ua = '%s'" % UA)[0][0] >= 1, 'L. the device (User-Agent) is stored while the eye is closed')
        st, _, j = hapi('GET', '/api/users', cookie=c1, extra=PLACE)
        ok(st == 200 and j['signinInfo'] is False and all(u.get('lastLoginIp') is None and u.get('lastLoginLocation') is None for u in j['users']),
           'L. eye closed: the users answer carries signinInfo false and null IP / location. Actual %r' % (j,))
        st, _, j = hapi('GET', '/api/activity?limit=50', cookie=c1)
        ok(st == 200 and all(e['ip'] is None and e['location'] is None for e in j['entries']), 'L. eye closed: the activity answer has null ip / location')
        # open
        st, _, j = hapi('POST', '/api/users/signin-info', {'enabled': True}, c1)
        ok(st == 200 and j['signinInfo'] is True, 'L. the admin opens the eye. Actual %d %r' % (st, j))
        me = [u for u in j['users'] if u['username'] == 'admin'][0]
        ok(me['lastLoginIp'] is None and me['lastLoginLocation'] is None,
           'L. eye open: the sign-in made while it was closed has no IP / location. Actual %r' % (me,))
        c2 = login(PLACE)
        st, _, j = hapi('GET', '/api/users', cookie=c2)
        me = [u for u in j['users'] if u['username'] == 'admin'][0]
        ok(me['lastLoginIp'] == '203.0.113.7' and me['lastLoginLocation'] == {'country': 'ZZ', 'region': 'Example Region', 'city': 'Example City'},
           'L. eye open: the users answer shows the IP and the location. Actual %r' % (me,))
        ok(dbq("SELECT last_login_ip, last_login_country, last_login_region, last_login_city FROM users WHERE username = 'admin'")[0]
           == ('203.0.113.7', 'ZZ', 'Example Region', 'Example City'), 'L. eye open: the user row stores the IP and the location')
        ok(dbq("SELECT COUNT(*) FROM sessions WHERE ip = '203.0.113.7'")[0][0] >= 1, 'L. eye open: the session stores the IP')
        ok(wait_for(lambda: dbq("SELECT COUNT(*) FROM activity_log WHERE action = 'auth.signin' AND ip = '203.0.113.7' AND country = 'ZZ'")[0][0] >= 1),
           'L. eye open: the sign-in entry stores the IP and the country')
        st, _, j = hapi('GET', '/api/activity?limit=50', cookie=c2)
        top = [e for e in j['entries'] if e['action'] == 'auth.signin' and e['ip']]
        ok(st == 200 and top and top[0]['location']['city'] == 'Example City', 'L. eye open: the activity answer shows IP and location. Actual %r' % (top[:1],))
        ok(any(e['action'] == 'auth.signin' and e['ip'] is None for e in j['entries']), 'L. eye open: the entry written while closed shows no IP')
        # malformed header values are ignored
        before = dbq('SELECT COUNT(*) FROM activity_log')[0][0]
        login({'X-Real-IP': 'not-an-ip', 'X-Geo-Country': 'zzz', 'X-Geo-City': '   '})
        ok(wait_for(lambda: dbq('SELECT COUNT(*) FROM activity_log')[0][0] > before), 'L. the next sign-in is logged')
        row = dbq("SELECT ip, country, city FROM activity_log WHERE action = 'auth.signin' ORDER BY id DESC LIMIT 1")[0]
        ok(row == ('127.0.0.1', None, None), 'L. a malformed address falls back to the connecting address; a malformed country and city are not stored. Actual %r' % (row,))
        # closed again: hidden, nothing is deleted, nothing new is recorded
        c3 = login(PLACE)
        time.sleep(1.0)
        kept = stored_places()
        st, _, j = hapi('POST', '/api/users/signin-info', {'enabled': False}, c3)
        ok(st == 200 and j['signinInfo'] is False and all(u.get('lastLoginIp') is None and u.get('lastLoginLocation') is None for u in j['users']),
           'L. the admin closes the eye again: the users answer hides IP / location')
        login({'X-Real-IP': '198.51.100.9', 'X-Geo-Country': 'YY'})
        time.sleep(1.0)
        now = stored_places()
        ok(all(n >= 1 for n in now) and now == kept,
           'L. eye closed again: every stored IP and location is kept and nothing new is recorded. Before %r, after %r' % (kept, now))
        ok(dbq("SELECT last_login_ip, last_login_country FROM users WHERE username = 'admin'")[0] == ('203.0.113.7', 'ZZ'),
           'L. eye closed: a new sign-in does not overwrite the last IP / location')
        st, _, j = hapi('GET', '/api/activity?limit=100', cookie=c3)
        ok(st == 200 and all(e['ip'] is None and e['location'] is None for e in j['entries']), 'L. eye closed again: the activity answer has null ip / location')
        ok(wait_for(lambda: dbq("SELECT COUNT(*) FROM activity_log WHERE action = 'users.signinInfo'")[0][0] >= 2), 'L. both switch changes are in the activity log')
        # the explicit clear (works while the eye is closed; guarded like every admin POST)
        st, _, _ = hapi('POST', '/api/users/signin-info/clear', {})
        ok(st == 401, 'L. the clear needs a session. Actual %d' % st)
        st, _, j = hapi('POST', '/api/users/signin-info/clear', {}, c3)
        ok(st == 200 and j['signinInfo'] is False and isinstance(j.get('users'), list), 'L. the admin clears the saved IP and location. Actual %d %r' % (st, j))
        ok(wait_for(lambda: dbq("SELECT COUNT(*) FROM activity_log WHERE action = 'users.signinInfoClear'")[0][0] == 1), 'L. the clear is in the activity log')
        older = dbq("SELECT COUNT(*) FROM activity_log WHERE (ip IS NOT NULL OR country IS NOT NULL OR region IS NOT NULL OR city IS NOT NULL) "
                    "AND id < (SELECT id FROM activity_log WHERE action = 'users.signinInfoClear')")[0][0]
        ok(stored_places()[:2] == (0, 0) and older == 0,
           'L. after the clear no user, session or earlier log row holds an IP or location (the entry of the clear itself is recorded as usual). Actual %r, %d' % (stored_places(), older))
        ok(dbq("SELECT COUNT(*) FROM sessions WHERE ua = '%s'" % UA)[0][0] >= 1, 'L. the clear keeps the sessions and their device')
        ok(wait_for(lambda: dbq("SELECT COUNT(*) FROM activity_log WHERE action = 'users.signinInfoClear' AND result = 'ok' AND detail LIKE '%\"rows\":%'")[0][0] == 1),
           'L. the clear is one activity entry with a row count')
        st, _, j = hapi('POST', '/api/users/signin-info', {'enabled': True}, c3)
        ok(st == 200 and j['signinInfo'] is True and all(u.get('lastLoginIp') is None for u in j['users']), 'L. eye open after the clear: nothing old comes back')
        login(PLACE)
        ok(dbq("SELECT last_login_ip FROM users WHERE username = 'admin'")[0][0] == '203.0.113.7', 'L. eye open after the clear: recording resumes')
        hapi('POST', '/api/users/signin-info', {'enabled': False}, c3)
    run_once(l, 'l1.log', check=matrix)

    # I: XEXAM_INTERNAL_TOKEN instead of the file; a token that is too short
    i = os.path.join(root, 'i')
    os.makedirs(i)
    prog, log = run_once(i, 'i1.log', extra_env={'XEXAM_INTERNAL_TOKEN': 'migrate-check-token-0123456789abcdefghij'})
    ok(prog is not None and 'AI link: using XEXAM_INTERNAL_TOKEN' in log, 'I. with XEXAM_INTERNAL_TOKEN set the log says so')
    ok(not os.path.exists(os.path.join(i, 'link', 'internal.token')), 'I. with XEXAM_INTERNAL_TOKEN set no token file is created')
    prog, log = run_once(i, 'i2.log', extra_env={'XEXAM_INTERNAL_TOKEN': 'too-short'})
    ok(prog is not None and 'XEXAM_INTERNAL_TOKEN is too short (at least 32 characters); AI link off' in log,
       'I. a token shorter than 32 characters turns the link off with a log line; the app still runs')
finally:
    shutil.rmtree(root, ignore_errors=True)

print('\n'.join(results))
print('\nAll %d checks passed' % len(results))
