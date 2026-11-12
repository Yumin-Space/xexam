# -*- coding: utf-8 -*-
"""Extra made-up data for the phone walk, on top of what tools/screenshots/seed.py already creates (through the public
API only). Everything here is invented; nothing comes from a real exam.

build_bank(folder)   writes "Long sample bank.xlsx": 51 six-column questions in two sheets (43 short arithmetic
                     questions, 6 multiple-choice "choose 2" questions, one very long question with an unbroken
                     https://www.example.com/... address, one 5-item hotspot with long choices).
extra(base, cookie, folder)
                     after seed.prepare / activity / seed_exams: the administrator imports that bank, adds riley (a user
                     without the AI+ feature, sees every bank), lee (the administrator that seed.activity created) gets 60
                     study records, and "jordan" (a made-up user) is locked by failed sign-ins. Returns a summary dict.

Users after the whole seeding (all passwords are made-up demo values):
  admin  administrator, AI+ always            sam    user + bank manager, AI+
  lee    administrator (60 records)           alex   user, AI+ paused
  riley  user, NO AI+                         kim    user, disabled
  jordan user, locked
"""
import os
import sys
import urllib.error

HERE = os.path.dirname(os.path.abspath(__file__))
SHOTS = os.path.join(os.path.dirname(HERE), 'screenshots')
if SHOTS not in sys.path:
    sys.path.insert(0, SHOTS)

import demo_banks  # noqa: E402
import seed  # noqa: E402

SP = demo_banks.SP
ans = demo_banks.ans
q = demo_banks.q
BANK_NAME = 'Long sample bank'
BANK_FILE = BANK_NAME + '.xlsx'
RECORDS_OF_LEE = 60
LOCKED_USER = 'jordan'
LETTERS = 'ABCDE'


def arith(i):
    """Short single-choice question: right answer B."""
    a, b = 3 + i, 4 + 2 * i
    right = a + b
    opts = [right - 2, right, right + 1, right + 3]
    lines = '\n'.join('%s. %d' % (LETTERS[k], v) for k, v in enumerate(opts))
    return q('What is %d + %d?\n\n%s' % (a, b, lines), '%d + %d 等于几？\n\n%s' % (a, b, lines),
             '%d + %d はいくつですか？\n\n%s' % (a, b, lines),
             ans('B', 'Add the two numbers: %d + %d = %d.' % (a, b, right), 'Count on from the bigger number.',
                 'B', '把两个数相加：%d + %d = %d。' % (a, b, right), '从大的数往上数。',
                 'B', '二つの数を足します：%d + %d = %d。' % (a, b, right), '大きい数から数え足す。'))


# (title en, zh, ja), two right items, three wrong items; every item is (en, zh, ja)
PICK_TWO = [
    (('living in the water', '生活在水里', '水の中で暮らす'),
     [('Fish', '鱼', 'さかな'), ('Whale', '鲸', 'くじら')],
     [('Sparrow', '麻雀', 'すずめ'), ('Horse', '马', 'うま'), ('Rabbit', '兔子', 'うさぎ')]),
    (('musical instruments', '乐器', '楽器'),
     [('Piano', '钢琴', 'ピアノ'), ('Violin', '小提琴', 'バイオリン')],
     [('Hammer', '锤子', 'ハンマー'), ('Ladder', '梯子', 'はしご'), ('Bucket', '水桶', 'バケツ')]),
    (('made of paper', '用纸做的', '紙でできている'),
     [('Book', '书', '本'), ('Envelope', '信封', '封筒')],
     [('Stone', '石头', '石'), ('Spoon', '勺子', 'スプーン'), ('Cup', '杯子', 'コップ')]),
    (('even numbers', '偶数', '偶数'),
     [('2', '2', '2'), ('8', '8', '8')],
     [('3', '3', '3'), ('7', '7', '7'), ('9', '9', '9')]),
    (('giving light', '会发光', '光を出す'),
     [('Lamp', '灯', 'ランプ'), ('Candle', '蜡烛', 'ろうそく')],
     [('Pillow', '枕头', '枕'), ('Towel', '毛巾', 'タオル'), ('Chair', '椅子', 'いす')]),
    (('colours of a traffic light', '交通信号灯的颜色', '信号の色'),
     [('Red', '红', '赤'), ('Green', '绿', '緑')],
     [('Purple', '紫', '紫'), ('Pink', '粉', 'ピンク'), ('Brown', '棕', '茶色')]),
]


def pick_two(i):
    (t_en, t_zh, t_ja), right, wrong = PICK_TWO[i % len(PICK_TWO)]
    order = [right[0], wrong[0], right[1], wrong[1], wrong[2]]        # right ones are A and C
    texts = []
    for col, lead in ((0, 'Which two of the following are %s? (Choose 2)'), (1, '下面哪两项%s？（选2项）'),
                      (2, '次のうち、%sものを2つ選んでください。（2つ選択してください）')):
        title = (t_en, t_zh, t_ja)[col]
        body = '\n'.join('%s. %s' % (LETTERS[k], o[col]) for k, o in enumerate(order))
        texts.append((lead % title) + '\n\n' + body)
    return q(texts[0], texts[1], texts[2],
             ans('A, C', 'Only the two marked items fit; the other three do not.', 'Check each item against the title.',
                 'A、C', '只有这两项符合，其余三项不符合。', '逐项对照题目。',
                 'A、C', '当てはまるのはこの二つだけで、ほかの三つは当てはまりません。', '一つずつ題と見比べる。'))


LONG_EN = ('A small team keeps a shared notebook of study tips at https://www.example.com/very/long/path/that/does/not/'
           'break/anywhere/nicely/study-tips-for-the-weekend-group-and-the-evening-group-with-no-spaces-at-all.html and '
           'wants everybody to read it before the next meeting. The notebook is long, so the team splits it into parts, '
           'gives each person one part to summarise in three sentences, and then everybody reads all the summaries '
           'together at the start of the meeting. Which approach describes what the team is doing?\n\n'
           'A. Each person reads the whole notebook alone and nobody shares anything with the others before the meeting\n'
           'B. The work is divided into parts, each person summarises one part, and the summaries are shared with everyone\n'
           'C. The team cancels the meeting because the notebook is too long to read in the time that is left before it\n'
           'D. One person writes a completely new notebook from the beginning and throws the old shared notebook away')
LONG_ZH = ('一个小组把学习技巧记在共享笔记里（https://www.example.com/very/long/path/that/does/not/break/anywhere/nicely/'
           'study-tips-for-the-weekend-group-and-the-evening-group-with-no-spaces-at-all.html），希望大家在下次见面前都读一遍。'
           '笔记很长，所以小组把它分成几部分，每人负责用三句话总结其中一部分，见面时大家一起先读所有的总结。下面哪一项描述了小组的做法？\n\n'
           'A. 每个人独自读完整本笔记，见面前谁也不和别人分享任何内容\n'
           'B. 把工作分成几部分，每人总结一部分，再把总结分享给所有人\n'
           'C. 因为笔记太长，见面前读不完，小组取消了这次见面\n'
           'D. 由一个人从头写一本全新的笔记，并把原来的共享笔记扔掉')
LONG_JA = ('ある小さなチームは、勉強のコツを共有ノート（https://www.example.com/very/long/path/that/does/not/break/anywhere/'
           'nicely/study-tips-for-the-weekend-group-and-the-evening-group-with-no-spaces-at-all.html）にまとめていて、'
           '次の集まりまでに全員に読んでほしいと考えています。ノートは長いので、チームはそれをいくつかの部分に分け、'
           '各自が一つの部分を三文で要約し、集まりの最初に全員で要約をまとめて読みます。チームのやり方を説明しているのはどれですか？\n\n'
           'A. 各自がノート全体を一人で読み、集まりの前には誰とも何も共有しない\n'
           'B. 作業を分担し、各自が一部分を要約して、その要約を全員で共有する\n'
           'C. ノートが長すぎて間に合わないので、チームは集まりを取りやめる\n'
           'D. 一人が最初からまったく新しいノートを書き、元の共有ノートを捨てる')
LONG = q(LONG_EN, LONG_ZH, LONG_JA,
         ans('B', 'Splitting the reading and sharing short summaries lets everyone learn the whole notebook in less time.',
             'Split it, summarise it, share it.',
             'B', '把阅读分工、分享简短总结，大家能用更少的时间了解整本笔记。', '分工、总结、分享。',
             'B', '読む作業を分担して短い要約を共有すれば、全員が短い時間でノート全体を知ることができます。', '分けて、まとめて、共有する。'))

HS_EN = ('HOTSPOT\nFor each statement about planning a study week, select the best choice.\n\n'
         "1. Reviewing yesterday's notes for ten minutes every morning." + SP + 'Helps a lot / Helps a little / Does not help\n\n'
         '2. Reading the same page again and again without any questions.' + SP + 'Helps a lot / Helps a little / Does not help\n\n'
         '3. Explaining a new idea to a friend in your own words.' + SP + 'Helps a lot / Helps a little / Does not help\n\n'
         '4. Sleeping only four hours the night before a test.' + SP + 'Helps a lot / Helps a little / Does not help\n\n'
         '5. Writing short practice questions for yourself after each chapter.' + SP + 'Helps a lot / Helps a little / Does not help')
HS_ZH = ('热点题\n对下面每条关于安排学习周的说法，选择最合适的一项。\n\n'
         '1. 每天早上花十分钟复习前一天的笔记。' + SP + '很有帮助 / 有点帮助 / 没有帮助\n\n'
         '2. 不带任何问题，一遍又一遍地读同一页。' + SP + '很有帮助 / 有点帮助 / 没有帮助\n\n'
         '3. 用自己的话把新知识讲给朋友听。' + SP + '很有帮助 / 有点帮助 / 没有帮助\n\n'
         '4. 考试前一晚只睡四个小时。' + SP + '很有帮助 / 有点帮助 / 没有帮助\n\n'
         '5. 每学完一章就给自己出几道小练习题。' + SP + '很有帮助 / 有点帮助 / 没有帮助')
HS_JA = ('ホットスポット\n学習の一週間の計画について、それぞれ最も適切なものを選択してください。\n\n'
         '1. 毎朝10分間、前日のノートを見直す。' + SP + 'とても役に立つ / 少し役に立つ / 役に立たない\n\n'
         '2. 何の問いも持たずに同じページを何度も読む。' + SP + 'とても役に立つ / 少し役に立つ / 役に立たない\n\n'
         '3. 新しい考えを自分の言葉で友だちに説明する。' + SP + 'とても役に立つ / 少し役に立つ / 役に立たない\n\n'
         '4. テストの前の晩に4時間しか寝ない。' + SP + 'とても役に立つ / 少し役に立つ / 役に立たない\n\n'
         '5. 各章のあとで自分用の短い練習問題を作る。' + SP + 'とても役に立つ / 少し役に立つ / 役に立たない')
HOTSPOT = q(HS_EN, HS_ZH, HS_JA,
            ans('1 Helps a lot; 2 Does not help; 3 Helps a lot; 4 Does not help; 5 Helps a lot',
                'Short regular reviews, explaining in your own words and testing yourself help; passive rereading and '
                'too little sleep do not.', 'Review, explain, test yourself, sleep.',
                '1 很有帮助；2 没有帮助；3 很有帮助；4 没有帮助；5 很有帮助',
                '短时间的定期复习、用自己的话讲解、自我测试有帮助；被动重读和睡眠不足没有帮助。', '复习、讲解、自测、睡好。',
                '1 とても役に立つ；2 役に立たない；3 とても役に立つ；4 役に立たない；5 とても役に立つ',
                '短い定期的な復習、自分の言葉での説明、自分でのテストは役に立ちます。受け身の読み直しと睡眠不足は役に立ちません。',
                '復習・説明・自己テスト・睡眠。'))


def build_bank(folder):
    """Writes the 51-question workbook into `folder`; returns its path."""
    os.makedirs(folder, exist_ok=True)
    warm = [arith(i) for i in range(18)]
    warm.insert(3, LONG)
    warm.insert(7, HOTSPOT)
    warm.insert(11, pick_two(0))
    warm.insert(15, pick_two(1))
    mixed = [arith(30 + i) for i in range(25)]
    for k, pos in enumerate((2, 6, 10, 14)):
        mixed.insert(pos, pick_two(2 + k))
    rows = len(warm) + len(mixed)
    if rows != 51:
        raise RuntimeError('the long sample bank must have 51 questions, has %d' % rows)
    return demo_banks._save(folder, BANK_FILE, [('Warm-up', warm), ('Mixed practice', mixed)])


def _attempt(sess, questions, kind, wrong, seconds):
    sess.call('/api/attempt', {'results': seed._results(questions, wrong), 'sheets': sorted({x['sheet'] for x in questions}),
                               'mode': 'all', 'seconds': seconds, 'kind': kind})


def extra(base, cookie, folder):
    """The extra users, bank and records (see the module text); `cookie` is the administrator's session."""
    admin = seed.Session(base)
    admin.cookie[seed.COOKIE] = cookie
    admin.import_new(build_bank(folder))
    admin.call('/api/users/create', {'username': 'riley', 'password': seed.INITIAL, 'role': 'user', 'banks': 'all'})
    riley = seed.Session(base).sign_in('riley', seed.INITIAL)
    lee = seed.Session(base).sign_in('lee', seed.INITIAL)
    bank = lee.call('/api/bank')['questions']
    pool = bank[:12]
    for i in range(RECORDS_OF_LEE):
        wrong = {(i * 3 + k) % len(pool) for k in range(1 + i % 4)}
        _attempt(lee, pool, 'exam' if i % 3 == 0 else 'practice', wrong, 120 + (i * 37) % 900)
    n_records = len(lee.call('/api/records')['records'])
    _attempt(riley, pool, 'practice', {0, 2}, 210)
    # a made-up user that is guessed until the account locks
    admin.call('/api/users/create', {'username': LOCKED_USER, 'password': seed.INITIAL, 'role': 'user', 'banks': 'all'})
    codes = []
    for i in range(10):
        try:
            seed.Session(base).sign_in(LOCKED_USER, 'guess-%d' % i)
            codes.append(200)
        except urllib.error.HTTPError as e:
            codes.append(e.code)
    summary = {'questions': len(bank), 'leeRecords': n_records, 'lockedAnswers': codes}
    print('  extra data: %d questions visible, %d records for lee, %s sign-in answers %s'
          % (len(bank), n_records, LOCKED_USER, ' '.join(map(str, codes))))
    return summary


if __name__ == '__main__':
    if len(sys.argv) != 2:
        sys.exit('usage: python tools/phone-walk/seed_extra.py <output folder>')
    print('written', build_bank(sys.argv[1]))
