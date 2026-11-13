# -*- coding: utf-8 -*-
"""AI database upgrade, end to end: schema 3 (the published v0.9.1 image) -> schema 6 (this version, v1.1), with Docker and made-up
data only. Run by hand or at the final gate; it is not part of CI because it pulls a published image.

Usage: python -I tests/integration/ai_upgrade_check.py [--old-image IMAGE] [--new-image IMAGE]
  --old-image  the published v0.9.1 ai-service image (default docker.io/yuminspace/xexam-ai-service:v0.9.1; pulled when
               it is not on this machine, and removed again at the end only when this script pulled it)
  --new-image  the ai-service image of this version (default: built from ai-service/Dockerfile under a temporary tag that
               is removed at the end)
Uses only Docker and the standard library. Every container, volume and image this script creates carries the prefix
xexam-aiupgrade- plus a random suffix and is removed at the end; nothing else is touched (no prune). The services run with
--network none (no backend, no provider): the AI database does not need them, and the shared token is a made-up value.

What is checked:
  1. v0.9.1 creates a schema-3 database on a fresh volume; the script then writes made-up rows into it (two users, a chat
     with a hint and a study chat of each kind, memories, usage rows, a changed set of limits; memory_chars left at the
     v0.9.1 default 300) and records every table's content
  2. This version starts on the same volume: the log says the database was upgraded after a backup copy; schema 6; exactly
     one backups/xexam-ai-schema3-*.db that still holds schema 3 and the old rows; every made-up row unchanged; every old
     limit intact (the retired message_chars column keeps its value); memory_chars 300 reads 200 (3.10) and memoryTokens
     1000 (schema 6: lengths in tokens); the three call caps 5 / 20 / 100 and replyMaxTokens 3000 at their defaults; sqlite_master equals that of a database this version
     created from scratch
  3. A second start writes no second backup, no second upgrade line and changes no row
  4. v0.9.1 refuses the schema-6 database (its newer-schema message) and changes nothing: the rollback is the backup copy
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
# Made-up shared token (at least 32 printable characters): lets ai-service start with its link configured but no backend
TOKEN = 'upgrade-check-made-up-token-0123456789'
PY = 'python:3.12-slim'

ap = argparse.ArgumentParser()
ap.add_argument('--old-image', default='docker.io/yuminspace/xexam-ai-service:v0.9.1')
ap.add_argument('--new-image', default='')
ARGS = ap.parse_args()
OLD = ARGS.old_image
NEW = ARGS.new_image or (PREFIX + ':new')

results = []
made = {'containers': [], 'volumes': [VOL, VOL_FRESH], 'images': []}


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

# One JSON report of a database: schema, content hashes and counts of every data table, limits, backups, sqlite_master
REPORT = r'''
import hashlib, json, os, sqlite3, sys
path = sys.argv[1] if len(sys.argv) > 1 else '/v/xexam-ai.db'
db = sqlite3.connect('file:%s?immutable=1' % path, uri=True)  # a cleanly stopped database: no lock, no -shm needed
out = {}
out['schema'] = (db.execute("SELECT value FROM meta WHERE key = 'schema'").fetchone() or [None])[0]
tables = ['users', 'chats', 'messages', 'memories', 'usage']
# columns a later schema adds with a default (schema 5: users.features_off; schema 6: usage.tokens_cache_write) are not part
# of the "unchanged" content
ADDED = {'users': {'features_off'}, 'usage': {'tokens_cache_write'}}
for t in tables:
    cols = [r[1] for r in db.execute('PRAGMA table_info(%s)' % t) if r[1] not in ADDED.get(t, set())]
    rows = db.execute('SELECT %s FROM %s ORDER BY id' % (', '.join(cols), t)).fetchall()
    out[t] = {'count': len(rows), 'hash': hashlib.sha256(json.dumps(rows, default=str).encode()).hexdigest()}
lim = db.execute('SELECT * FROM limits WHERE id = 1')
out['limits'] = dict(zip([d[0] for d in lim.description], lim.fetchone()))
out['master'] = sorted([list(r) for r in db.execute("SELECT type, name, tbl_name, COALESCE(sql, '') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'")])
out['backups'] = sorted(os.listdir('/v/backups')) if os.path.isdir('/v/backups') else []
print(json.dumps(out))
'''


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
    ok(re.search(r'Upgraded the AI database to schema 6\. A copy of the previous version was saved first: \S*xexam-ai-schema3-', logs1) is not None,
       'The log says the database was upgraded to schema 6 after a backup copy. Log:\n%s' % logs1[-600:])
    after = report(VOL)
    ok(after['schema'] == '6', 'meta.schema is 6. Actual %r' % after['schema'])
    ok(len(after['backups']) == 1 and re.fullmatch(r'xexam-ai-schema3-\d{8}-\d{6}\.db', after['backups'][0]) is not None,
       'Exactly one backups/xexam-ai-schema3-<date-time>.db. Actual %r' % after['backups'])
    for t in ('users', 'chats', 'messages', 'memories', 'usage'):
        ok(after[t] == before[t], 'Table %s is unchanged (count %d, same content hash)' % (t, before[t]['count']))
    L, B = after['limits'], before['limits']
    for k in ('per_user_running', 'global_running', 'chats_per_user', 'messages_per_chat', 'memories_per_user', 'message_chars',
              'updated_at', 'updated_by'):
        ok(L[k] == B[k], 'Old limit %s intact (%r)' % (k, B[k]))
    ok(L['memory_chars'] == 200, 'memory_chars left at the v0.9.1 default 300 reads 200 now. Actual %r' % L['memory_chars'])
    ok(L['memory_tokens'] == 1000, 'memory_chars at the default 200 becomes memoryTokens 1000 (schema 6). Actual %r' % L['memory_tokens'])
    ok((L['calls_per_minute'], L['calls_per_hour'], L['calls_per_day'], L['reply_max_tokens']) == (5, 20, 100, 3000),
       'The call caps 5 / 20 / 100 and replyMaxTokens 3000 are at their defaults. Actual %r' % ((L['calls_per_minute'], L['calls_per_hour'], L['calls_per_day'], L['reply_max_tokens']),))
    bk = report(VOL, '/v/backups/' + after['backups'][0])
    ok(bk['schema'] == '3' and all(bk[t] == before[t] for t in ('users', 'chats', 'messages', 'memories', 'usage'))
       and bk['limits'] == before['limits'],
       'The backup copy is the schema-3 database as it was before the upgrade (rows and limits)')
    # a database this version created from scratch has the same tables, columns and indexes
    c = start(NEW, VOL_FRESH, 'fresh')
    up = wait_up(c)
    code, _ = stop(c)
    ok(up and code == '0', 'This version creates a fresh database on the second volume')
    fresh = report(VOL_FRESH)
    ok(fresh['schema'] == '6' and fresh['backups'] == [] and fresh['limits']['memory_tokens'] == 1000,
       'A new database is schema 6 with no backup and memoryTokens 1000. Actual %r' % {k: fresh[k] for k in ('schema', 'backups')})
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

    # -- 4. v0.9.1 refuses schema 6 --
    c = start(OLD, VOL, 'old2')
    time.sleep(3)
    running = docker('inspect', '-f', '{{.State.Running}}', c).stdout.strip()
    code, logs3 = stop(c) if running == 'true' else (docker('inspect', '-f', '{{.State.ExitCode}}', c).stdout.strip(), (docker('logs', c).stdout + docker('logs', c).stderr))
    ok(code != '0' and 'has schema 6' in logs3 and 'newer ai-service' in logs3,
       'v0.9.1 refuses the schema-6 database with its newer-schema message (exit code %s). Log:\n%s' % (code, logs3[-400:]))
    final = report(VOL)
    ok(all(final[t] == after[t] for t in ('users', 'chats', 'messages', 'memories', 'usage')) and final['schema'] == '6'
       and final['backups'] == after['backups'], 'The refusal changed nothing; the rollback is the backup copy')
finally:
    cleanup()

print('\n'.join(results))
print('\nAll %d checks passed' % len(results))
