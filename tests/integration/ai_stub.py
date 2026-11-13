# -*- coding: utf-8 -*-
"""A made-up AI provider for the checks and the screenshot tool: answers like an OpenAI-compatible API, the Anthropic
API and a Cloudflare AI Gateway would, with deterministic self-made text. Standard library only.

usage: python3 -I tests/integration/ai_stub.py [--host 127.0.0.1] [--port 9000]

Routes (base URLs as an administrator would enter them in AI settings, e.g. http://ai-stub:9000/v1):
  OpenAI format     /v1, /gw/openai, /nolist/v1     GET <base>/models, POST <base>/chat/completions
  Anthropic format  /anthropic, /gw/anthropic       GET <base>/v1/models, POST <base>/v1/messages
  /nolist/v1/models answers 404 (a provider that does not list its models: manual model entry)
  GET /healthz      {"ok": true}

Keys: OpenAI paths need "Authorization: Bearer sk-demo-…"; Anthropic paths need "x-api-key: sk-demo-…" and
"anthropic-version". The gateway paths (/gw/…) need the gateway key gw-demo-… ("cf-aig-authorization: Bearer gw-demo-…",
or "Authorization: Bearer gw-demo-…" / "x-api-key: gw-demo-…" for the other two gateway schemes) and accept a missing
provider key (BYOK). A wrong key answers 401 with a body that echoes the key masked (sk-de****1234), like real providers,
so a check can prove such a body is never shown or logged.

Models: demo-chat-small, demo-chat-large (normal answers); demo-nousage (OpenAI: 400 when the body has stream_options);
demo-error (200, then an in-band error); demo-length (stops at the length limit); demo-4k (Anthropic: 400 when
max_tokens > 4096); demo-reasoning (OpenAI: first 6 chunks that carry only choices[0].delta.reasoning_content, made-up
English words 150 ms apart, then the normal answer); demo-embedding (OpenAI: listed by /models, but a chat request
answers 404 model_not_found, like an embedding model of a real gateway). The OpenAI model list has 7 ids.

Answer language: the one the system text names (OpenAI: the system message; Anthropic: "system"), in the line
"Always reply in <English|Simplified Chinese|Japanese> (" of the study assistant / free mode, or "Write the note in
<language>" of the memory suggestion; without such a line, the language of the last user message (any kana: Japanese,
else any Han character: Chinese, else English).
Answers stream in about 12 pieces 40 ms apart: a short note on spaced repetition, or a one-week plan when the message
asks for a plan or a week. Both are self-made Markdown with one bold phrase, a 3-item list and one inline code. A
message "Test" is answered "OK". A last user message containing [slow] waits 20 s before the first piece, then sends
one piece every 0.5 s for 10 s (keep-alive and revocation checks). The usage chunk comes when
stream_options.include_usage is set (OpenAI) and always in message_start / message_delta (Anthropic).

Study answers (v0.9; the system text decides): "asked for a HINT" answers a short self-made hint without option letters
and a knowledge-points list; when the user turn contains [leak] the hint ends with a leak ("The answer is B." in English,
"答案是 B。", "正解は B です。"; with the word "statements" in the user turn: "Statement 2 is true.", "第2项选是。",
"2番目は「はい」です。") so the guard test can see it withheld. "asked for an EXPLANATION" answers "**Answer: <the text inside
<answer_key>>**" and three short paragraphs. "answered wrong" (the summary) answers four Markdown headings and one line
"Items: <n>" (n = the number of <item tags). [slow] in the last user turn keeps working for all of them.

AI banks (v1.1; the system text decides, by its first words):
"You write new multiple-choice exam questions" (bank generation) and "You write exam questions from a user's study
material" (import, material mode): one valid made-up question per slot of the SLOTS list (three languages, the options and
the number of correct ones the slot names). "You sort a user's file": {"kind": ...} by a marker in the sample: "STUBQ" =
questions, "STUBM" = material, "STUBN" = none, else material. "You find the exam questions": the ranges of the numbered
lines whose text starts with "Q" followed by a digit (a question ends before the next one). "You convert exam questions":
one conversion per slot that keeps the source's stem and choices (a "make" slot gets four made-up choices, a "truefalse"
slot the two constants). All of them use only self-made words (no real question text ever).

One line per request is printed to stdout: method, path (without the query) and status. Never headers or bodies.
"""
import argparse
import http.server
import itertools
import json
import random
import re
import signal
import sys
import time

MODELS_OPENAI = ['demo-chat-small', 'demo-chat-large', 'demo-nousage', 'demo-error', 'demo-length', 'demo-reasoning',
                 'demo-embedding']
# Listed, but not chat models: a chat request answers 404 model_not_found
NOT_CHAT = ('demo-embedding',)
# Anthropic lists models newest first
MODELS_ANTHROPIC = ['demo-chat-small', 'demo-chat-large', 'demo-4k', 'demo-error', 'demo-length']
MAX_BODY = 1 << 20

ANSWERS = {
    'en': ('A good way to remember new material is **spaced repetition**:\n\n'
           '- Review it on the next day.\n'
           '- Review it again after three days.\n'
           '- Then review it once a week.\n\n'
           'Keep each note short, for example `term: meaning`.'),
    'zh': ('记住新知识的一个好办法是**间隔复习**：\n\n'
           '- 第二天复习一次。\n'
           '- 三天后再复习一次。\n'
           '- 之后每周复习一次。\n\n'
           '每条笔记写得简短一些，例如 `术语：含义`。'),
    'ja': ('新しい内容を覚えるには**間隔反復**がおすすめです。\n\n'
           '- 翌日に一度復習します。\n'
           '- 三日後にもう一度復習します。\n'
           '- その後は週に一度復習します。\n\n'
           'メモは短く書きましょう。例：`用語：意味`。'),
}
PLANS = {
    'en': ('Here is a **one-week plan**:\n\n'
           '- Days 1 and 2: learn 20 new words a day.\n'
           '- Days 3 to 5: review them and add 10 more.\n'
           '- Days 6 and 7: test yourself with the cards.\n\n'
           'Mark the hard ones with `*` so they come back more often.'),
    'zh': ('这是一个**一周计划**：\n\n'
           '- 第 1–2 天：每天学 20 个新词。\n'
           '- 第 3–5 天：复习，并再加 10 个。\n'
           '- 第 6–7 天：用卡片自测。\n\n'
           '难记的词标上 `*`，以后多看几次。'),
    'ja': ('**一週間のプラン**の例です。\n\n'
           '- 1〜2 日目：毎日新しい単語を 20 個覚えます。\n'
           '- 3〜5 日目：復習して、さらに 10 個追加します。\n'
           '- 6〜7 日目：カードで自分をテストします。\n\n'
           '覚えにくい単語には `*` を付けておきましょう。'),
}
PLAN_WORDS = ('plan', 'week', '计划', '周', 'プラン', '週')
NOTES = {
    'en': 'Prefers short explanations with one example.',
    'zh': '希望解释简短，并带一个例子。',
    'ja': '短い説明と例を一つ希望しています。',
}
# The reply language named by the system text (ai-service's prompt.rs): the study assistant / free mode line, or the
# memory suggestion's line
LANGUAGE_LINES = (re.compile(r'Always reply in (English|Simplified Chinese|Japanese) \('),
                  re.compile(r'Write the note in (English|Simplified Chinese|Japanese)'))
LANGUAGE_NAMES = {'English': 'en', 'Simplified Chinese': 'zh', 'Japanese': 'ja'}
NL = '\n'

# ---- v1.1: AI banks (generation and file import)
CONS = 'bcdfghjklmprstvwz'
VOW = 'aeiou'


def pw(n):
    """A unique pseudo word of six letters (the same generator idea as the Rust test kit; never a real word)."""
    out = ''
    x = n
    for _ in range(3):
        out += CONS[x % len(CONS)]
        x //= len(CONS)
        out += VOW[x % len(VOW)]
        x //= len(VOW)
    return out


def slot_list(user):
    """The JSON array after the line "SLOTS" of a generation / conversion / material user message."""
    m = re.search(r'SLOTS\n(\[.*?\])\n', user, re.S)
    try:
        return json.loads(m.group(1)) if m else []
    except ValueError:
        return []


def tri(text):
    return {'en': text, 'zh': '这是中文题目：' + text, 'ja': 'これは日本語の問題です：' + text}


# A process-wide counter: every made-up question gets fresh pseudo words, so ai-service's de-duplication (4-gram / bigram
# Jaccard >= 0.5 across the whole job) never sees two alike stems; the template words are kept short for the same reason
SEQ = itertools.count(1)


def made_up(slot, seq):
    """One valid made-up question for a generation / material slot (`seq` is ignored; the counter keeps stems apart)."""
    seq = next(SEQ) * 97
    n_opts = int(slot.get('options') or 4)
    n_right = max(1, int(slot.get('correct') or 1))
    words = ' '.join(pw(random.Random(seq * 10 + k).randrange(614125)) for k in range(5))
    # zh is compared by character bigrams: its pseudo words are made of Han characters (an arbitrary block, no meaning)
    rnd = random.Random(seq)
    han = ''.join(chr(0x4E00 + rnd.randrange(6000)) for _ in range(12))
    stem = {'en': 'Which %s fits?' % words, 'zh': '哪一项符合%s？' % han, 'ja': 'どれが %s に合いますか？' % words}
    options = []
    for k in range(n_opts):
        word = pw(seq * 31 + k * 5 + 11) + ' ' + pw(seq * 17 + k * 3 + 5)
        o = tri('The %s works as described' % word)
        o['correct'] = k < n_right
        options.append(o)
    return {'slot': slot.get('slot'), 'stem': stem, 'options': options,
            'explanation': tri('The marked statements hold because of the %s rule; the others contradict it in this case.' % pw(seq * 13 + 3))}


WHOLE_SYSTEM = 'You work on one file of a user of Xexam'
SEEN_PREFIXES = set()


def file_lines(last):
    """{line number: text} of the L<n> lines of a whole-file prefix."""
    return {int(a): b for a, b in re.findall(r'^L(\d+): (.*)$', last, re.M)}


def lines_of(rng, lines):
    """The non-empty texts of the lines a slot names ("L12-L18" or "L5")."""
    m = re.match(r'L(\d+)(?:-L(\d+))?$', str(rng or ''))
    if not m:
        return []
    lo, hi = int(m.group(1)), int(m.group(2) or m.group(1))
    return [lines[n].strip() for n in range(lo, hi + 1) if n in lines and lines[n].strip()]


def whole_answer(last):
    """The reply of a whole-file call (TASK RECOGNISE / CONVERT / WRITE) of the import."""
    lines = file_lines(last)
    task = last[last.rfind('TASK '):] if 'TASK ' in last else ''
    if task.startswith('TASK RECOGNISE'):
        starts = [n for n in sorted(lines) if re.match(r'(Q\d|\d+[.)．、])', lines[n].strip())]
        if 'STUBN' in last:
            kind = 'none'
        elif 'STUBM' in last:
            kind = 'material'
        else:
            kind = 'questions' if 'STUBQ' in last or len(starts) >= 3 else 'material'
        qs = []
        if kind == 'questions':
            top = max(lines) if lines else 1
            for i, n in enumerate(starts):
                end = (starts[i + 1] - 1) if i + 1 < len(starts) else top
                qs.append({'start': n, 'end': max(n, end), 'type': 'single'})
        return json.dumps({'kind': kind, 'language': 'en', 'questions': qs})
    slots = slot_list(task)
    if task.startswith('TASK CONVERT'):
        return convert_slots(slots, lambda sl: lines_of(sl.get('lines'), lines))
    return json.dumps({'questions': [made_up(sl, i) for i, sl in enumerate(slots)]}, ensure_ascii=False)


def cache_usage(system, last):
    """(cache read, cache write) tokens of a whole-file call: the first call of a prefix writes it, later ones read it."""
    if not system.startswith(WHOLE_SYSTEM) or 'TASK ' not in last:
        return 0, 0
    prefix = last[:last.rfind('TASK ')]
    n = len(prefix) // 4 + 1
    if prefix in SEEN_PREFIXES:
        return n, 0
    SEEN_PREFIXES.add(prefix)
    return 0, n


def convert_slots(slots, region_lines):
    """The answers of a conversion call; `region_lines(slot)` gives the source lines of a slot."""
    out = []
    for slot in slots:
        no = slot.get('slot')
        lines = region_lines(slot)
        stem_line = re.sub(r'^Q?\d+[.)]?\s*', '', lines[0]) if lines else 'Made-up question'
        kind = slot.get('choices')
        if kind == 'truefalse':
            opts = [{'en': 'True', 'zh': '正确', 'ja': '正しい', 'correct': True}, {'en': 'False', 'zh': '错误', 'ja': '誤り', 'correct': False}]
        elif kind == 'make':
            opts = []
            for k in range(int(slot.get('count') or 4)):
                o = tri('The %s choice number %d' % (pw(no * 9 + k), k + 1))
                o['correct'] = k == 0
                opts.append(o)
        else:
            texts = [re.sub(r'^\(?[A-Ha-h]\)?[.)．、:：]?\s*', '', x) for x in lines[1:] if re.match(r'^\(?[A-Ha-h][.)．、:：)]', x)]
            texts = texts or ['First choice', 'Second choice']
            opts = []
            for k, text in enumerate(texts):
                o = {'en': text, 'zh': '选项：' + text, 'ja': '選択肢：' + text}
                o['correct'] = k == 0
                opts.append(o)
        out.append({'slot': no, 'stem': {'en': stem_line, 'zh': '这是中文题目：' + stem_line, 'ja': 'これは日本語の問題です：' + stem_line},
                    'options': opts, 'explanation': tri('The marked choice is right because of the %s rule; the others do not fit.' % pw(no * 5 + 1))})
    return json.dumps({'questions': out}, ensure_ascii=False)


def import_answer(system, last):
    """The reply of an import / generation request by the first words of its system text, or None for any other request."""
    if system.startswith(WHOLE_SYSTEM):
        return whole_answer(last)
    if system.startswith("You sort a user's file"):
        kind = 'questions' if 'STUBQ' in last else 'none' if 'STUBN' in last else 'material'
        return json.dumps({'kind': kind, 'language': 'en'})
    if system.startswith('You find the exam questions'):
        m = re.search(r'start on lines L(\d+) to L(\d+)', last)
        lo, hi = (int(m.group(1)), int(m.group(2))) if m else (1, 1)
        lines = {int(a): b for a, b in re.findall(r'^L(\d+): (.*)$', last, re.M)}
        starts = [n for n in sorted(lines) if lo <= n <= hi and re.match(r'Q\d', lines[n].strip())]
        top = max(lines) if lines else hi
        qs = []
        for i, n in enumerate(starts):
            end = (starts[i + 1] - 1) if i + 1 < len(starts) else min(top, hi)
            qs.append({'start': n, 'end': max(n, end), 'type': 'single'})
        return json.dumps({'questions': qs})
    if system.startswith('You convert exam questions'):
        def region_lines(slot):
            src = str(slot.get('source'))
            region = re.search(r'<<<%s [0-9a-f]+>>>\n(.*?)\n<<<END %s ' % (re.escape(src), re.escape(src)), last, re.S)
            return [x.strip() for x in (region.group(1) if region else '').split('\n') if x.strip()]
        return convert_slots(slot_list(last), region_lines)
    if system.startswith('You write new multiple-choice exam questions') or system.startswith("You write exam questions from a user's study material"):
        slots = slot_list(last)
        seq0 = sum(ord(c) for c in re.findall(r'<<<(?:SOURCE|MATERIAL) ([0-9a-f]+)>>>', last)[:1] and last[:40]) % 50
        return json.dumps({'questions': [made_up(sl, seq0 + int(sl.get('slot') or i + 1) * 3 + i) for i, sl in enumerate(slots)]}, ensure_ascii=False)
    return None
# Study answers (v0.9): made-up texts, in the reply language; the hints name no option letter
HINTS = {
    'en': ('First work out what the question is really asking, then compare it with what you know about the topic, one step at a time.' + NL + NL +
           '## Knowledge points' + NL + NL +
           '- Recall the basic definition behind the question.' + NL +
           '- Think about the usual exceptions to the rule.' + NL +
           '- Check the units and the wording before you decide.'),
    'zh': ('先弄清题目到底在问什么，再一步一步对照你对这个主题的了解。' + NL + NL +
           '## 知识点' + NL + NL +
           '- 回忆题目背后的基本定义。' + NL +
           '- 想一想这条规则常见的例外。' + NL +
           '- 做决定之前先核对单位和措辞。'),
    'ja': ('まず問題が本当に何を尋ねているのかを整理し、このテーマについて知っていることと一つずつ照らし合わせましょう。' + NL + NL +
           '## 知識ポイント' + NL + NL +
           '- 問題の背景にある基本的な定義を思い出します。' + NL +
           '- そのルールのよくある例外を考えます。' + NL +
           '- 決める前に単位と言い回しを確認します。'),
}
LEAKS = {
    'en': ('The answer is B.', 'Statement 2 is true.'),
    'zh': ('答案是 B。', '第2项选是。'),
    'ja': ('正解は B です。', '2番目は「はい」です。'),
}
EXPLAIN_TAIL = {
    'en': ('This is the reasoning behind the answer.', 'The other choices fail for a simple reason.', 'Remember the rule in one short sentence.'),
    'zh': ('这是得出答案的思路。', '其他选择不成立，原因很简单。', '用一句话记住这条规则。'),
    'ja': ('これが答えに至る考え方です。', '他の選択肢が成り立たない理由は単純です。', 'ルールを一文で覚えましょう。'),
}
SUMMARY_HEADS = {
    'en': ('## Weak knowledge points', '## What to watch out for', '## How to approach such questions', '## Summary of the knowledge points'),
    'zh': ('## 薄弱知识点', '## 需要注意的地方', '## 今后如何做这类题', '## 知识点小结'),
    'ja': ('## 弱い知識ポイント', '## 注意すること', '## これからの取り組み方', '## 知識ポイントのまとめ'),
}
ANSWER_KEY = re.compile(r'<answer_key>(.*?)</answer_key>', re.S)
# What demo-reasoning "thinks" before it answers: made-up words, never shown to anyone
REASONING = ['Thinking about', ' the question:', ' review timing,', ' short notes,', ' one example.', ' Writing now.']


def named_lang(system):
    """The language the system text asks for ('en' / 'zh' / 'ja'), or None."""
    for line in LANGUAGE_LINES:
        m = line.search(system)
        if m:
            return LANGUAGE_NAMES[m.group(1)]
    return None


def lang_of(text):
    if any('\u3040' <= c <= '\u30ff' or '\u31f0' <= c <= '\u31ff' or '\uff66' <= c <= '\uff9f' for c in text):
        return 'ja'
    if any('\u4e00' <= c <= '\u9fff' or '\u3400' <= c <= '\u4dbf' for c in text):
        return 'zh'
    return 'en'


def text_of(content):
    """The text of a message content: a string, or a list of parts / blocks with "text"."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return ''.join(p.get('text', '') for p in content if isinstance(p, dict) and isinstance(p.get('text'), str))
    return ''


def pieces(text, n):
    """`text` in `n` pieces of about equal length (fewer when the text is shorter)."""
    n = max(1, min(n, len(text)))
    size = len(text) / n
    return [text[round(i * size):round((i + 1) * size)] for i in range(n)]


def mask(key):
    return key[:5] + '****' + key[-4:] if len(key) >= 12 else '****'


def tokens(text):
    return len(text) // 4 + 1


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    server_version = 'demo-ai'
    sys_version = ''

    # ------------------------------------------------------------------ plumbing

    def log_message(self, *args):
        pass

    def _line(self, status):
        print('%s %s %d' % (self.command, self.path.split('?', 1)[0], status), flush=True)

    def _body(self):
        if (self.headers.get('Transfer-Encoding') or '').lower() == 'chunked':
            data = b''
            while True:
                n = int(self.rfile.readline().split(b';', 1)[0].strip() or b'0', 16)
                if n == 0:
                    self.rfile.readline()
                    return data
                data += self.rfile.read(n)
                self.rfile.readline()
                if len(data) > MAX_BODY:
                    raise ValueError('body too large')
        n = int(self.headers.get('Content-Length') or 0)
        if n > MAX_BODY:
            raise ValueError('body too large')
        return self.rfile.read(n) if n else b''

    def _json(self, status, obj):
        out = json.dumps(obj, ensure_ascii=False).encode('utf-8')
        self._line(status)
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def _stream_start(self):
        self._line(200)
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream; charset=utf-8')
        self.send_header('Cache-Control', 'no-cache')
        self.send_header('Transfer-Encoding', 'chunked')
        self.end_headers()

    def _chunk(self, text):
        data = text.encode('utf-8')
        self.wfile.write(b'%x\r\n%s\r\n' % (len(data), data))
        self.wfile.flush()

    def _stream_end(self):
        self.wfile.write(b'0\r\n\r\n')
        self.wfile.flush()

    # ------------------------------------------------------------------ routing

    def do_GET(self):
        self._handle('GET')

    def do_POST(self):
        self._handle('POST')

    def _handle(self, method):
        path = self.path.split('?', 1)[0]
        try:
            body = self._body() if method == 'POST' else b''
        except ValueError:
            self.close_connection = True
            return self._json(413, {'error': {'message': 'Request too large', 'type': 'invalid_request_error'}})
        try:
            if method == 'GET' and path == '/healthz':
                return self._json(200, {'ok': True})
            for prefix, gateway in (('/v1', False), ('/gw/openai', True), ('/nolist/v1', False)):
                if path in (prefix + '/models', prefix + '/chat/completions'):
                    return self._openai(method, path[len(prefix):], body, gateway, listed=prefix != '/nolist/v1')
            for prefix, gateway in (('/anthropic', False), ('/gw/anthropic', True)):
                if path in (prefix + '/v1/models', prefix + '/v1/messages'):
                    return self._anthropic(method, path[len(prefix):], body, gateway)
            return self._json(404, {'error': {'message': 'Not found', 'type': 'invalid_request_error', 'code': 'not_found'}})
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            self.close_connection = True     # the caller went away (Stop, revocation): nothing more to send

    # ------------------------------------------------------------------ keys

    def _gateway_ok(self):
        """The gateway key, in any of the three gateway schemes. Returns (ok, the wrong key or None)."""
        for name, prefix in (('cf-aig-authorization', 'Bearer '), ('Authorization', 'Bearer '), ('x-api-key', '')):
            value = self.headers.get(name) or ''
            if value.startswith(prefix) and value[len(prefix):].startswith('gw-demo-'):
                return True, None
        wrong = (self.headers.get('cf-aig-authorization') or '').replace('Bearer ', '', 1)
        return False, wrong or None

    def _provider_key(self, header, prefix):
        """(present, ok, key) of the provider key in `header` (a gateway key there does not count as a provider key)."""
        value = self.headers.get(header) or ''
        if not value:
            return False, False, ''
        key = value[len(prefix):] if value.startswith(prefix) else value
        if key.startswith('gw-demo-'):
            return False, False, ''
        return True, key.startswith('sk-demo-'), key

    def _refuse_key(self, key, anthropic):
        if anthropic:
            return self._json(401, {'type': 'error', 'error': {
                'type': 'authentication_error', 'message': 'invalid x-api-key: %s' % mask(key)}})
        return self._json(401, {'error': {
            'message': 'Incorrect API key provided: %s. You can find your API key in your account settings.' % mask(key),
            'type': 'invalid_request_error', 'param': None, 'code': 'invalid_api_key'}})

    # ------------------------------------------------------------------ OpenAI format

    def _openai(self, method, route, body, gateway, listed):
        if gateway:
            good, wrong = self._gateway_ok()
            if not good:
                return self._refuse_key(wrong or '', False)
        present, good, key = self._provider_key('Authorization', 'Bearer ')
        if (present and not good) or (not present and not gateway):
            return self._refuse_key(key, False)
        if route == '/models':
            if method != 'GET':
                return self._json(405, {'error': {'message': 'Method not allowed', 'type': 'invalid_request_error'}})
            if not listed:
                return self._json(404, {'error': {'message': 'Not found', 'type': 'invalid_request_error'}})
            return self._json(200, {'object': 'list', 'data': [
                {'id': m, 'object': 'model', 'created': 1767225600, 'owned_by': 'demo'} for m in MODELS_OPENAI]})
        if method != 'POST':
            return self._json(405, {'error': {'message': 'Method not allowed', 'type': 'invalid_request_error'}})
        try:
            req = json.loads(body or b'{}')
            model, messages = req['model'], req['messages']
        except (ValueError, KeyError, TypeError):
            return self._json(400, {'error': {'message': 'Invalid request body', 'type': 'invalid_request_error'}})
        if model not in MODELS_OPENAI:
            return self._json(404, {'error': {'message': 'The model `%s` does not exist' % str(model)[:80],
                                              'type': 'invalid_request_error', 'code': 'model_not_found'}})
        if model in NOT_CHAT:
            return self._json(404, {'error': {'message': 'The model `%s` is an embedding model and does not support chat '
                                                         'completions' % model,
                                              'type': 'invalid_request_error', 'code': 'model_not_found'}})
        if model == 'demo-nousage' and 'stream_options' in req:
            return self._json(400, {'error': {'message': 'Unrecognized request argument supplied: stream_options',
                                              'type': 'invalid_request_error', 'param': None, 'code': None}})
        system = ' '.join(text_of(m.get('content')) for m in messages if isinstance(m, dict) and m.get('role') == 'system')
        last = next((text_of(m.get('content')) for m in reversed(messages)
                     if isinstance(m, dict) and m.get('role') == 'user'), '')
        answer, slow = self._answer(system, last)
        c_read, c_write = cache_usage(system, last)
        usage = bool((req.get('stream_options') or {}).get('include_usage'))
        prompt = sum(len(text_of(m.get('content'))) for m in messages if isinstance(m, dict))

        def chunk(delta, finish=None):
            return 'data: %s\n\n' % json.dumps({
                'id': 'chatcmpl-demo', 'object': 'chat.completion.chunk', 'created': int(time.time()), 'model': model,
                'choices': [{'index': 0, 'delta': delta, 'finish_reason': finish}]}, ensure_ascii=False)

        self._stream_start()
        self._chunk(chunk({'role': 'assistant', 'content': ''}))
        if model == 'demo-reasoning':
            for words in REASONING:
                time.sleep(0.15)
                self._chunk(chunk({'reasoning_content': words}))
        parts, gap = self._timing(answer, slow)
        for i, piece in enumerate(parts):
            time.sleep(gap if i or not slow else 20)
            self._chunk(chunk({'content': piece}))
            if model == 'demo-error':
                self._chunk('data: %s\n\n' % json.dumps({'error': {
                    'message': 'The demo model failed on purpose.', 'type': 'api_error', 'code': 'api_error'}}))
                return self._stream_end()
        self._chunk(chunk({}, 'length' if model == 'demo-length' else 'stop'))
        if usage:
            self._chunk('data: %s\n\n' % json.dumps({
                'id': 'chatcmpl-demo', 'object': 'chat.completion.chunk', 'created': int(time.time()), 'model': model,
                'choices': [], 'usage': {'prompt_tokens': tokens(' ' * prompt), 'completion_tokens': tokens(answer),
                                         'total_tokens': tokens(' ' * prompt) + tokens(answer),
                                         'prompt_tokens_details': {'cached_tokens': c_read, 'cache_creation_tokens': c_write}}}))
        self._chunk('data: [DONE]\n\n')
        self._stream_end()

    # ------------------------------------------------------------------ Anthropic format

    def _anthropic(self, method, route, body, gateway):
        if gateway:
            good, wrong = self._gateway_ok()
            if not good:
                return self._refuse_key(wrong or '', True)
        present, good, key = self._provider_key('x-api-key', '')
        if (present and not good) or (not present and not gateway):
            return self._refuse_key(key, True)
        if not self.headers.get('anthropic-version'):
            return self._json(400, {'type': 'error', 'error': {
                'type': 'invalid_request_error', 'message': 'anthropic-version: header is required'}})
        if route == '/v1/models':
            if method != 'GET':
                return self._json(405, {'type': 'error', 'error': {'type': 'invalid_request_error', 'message': 'Method not allowed'}})
            data = [{'type': 'model', 'id': m, 'display_name': m.replace('-', ' ').title(),
                     'created_at': '2026-01-01T00:00:00Z'} for m in MODELS_ANTHROPIC]
            return self._json(200, {'data': data, 'has_more': False, 'first_id': data[0]['id'], 'last_id': data[-1]['id']})
        if method != 'POST':
            return self._json(405, {'type': 'error', 'error': {'type': 'invalid_request_error', 'message': 'Method not allowed'}})
        try:
            req = json.loads(body or b'{}')
            model, messages, max_tokens = req['model'], req['messages'], int(req['max_tokens'])
        except (ValueError, KeyError, TypeError):
            return self._json(400, {'type': 'error', 'error': {'type': 'invalid_request_error', 'message': 'Invalid request body'}})
        if model not in MODELS_ANTHROPIC:
            return self._json(404, {'type': 'error', 'error': {'type': 'not_found_error', 'message': 'model: %s' % str(model)[:80]}})
        if model == 'demo-4k' and max_tokens > 4096:
            return self._json(400, {'type': 'error', 'error': {
                'type': 'invalid_request_error',
                'message': 'max_tokens: %d > 4096, which is the maximum allowed number of output tokens for demo-4k' % max_tokens}})
        system = text_of(req.get('system'))
        last = next((text_of(m.get('content')) for m in reversed(messages)
                     if isinstance(m, dict) and m.get('role') == 'user'), '')
        answer, slow = self._answer(system, last)
        c_read, c_write = cache_usage(system, last)
        prompt = len(system) + sum(len(text_of(m.get('content'))) for m in messages if isinstance(m, dict))

        def event(name, obj):
            return 'event: %s\ndata: %s\n\n' % (name, json.dumps(obj, ensure_ascii=False))

        self._stream_start()
        self._chunk(event('message_start', {'type': 'message_start', 'message': {
            'id': 'msg_demo', 'type': 'message', 'role': 'assistant', 'model': model, 'content': [],
            'stop_reason': None, 'stop_sequence': None,
            'usage': {'input_tokens': tokens(' ' * prompt), 'output_tokens': 1,
                      'cache_read_input_tokens': c_read, 'cache_creation_input_tokens': c_write}}}))
        self._chunk(event('content_block_start', {'type': 'content_block_start', 'index': 0,
                                                  'content_block': {'type': 'text', 'text': ''}}))
        self._chunk(event('ping', {'type': 'ping'}))
        parts, gap = self._timing(answer, slow)
        for i, piece in enumerate(parts):
            time.sleep(gap if i or not slow else 20)
            self._chunk(event('content_block_delta', {'type': 'content_block_delta', 'index': 0,
                                                      'delta': {'type': 'text_delta', 'text': piece}}))
            if model == 'demo-error':
                self._chunk(event('error', {'type': 'error', 'error': {'type': 'overloaded_error', 'message': 'Overloaded'}}))
                return self._stream_end()
        self._chunk(event('content_block_stop', {'type': 'content_block_stop', 'index': 0}))
        self._chunk(event('message_delta', {'type': 'message_delta',
                                            'delta': {'stop_reason': 'max_tokens' if model == 'demo-length' else 'end_turn',
                                                      'stop_sequence': None},
                                            'usage': {'output_tokens': tokens(answer)}}))
        self._chunk(event('message_stop', {'type': 'message_stop'}))
        self._stream_end()

    # ------------------------------------------------------------------ answers

    @staticmethod
    def _answer(system, last):
        """(answer text, slow) for the last user message, in the language the system text names (else the message's)."""
        lang = named_lang(system) or lang_of(last)
        ai_bank = import_answer(system, last)
        if ai_bank is not None:
            return ai_bank, False
        if last.strip() == 'Test':
            return 'OK', False
        if 'remember for later chats' in system:     # the memory suggestion (one short note)
            return NOTES[lang], False
        study = Handler._study(system, last, lang)
        if study is not None:
            return study, '[slow]' in last
        text = PLANS[lang] if any(w in last.lower() for w in PLAN_WORDS) else ANSWERS[lang]
        return text, '[slow]' in last

    @staticmethod
    def _study(system, last, lang):
        """The answer of a study request (hint / explanation / summary), or None for any other request."""
        if 'asked for a HINT' in system:
            text = HINTS[lang]
            if '[leak]' in last:
                text += NL + NL + LEAKS[lang][1 if 'statements' in last else 0]
            return text
        if 'asked for an EXPLANATION' in system:
            m = ANSWER_KEY.search(last)
            key = m.group(1).strip() if m else '-'
            return ('**Answer: %s**' % key) + NL + NL + (NL + NL).join(EXPLAIN_TAIL[lang])
        if 'answered wrong' in system:
            h = SUMMARY_HEADS[lang]
            return (NL + NL).join('%s%s- point %d' % (h[i], NL + NL, i + 1) for i in range(4)) + NL + NL + 'Items: %d' % last.count('<item')
        return None

    @staticmethod
    def _timing(answer, slow):
        """The pieces and the pause between them: 12 pieces 40 ms apart, or 20 pieces 0.5 s apart when slow."""
        return (pieces(answer, 20), 0.5) if slow else (pieces(answer, 12), 0.04)


def main():
    p = argparse.ArgumentParser(description='A made-up AI provider (OpenAI-compatible, Anthropic, AI Gateway) for checks.')
    p.add_argument('--host', default='127.0.0.1')
    p.add_argument('--port', type=int, default=9000)
    args = p.parse_args()
    # As PID 1 in a container Python would ignore SIGTERM: exit at once instead
    signal.signal(signal.SIGTERM, lambda *a: sys.exit(0))
    server = http.server.ThreadingHTTPServer((args.host, args.port), Handler)
    server.daemon_threads = True
    print('demo AI provider listening on %s:%d' % (args.host, args.port), flush=True)
    try:
        server.serve_forever()
    except (KeyboardInterrupt, SystemExit):
        pass
    finally:
        server.server_close()


if __name__ == '__main__':
    main()
