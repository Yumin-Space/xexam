# -*- coding: utf-8 -*-
"""AI database upgrade, end to end: schema 3 (the published v0.9.1 image) -> schema 10 (this version, v2.0), with Docker and made-up
data only. Run by hand or at the final gate; it is not part of CI because it pulls published images.

Usage: python -I tests/integration/ai_upgrade_check.py [--old-image IMAGE] [--v19-image IMAGE] [--new-image IMAGE]
  --old-image  the published v0.9.1 ai-service image (default docker.io/yuminspace/xexam-ai-service:v0.9.1; pulled when
               it is not on this machine, and removed again at the end only when this script pulled it)
  --v19-image  the published v1.9 ai-service image (default docker.io/yuminspace/xexam-ai-service:v1.9; pulled and removed
               the same way): it writes the genuine schema-9 database of step 6 and must refuse schema 10
  --new-image  the ai-service image of this version (default: built from ai-service/Dockerfile under a temporary tag that
               is removed at the end)
Uses only Docker and the standard library. Every container, volume and image this script creates carries the prefix
xexam-aiupgrade- plus a random suffix and is removed at the end; nothing else is touched (no prune). The services run with
--network none (no backend, no provider): the AI database does not need them, and the shared token is a made-up value.

What is checked:
  1. v0.9.1 creates a schema-3 database on a fresh volume; the script then writes made-up rows into it (two users, a chat
     with a hint and a study chat of each kind, memories, usage rows, a changed set of limits; memory_chars left at the
     v0.9.1 default 300) and records every table's content
  2. This version starts on the same volume: the log says the database was upgraded after a backup copy; schema 10; exactly
     one backups/xexam-ai-schema3-*.db that still holds schema 3, the old rows, the secrets table and the retired columns
     a schema-3 file has (the ten settings columns, limits message_chars / memory_chars; the other three limits columns came
     with schema 4 / 5);
     every made-up row unchanged; every old limit that still exists intact; the retired items (v2.0: the secrets table, ten
     settings columns, five limits columns) are gone from the live file and the log says so (counts only); memoryTokens
     200 (schema 6: lengths in tokens, 1000; schema 7: the v1.3 default 200 because it was never changed); the v1.3 tables (connections, connection_secrets, models, model_slots, study_shared, gen_items)
     exist and every chat is a straight line (messages.parent_id = the message before, chats.leaf_id = the last message); the three call caps 5 / 20 / 100 and replyMaxTokens 3000 at their defaults; sqlite_master equals that of a database this version
     created from scratch; the seven model slots (default, chat, hint, explain, summary, generate, import; no study), every
     connection on, and the three pacing columns of limits (v1.9) at their defaults
  3. A second start writes no second backup, no second upgrade line and changes no row
  4. v0.9.1 refuses the schema-10 database (its newer-schema message) and changes nothing: the rollback is the backup copy
  5. (v1.9) A schema-8 database (a fresh one taken back: the study slot with a made-up model choice and the removed
     gemma:true thinking strength, none of the v1.9 columns) is upgraded to schema 10 with exactly one
     backups/xexam-ai-schema8-*.db: hint and explain both hold the old study row (updated_at, updated_by, the effort now
     google:HIGH), no study slot, connections.enabled and the pacing columns exist with their defaults, the tables have the
     same columns as a new database, and a second start changes nothing
     (v2.0) The prepared schema-8 file also gets the retired items back (a secrets table with a made-up row, the ten settings
     and five limits columns with made-up values), so it is a realistic schema 8: it is upgraded straight to schema 10 with one
     xexam-ai-schema8-* backup that holds them, the live file has none of them, and sqlite_master equals that of a new database
  6. (v2.0) The published v1.9 image creates a genuine schema-9 database on a fresh volume; a made-up secrets row (ciphertext
     marker) and made-up old settings / limits values go in via sqlite3; this version upgrades it: one xexam-ai-schema9-*
     backup, schema 10, the log line about the removed secrets table (1 row) and 15 columns, secrets and the retired columns gone
     (the made-up marker is in neither xexam-ai.db nor its -wal; only the backup holds it), sqlite_master equal to a new
     database, enabled / retention / updated_* of settings unchanged, a second start changes nothing; then the v1.9 image
     refuses schema 10 and changes nothing
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import uuid

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
RUN = uuid.uuid4().hex[:6]
PREFIX = 'xexam-aiupgrade-' + RUN
VOL = PREFIX + '-data'
VOL_FRESH = PREFIX + '-fresh'
VOL8 = PREFIX + '-schema8'   # v1.9: a schema-8 database, made from a fresh one
VOL19 = PREFIX + '-v19'      # v2.0: a genuine schema-9 database written by the v1.9 image
# Made-up shared token (at least 32 printable characters): lets ai-service start with its link configured but no backend
TOKEN = 'upgrade-check-made-up-token-0123456789'
PY = 'python:3.12-slim'

ap = argparse.ArgumentParser()
ap.add_argument('--old-image', default='docker.io/yuminspace/xexam-ai-service:v0.9.1')
ap.add_argument('--v19-image', default='docker.io/yuminspace/xexam-ai-service:v1.9')
ap.add_argument('--new-image', default='')
ARGS = ap.parse_args()
OLD = ARGS.old_image
V19 = ARGS.v19_image
NEW = ARGS.new_image or (PREFIX + ':new')

results = []
made = {'containers': [], 'volumes': [VOL, VOL_FRESH, VOL8, VOL19], 'images': []}


def docker(*a, check=False):
    r = subprocess.run(['docker', *a], capture_output=True, text=True, encoding='utf-8', errors='replace',
                       env=dict(os.environ, MSYS_NO_PATHCONV='1'))
    if check and r.returncode != 0:
        raise SystemExit('docker %s failed:\n%s' % (' '.join(a), (r.stdout + r.stderr)[-1500:]))
    return r


def cleanup():
    for c in made['containers']:
        docker('rm', '-f', c)
    for v in made['volumes']:
        docker('volume', 'rm', '-f', v)
    for i in made['images']:
        docker('rmi', i)


def ok(cond, msg):
    results.append(('PASS ' if cond else 'FAIL ') + msg)
    if not cond:
        print('\n'.join(results))
        cleanup()
        sys.exit(1)


# Helper scripts run in a throw-away python container as the service's own uid on the data volume (so files keep their owner)
SEED = r'''
import json, sqlite3
db = sqlite3.connect('/v/xexam-ai.db')
db.execute('PRAGMA foreign_keys = ON')
NOW = 1760000000
db.executescript("""
INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at, function_enabled, free_mode) VALUES
  (1, 'stamp-aaa', 'ann', 1759000000, 1759900000, 1, 0), (2, 'stamp-bbb', 'bob', 1759100000, 1759800000, 0, 0);
INSERT INTO chats (id, user_id, title, created_at, updated_at, kind, ref, record_id) VALUES
  (1, 1, 'Made-up chat about gardens', 1759000100, 1759000200, 'chat', NULL, NULL),
  (2, 1, '2026-11-10 09:00-practice', 1759000300, 1759000400, 'practice', '0123456789abcdef0123456789abcdef', NULL),
  (3, 2, '2026-11-10 10:00-summary', 1759000500, 1759000600, 'summary', 'r42@202611101000', 42);
INSERT INTO messages (id, chat_id, role, content, status, truncated, model, created_at, finished_at, free, kind,
                      question_id, question_no, source, lang, sample_used, sample_usable, sample_wrong) VALUES
  (1, 1, 'user', 'Which herbs like shade?', 'done', 0, NULL, 1759000100, 1759000100, 0, 'chat', NULL, NULL, NULL, NULL, NULL, NULL, NULL),
  (2, 1, 'assistant', 'Mint and parsley do well in light shade.', 'done', 0, 'demo-chat-small', 1759000110, 1759000150, 0, 'chat', NULL, NULL, NULL, NULL, NULL, NULL, NULL),
  (3, 2, 'user', '[AI hint] question 1', 'done', 0, NULL, 1759000300, 1759000300, 0, 'hint', 'q1', 1, 'practice', 'en', NULL, NULL, NULL),
  (4, 2, 'assistant', 'Think about which tool has a serrated edge.', 'done', 0, 'demo-chat-small', 1759000310, 1759000350, 0, 'hint', 'q1', 1, 'practice', 'en', NULL, NULL, NULL),
  (5, 3, 'user', '[AI summary] 2026-11-10 10:00 exam 3/3/3', 'done', 0, NULL, 1759000500, 1759000500, 0, 'summary', NULL, NULL, NULL, 'en', 3, 3, 3),
  (6, 3, 'assistant', 'Weak points: made-up topic one, made-up topic two.', 'done', 1, 'demo-chat-small', 1759000510, 1759000590, 0, 'summary', NULL, NULL, NULL, 'en', 3, 3, 3);
INSERT INTO memories (id, user_id, content, source, created_at, updated_at) VALUES
  (1, 1, 'Prefers short answers.', 'manual', 1759000700, 1759000700),
  (2, 1, 'Studying for a made-up gardening exam.', 'chat', 1759000800, 1759000800),
  (3, 2, 'Likes examples with herbs.', 'manual', 1759000900, 1759000900);
INSERT INTO usage (id, user_id, at, day, kind, provider, model, tokens_in, tokens_out, latency_ms, first_byte_ms, status, error_code, tokens_cached, purpose) VALUES
  (1, 1, 1759000150, '2025-09-27', 'chat', 'openai', 'demo-chat-small', 30, 12, 900, 300, 'ok', NULL, NULL, NULL),
  (2, 1, 1759000350, '2025-09-27', 'chat', 'openai', 'demo-chat-small', 40, 15, 1100, 350, 'ok', NULL, 5, 'hint'),
  (3, 2, 1759000590, '2025-09-27', 'chat', 'openai', 'demo-chat-small', 200, 90, 3000, 700, 'ok', NULL, NULL, 'summary'),
  (4, NULL, 1759000600, '2025-09-27', 'verify', 'openai', 'demo-chat-small', NULL, NULL, 500, NULL, 'ok', NULL, NULL, NULL);
UPDATE limits SET per_user_running = 2, global_running = 9, chats_per_user = 777, messages_per_chat = 123,
       memories_per_user = 12, message_chars = 4321, updated_by = 'root' WHERE id = 1;
""")
db.commit()
db.close()
print('seeded')
'''

# The retired items as the schema-9 era files had them (v1 text of the secrets table; definitions of the columns as the history
# lists add them). Made-up marker bytes: the "ciphertext" of the made-up secret
MARKER = 'MADE-UP-CIPHERTEXT-MARKER-0123'
RETIRED_DDL = r'''
CREATE TABLE secrets (
  name        TEXT PRIMARY KEY NOT NULL CHECK (name IN ('api_key', 'gateway_key')),
  key_id      TEXT NOT NULL,
  nonce       BLOB NOT NULL CHECK (length(nonce) = 24),
  ciphertext  BLOB NOT NULL,
  hint        TEXT NOT NULL DEFAULT '',
  rev         INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
ALTER TABLE settings ADD COLUMN provider TEXT NOT NULL DEFAULT 'openai' CHECK (provider IN ('openai', 'anthropic', 'gateway'));
ALTER TABLE settings ADD COLUMN base_url TEXT NOT NULL DEFAULT 'https://api.openai.com/v1';
ALTER TABLE settings ADD COLUMN format TEXT NOT NULL DEFAULT 'openai' CHECK (format IN ('openai', 'anthropic'));
ALTER TABLE settings ADD COLUMN gateway_auth TEXT NOT NULL DEFAULT 'bearer' CHECK (gateway_auth IN ('bearer', 'x-api-key', 'cf-aig-authorization'));
ALTER TABLE settings ADD COLUMN model TEXT NOT NULL DEFAULT '';
ALTER TABLE settings ADD COLUMN manual_model INTEGER NOT NULL DEFAULT 0;
ALTER TABLE settings ADD COLUMN usage_option INTEGER NOT NULL DEFAULT 1;
ALTER TABLE settings ADD COLUMN max_tokens INTEGER NOT NULL DEFAULT 8192 CHECK (max_tokens IN (4096, 8192));
ALTER TABLE settings ADD COLUMN verified_at INTEGER;
ALTER TABLE settings ADD COLUMN verified_fp TEXT;
ALTER TABLE limits ADD COLUMN message_chars INTEGER NOT NULL DEFAULT 16000 CHECK (message_chars BETWEEN 1000 AND 32000);
ALTER TABLE limits ADD COLUMN memory_chars INTEGER NOT NULL DEFAULT 300 CHECK (memory_chars BETWEEN 50 AND 2000);
ALTER TABLE limits ADD COLUMN reply_max_chars INTEGER NOT NULL DEFAULT 1000 CHECK (reply_max_chars BETWEEN 1 AND 16000);
ALTER TABLE limits ADD COLUMN gen_call_chars INTEGER NOT NULL DEFAULT 10000 CHECK (gen_call_chars BETWEEN 1000 AND 32000);
ALTER TABLE limits ADD COLUMN import_max_questions INTEGER NOT NULL DEFAULT 50 CHECK (import_max_questions BETWEEN 1 AND 50);
'''
# Made-up values for them, and for the three settings that stay (enabled, retention, who changed it)
RETIRED_VALUES = r'''
INSERT INTO secrets (name, key_id, nonce, ciphertext, hint, rev, updated_at)
  VALUES ('api_key', 'made-up-key-id', zeroblob(24), CAST('MADE-UP-CIPHERTEXT-MARKER-0123' AS BLOB), 'sk-...made', 1, 1234);
UPDATE settings SET provider = 'anthropic', base_url = 'https://example.invalid/v1', format = 'anthropic', gateway_auth = 'x-api-key',
  model = 'made-up-model', manual_model = 1, usage_option = 0, max_tokens = 4096, verified_at = 1234, verified_fp = 'made-up-fp',
  enabled = 1, retention_days = 45, updated_at = 1234, updated_by = 'old-admin' WHERE id = 1;
UPDATE limits SET message_chars = 4321, memory_chars = 111, reply_max_chars = 2222, gen_call_chars = 3333, import_max_questions = 7 WHERE id = 1;
'''

# v1.9: takes a freshly created (schema 9) database back to what schema 8 looked like: no connections.enabled, no pacing
# columns on limits, a model_slots table with the single study slot (a made-up choice and the removed gemma:true effort)
DOWNGRADE = r'''
import sqlite3
db = sqlite3.connect('/v/xexam-ai.db')
db.execute('PRAGMA foreign_keys = OFF')
for table, column in (('connections', 'enabled'), ('limits', 'pace_on'), ('limits', 'pace_rpm'), ('limits', 'pace_tpm')):
    db.execute('ALTER TABLE %s DROP COLUMN %s' % (table, column))
db.executescript("""
DROP TABLE model_slots;
CREATE TABLE model_slots (
  slot TEXT PRIMARY KEY NOT NULL CHECK (slot IN ('default', 'chat', 'study', 'summary', 'generate', 'import')),
  model_id INTEGER REFERENCES models(id) ON DELETE SET NULL,
  updated_at INTEGER NOT NULL,
  updated_by TEXT NOT NULL DEFAULT '',
  effort TEXT
);
INSERT INTO model_slots (slot, model_id, updated_at, updated_by, effort) VALUES
  ('default', NULL, 1, '', NULL), ('chat', NULL, 1, '', NULL), ('study', NULL, 1234, 'old-admin', 'gemma:true'),
  ('summary', NULL, 1, '', NULL), ('generate', NULL, 1, '', NULL), ('import', NULL, 1, '', NULL);
UPDATE meta SET value = '8' WHERE key = 'schema';
""")
db.executescript(__SQL__)  # filled in by the caller: the retired items and their made-up values
db.commit()
db.close()
print('downgraded')
'''

# v2.0: a genuine schema-9 file written by the v1.9 image: only the made-up values go in (it already has the retired items)
SEED19 = r'''
import sqlite3
db = sqlite3.connect('/v/xexam-ai.db')
db.executescript(__SQL__)  # filled in by the caller: the made-up values
db.commit()
db.close()
print('seeded19')
'''

# One JSON report of a database: schema, content hashes and counts of every data table, limits, backups, sqlite_master
REPORT = r'''
import hashlib, json, os, sqlite3, sys
path = sys.argv[1] if len(sys.argv) > 1 else '/v/xexam-ai.db'
db = sqlite3.connect('file:%s?immutable=1' % path, uri=True)  # a cleanly stopped database: no lock, no -shm needed
out = {}
out['schema'] = (db.execute("SELECT value FROM meta WHERE key = 'schema'").fetchone() or [None])[0]
tables = ['users', 'chats', 'messages', 'memories', 'usage']
# columns a later schema adds with a default (schema 5: users.features_off; schema 6: usage.tokens_cache_write; schema 7: the branch, connection and reasoning columns; schema 8 adds only the geo_settings table) are not part
# of the "unchanged" content
ADDED = {'users': {'features_off'}, 'usage': {'tokens_cache_write', 'connection_id', 'connection'},
         'chats': {'leaf_id', 'pick_conn', 'pick_model'}, 'messages': {'parent_id', 'connection', 'reasoning', 'reasoning_ms', 'reasoning_truncated'}}
for t in tables:
    cols = [r[1] for r in db.execute('PRAGMA table_info(%s)' % t) if r[1] not in ADDED.get(t, set())]
    rows = db.execute('SELECT %s FROM %s ORDER BY id' % (', '.join(cols), t)).fetchall()
    out[t] = {'count': len(rows), 'hash': hashlib.sha256(json.dumps(rows, default=str).encode()).hexdigest()}
lim = db.execute('SELECT * FROM limits WHERE id = 1')
out['limits'] = dict(zip([d[0] for d in lim.description], lim.fetchone()))
out['master'] = sorted([list(r) for r in db.execute("SELECT type, name, tbl_name, COALESCE(sql, '') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'")])
cols_of = lambda t: [r[1] for r in db.execute('PRAGMA table_info(%s)' % t)]
# v2.0: what is left of the retired items (the secrets table with its row count, the columns of settings / limits / chats)
tabs = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type = 'table'")}
out['retired'] = {'secrets': db.execute('SELECT COUNT(*) FROM secrets').fetchone()[0] if 'secrets' in tabs else None,
                  'settings': cols_of('settings'), 'limits': cols_of('limits'), 'chats': cols_of('chats')}
keep = [c for c in ('enabled', 'retention_days', 'updated_at', 'updated_by', 'features_off') if c in cols_of('settings')]
out['settings'] = dict(zip(keep, db.execute('SELECT %s FROM settings WHERE id = 1' % ', '.join(keep)).fetchone()))
if 'parent_id' in cols_of('messages') and 'leaf_id' in cols_of('chats'):
    straight = True
    for (cid,) in db.execute('SELECT id FROM chats').fetchall():
        ids = [r[0] for r in db.execute('SELECT id FROM messages WHERE chat_id = ? ORDER BY id', (cid,))]
        parents = [r[0] for r in db.execute('SELECT parent_id FROM messages WHERE chat_id = ? ORDER BY id', (cid,))]
        leaf = db.execute('SELECT leaf_id FROM chats WHERE id = ?', (cid,)).fetchone()[0]
        straight = straight and parents == [None] + ids[:-1] if ids else straight and leaf is None
        straight = straight and (leaf == (ids[-1] if ids else None))
    out['straight'] = straight
has_slots = bool(db.execute("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'model_slots'").fetchone())
out['slots'] = [list(r) for r in db.execute('SELECT slot, model_id, updated_at, updated_by, effort FROM model_slots ORDER BY slot')] if has_slots and 'effort' in cols_of('model_slots') else []
out['slot_sql'] = (db.execute("SELECT sql FROM sqlite_master WHERE name = 'model_slots'").fetchone() or [''])[0]
out['cols'] = {t: cols_of(t) for t in ('connections', 'limits', 'model_slots')}
out['conn_enabled'] = [r[0] for r in db.execute('SELECT enabled FROM connections ORDER BY id')] if 'enabled' in cols_of('connections') else None
out['v7_tables'] = sorted(t for t in ('connections', 'connection_secrets', 'models', 'model_slots', 'study_shared', 'gen_items')
                          if db.execute("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?", (t,)).fetchone())
out['backups'] = sorted(os.listdir('/v/backups')) if os.path.isdir('/v/backups') else []
print(json.dumps(out))
'''


# v2.0: whether the made-up ciphertext marker is still in xexam-ai.db or xexam-ai.db-wal (read as raw bytes)
MARKER_SCAN = r'''
import json, os, sys
marker = sys.argv[1].encode()
out = {}
for name in ('xexam-ai.db', 'xexam-ai.db-wal'):
    path = '/v/' + name
    out[name] = (marker in open(path, 'rb').read()) if os.path.exists(path) else None
print(json.dumps(out))
'''
RETIRED_SQL = RETIRED_DDL + RETIRED_VALUES
RETIRED_COLS = {'settings': ['provider', 'base_url', 'format', 'gateway_auth', 'model', 'manual_model', 'usage_option', 'max_tokens',
                             'verified_at', 'verified_fp'],
                'limits': ['message_chars', 'memory_chars', 'reply_max_chars', 'gen_call_chars', 'import_max_questions']}
# The log line of the schema-10 step (counts only, never values): the secrets rows and the number of columns dropped
LOG_RETIRED = r'Removed the unused legacy table secrets \((\d+) rows\) and (\d+) unused columns \(copy in the backup\)\.'


def retired_gone(rep):
    """True when `rep` (a report) has no secrets table and none of the retired settings / limits columns."""
    r = rep['retired']
    return r['secrets'] is None and all(c not in r[t] for t, cs in RETIRED_COLS.items() for c in cs) and 'free' not in r['chats']


# The retired limits columns of version 1; reply_max_chars (schema 4) and gen_call_chars / import_max_questions (schema 5)
# do not exist yet in a schema-3 (v0.9.1) file
V1_LIMITS = ['message_chars', 'memory_chars']


def retired_present(rep, rows, limits=None):
    """True when `rep` still has the secrets table with `rows` rows, every retired settings column and the retired limits
    columns `limits` (default: all five)."""
    r = rep['retired']
    cols = dict(RETIRED_COLS, limits=RETIRED_COLS['limits'] if limits is None else limits)
    return r['secrets'] == rows and all(c in r[t] for t, cs in cols.items() for c in cs)


def scan_marker(volume):
    r = helper(MARKER_SCAN, volume, MARKER)
    try:
        return json.loads(r.stdout)
    except ValueError:
        raise SystemExit('Could not scan the database files:\n%s' % (r.stdout + r.stderr)[-800:])


def helper(script, volume, *args, rw=False):
    """Runs a script in a throw-away python container as uid 10001 on `volume` (mounted at /v)."""
    r = docker('run', '--rm', '--network', 'none', '--user', '10001:10001', '-v', volume + (':/v' if rw else ':/v:ro'),
               PY, 'python', '-I', '-c', script, *args)
    return r


def report(volume, path=None):
    r = helper(REPORT, volume, *([path] if path else []))
    try:
        return json.loads(r.stdout)
    except ValueError:
        raise SystemExit('Could not read the database report:\n%s' % (r.stdout + r.stderr)[-800:])


def start(image, volume, tag):
    """Starts an ai-service container on the volume; returns its name."""
    name = '%s-%s' % (PREFIX, tag)
    made['containers'].append(name)
    r = docker('run', '-d', '--name', name, '--network', 'none', '--read-only', '--tmpfs', '/tmp', '--cap-drop', 'ALL',
               '--security-opt', 'no-new-privileges:true', '-e', 'XEXAM_INTERNAL_TOKEN=' + TOKEN, '-e', 'TZ=UTC',
               '-v', volume + ':/data', image)
    ok(r.returncode == 0, 'The container %s starts. %s' % (tag, r.stderr.strip()[-300:]))
    return name


def wait_up(name, secs=40):
    """True once the service answers its own health check."""
    t0 = time.time()
    while time.time() - t0 < secs:
        if docker('exec', name, '/usr/local/bin/xexam-ai-service', 'healthcheck').returncode == 0:
            return True
        if docker('inspect', '-f', '{{.State.Running}}', name).stdout.strip() != 'true':
            return False
        time.sleep(1)
    return False


def stop(name):
    r = docker('stop', '-t', '20', name)
    code = docker('inspect', '-f', '{{.State.ExitCode}}', name).stdout.strip()
    logs = docker('logs', name)
    return code, logs.stdout + logs.stderr


try:
    # -- images --
    r = docker('image', 'inspect', OLD)
    if r.returncode != 0:
        pulled = docker('pull', OLD)
        ok(pulled.returncode == 0, 'The published v0.9.1 image %s is pulled. %s' % (OLD, pulled.stderr.strip()[-300:]))
        made['images'].append(OLD)
    if not ARGS.new_image:
        b = docker('build', '-q', '-f', os.path.join(ROOT, 'ai-service', 'Dockerfile'), '-t', NEW, ROOT)
        ok(b.returncode == 0, 'The ai-service image of this version is built. %s' % b.stderr.strip()[-400:])
        made['images'].append(NEW)
    ok(docker('volume', 'create', VOL).returncode == 0 and docker('volume', 'create', VOL_FRESH).returncode == 0,
       'Two temporary volumes are created')

    # -- 1. v0.9.1 creates schema 3; made-up rows go in --
    c = start(OLD, VOL, 'old')
    ok(wait_up(c), 'v0.9.1 starts and answers its health check')
    code, logs = stop(c)
    ok(code == '0', 'v0.9.1 stops cleanly (exit code %s)' % code)
    old = report(VOL)
    ok(old['schema'] == '3', 'v0.9.1 wrote a schema-3 database. Actual %r' % old['schema'])
    ok(old['limits']['memory_chars'] == 300 and old['limits']['message_chars'] == 16000 and 'calls_per_minute' not in old['limits'],
       'The v0.9.1 limits: memory_chars 300, message_chars 16000, no call caps. Actual %r' % old['limits'])
    r = helper(SEED, VOL, rw=True)
    ok(r.returncode == 0 and 'seeded' in r.stdout, 'Made-up chats, memories, a study answer, usage rows and limits are written. %s' % (r.stdout + r.stderr)[-300:])
    before = report(VOL)
    ok(before['users']['count'] == 2 and before['chats']['count'] == 3 and before['messages']['count'] == 6
       and before['memories']['count'] == 3 and before['usage']['count'] == 4,
       'The seeded counts: 2 users, 3 chats, 6 messages, 3 memories, 4 usage rows. Actual %r' % {k: before[k]['count'] for k in ('users', 'chats', 'messages', 'memories', 'usage')})
    ok(before['limits']['per_user_running'] == 2 and before['limits']['message_chars'] == 4321 and before['backups'] == [],
       'The seeded limits are in and there is no backup yet. Actual %r' % before['limits'])

    # -- 2. this version upgrades it --
    c = start(NEW, VOL, 'new1')
    up = wait_up(c)
    code, logs1 = stop(c)
    ok(up and code == '0', 'This version starts on the schema-3 volume, answers its health check and stops cleanly. Exit code %s. Log:\n%s' % (code, logs1[-600:]))
    ok(re.search(r'Upgraded the AI database to schema 10\. A copy of the previous version was saved first: \S*xexam-ai-schema3-', logs1) is not None,
       'The log says the database was upgraded to schema 10 after a backup copy. Log:\n%s' % logs1[-600:])
    m = re.search(LOG_RETIRED, logs1)
    ok(m is not None and (int(m.group(1)), int(m.group(2))) == (0, 15),
       'The log says the unused legacy table secrets (0 rows) and 15 unused columns were removed (counts only). Log:\n%s' % logs1[-800:])
    after = report(VOL)
    ok(after['schema'] == '10', 'meta.schema is 10. Actual %r' % after['schema'])
    ok(retired_gone(after), 'v2.0: the secrets table and the ten settings / five limits columns are gone from the live file. Actual %r' % after['retired'])
    SLOTS9 = ['chat', 'default', 'explain', 'generate', 'hint', 'import', 'summary']
    ok([r[0] for r in after['slots']] == SLOTS9 and "'study'" not in after['slot_sql'] and "'explain'" in after['slot_sql'],
       'v1.9: the seven model slots exist and study does not. Actual %r' % [r[0] for r in after['slots']])
    ok('enabled' in after['cols']['connections'] and all(c in after['cols']['limits'] for c in ('pace_on', 'pace_rpm', 'pace_tpm'))
       and (after['limits']['pace_on'], after['limits']['pace_rpm'], after['limits']['pace_tpm']) == (1, 15, 250000),
       'v1.9: connections.enabled exists and the pacing columns of limits hold 1 / 15 / 250000. Actual %r' % after['cols'])
    ok(len(after['backups']) == 1 and re.fullmatch(r'xexam-ai-schema3-\d{8}-\d{6}\.db', after['backups'][0]) is not None,
       'Exactly one backups/xexam-ai-schema3-<date-time>.db. Actual %r' % after['backups'])
    ok(all(after['settings'].get(k) == v for k, v in before['settings'].items()),
       'The settings row (enabled, retention, who changed it) is unchanged. Before %r, after %r' % (before['settings'], after['settings']))
    ok(after.get('v7_tables') == sorted(['connection_secrets', 'connections', 'gen_items', 'model_slots', 'models', 'study_shared']),
       'The six v1.3 tables exist after the upgrade. Actual %r' % after.get('v7_tables'))
    ok(after.get('straight') is True, 'Every old chat is a straight line: parent_id chain in the old order, leaf_id = the last message')
    for t in ('users', 'chats', 'messages', 'memories', 'usage'):
        ok(after[t] == before[t], 'Table %s is unchanged (count %d, same content hash)' % (t, before[t]['count']))
    L, B = after['limits'], before['limits']
    for k in ('per_user_running', 'global_running', 'chats_per_user', 'messages_per_chat', 'memories_per_user',
              'updated_at', 'updated_by'):
        ok(L[k] == B[k], 'Old limit %s intact (%r)' % (k, B[k]))
    ok('message_chars' not in L and 'memory_chars' not in L, 'The retired limits message_chars and memory_chars are gone from the live file')
    ok(L['memory_tokens'] == 200,
       'memory_chars at the default becomes memoryTokens 1000 (schema 6) and, still the old default, 200 (schema 7, v1.3). Actual %r' % L['memory_tokens'])
    ok((L['calls_per_minute'], L['calls_per_hour'], L['calls_per_day'], L['reply_max_tokens']) == (5, 20, 100, 3000),
       'The call caps 5 / 20 / 100 and replyMaxTokens 3000 are at their defaults. Actual %r' % ((L['calls_per_minute'], L['calls_per_hour'], L['calls_per_day'], L['reply_max_tokens']),))
    bk = report(VOL, '/v/backups/' + after['backups'][0])
    ok(bk['schema'] == '3' and all(bk[t] == before[t] for t in ('users', 'chats', 'messages', 'memories', 'usage'))
       and bk['limits'] == before['limits'],
       'The backup copy is the schema-3 database as it was before the upgrade (rows and limits)')
    ok(retired_present(bk, 0, V1_LIMITS) and bk['limits']['message_chars'] == 4321 and bk['limits']['memory_chars'] == 300,
       'v2.0: the backup still has the secrets table, the ten retired settings columns and the two schema-3 retired limits '
       'columns with their old values (message_chars 4321, memory_chars 300). Actual %r' % bk['retired'])
    # a database this version created from scratch has the same tables, columns and indexes
    c = start(NEW, VOL_FRESH, 'fresh')
    up = wait_up(c)
    code, _ = stop(c)
    ok(up and code == '0', 'This version creates a fresh database on the second volume')
    fresh = report(VOL_FRESH)
    ok(fresh['schema'] == '10' and fresh['backups'] == [] and fresh['limits']['memory_tokens'] == 200 and retired_gone(fresh),
       'A new database is schema 10 with no backup, memoryTokens 200 and none of the retired items. Actual %r' % {k: fresh[k] for k in ('schema', 'backups', 'retired')})
    ok(fresh['master'] == after['master'], 'sqlite_master of the upgraded database equals that of a new one (%d objects)' % len(fresh['master']))

    # -- 3. the next start writes nothing --
    c = start(NEW, VOL, 'new2')
    up = wait_up(c)
    code, logs2 = stop(c)
    ok(up and code == '0', 'The next start works. Exit code %s' % code)
    ok('Upgraded the AI database' not in logs2, 'The next start logs no upgrade')
    again = report(VOL)
    ok(again['backups'] == after['backups'], 'No second backup copy. Actual %r' % again['backups'])
    ok(all(again[t] == after[t] for t in ('users', 'chats', 'messages', 'memories', 'usage')) and again['limits'] == after['limits']
       and again['master'] == after['master'], 'The next start changed no row, no limit and no table')

    # -- 4. v0.9.1 refuses schema 10 --
    c = start(OLD, VOL, 'old2')
    time.sleep(3)
    running = docker('inspect', '-f', '{{.State.Running}}', c).stdout.strip()
    code, logs3 = stop(c) if running == 'true' else (docker('inspect', '-f', '{{.State.ExitCode}}', c).stdout.strip(), (docker('logs', c).stdout + docker('logs', c).stderr))
    ok(code != '0' and 'has schema 10' in logs3 and 'newer ai-service' in logs3,
       'v0.9.1 refuses the schema-10 database with its newer-schema message (exit code %s). Log:\n%s' % (code, logs3[-400:]))
    final = report(VOL)
    ok(all(final[t] == after[t] for t in ('users', 'chats', 'messages', 'memories', 'usage')) and final['schema'] == '10'
       and final['backups'] == after['backups'], 'The refusal changed nothing; the rollback is the backup copy')

    # -- 5. schema 8 -> 10 (v1.9 + v2.0): the study slot is split; the removed gemma strength is rewritten; the retired items go --
    ok(docker('volume', 'create', VOL8).returncode == 0, 'A third temporary volume is created')
    c = start(NEW, VOL8, 'seed8')
    up = wait_up(c)
    code, _ = stop(c)
    ok(up and code == '0', 'This version creates a database on the third volume')
    r = helper(DOWNGRADE.replace('__SQL__', repr(RETIRED_SQL)), VOL8, rw=True)
    ok(r.returncode == 0 and 'downgraded' in r.stdout,
       'The new database is taken back to schema 8 (study slot, no v1.9 columns, the retired items back). %s' % (r.stdout + r.stderr)[-400:])
    old8 = report(VOL8)
    ok(old8['schema'] == '8' and [r[0] for r in old8['slots']] == ['chat', 'default', 'generate', 'import', 'study', 'summary']
       and 'enabled' not in old8['cols']['connections'] and 'pace_on' not in old8['cols']['limits'],
       'The prepared database looks like schema 8. Actual %r' % {k: old8[k] for k in ('schema', 'cols')})
    ok(retired_present(old8, 1) and old8['settings']['enabled'] == 1 and old8['settings']['retention_days'] == 45,
       'v2.0: the prepared schema-8 file holds a secrets table with one made-up row and every retired column. Actual %r' % old8['retired'])
    c = start(NEW, VOL8, 'up8')
    up = wait_up(c)
    code, logs8 = stop(c)
    ok(up and code == '0', 'This version starts on the schema-8 volume and stops cleanly. Exit code %s. Log:\n%s' % (code, logs8[-600:]))
    ok(re.search(r'Upgraded the AI database to schema 10\. A copy of the previous version was saved first: \S*xexam-ai-schema8-', logs8) is not None,
       'The log says the database was upgraded to schema 10 after a backup copy named xexam-ai-schema8-. Log:\n%s' % logs8[-600:])
    m = re.search(LOG_RETIRED, logs8)
    ok(m is not None and (int(m.group(1)), int(m.group(2))) == (1, 15),
       'The log says the unused legacy table secrets (1 row) and 15 unused columns were removed. Log:\n%s' % logs8[-800:])
    new8 = report(VOL8)
    ok(new8['schema'] == '10' and len(new8['backups']) == 1 and re.fullmatch(r'xexam-ai-schema8-\d{8}-\d{6}\.db', new8['backups'][0]) is not None,
       'meta.schema is 9 and there is exactly one backups/xexam-ai-schema8-<date-time>.db. Actual %r' % {k: new8[k] for k in ('schema', 'backups')})
    slots8 = {r[0]: r[1:] for r in new8['slots']}
    ok(sorted(slots8) == SLOTS9 and slots8['hint'] == slots8['explain'] == [None, 1234, 'old-admin', 'google:HIGH'],
       'hint and explain both hold the old study row (updated_at 1234, updated_by old-admin) and gemma:true is now google:HIGH. Actual %r' % slots8)
    ok(slots8['default'] == [None, 1, '', None] and slots8['chat'] == [None, 1, '', None] and slots8['summary'] == [None, 1, '', None],
       'The other slots are unchanged. Actual %r' % slots8)
    ok("'study'" not in new8['slot_sql'] and "'explain'" in new8['slot_sql'], 'The model_slots table no longer allows study')
    ok(new8['cols'] == fresh['cols'], 'connections, limits and model_slots have the same columns as a new database. Actual %r' % new8['cols'])
    ok(retired_gone(new8), 'v2.0: the secrets table and the retired columns are gone. Actual %r' % new8['retired'])
    ok(new8['master'] == fresh['master'], 'v2.0: sqlite_master of the upgraded schema-8 file equals that of a new database (%d objects)' % len(fresh['master']))
    ok(new8['settings'] == old8['settings'], 'v2.0: enabled, retention and who changed it are unchanged. Before %r, after %r' % (old8['settings'], new8['settings']))
    ok((new8['limits']['pace_on'], new8['limits']['pace_rpm'], new8['limits']['pace_tpm']) == (1, 15, 250000),
       'The pacing columns hold their defaults 1 / 15 / 250000. Actual %r' % new8['limits'])
    bk8 = report(VOL8, '/v/backups/' + new8['backups'][0])
    ok(bk8['schema'] == '8' and [r[0] for r in bk8['slots']] == ['chat', 'default', 'generate', 'import', 'study', 'summary']
       and dict((r[0], r[4]) for r in bk8['slots'])['study'] == 'gemma:true',
       'The backup copy is the schema-8 database as it was (study slot, gemma:true)')
    ok(retired_present(bk8, 1) and bk8['limits']['message_chars'] == 4321 and bk8['limits']['import_max_questions'] == 7,
       'v2.0: the backup still holds the secrets row and the retired columns with their made-up values')
    c = start(NEW, VOL8, 'up8b')
    up = wait_up(c)
    code, logs8b = stop(c)
    again8 = report(VOL8)
    ok(up and code == '0' and 'Upgraded the AI database' not in logs8b and again8['backups'] == new8['backups'] and again8['slots'] == new8['slots']
       and again8['limits'] == new8['limits'] and again8['master'] == new8['master'] and again8['settings'] == new8['settings']
       and retired_gone(again8), 'The next start logs no upgrade, writes no second backup and changes no slot, limit or table')

    # -- 6. (v2.0) a genuine schema-9 file written by the published v1.9 image --
    if docker('image', 'inspect', V19).returncode != 0:
        pulled = docker('pull', V19)
        ok(pulled.returncode == 0, 'The published v1.9 image %s is pulled. %s' % (V19, pulled.stderr.strip()[-300:]))
        made['images'].append(V19)
    ok(docker('volume', 'create', VOL19).returncode == 0, 'A fourth temporary volume is created')
    c = start(V19, VOL19, 'v19a')
    up = wait_up(c)
    code, _ = stop(c)
    ok(up and code == '0', 'The v1.9 image creates its database on the fourth volume')
    g9 = report(VOL19)
    ok(g9['schema'] == '9' and retired_present(g9, 0) and g9['backups'] == [],
       'The v1.9 image wrote a genuine schema-9 database (secrets table, retired columns, no backup). Actual %r' % {k: g9[k] for k in ('schema', 'backups', 'retired')})
    r = helper(SEED19.replace('__SQL__', repr(RETIRED_VALUES)), VOL19, rw=True)
    ok(r.returncode == 0 and 'seeded19' in r.stdout, 'A made-up secrets row and made-up old settings / limits values are written. %s' % (r.stdout + r.stderr)[-400:])
    before19 = report(VOL19)
    ok(retired_present(before19, 1) and before19['settings']['enabled'] == 1 and before19['settings']['retention_days'] == 45
       and before19['settings']['updated_by'] == 'old-admin', 'The seeded schema-9 file holds one secrets row and the made-up settings. Actual %r' % before19['settings'])
    ok(scan_marker(VOL19)['xexam-ai.db'] is True or scan_marker(VOL19).get('xexam-ai.db-wal') is True,
       'Before the upgrade the made-up ciphertext marker is in the live file or its -wal (the scan works)')
    c = start(NEW, VOL19, 'up19')
    up = wait_up(c)
    code, logs19 = stop(c)
    ok(up and code == '0', 'This version starts on the schema-9 volume and stops cleanly. Exit code %s. Log:\n%s' % (code, logs19[-600:]))
    ok(re.search(r'Upgraded the AI database to schema 10\. A copy of the previous version was saved first: \S*xexam-ai-schema9-', logs19) is not None,
       'The log says the database was upgraded to schema 10 after a backup copy named xexam-ai-schema9-. Log:\n%s' % logs19[-600:])
    m = re.search(LOG_RETIRED, logs19)
    ok(m is not None and (int(m.group(1)), int(m.group(2))) == (1, 15),
       'The log says the unused legacy table secrets (1 row) and 15 unused columns were removed. Log:\n%s' % logs19[-800:])
    ok(MARKER not in logs19 and 'made-up-key-id' not in logs19 and 'made-up-fp' not in logs19, 'The log holds none of the made-up secret values')
    new19 = report(VOL19)
    ok(new19['schema'] == '10' and len(new19['backups']) == 1 and re.fullmatch(r'xexam-ai-schema9-\d{8}-\d{6}\.db', new19['backups'][0]) is not None,
       'meta.schema is 10 and there is exactly one backups/xexam-ai-schema9-<date-time>.db. Actual %r' % {k: new19[k] for k in ('schema', 'backups')})
    ok(retired_gone(new19), 'The secrets table and the retired columns are gone from the live file. Actual %r' % new19['retired'])
    ok(new19['master'] == fresh['master'], 'sqlite_master of the upgraded v1.9 file equals that of a new database (%d objects)' % len(fresh['master']))
    ok(new19['settings'] == before19['settings'], 'enabled, retention and who changed it are unchanged. Before %r, after %r' % (before19['settings'], new19['settings']))
    ok(all(new19['limits'][k] == before19['limits'][k] for k in new19['limits']), 'Every limit that still exists is unchanged')
    seen = scan_marker(VOL19)
    ok(seen['xexam-ai.db'] is False and seen['xexam-ai.db-wal'] in (False, None),
       'The made-up ciphertext marker is in neither xexam-ai.db nor its -wal any more. Actual %r' % seen)
    bk19 = report(VOL19, '/v/backups/' + new19['backups'][0])
    ok(bk19['schema'] == '9' and retired_present(bk19, 1) and bk19['settings'] == before19['settings'] and bk19['limits'] == before19['limits']
       and bk19['limits']['message_chars'] == 4321, 'The backup copy is the schema-9 file as it was (the secrets row and the retired columns included)')
    ok(scan_marker(VOL19) == seen, 'Scanning again changes nothing (read-only)')
    c = start(NEW, VOL19, 'up19b')
    up = wait_up(c)
    code, logs19b = stop(c)
    again19 = report(VOL19)
    ok(up and code == '0' and 'Upgraded the AI database' not in logs19b and again19['backups'] == new19['backups']
       and again19['master'] == new19['master'] and again19['settings'] == new19['settings'] and again19['limits'] == new19['limits']
       and all(again19[t] == new19[t] for t in ('users', 'chats', 'messages', 'memories', 'usage')) and retired_gone(again19),
       'The next start logs no upgrade, writes no second backup and changes no table, setting or limit')
    c = start(V19, VOL19, 'v19b')
    time.sleep(3)
    running = docker('inspect', '-f', '{{.State.Running}}', c).stdout.strip()
    code, logs19c = stop(c) if running == 'true' else (docker('inspect', '-f', '{{.State.ExitCode}}', c).stdout.strip(), (docker('logs', c).stdout + docker('logs', c).stderr))
    ok(code != '0' and 'has schema 10' in logs19c and 'newer ai-service' in logs19c,
       'The v1.9 image refuses the schema-10 database with its newer-schema message (exit code %s). Log:\n%s' % (code, logs19c[-400:]))
    last19 = report(VOL19)
    ok(last19['schema'] == '10' and last19['master'] == new19['master'] and last19['backups'] == new19['backups']
       and last19['settings'] == new19['settings'], 'The refusal changed nothing; going back to v1.9 needs the xexam-ai-schema9-* backup copy')
finally:
    cleanup()

print('\n'.join(results))
print('\nAll %d checks passed' % len(results))
