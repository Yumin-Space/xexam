# -*- coding: utf-8 -*-
"""SQLite 存储：题库 + 学习记录。只用标准库 sqlite3。

表：
  questions   题库。active=0 表示「当前上传的 Excel 里已经没有这道题」（下架），学习记录不删，以后再出现会自动恢复。
  attempts    每场考试一条（成绩历史）。
  stats       每道题的累计答题情况（错题本靠它）。
  imports     每次上传导入的记录。
  meta        杂项（schema 版本、是否已迁移过旧的 progress.json）。

题目的 id 来自 xlsx_parser.make_id（工作表名 + 题干/首选项的内容指纹），所以：
  - 同一道题在 Excel 里换了行号，id 不变，学习记录跟着走；
  - 改了题干的措辞，会被当成一道新题（旧题下架）。
"""
import json
import os
import sqlite3
import threading
from contextlib import contextmanager
from datetime import datetime

SCHEMA_VERSION = 1
MASTER_STREAK = 2          # 错题连续答对几次后移出错题本
HISTORY_LIMIT = 60

SCHEMA = '''
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS questions (
  id        TEXT PRIMARY KEY NOT NULL,   -- SQLite 里 TEXT 主键默认允许 NULL，必须显式 NOT NULL
  sheet     TEXT NOT NULL,
  row_no    INTEGER NOT NULL,
  seq       INTEGER NOT NULL,          -- 题库里的顺序（先按工作表出现的顺序，再按行号）
  data      TEXT NOT NULL,             -- 整道题的 JSON（题干、选项、答案、解析……）
  active    INTEGER NOT NULL DEFAULT 1,
  added_at  TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_questions_sheet ON questions(sheet, active);
CREATE TABLE IF NOT EXISTS attempts (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  at       TEXT NOT NULL,
  sheets   TEXT NOT NULL,              -- JSON 数组
  mode     TEXT NOT NULL,
  total    INTEGER NOT NULL,
  correct  INTEGER NOT NULL,
  percent  INTEGER NOT NULL,
  seconds  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS stats (
  question_id TEXT PRIMARY KEY NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  correct  INTEGER NOT NULL DEFAULT 0,
  wrong    INTEGER NOT NULL DEFAULT 0,
  streak   INTEGER NOT NULL DEFAULT 0,
  last     INTEGER,
  at       TEXT
);
CREATE TABLE IF NOT EXISTS imports (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  at        TEXT NOT NULL,
  filename  TEXT NOT NULL,
  added     INTEGER NOT NULL,
  updated   INTEGER NOT NULL,
  removed   INTEGER NOT NULL,
  restored  INTEGER NOT NULL,
  skipped   INTEGER NOT NULL,
  total     INTEGER NOT NULL           -- 导入后上架的题数
);
'''


def now():
    return datetime.now().strftime('%Y-%m-%d %H:%M')


def _q_hash(q):
    """用来判断「同 id 的题内容有没有变」。"""
    return json.dumps({k: v for k, v in q.items() if k not in ('row',)}, sort_keys=True, ensure_ascii=False)


class Store:
    def __init__(self, path):
        self.path = path
        d = os.path.dirname(os.path.abspath(path))
        os.makedirs(d, exist_ok=True)
        self.lock = threading.RLock()
        self.conn = sqlite3.connect(path, check_same_thread=False, isolation_level=None)   # 手动控制事务
        self.conn.row_factory = sqlite3.Row
        # 不用 WAL：WAL 要用共享内存文件，Docker Desktop 把 Windows 文件夹挂载进容器时不可靠。
        # 用默认的回滚日志模式 + FULL 同步：单人使用，速度不是问题，数据安全更重要。
        self.conn.execute('PRAGMA journal_mode=DELETE')
        self.conn.execute('PRAGMA foreign_keys=ON')
        self.conn.execute('PRAGMA synchronous=FULL')
        self.conn.executescript(SCHEMA)
        self._set_meta('schema', str(SCHEMA_VERSION), only_if_missing=True)

    # ---------------------------------------------------------------- 基础
    @contextmanager
    def tx(self):
        with self.lock:
            self.conn.execute('BEGIN IMMEDIATE')
            try:
                yield self.conn
            except BaseException:
                self.conn.execute('ROLLBACK')
                raise
            else:
                try:
                    self.conn.execute('COMMIT')
                except BaseException:
                    self.conn.execute('ROLLBACK')
                    raise

    def _set_meta(self, key, value, only_if_missing=False):
        with self.lock:
            if only_if_missing:
                self.conn.execute('INSERT OR IGNORE INTO meta(key,value) VALUES(?,?)', (key, value))
            else:
                self.conn.execute('INSERT INTO meta(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value', (key, value))

    def _get_meta(self, key, default=None):
        with self.lock:
            r = self.conn.execute('SELECT value FROM meta WHERE key=?', (key,)).fetchone()
        return r['value'] if r else default

    def close(self):
        with self.lock:
            self.conn.close()

    # ---------------------------------------------------------------- 题库
    def count_active(self):
        with self.lock:
            return self.conn.execute('SELECT COUNT(*) c FROM questions WHERE active=1').fetchone()['c']

    def bank(self):
        """给前端用的题库：只含上架的题；sheets 里还会带下架数量。"""
        with self.lock:
            rows = self.conn.execute('SELECT id, sheet, row_no, data, active FROM questions ORDER BY seq').fetchall()
        sheets, order, questions = {}, [], []
        for r in rows:
            s = sheets.get(r['sheet'])
            if s is None:
                s = sheets[r['sheet']] = {'name': r['sheet'], 'usable': 0, 'inactive': 0}
                order.append(r['sheet'])
            if r['active']:
                s['usable'] += 1
                q = json.loads(r['data'])
                q['id'], q['sheet'], q['row'] = r['id'], r['sheet'], r['row_no']
                questions.append(q)
            else:
                s['inactive'] += 1
        last = self.last_import()
        return {
            'file': last['filename'] if last else '', 'mtime': last['at'] if last else '',
            'error': '', 'questions': questions,
            'sheets': [dict(sheets[n], total=sheets[n]['usable'], skipped=[]) for n in order if sheets[n]['usable'] or sheets[n]['inactive']],
        }

    def plan_import(self, parsed_sheets, parsed_questions):
        """比较「上传的题」和「库里现有的题」，只算不写。返回预览。"""
        with self.lock:
            rows = self.conn.execute('SELECT id, sheet, row_no, data, active FROM questions').fetchall()
        current = {r['id']: r for r in rows}
        new_ids = {q['id'] for q in parsed_questions}
        added, updated, unchanged, restored = [], [], [], []
        for q in parsed_questions:
            r = current.get(q['id'])
            if r is None:
                added.append(q)
            else:
                old = json.loads(r['data'])
                old['id'], old['sheet'], old['row'] = r['id'], r['sheet'], r['row_no']
                if not r['active']:
                    restored.append(q)
                elif _q_hash(old) != _q_hash(q):
                    updated.append(q)
                else:
                    unchanged.append(q)
        removed = [r for i, r in current.items() if r['active'] and i not in new_ids]
        by_sheet = {}
        for s in parsed_sheets:
            by_sheet[s['name']] = {'name': s['name'], 'total': s['total'], 'usable': s['usable'], 'skipped': s['skipped'],
                                   'added': 0, 'updated': 0, 'removed': 0, 'restored': 0}
        for q in added: by_sheet[q['sheet']]['added'] += 1
        for q in updated: by_sheet[q['sheet']]['updated'] += 1
        for q in restored: by_sheet[q['sheet']]['restored'] += 1
        for r in removed:
            by_sheet.setdefault(r['sheet'], {'name': r['sheet'], 'total': 0, 'usable': 0, 'skipped': [],
                                              'added': 0, 'updated': 0, 'removed': 0, 'restored': 0})['removed'] += 1
        return {
            'sheets': list(by_sheet.values()),
            'added': len(added), 'updated': len(updated), 'unchanged': len(unchanged),
            'restored': len(restored), 'removed': len(removed),
            'skipped': sum(len(s['skipped']) for s in parsed_sheets),
            'usable': len(parsed_questions),
            'removed_samples': [json.loads(r['data']).get('stem', '')[:40] for r in removed[:5]],
            'removed_stats': self._removed_with_history([r['id'] for r in removed]),
        }

    def _removed_with_history(self, ids):
        if not ids:
            return 0
        with self.lock:
            q = ','.join('?' * len(ids))
            return self.conn.execute('SELECT COUNT(*) c FROM stats WHERE question_id IN (%s)' % q, ids).fetchone()['c']

    def apply_import(self, filename, parsed_sheets, parsed_questions):
        """真正写入。整个过程一个事务：要么全成功，要么什么都不变。"""
        plan = self.plan_import(parsed_sheets, parsed_questions)
        t = now()
        with self.tx() as c:
            new_ids = set()
            for seq, q in enumerate(parsed_questions):
                new_ids.add(q['id'])
                data = json.dumps({k: v for k, v in q.items() if k not in ('id', 'sheet', 'row')}, ensure_ascii=False)
                c.execute(
                    'INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at) VALUES(?,?,?,?,?,1,?,?) '
                    'ON CONFLICT(id) DO UPDATE SET sheet=excluded.sheet, row_no=excluded.row_no, seq=excluded.seq, '
                    'data=excluded.data, active=1, updated_at=excluded.updated_at',
                    (q['id'], q['sheet'], q['row'], seq, data, t, t))
            # 上传的文件里没有的题 -> 下架（学习记录保留）
            base = len(parsed_questions)
            gone = [r['id'] for r in c.execute('SELECT id FROM questions WHERE active=1').fetchall() if r['id'] not in new_ids]
            for i, qid in enumerate(gone):
                c.execute('UPDATE questions SET active=0, seq=?, updated_at=? WHERE id=?', (base + i, t, qid))
            c.execute('INSERT INTO imports(at, filename, added, updated, removed, restored, skipped, total) VALUES(?,?,?,?,?,?,?,?)',
                      (t, filename, plan['added'], plan['updated'], plan['removed'], plan['restored'], plan['skipped'], len(parsed_questions)))
        return plan

    def last_import(self):
        with self.lock:
            r = self.conn.execute('SELECT * FROM imports ORDER BY id DESC LIMIT 1').fetchone()
        return dict(r) if r else None

    def import_history(self, limit=10):
        with self.lock:
            rows = self.conn.execute('SELECT * FROM imports ORDER BY id DESC LIMIT ?', (limit,)).fetchall()
        return [dict(r) for r in rows]

    # ---------------------------------------------------------------- 学习记录
    def progress(self):
        with self.lock:
            stats = self.conn.execute('SELECT * FROM stats').fetchall()
            hist = self.conn.execute('SELECT * FROM attempts ORDER BY id DESC LIMIT ?', (HISTORY_LIMIT,)).fetchall()
        questions = {}
        for r in stats:
            rec = {'attempts': r['attempts'], 'correct': r['correct'], 'wrong': r['wrong'], 'streak': r['streak']}
            if r['last'] is not None:
                rec['last'] = bool(r['last'])
            if r['at']:
                rec['at'] = r['at']
            questions[r['question_id']] = rec
        history = [{'at': r['at'], 'sheets': json.loads(r['sheets']), 'mode': r['mode'], 'total': r['total'],
                    'correct': r['correct'], 'percent': r['percent'], 'seconds': r['seconds']} for r in hist]
        return {'version': 1, 'questions': questions, 'history': history, 'masterStreak': MASTER_STREAK}

    def record_attempt(self, attempt):
        t = now()
        results = attempt['results']
        total = len(results)
        good = sum(1 for r in results if r['correct'])
        with self.tx() as c:
            for r in results:
                c.execute('INSERT OR IGNORE INTO stats(question_id) VALUES(?)', (r['id'],))
                if r['correct']:
                    c.execute('UPDATE stats SET attempts=attempts+1, correct=correct+1, streak=streak+1, last=1, at=? WHERE question_id=?', (t, r['id']))
                else:
                    c.execute('UPDATE stats SET attempts=attempts+1, wrong=wrong+1, streak=0, last=0, at=? WHERE question_id=?', (t, r['id']))
            c.execute('INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds) VALUES(?,?,?,?,?,?,?)',
                      (t, json.dumps(attempt['sheets'], ensure_ascii=False), attempt['mode'], total, good,
                       round(100 * good / total), attempt['seconds']))
            # 只保留最近的 HISTORY_LIMIT 场（和之前一致）
            c.execute('DELETE FROM attempts WHERE id NOT IN (SELECT id FROM attempts ORDER BY id DESC LIMIT ?)', (HISTORY_LIMIT,))
        return self.progress()

    def reset_progress(self):
        with self.tx() as c:
            c.execute('DELETE FROM stats')
            c.execute('DELETE FROM attempts')
        return self.progress()

    # ---------------------------------------------------------------- 从旧版 progress.json 迁移
    def migrate_legacy_progress(self, path):
        """把旧版的 progress.json 导入一次。已经迁移过、或已经有学习记录就不再动。返回 (是否迁移, 说明)。"""
        if self._get_meta('legacy_migrated'):
            return False, '之前已经迁移过'
        if not os.path.isfile(path):
            return False, '没有找到旧的 progress.json'
        try:
            with open(path, 'r', encoding='utf-8-sig') as f:
                d = json.load(f)
            qs, hist = d['questions'], d['history']
            assert isinstance(qs, dict) and isinstance(hist, list)
        except (OSError, ValueError, KeyError, AssertionError, TypeError, AttributeError) as e:
            return False, '旧的 progress.json 读不了：%s' % e
        with self.lock:
            has = self.conn.execute('SELECT (SELECT COUNT(*) FROM stats) + (SELECT COUNT(*) FROM attempts) n').fetchone()['n']
        if has:
            return False, '数据库里已经有学习记录，没有覆盖'
        try:
            return self._migrate_rows(qs, hist)
        except (TypeError, ValueError, KeyError, AttributeError) as e:
            return False, '旧的 progress.json 里有格式不对的内容，没有迁移：%s' % e

    def _migrate_rows(self, qs, hist):
        with self.tx() as c:
            for qid, r in qs.items():
                c.execute('INSERT OR REPLACE INTO stats(question_id, attempts, correct, wrong, streak, last, at) VALUES(?,?,?,?,?,?,?)',
                          (qid, int(r.get('attempts', 0)), int(r.get('correct', 0)), int(r.get('wrong', 0)), int(r.get('streak', 0)),
                           (1 if r.get('last') else 0) if 'last' in r else None, r.get('at')))
            for h in reversed(hist):         # 旧文件最新的在前，按时间从旧到新写入，这样 id 递增和时间一致
                c.execute('INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds) VALUES(?,?,?,?,?,?,?)',
                          (h.get('at', ''), json.dumps(h.get('sheets', []), ensure_ascii=False), h.get('mode', 'all'),
                           int(h['total']), int(h['correct']), int(h['percent']), int(h.get('seconds', 0))))
            c.execute("INSERT INTO meta(key,value) VALUES('legacy_migrated', ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (now(),))
        return True, '迁移了 %d 场成绩、%d 道题的答题记录' % (len(hist), len(qs))
