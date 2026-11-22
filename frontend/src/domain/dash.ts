// v2.5 / v2.6: pure helpers of the Home dashboard.
import type { WrongItem } from '../api/dashboard.ts';
import type { Question } from '../types/api.ts';
import { shown, type Lang } from './language.ts';
import { shownName, type NamedUser } from './user-name.ts';

/**
 * The name the greeting uses: the display name when the user has one; else (not set yet) the old rule, the part of the login
 * before the first "@" (a plain username is kept as it is)
 */
export function greetName(user: NamedUser): string {
  const chosen = (user.displayName ?? '').trim();
  if (chosen) return shownName(user);
  const name = String(user.username ?? '').trim();
  const at = name.indexOf('@');
  if (at < 0) return name;
  return name.slice(0, at).trim() || name;
}

/** How many places the most-missed board has (the server sends at most this many) */
export const PODIUM_PLACES = 5;

/** Left-to-right order of the places: place 1 in the middle (the vertex), the next two beside it, 4 and 5 at the edges */
export const PODIUM_ORDER: readonly number[] = [4, 2, 1, 3, 5];

export interface WrongRow {
  /** 1-based place after unknown ids were skipped */
  rank: number;
  id: string;
  bankId: number;
  sheet: string;
  wrong: number;
  attempts: number;
  /** The full stem in the wanted language, whitespace kept (the page shows it with line breaks) */
  stem: string;
  /** The language that stem and answer are actually in (the wanted one, or the question's own when it has no translation) */
  lang: Lang;
  /** The correct answer in the wanted language, one line each: "B. text" per correct option, "item → choice" per hotspot item */
  answer: string[];
}

/** What the board needs from the loaded bank */
export interface QuestionSource {
  questions: ReadonlyArray<Question>;
}

/**
 * Resolves the server's ids against the loaded bank; an id the client does not know (not loaded, retired) is skipped and
 * the board shrinks (places are numbered again). The stem comes through `shown()`, so it is in `lang` when the question
 * has it and in its own base language otherwise.
 */
export function wrongRows(items: readonly WrongItem[], bank: QuestionSource | null | undefined, lang: Lang): WrongRow[] {
  if (!bank) return [];
  const byId = new Map(bank.questions.map((q) => [q.id, q]));
  const rows: WrongRow[] = [];
  for (const it of items) {
    if (rows.length >= PODIUM_PLACES) break;
    const q = byId.get(it.questionId);
    if (!q) continue;
    const v = shown(q, lang);
    rows.push({
      rank: rows.length + 1, id: q.id, bankId: it.bankId, sheet: q.sheet, wrong: it.wrong, attempts: it.attempts,
      stem: v.stem.replace(/\r\n?/g, '\n').trim(),
      lang: v.lang,
      answer: answerLines(q, lang),
    });
  }
  return rows;
}

/** The rows in left-to-right order (4, 2, 1, 3, 5), also the tab order of the points; missing places are left out */
export function podiumOrder<T extends { rank: number }>(rows: readonly T[]): T[] {
  const byRank = new Map(rows.map((r) => [r.rank, r]));
  const out: T[] = [];
  for (const rank of PODIUM_ORDER) {
    const r = byRank.get(rank);
    if (r) out.push(r);
  }
  return out;
}

// ---------------------------------------------------------------- the chalk parabola (v2.6, replaces the podium blocks)

/** Height of the board: phones / wider boards (px) */
export const BOARD_H_PHONE = 200;
export const BOARD_H_WIDE = 220;
/** A board this wide or wider uses the taller height */
export const BOARD_WIDE_FROM = 400;
/** The tap target over each point (px): at least 44 wide, tall enough to cover the number above and the label below */
export const TARGET_W = 44;
export const TARGET_UP = 30;
export const TARGET_DOWN = 32;
/** Font sizes of the chalk texts (px): the place number (place 1 a little bigger) and the misses label */
export const NUM_PX = 14;
export const NUM1_PX = 16;
export const LABEL_PX = 12;
/** Font sizes a long misses label may step down to before it moves under its dot */
export const LABEL_SIZES: readonly number[] = [12, 11, 10];

export interface Box2 {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface CurvePlace {
  rank: number;
  /** The point on the parabola */
  x: number;
  y: number;
  /** Dot radius (place 1 a little bigger) */
  r: number;
  /** Baseline of the place number above the dot, centred on numX (the edge places lean outward, off the rising arm) */
  numY: number;
  numX: number;
  /** Anchor point, text-anchor and baseline of the misses label */
  labelX: number;
  labelY: number;
  anchor: 'start' | 'middle' | 'end';
  /** Font size of the label (a long label beside a dot may be a step smaller) */
  labelPx: number;
  /** The label's estimated box (for the layout rules and tests) */
  labelBox: Box2;
  /** The tap target, board coordinates */
  target: Box2;
}

export interface BoardLayout {
  w: number;
  h: number;
  /** The parabola itself (sampled), drawn whether or not every place exists */
  curve: { x: number; y: number }[];
  /** One entry per place 1..5 (the page draws only the existing ones; positions never move) */
  places: CurvePlace[];
}

/**
 * A rough width (px) of a chalk label in the system font: CJK glyphs are one em, digits and Latin letters a little more than
 * half, plus a safety margin. Used to keep labels on the board and apart; real text is never wider than this guess.
 */
export function labelWidth(text: string, px: number): number {
  let em = 0;
  for (const ch of text) {
    const c = ch.codePointAt(0) ?? 0;
    if (c >= 0x2e80) em += 1;
    else if (ch === ' ') em += 0.3;
    else if (/[mwMW]/.test(ch)) em += 0.85;
    else em += 0.58;
  }
  return Math.ceil(em * px);
}

const PARA_TOP = 36;
const PARA_BOTTOM_GAP = 38;
/** The curve runs a little past the outer points so it reads as a function, not a polyline */
const PARA_DX_MAX = 1.04;

/**
 * Geometry of the board for a width: a downward-opening parabola y = top + depth * u^2 with u in [-1, 1] across the five
 * slots; the slots stand left to right as 4, 2, 1, 3, 5 (place 1 at the vertex, 2 / 3 at u = -/+0.5, 4 / 5 at u = -/+1).
 * `labels[rank]` is the misses label of each place (for the width guess); missing labels count as empty.
 */
export function boardLayout(width: number, labels: Readonly<Record<number, string>> = {}): BoardLayout {
  const w = Math.max(200, Math.round(width));
  const h = w >= BOARD_WIDE_FROM ? BOARD_H_WIDE : BOARD_H_PHONE;
  const pad = Math.max(32, Math.round(w * 0.11));
  const cx = w / 2;
  const half = cx - pad;
  const depth = h - PARA_BOTTOM_GAP - PARA_TOP;
  const yAt = (u: number): number => PARA_TOP + depth * u * u;
  const curve: { x: number; y: number }[] = [];
  const steps = 24;
  for (let i = 0; i <= steps; i++) {
    const u = -PARA_DX_MAX + (2 * PARA_DX_MAX * i) / steps;
    curve.push({ x: cx + u * half, y: Math.min(h - 4, yAt(u)) });
  }
  // Where each label goes so the chalk line never runs through it:
  // - place 1: under the vertex, between the arms (centred, kept on the board, a step smaller when long);
  // - places 2 / 3: beside the dot on the outer side (the arm falls away below it there); a long label first gets a
  //   smaller font, and only if it still does not fit it hangs under the dot on the inner side;
  // - places 4 / 5: beside the dot on the inner side (the arm rises away above it there), never past the middle.
  const fit = (text: string, room: number): { px: number; lw: number } | null => {
    for (const px of LABEL_SIZES) {
      const lw = labelWidth(text, px);
      if (lw <= room) return { px, lw };
    }
    return null;
  };
  const places = PODIUM_ORDER.map((rank, slot): CurvePlace => {
    const u = (slot - 2) / 2;
    const side = Math.sign(u);                 // -1 left arm, 0 vertex, 1 right arm
    const x = cx + u * half, y = yAt(u);
    const r = rank === 1 ? 7 : 5;
    const text = labels[rank] ?? '';
    const gap = r + 12;
    let labelX: number, labelY: number, anchor: CurvePlace['anchor'];
    let px = LABEL_PX, lw = labelWidth(text, LABEL_PX);
    let beside = false;
    if (side === 0) {
      // Between the rings that places 2 and 3 get when picked (radius 5 + 8), a step smaller when long
      const f = fit(text, 2 * (half / 2 - 5 - 8 - 3));
      const last = LABEL_SIZES[LABEL_SIZES.length - 1]!;
      ({ px, lw } = f ?? { px: last, lw: labelWidth(text, last) });
      labelX = Math.min(w - lw / 2 - 4, Math.max(lw / 2 + 4, x));
      labelY = y + r + 23;
      anchor = 'middle';
    } else {
      const outer = Math.abs(u) < 1;
      // Room from the dot's side to the board edge (outer) or to just short of the middle (inner)
      const room = outer ? (side < 0 ? x - gap - 4 : w - 4 - x - gap) : (side < 0 ? cx - 3 - x - gap : x - gap - cx - 3);
      const f = fit(text, room);
      if (f) {
        ({ px, lw } = f);
        beside = true;
        labelX = x + (outer ? side : -side) * gap;
        labelY = y + 4;
        anchor = (outer ? side < 0 : side > 0) ? 'end' : 'start';
      } else {
        labelX = x - side * 2;
        labelY = y + r + 20;                   // clear of the ring a picked dot gets
        anchor = side < 0 ? 'start' : 'end';
      }
    }
    const left = anchor === 'start' ? labelX : anchor === 'end' ? labelX - lw : labelX - lw / 2;
    const labelBox = { x: left, y: labelY - px + 2, w: lw, h: px + 2 };
    // The tap target: 44 px around the dot, widened over a label beside it (places 2-5) so a tap on the words picks too,
    // but never past the middle of the board (the other side has its own targets)
    let tx0 = x - TARGET_W / 2, tx1 = x + TARGET_W / 2;
    if (beside) {
      tx0 = Math.min(tx0, labelBox.x - 2);
      tx1 = Math.max(tx1, labelBox.x + lw + 2);
      if (side < 0) tx1 = Math.min(tx1, Math.max(x + TARGET_W / 2, cx - 1));
      else tx0 = Math.max(tx0, Math.min(x - TARGET_W / 2, cx + 1));
    }
    tx0 = Math.max(0, tx0);
    tx1 = Math.min(w, tx1);
    return {
      rank, x, y, r, numY: y - r - 11, numX: Math.abs(u) === 1 ? x + side * 4 : x, labelX, labelY, anchor, labelPx: px, labelBox,
      target: { x: tx0, y: y - TARGET_UP, w: tx1 - tx0, h: TARGET_UP + TARGET_DOWN },
    };
  }).sort((a, b) => a.rank - b.rank);
  // Two labels hanging under places 2 and 3 could meet in the middle: the right one then hangs one line lower
  const [p2, p3] = [places[1]!, places[2]!];
  if (p2.labelBox.x + p2.labelBox.w > p3.labelBox.x && p2.labelY === p3.labelY && p2.anchor !== 'end' && p3.anchor !== 'start') {
    p3.labelY += LABEL_PX + 3;
    p3.labelBox = { ...p3.labelBox, y: p3.labelBox.y + LABEL_PX + 3 };
  }
  return { w, h, curve, places };
}

/** The pick after a click on a point: the same point again unpins it */
export const togglePin = (pinned: string | null, id: string): string | null => (pinned === id ? null : id);

/** The point whose question the detail region shows: the hovered / focused one while there is one, else the pinned one */
export const detailId = (hover: string | null, pinned: string | null): string | null => hover ?? pinned;

/**
 * The one line the leaderboard shows besides its rows: with nobody ranked only the invitation to answer (an empty install
 * would otherwise say "nobody has answered" and "you have not answered" one under the other); with others ranked the
 * invitation for a viewer who is not; else nothing. `empty` only for a board without rows that still ranks the viewer.
 */
export function rankNote(topCount: number, meRank: number | null): 'noneYou' | 'empty' | null {
  if (meRank === null) return 'noneYou';
  return topCount > 0 ? null : 'empty';
}

/** The correct answer of a question as text lines in the wanted language (see WrongRow.answer) */
export function answerLines(q: Question, lang: Lang): string[] {
  const v = shown(q, lang);
  if (q.kind === 'hotspot') {
    return (q.answer as number[]).map((j, i) => `${v.itemText(i).trim()} → ${v.choiceText(i, j).trim()}`);
  }
  const keys = new Set((q.answer as string[]).map(String));
  return q.options.flatMap((o, i) => (keys.has(o.key) ? [`${o.key}. ${v.optionText(i).trim()}`] : []));
}
