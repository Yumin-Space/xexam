// v2.6 preview (owner): hand-drawn doodles for blackboard mode. A paper plane on a looping flight line for the Home's
// "Exam & practice" tile, drawn with the same seeded sketch strokes as the boards (two slightly offset strokes per line), in
// the tile's text colour. Decoration only: aria-hidden, no pointer events, shown by the stylesheet only in blackboard mode.
import { hachure, rng, sketchCircle, sketchCurve, sketchLine, type Pt } from '../domain/sketch.ts';
import { strokes, svg } from './chalk-board.ts';

const W = 220;
const H = 68;

/** The paper plane with its flight line (three loops, then a stroke that meets the plane at its underside) */
export function planeDoodle(): SVGSVGElement {
  const box = { w: W, h: H };
  // The flight line: low at the left, rising steadily and faster towards the plane (the loop centres follow about
  // y = 58.5 - 25 (x / 188)^1.7, so the whole line climbs with the plane and ends at the plane's own angle), three loops
  // of growing size on the way
  const trail: Pt[] = [{ x: 3, y: 66 }];
  const loops: Array<[number, number, number]> = [[30, 57.4, 9], [74, 53.5, 10], [120, 46.8, 11]];
  for (const [cx, cy, r] of loops) {
    trail.push({ x: cx - r * 1.6, y: cy + r * 0.9 }, { x: cx + r * 0.2, y: cy + r }, { x: cx + r, y: cy }, { x: cx, y: cy - r },
      { x: cx - r, y: cy - r * 0.1 }, { x: cx - r * 0.3, y: cy + r * 0.8 }, { x: cx + r * 1.4, y: cy + r * 0.4 });
  }
  // the line climbs all the way: after the last loop it still rises and meets the plane low on its keel, under the far
  // wing (the plane stands a little above the line, so its folded shape stays clear and it does not read as an arrow)
  trail.push({ x: 150, y: 41.5 }, { x: 165, y: 38.4 }, { x: 177, y: 35.6 }, { x: 186.4, y: 33.6 });
  // A folded paper plane seen from the side and a little above, nose top right: the near wing on top (nose, wing tip,
  // fold), the far wing peeking out behind it, and the keel hanging under the fold; the keel is hatched as the shaded
  // side, which is what makes it read as folded paper rather than a flat triangle
  const nose = { x: 216, y: 5 };
  const wingTip = { x: 166, y: 15 };       // the near wing's back corner
  const fold = { x: 184, y: 25 };          // where the wings meet the keel
  const farTip = { x: 176, y: 31 };        // the far wing, behind
  const keel = { x: 190, y: 47 };          // the keel's lowest corner
  const paths = [
    ...strokes(sketchCurve(trail, rng(41), box, { jitter: 2.2 }), 'dash-doodle-line'),
    // the shaded keel first (under the outlines)
    svg('path', { d: hachure([nose, fold, keel], 2.6, 62, rng(47), box), class: 'dash-doodle-shade' }),
    // near wing
    ...strokes(sketchLine(wingTip, nose, rng(42), box), 'dash-doodle-line'),
    ...strokes(sketchLine(wingTip, fold, rng(43), box), 'dash-doodle-line'),
    ...strokes(sketchLine(fold, nose, rng(44), box), 'dash-doodle-line'),
    // far wing
    ...strokes(sketchLine(fold, farTip, rng(48), box), 'dash-doodle-line'),
    ...strokes(sketchLine(farTip, { x: 199, y: 20 }, rng(49), box), 'dash-doodle-line'),
    // keel
    ...strokes(sketchLine(fold, keel, rng(45), box), 'dash-doodle-line'),
    ...strokes(sketchLine(keel, nose, rng(46), box), 'dash-doodle-line'),
  ];
  return svg('svg', { class: 'dash-doodle', viewBox: `0 0 ${W} ${H}`, 'aria-hidden': 'true', focusable: 'false' }, ...paths);
}

const ICON = { w: 32, h: 24 };

/** A hand-drawn ruler (blackboard mode, on Start exam): a slanted bar with tick marks */
export function rulerDoodle(): SVGSVGElement {
  const box = ICON;
  // the bar, tilted a little (top-left to bottom-right corners)
  const a = { x: 2, y: 9 }, b = { x: 29, y: 3 }, c = { x: 31, y: 12 }, d = { x: 4, y: 18 };
  const paths = [
    ...strokes(sketchLine(a, b, rng(61), box), 'dash-doodle-line'),
    ...strokes(sketchLine(b, c, rng(62), box), 'dash-doodle-line'),
    ...strokes(sketchLine(c, d, rng(63), box), 'dash-doodle-line'),
    ...strokes(sketchLine(d, a, rng(64), box), 'dash-doodle-line'),
  ];
  // ticks along the top edge: every other one longer
  for (let i = 1; i <= 7; i++) {
    const tt = i / 8;
    const p = { x: a.x + (b.x - a.x) * tt, y: a.y + (b.y - a.y) * tt };
    const len = i % 2 ? 3.2 : 5.5;
    paths.push(svg('path', { d: `M${p.x.toFixed(1)} ${p.y.toFixed(1)} l${(0.45 * len).toFixed(1)} ${len.toFixed(1)}`, class: 'dash-doodle-tick' }));
  }
  return svg('svg', { class: 'hand-ico', viewBox: `0 0 ${box.w} ${box.h}`, 'aria-hidden': 'true', focusable: 'false' }, ...paths);
}

/** A hand-drawn pencil (blackboard mode, on Start practice): body, sharpened tip with its lead, and the eraser end */
export function pencilDoodle(): SVGSVGElement {
  const box = ICON;
  // the pencil lies slanted, tip at the bottom left
  const tip = { x: 3, y: 21 };
  const t1 = { x: 9, y: 13 }, t2 = { x: 12, y: 17 };       // where the wood is sharpened
  const e1 = { x: 25, y: 2 }, e2 = { x: 29, y: 7 };        // the far end
  const r1 = { x: 22, y: 5 }, r2 = { x: 26, y: 10 };       // the eraser band
  const paths = [
    ...strokes(sketchLine(t1, e1, rng(71), box), 'dash-doodle-line'),
    ...strokes(sketchLine(t2, e2, rng(72), box), 'dash-doodle-line'),
    ...strokes(sketchLine(e1, e2, rng(73), box), 'dash-doodle-line'),
    ...strokes(sketchLine(r1, r2, rng(74), box), 'dash-doodle-line'),
    ...strokes(sketchLine(t1, tip, rng(75), box), 'dash-doodle-line'),
    ...strokes(sketchLine(t2, tip, rng(76), box), 'dash-doodle-line'),
    ...strokes(sketchLine(t1, t2, rng(77), box), 'dash-doodle-line'),
    // the lead: a short hatched point
    svg('path', { d: hachure([tip, { x: 5.5, y: 17.4 }, { x: 7, y: 19.5 }], 1.2, 40, rng(78), box), class: 'dash-doodle-shade' }),
  ];
  return svg('svg', { class: 'hand-ico', viewBox: `0 0 ${box.w} ${box.h}`, 'aria-hidden': 'true', focusable: 'false' }, ...paths);
}

/**
 * The AI Chat welcome picture in blackboard mode (owner): a careful pencil sketch — a stack of three closed books with an open
 * book lying in front of it, a small globe on the left, a pencil and a ruler on the right, and the name written large under
 * it. Exact outlines (`sk-main`) over faint construction strokes (`sk-faint`) and fine hatching for the shade (`sk-hatch`).
 */
export function aiWelcomeDoodle(): SVGSVGElement {
  const box = { w: 300, h: 176 };
  const faint: SVGElement[] = [];
  const main: SVGElement[] = [];
  const shade: SVGElement[] = [];
  let seed = 200;
  const P = (pts: Pt[], close = false): string => 'M' + pts.map((q) => `${q.x.toFixed(1)} ${q.y.toFixed(1)}`).join(' L') + (close ? ' Z' : '');
  /** One exact outline plus its faint construction stroke */
  const line = (pts: Pt[], close = false): void => {
    main.push(svg('path', { d: P(pts, close), class: 'sk-main' }));
    // the construction strokes follow each straight edge on its own (a smooth curve through corners would loop)
    const ring = close ? [...pts, pts[0]!] : pts;
    for (let i = 0; i + 1 < ring.length; i++) faint.push(...strokes(sketchLine(ring[i]!, ring[i + 1]!, rng(seed++), box), 'sk-faint'));
  };
  const curve = (d: string, approx: Pt[]): void => {
    main.push(svg('path', { d, class: 'sk-main' }));
    faint.push(...strokes(sketchCurve(approx, rng(seed++), box, { jitter: 0.9 }), 'sk-faint'));
  };
  const hatch = (poly: Pt[], gap: number, angle: number): void => {
    shade.push(svg('path', { d: hachure(poly, gap, angle, rng(seed++), box), class: 'sk-hatch' }));
  };

  // ---- the stack of closed books (bottom to top), drawn as boxes seen a little from above and from the right: the cover on
  // top, the spine at the front with two bands, the page block on the right side with its lines; each book a little
  // narrower, shifted and turned, so the stack reads as books at a glance
  const stack: SVGElement[] = [];
  const book = (cx: number, cy: number, w: number, h: number, tilt: number): void => {
    const keepF = faint.length, keepM = main.length, keepS = shade.length;
    const a = (tilt * Math.PI) / 180;
    const R = (x: number, y: number): Pt => ({ x: cx + x * Math.cos(a) - y * Math.sin(a), y: cy + x * Math.sin(a) + y * Math.cos(a) });
    const dx = 13, dy = -9;                                  // the depth: up and to the right
    const x0 = -w / 2, x1 = w / 2, y0 = -h / 2, y1 = h / 2;
    // the spine (front face) and its bands
    line([R(x0, y0), R(x1, y0), R(x1, y1), R(x0, y1)], true);
    line([R(x0 + 9, y0), R(x0 + 9, y1)]);
    line([R(x1 - 9, y0), R(x1 - 9, y1)]);
    // the cover (top face)
    line([R(x0, y0), R(x0 + dx, y0 + dy), R(x1 + dx, y0 + dy), R(x1, y0)]);
    // the page block (right face): its outline and the paper lines
    line([R(x1 + dx, y0 + dy), R(x1 + dx, y1 + dy), R(x1, y1)]);
    for (let t = 0.25; t < 0.95; t += 0.18) {
      main.push(svg('path', { d: P([R(x1 + dx * t, y0 + dy * t + 2), R(x1 + dx * t, y1 + dy * t - 1.5)]), class: 'sk-thin' }));
    }
    // shade: the spine is the side away from the light
    hatch([R(x0, y0), R(x1, y0), R(x1, y1), R(x0, y1)], 2.4, 60);
    // the whole silhouette filled with the background, then this book's own strokes
    const face = svg('path', { d: P([R(x0, y1), R(x0, y0), R(x0 + dx, y0 + dy), R(x1 + dx, y0 + dy), R(x1 + dx, y1 + dy), R(x1, y1)], true), class: 'sk-face' });
    stack.push(svg('g', {}, face, ...shade.splice(keepS), ...faint.splice(keepF), ...main.splice(keepM)));
  };
  book(148, 106, 86, 15, 1.5);
  book(150, 91, 78, 14, -2.5);
  book(147, 77, 72, 13, 2);
  // the stack's cast shadow to the lower right
  hatch([{ x: 192, y: 112 }, { x: 204, y: 104 }, { x: 212, y: 106 }, { x: 202, y: 115 }], 2, 45);

  // ---- loose scrap paper on both sides of the name: tilted sheets with a folded corner and a few lines of scribble (one
  // with a little sum), each filled with the background so a sheet on top hides the one under it
  const papers: SVGElement[] = [];
  const paper = (cx: number, cy: number, w: number, h: number, tilt: number, kind: 'lines' | 'sum' | 'tick', curl: 1 | -1): void => {
    const keepF = faint.length, keepM = main.length;
    const a = (tilt * Math.PI) / 180;
    const R = (x: number, y: number): Pt => ({ x: cx + x * Math.cos(a) - y * Math.sin(a), y: cy + x * Math.sin(a) + y * Math.cos(a) });
    const x0 = -w / 2, x1 = w / 2, y0 = -h / 2, y1 = h / 2;
    const rnd = rng(seed++);
    const wob = (amp: number): number => (rnd() * 2 - 1) * amp;
    // A crumpled, worn sheet like old parchment: every edge waves a little, and one bottom corner rolls up into a small
    // curl (on the right for curl = 1, on the left for -1)
    const edge = (from: Pt, to: Pt, n: number, amp: number): Pt[] =>
      Array.from({ length: n }, (_, i) => {
        const t = i / n;
        const nx = -(to.y - from.y), ny = to.x - from.x, len = Math.hypot(nx, ny) || 1;
        const off = Math.sin(t * Math.PI * 2 + rnd()) * amp + wob(amp * 0.5);
        return { x: from.x + (to.x - from.x) * t + (nx / len) * off, y: from.y + (to.y - from.y) * t + (ny / len) * off };
      });
    const cx1 = curl > 0 ? x1 : x0;                               // the curled corner's side
    const local: Pt[] = curl > 0
      ? [...edge({ x: x0, y: y0 }, { x: x1, y: y0 }, 4, 1.2), ...edge({ x: x1, y: y0 }, { x: x1, y: y1 - 8 }, 4, 1.1),
         // the roll: the corner lifts and turns back on itself
         { x: x1 + 1, y: y1 - 4 }, { x: x1 - 2, y: y1 + 1 }, { x: x1 - 7, y: y1 - 1 }, { x: x1 - 6, y: y1 - 5 }, { x: x1 - 2.5, y: y1 - 3.5 },
         { x: x1 - 5, y: y1 }, ...edge({ x: x1 - 10, y: y1 + 1 }, { x: x0, y: y1 }, 4, 1.2), ...edge({ x: x0, y: y1 }, { x: x0, y: y0 }, 4, 1.1)]
      : [...edge({ x: x0, y: y0 }, { x: x1, y: y0 }, 4, 1.2), ...edge({ x: x1, y: y0 }, { x: x1, y: y1 }, 4, 1.1),
         ...edge({ x: x1, y: y1 }, { x: x0 + 10, y: y1 + 1 }, 4, 1.2),
         { x: x0 + 5, y: y1 }, { x: x0 + 2.5, y: y1 - 3.5 }, { x: x0 + 6, y: y1 - 5 }, { x: x0 + 7, y: y1 - 1 }, { x: x0 + 2, y: y1 + 1 }, { x: x0 - 1, y: y1 - 4 },
         ...edge({ x: x0, y: y1 - 8 }, { x: x0, y: y0 }, 4, 1.1)];
    const pts = local.map((q) => R(q.x, q.y));
    const ring = [...pts, pts[0]!, pts[1]!];
    const outlineD = sketchCurve(ring, rng(seed++), box, { jitter: 0 })[0];
    main.push(svg('path', { d: outlineD, class: 'sk-main' }));
    faint.push(...strokes(sketchCurve(ring, rng(seed++), box, { jitter: 1 }), 'sk-faint'));
    // creases where it was crumpled: two faint folds across the sheet
    main.push(svg('path', { d: sketchCurve([R(x0 + 2, y0 + h * 0.35), R(0, y0 + h * 0.42), R(x1 - 2, y0 + h * 0.3)], rng(seed++), box, { jitter: 0.4 })[0], class: 'sk-crease' }));
    main.push(svg('path', { d: sketchCurve([R(x0 + w * 0.3, y0 + 2), R(x0 + w * 0.42, 0), R(x0 + w * 0.36, y1 - 3)], rng(seed++), box, { jitter: 0.4 })[0], class: 'sk-crease' }));
    // the writing: wavy hand lines (and a little sum or a tick)
    const scrib = (y: number, xa: number, xb: number): void => {
      main.push(svg('path', { d: sketchCurve([R(xa, y), R((xa * 2 + xb) / 3, y - 1.2), R((xa + xb * 2) / 3, y + 1), R(xb, y - 0.5)], rng(seed++), box, { jitter: 0.3 })[0], class: 'sk-thin' }));
    };
    if (kind === 'sum') {
      const s0 = x0 + 5, yy = y0 + 10;
      const thin = (q: Pt[]): void => { main.push(svg('path', { d: P(q), class: 'sk-thin' })); };
      thin([R(s0, yy - 4), R(s0, yy + 4)]);
      thin([R(s0 + 5, yy), R(s0 + 11, yy)]); thin([R(s0 + 8, yy - 3), R(s0 + 8, yy + 3)]);
      main.push(svg('path', { d: sketchCurve([R(s0 + 14, yy - 3), R(s0 + 18, yy - 4.5), R(s0 + 18, yy - 0.5), R(s0 + 14, yy + 4), R(s0 + 19, yy + 4)], rng(seed++), box, { jitter: 0 })[0], class: 'sk-thin' }));
      scrib(y0 + 20, x0 + 5, x1 - 6);
      scrib(y0 + 26, x0 + 5, x1 - 11);
    } else {
      for (let i = 0; i < 4; i++) scrib(y0 + 9 + i * 6, x0 + 5, x1 - 5 - (i % 2) * 7);
      if (kind === 'tick') main.push(svg('path', { d: P([R(x1 - 15, y1 - 12), R(x1 - 11, y1 - 8), R(x1 - 4, y1 - 16)]), class: 'sk-thin' }));
    }
    void cx1;
    const face = svg('path', { d: outlineD + ' Z', class: 'sk-face' });
    papers.push(svg('g', {}, face, ...faint.splice(keepF), ...main.splice(keepM)));
  };
  // owner: only the outer sheet on the right stays (the others crowded the globe and the name)
  paper(258, 146, 28, 34, -9, 'lines', 1);

  // ---- the open book lying in front of the stack, turned a little: two pages curving up from the gutter, layered page
  // edges under each page, the cover showing under them, lines of text on the pages and the gutter in shade
  const ob = { faint: [] as SVGElement[], main: [] as SVGElement[], shade: [] as SVGElement[] };
  {
    const keepF = faint.length, keepM = main.length, keepS = shade.length;
    const gx = 152, top = 47, bot = 65;
    curve(`M${gx} ${top + 3} Q${gx - 22} ${top - 7} ${gx - 52} ${top}`, [{ x: gx, y: top + 3 }, { x: gx - 26, y: top - 3 }, { x: gx - 52, y: top }]);
    curve(`M${gx} ${top + 3} Q${gx + 22} ${top - 7} ${gx + 52} ${top}`, [{ x: gx, y: top + 3 }, { x: gx + 26, y: top - 3 }, { x: gx + 52, y: top }]);
    line([{ x: gx - 52, y: top }, { x: gx - 49, y: bot - 4 }]);
    line([{ x: gx + 52, y: top }, { x: gx + 49, y: bot - 4 }]);
    curve(`M${gx - 49} ${bot - 4} Q${gx - 24} ${bot - 7} ${gx} ${bot}`, [{ x: gx - 49, y: bot - 4 }, { x: gx - 25, y: bot - 5 }, { x: gx, y: bot }]);
    curve(`M${gx + 49} ${bot - 4} Q${gx + 24} ${bot - 7} ${gx} ${bot}`, [{ x: gx + 49, y: bot - 4 }, { x: gx + 25, y: bot - 5 }, { x: gx, y: bot }]);
    line([{ x: gx, y: top + 3 }, { x: gx, y: bot }]);
    // the layered page edges and the cover under each page
    for (let i = 1; i <= 3; i++) {
      const o = i * 1.6;
      main.push(svg('path', { d: `M${gx - 49 - i * 0.6} ${(bot - 4 + o).toFixed(1)} Q${gx - 24} ${(bot - 7 + o).toFixed(1)} ${gx} ${(bot + o).toFixed(1)} Q${gx + 24} ${(bot - 7 + o).toFixed(1)} ${gx + 49 + i * 0.6} ${(bot - 4 + o).toFixed(1)}`, class: i === 3 ? 'sk-main' : 'sk-thin' }));
    }
    for (let i = 0; i < 4; i++) {
      const y = top + 4.5 + i * 3.3;
      main.push(svg('path', { d: `M${gx - 44} ${(y - 1).toFixed(1)} Q${gx - 24} ${(y - 2.6).toFixed(1)} ${gx - 7} ${(y + 1.2).toFixed(1)}`, class: 'sk-thin' }));
      main.push(svg('path', { d: `M${gx + 7} ${(y + 1.2).toFixed(1)} Q${gx + 24} ${(y - 2.6).toFixed(1)} ${gx + 44} ${(y - 1).toFixed(1)}`, class: 'sk-thin' }));
    }
    hatch([{ x: gx - 8, y: top + 1 }, { x: gx, y: top + 3 }, { x: gx, y: bot }, { x: gx - 7, y: bot - 1.5 }], 1.4, 80);
    hatch([{ x: gx, y: top + 3 }, { x: gx + 8, y: top + 1 }, { x: gx + 7, y: bot - 1.5 }, { x: gx, y: bot }], 1.4, 100);
    ob.faint = faint.splice(keepF);
    ob.main = main.splice(keepM);
    ob.shade = shade.splice(keepS);
    // its silhouette filled with the background, so it hides the top of the book under it
    ob.main.unshift(svg('path', { d: `M${gx - 52} ${top} Q${gx - 22} ${top - 7} ${gx} ${top + 3} Q${gx + 22} ${top - 7} ${gx + 52} ${top} L${gx + 51} ${bot + 1} Q${gx + 24} ${bot - 2} ${gx} ${bot + 5} Q${gx - 24} ${bot - 2} ${gx - 51} ${bot + 1} Z`, class: 'sk-face' }));
  }
  // drawn after the stack, turned a little, as if just put down on the top book
  const openBook = svg('g', { transform: 'rotate(-4 152 57)' }, ob.main[0]!, ...ob.shade, ...ob.faint, ...ob.main.slice(1));

  // ---- the globe on the left (higher and larger, clear of the papers): the ball, equator and meridian, the half ring of
  // its stand and the foot; every size follows the radius
  const g = { x: 46, y: 70 }, gr = 23, q = gr / 17;
  main.push(svg('circle', { cx: g.x, cy: g.y, r: gr, class: 'sk-main' }), svg('ellipse', { cx: g.x, cy: g.y + 0.5, rx: gr, ry: 4 * q, class: 'sk-main' }),
    svg('ellipse', { cx: g.x, cy: g.y, rx: 7.5 * q, ry: gr, class: 'sk-main' }));
  faint.push(...strokes(sketchCircle(g, gr, rng(seed++), box), 'sk-faint'));
  curve(`M${g.x - 17 * q} ${g.y - 12 * q} A ${22 * q} ${22 * q} 0 0 0 ${g.x + 2 * q} ${g.y + 23 * q}`,
    [{ x: g.x - 17 * q, y: g.y - 12 * q }, { x: g.x - 21 * q, y: g.y + 6 * q }, { x: g.x + 2 * q, y: g.y + 23 * q }]);
  line([{ x: g.x, y: g.y + 23 * q }, { x: g.x, y: g.y + 31 * q }]);
  line([{ x: g.x - 11 * q, y: g.y + 32 * q }, { x: g.x + 11 * q, y: g.y + 32 * q }]);
  hatch(crescent(g, gr, 6 * q), 1.8, 50);
  hatch(ellipsePoly({ x: g.x + 3 * q, y: g.y + 34 * q }, 15 * q, 2.4 * q), 1.8, 8);

  // ---- the ruler (slanted, ticks on its top edge) and the pencil leaning over it, on the right
  const v = { x: 0.973, y: -0.233 }, m = { x: 0.233, y: 0.973 };
  const A = { x: 222, y: 116 };
  const B = { x: A.x + v.x * 66, y: A.y + v.y * 66 };
  const C = { x: B.x + m.x * 9, y: B.y + m.y * 9 };
  const D = { x: A.x + m.x * 9, y: A.y + m.y * 9 };
  line([A, B, C, D], true);
  for (let i = 1; i <= 7; i++) {
    const p = { x: A.x + v.x * 8.2 * i, y: A.y + v.y * 8.2 * i };
    const len = i % 2 ? 2.5 : 4.2;
    main.push(svg('path', { d: `M${p.x.toFixed(1)} ${p.y.toFixed(1)} l${(m.x * len).toFixed(1)} ${(m.y * len).toFixed(1)}`, class: 'sk-thin' }));
  }
  hatch([D, C, { x: C.x + 2, y: C.y + 3 }, { x: D.x + 2, y: D.y + 3 }], 1.5, 60);
  const u = { x: 0.622, y: -0.783 }, n = { x: 0.783, y: 0.622 };
  const T = { x: 232, y: 104 };
  const at = (s: number, o: number): Pt => ({ x: T.x + u.x * s + n.x * o, y: T.y + u.y * s + n.y * o });
  line([T, at(12, -3.8), at(68, -3.8), at(68, 3.8), at(12, 3.8)], true);
  line([at(12, -3.8), at(12, 3.8)]);
  line([at(60, -3.8), at(60, 3.8)]);
  hatch([T, at(5, -1.5), at(5, 1.5)], 1, 30);
  hatch([at(12, 0.4), at(60, 0.4), at(60, 3.8), at(12, 3.8)], 1.4, 75);

  // ---- the name, written large under the books
  const name = svg('text', { x: 150, y: 160, 'text-anchor': 'middle', class: 'ai-welcome-name' }, document.createTextNode('Xexam'));
  return svg('svg', { class: 'ai-welcome-art', viewBox: `0 0 ${box.w} ${box.h}`, 'aria-hidden': 'true', focusable: 'false' },
    ...shade, ...faint, ...main, ...stack, openBook, ...papers, name);
}

/** A crescent on the lower right of a circle: the ball minus the same ball moved `shift` up and to the left */
function crescent(c: Pt, r: number, shift: number): Pt[] {
  const out: Pt[] = [];
  for (let i = 0; i <= 12; i++) {
    const a = (-60 + (210 * i) / 12) * Math.PI / 180;
    out.push({ x: c.x + Math.cos(a) * r, y: c.y + Math.sin(a) * r });
  }
  const d = shift / Math.SQRT2;
  for (let i = 12; i >= 0; i--) {
    const a = (-60 + (210 * i) / 12) * Math.PI / 180;
    out.push({ x: c.x - d + Math.cos(a) * r, y: c.y - d + Math.sin(a) * r });
  }
  return out;
}

/** A flat ellipse as a polygon (for a cast shadow) */
function ellipsePoly(c: Pt, rx: number, ry: number): Pt[] {
  return Array.from({ length: 16 }, (_, i) => ({ x: c.x + Math.cos((i / 16) * Math.PI * 2) * rx, y: c.y + Math.sin((i / 16) * Math.PI * 2) * ry }));
}
