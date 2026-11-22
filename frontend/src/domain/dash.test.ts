import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { Question } from '../types/api.ts';
import {
  BOARD_H_PHONE, BOARD_H_WIDE, boardLayout, detailId, greetName, labelWidth, LABEL_SIZES, NUM1_PX, NUM_PX, PODIUM_ORDER, podiumOrder,
  rankNote, togglePin, wrongRows, type Box2,
} from './dash.ts';

test('greetName: the display name when set, else the local part of the login', () => {
  assert.equal(greetName({ username: 'bob@example.invalid', displayName: 'Bobby' }), 'Bobby');
  assert.equal(greetName({ username: 'bob@example.invalid', displayName: '  Bobby  ' }), 'Bobby');
  assert.equal(greetName({ username: 'bob@example.invalid', displayName: null }), 'bob');
  assert.equal(greetName({ username: 'alice' }), 'alice');
  assert.equal(greetName({ username: '  carol  ' }), 'carol');
  assert.equal(greetName({ username: '@example.invalid' }), '@example.invalid');
  assert.equal(greetName({ username: '小林@example.invalid' }), '小林');
  assert.equal(greetName({ username: '' }), '');
});

const q = (id: string, stem: string, sheet: string, bank: number, extra: Partial<Question> = {}): Question =>
  ({ kind: 'single', tag: '', stem, options: [], items: [], answer: [], translation: '', zh: null, explanation: '', mnemonic: '', note: '', id, sheet, row: 1, bank, ...extra }) as unknown as Question;

const bank = {
  questions: [
    q('q1', 'Which   port\nis HTTP?', 'Net', 1, { zh: { stem: '哪个端口是 HTTP？', options: [], items: [] } as never }),
    q('q2', 'Second sample', 'Misc', 2),
    q('q3', 'Third sample', 'Misc', 2),
  ],
};

test('wrongRows resolves bank, sheet and the full stem, skips unknown ids and numbers the places again', () => {
  const rows = wrongRows(
    [
      { questionId: 'q2', bankId: 2, wrong: 9, attempts: 12 },
      { questionId: 'gone', bankId: 1, wrong: 7, attempts: 7 },
      { questionId: 'q1', bankId: 1, wrong: 3, attempts: 5 },
    ],
    bank,
    'en',
  );
  assert.deepEqual(rows.map((r) => [r.rank, r.id, r.sheet, r.wrong, r.attempts]), [[1, 'q2', 'Misc', 9, 12], [2, 'q1', 'Net', 3, 5]]);
  // The stem keeps its line breaks and inner spaces (the page shows it as written), only the ends are trimmed
  assert.equal(rows[1]!.stem, 'Which   port\nis HTTP?');
});

test('wrongRows reads the stem in the wanted language and falls back to the base language', () => {
  const items = [{ questionId: 'q1', bankId: 1, wrong: 2, attempts: 3 }, { questionId: 'q2', bankId: 2, wrong: 1, attempts: 1 }];
  const zh = wrongRows(items, bank, 'zh');
  assert.equal(zh[0]!.stem, '哪个端口是 HTTP？');
  // q2 has no Chinese: its base language stands in
  assert.equal(zh[1]!.stem, 'Second sample');
});

test('wrongRows keeps at most five places and is empty without a loaded bank', () => {
  const many = Array.from({ length: 8 }, () => ({ questionId: 'q3', bankId: 2, wrong: 1, attempts: 1 }));
  assert.equal(wrongRows(many, bank, 'en').length, 5);
  assert.deepEqual(wrongRows([{ questionId: 'q1', bankId: 1, wrong: 1, attempts: 1 }], null, 'en'), []);
});

test('podiumOrder stands the places 4, 2, 1, 3, 5 and leaves no gap for missing ones', () => {
  assert.deepEqual(PODIUM_ORDER, [4, 2, 1, 3, 5]);
  const r = (rank: number) => ({ rank });
  assert.deepEqual(podiumOrder([1, 2, 3, 4, 5].map(r)).map((x) => x.rank), [4, 2, 1, 3, 5]);
  assert.deepEqual(podiumOrder([1, 2, 3].map(r)).map((x) => x.rank), [2, 1, 3]);
  assert.deepEqual(podiumOrder([1, 2].map(r)).map((x) => x.rank), [2, 1]);
  assert.deepEqual(podiumOrder([r(1)]).map((x) => x.rank), [1]);
  assert.deepEqual(podiumOrder([]), []);
});

test('a tap pins a point, the same point again unpins it; hover wins while there is one', () => {
  assert.equal(togglePin(null, 'a'), 'a');
  assert.equal(togglePin('a', 'a'), null);
  assert.equal(togglePin('a', 'b'), 'b');
  assert.equal(detailId(null, null), null);
  assert.equal(detailId(null, 'p'), 'p');
  assert.equal(detailId('h', 'p'), 'h');
});

test('rankNote: an empty board shows one line only, the invitation', () => {
  // Empty install: nobody ranked, the viewer neither -> only the invitation, not "nobody has answered" as well
  assert.equal(rankNote(0, null), 'noneYou');
  // Others ranked, the viewer not -> the invitation under the rows
  assert.equal(rankNote(3, null), 'noneYou');
  // The viewer ranked -> nothing more
  assert.equal(rankNote(3, 2), null);
  assert.equal(rankNote(0, 1), 'empty');
});

// ---------------------------------------------------------------- the chalk parabola (v2.6)

const LONG: Record<string, Record<number, string>> = {
  en: { 1: '999 misses', 2: '999 misses', 3: '999 misses', 4: '999 misses', 5: '999 misses' },
  short: { 1: '1 miss', 2: '18 misses', 3: '12 misses', 4: '10 misses', 5: '1 miss' },
  zh: { 1: '错 999 次', 2: '错 999 次', 3: '错 99 次', 4: '错 99 次', 5: '错 1 次' },
  ja: { 1: '999 回', 2: '999 回', 3: '999 回', 4: '999 回', 5: '1 回' },
  // Far beyond real counts: places 2 and 3 run out of room beside their dots and hang under them
  huge: { 1: '99999 misses', 2: '99999 misses', 3: '99999 misses', 4: '99999 misses', 5: '99999 misses' },
};
// Board widths from a 320 px phone (about 270) up to the desktop card
const WIDTHS = Array.from({ length: 31 }, (_, i) => 260 + i * 10);

const hit = (a: Box2, b: Box2): boolean => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
const within = (a: Box2, w: number, hgt: number): boolean => a.x >= 0 && a.y >= 0 && a.x + a.w <= w && a.y + a.h <= hgt;
/** The place number's box: centred on the dot, its baseline numY */
const numBox = (p: { numX: number; numY: number; rank: number }): Box2 => {
  const px = p.rank === 1 ? NUM1_PX : NUM_PX;
  return { x: p.numX - px * 0.35, y: p.numY - px * 0.75, w: px * 0.7, h: px * 0.75 };
};

test('boardLayout: a downward parabola, places left to right 4, 2, 1, 3, 5, place 1 highest', () => {
  for (const w of [270, 330, 510]) {
    const lay = boardLayout(w);
    const at = (rank: number) => lay.places.find((p) => p.rank === rank)!;
    assert.deepEqual([...lay.places].sort((a, b) => a.x - b.x).map((p) => p.rank), [4, 2, 1, 3, 5]);
    assert.ok(at(1).y < at(2).y && at(2).y < at(4).y, 'place 1 is the vertex, 4 / 5 lowest');
    assert.equal(at(2).y, at(3).y);
    assert.equal(at(4).y, at(5).y);
    assert.ok(at(1).r > at(2).r, 'place 1 has the bigger dot');
    // Symmetric about the middle
    assert.ok(Math.abs(at(2).x + at(3).x - w) < 1e-9 && Math.abs(at(4).x + at(5).x - w) < 1e-9);
    // The curve: highest in the middle, falling to both sides, inside the board
    const ys = lay.curve.map((c) => c.y);
    const mid = Math.floor(ys.length / 2);
    assert.equal(Math.min(...ys), ys[mid]);
    for (let i = 1; i <= mid; i++) assert.ok(ys[mid - i]! >= ys[mid - i + 1]! && ys[mid + i]! >= ys[mid + i - 1]!);
    assert.ok(lay.curve.every((c) => c.x >= 0 && c.x <= lay.w && c.y >= 0 && c.y <= lay.h));
  }
});

test('boardLayout: the board is 200 px high on phones and 220 px on wider cards', () => {
  assert.equal(boardLayout(270).h, BOARD_H_PHONE);
  assert.equal(boardLayout(399).h, BOARD_H_PHONE);
  assert.equal(boardLayout(400).h, BOARD_H_WIDE);
  assert.equal(boardLayout(520).h, BOARD_H_WIDE);
  assert.ok(BOARD_H_PHONE >= 180 && BOARD_H_WIDE <= 220);
});

test('boardLayout: the points never move with the labels (places stay where they are with fewer rows)', () => {
  const a = boardLayout(300), b = boardLayout(300, LONG.en);
  assert.deepEqual(a.places.map((p) => [p.rank, p.x, p.y]), b.places.map((p) => [p.rank, p.x, p.y]));
  assert.deepEqual(a.curve, b.curve);
});

test('boardLayout: tap targets are >= 44 x 44, never overlap and stay on the board, at every width and label length', () => {
  for (const labels of Object.values(LONG)) {
    for (const w of WIDTHS) {
      const lay = boardLayout(w, labels);
      for (const p of lay.places) {
        assert.ok(p.target.w >= 44 && p.target.h >= 44, `${w}: place ${p.rank} ${p.target.w} x ${p.target.h}`);
        assert.ok(within(p.target, lay.w, lay.h), `${w}: target ${p.rank} on the board`);
        // The target covers the dot
        assert.ok(p.x - p.r >= p.target.x && p.x + p.r <= p.target.x + p.target.w && p.y > p.target.y && p.y < p.target.y + p.target.h);
      }
      for (const p of lay.places) {
        for (const q of lay.places) if (p.rank < q.rank) assert.ok(!hit(p.target, q.target), `${w}: targets ${p.rank} / ${q.rank}`);
      }
    }
  }
});

test('boardLayout: labels and numbers stay on the board, apart from each other, the dots and the chalk line', () => {
  for (const [name, labels] of Object.entries(LONG)) {
    for (const w of WIDTHS) {
      const lay = boardLayout(w, labels);
      const boxes: [string, Box2][] = [];
      for (const p of lay.places) {
        assert.equal(p.labelBox.w, labelWidth(labels[p.rank]!, p.labelPx));
        assert.ok(LABEL_SIZES.includes(p.labelPx));
        boxes.push([`label ${p.rank}`, p.labelBox], [`number ${p.rank}`, numBox(p)]);
      }
      for (const [n, b] of boxes) assert.ok(within(b, lay.w, lay.h), `${name} ${w}: ${n} on the board`);
      for (let i = 0; i < boxes.length; i++) {
        for (let j = i + 1; j < boxes.length; j++) {
          assert.ok(!hit(boxes[i]![1], boxes[j]![1]), `${name} ${w}: ${boxes[i]![0]} / ${boxes[j]![0]}`);
        }
      }
      // No text on any dot (the dot plus the ring drawn around a picked one, radius r + 8)
      for (const p of lay.places) {
        const ring = { x: p.x - p.r - 8, y: p.y - p.r - 8, w: 2 * (p.r + 8), h: 2 * (p.r + 8) };
        for (const [n, b] of boxes) assert.ok(!hit(ring, b), `${name} ${w}: ${n} on the ring of ${p.rank}`);
      }
      // The chalk line keeps 3 px clear of every place number
      for (let i = 0; i + 1 < lay.curve.length; i++) {
        const a = lay.curve[i]!, b = lay.curve[i + 1]!;
        for (let k = 0; k <= 10; k++) {
          const x = a.x + ((b.x - a.x) * k) / 10, y = a.y + ((b.y - a.y) * k) / 10;
          for (const p of lay.places) {
            const nb = numBox(p);
            assert.ok(!(x > nb.x - 3 && x < nb.x + nb.w + 3 && y > nb.y - 3 && y < nb.y + nb.h + 3), `${name} ${w}: curve near number ${p.rank}`);
          }
        }
      }
      // The chalk line never runs through a label (sampled densely along the curve)
      for (let i = 0; i + 1 < lay.curve.length; i++) {
        const a = lay.curve[i]!, b = lay.curve[i + 1]!;
        for (let k = 0; k <= 10; k++) {
          const x = a.x + ((b.x - a.x) * k) / 10, y = a.y + ((b.y - a.y) * k) / 10;
          for (const p of lay.places) {
            const lb = p.labelBox;
            assert.ok(!(x > lb.x && x < lb.x + lb.w && y > lb.y && y < lb.y + lb.h), `${name} ${w}: curve through label ${p.rank}`);
          }
        }
      }
    }
  }
});

test('labelWidth: CJK glyphs count a full em, Latin letters and digits a little more than half', () => {
  assert.equal(labelWidth('', 12), 0);
  assert.ok(labelWidth('错 3 次', 12) > labelWidth('3 回', 12));
  assert.ok(labelWidth('12 misses', 12) > labelWidth('1 miss', 12));
  assert.equal(labelWidth('回', 12), 12);
});
