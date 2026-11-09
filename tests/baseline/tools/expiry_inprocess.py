# In-process test (no server, no port): Pending TTL / one-time semantics / concurrent take. Imports server.py read-only.
import sys, time, threading, json
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import paths  # noqa: E402
sys.path.insert(0, paths.LEGACY_ROOT)
import server as S
res = {}
p = S.Pending()
t = p.put('a.xlsx', [], [{'id': 'x'}]); assert p.take(t) is not None; res['take_twice_second_is_none'] = p.take(t) is None
S.PENDING_TTL = -1                      # monkeypatch: token born already expired
t = p.put('a.xlsx', [], [{'id': 'x'}]); res['expired_take_none'] = p.take(t) is None
res['expired_item_NOT_cleared_still_in_memory'] = p.item is not None
S.PENDING_TTL = 30 * 60
t = p.put('a.xlsx', [], [{'id': 'x'}]); p.item['expires'] = time.time() - 0.001
res['expired_by_clock_take_none'] = p.take(t) is None
t1 = p.put('1', [], []); t2 = p.put('2', [], []); res['second_put_replaces_first'] = p.take(t1) is None and p.take(t2) is not None
t = p.put('a', [], []); p.drop(); res['drop_then_take_none'] = p.take(t) is None
# wrong token does NOT consume the pending item
t = p.put('a', [], []); p.take('wrong'); res['wrong_token_keeps_item'] = p.take(t) is not None
# concurrency: 50 threads take the same token, exactly one wins
t = p.put('a', [], []); wins = []
def w():
    if p.take(t) is not None: wins.append(1)
th = [threading.Thread(target=w) for _ in range(50)]; [x.start() for x in th]; [x.join() for x in th]
res['concurrent_take_winners'] = len(wins)
# non-ASCII token -> compare_digest raises TypeError (surfaces as dropped connection over HTTP)
t = p.put('a', [], [])
try: p.take('令牌'); res['non_ascii_token'] = 'no error'
except Exception as e: res['non_ascii_token'] = type(e).__name__
res['token_len'] = len(t); res['token_alphabet_urlsafe'] = all(c.isalnum() or c in '-_' for c in t)
print(json.dumps(res, indent=1))
