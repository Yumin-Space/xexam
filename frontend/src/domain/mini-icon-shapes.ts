// The shapes of the small 16×16 line icons (v0.8, extended in v1.0 to the inventory of spec 4.2). Pure data, no DOM: the
// painting lives in components/mini-icons.ts, the checks in mini-icon-shapes.test.ts. Style: stroke 1.5, round caps and
// joins, currentColor; a dot is a zero-length stroke ("M8 8h.01").
export type Shape = { d: string } | { cx: number; cy: number; r: number } | { x: number; y: number; w: number; h: number; rx: number };

export const MINI = {
  /** A key: "Manage banks" */
  key: [{ cx: 5.2, cy: 10.8, r: 2.7 }, { d: 'M7.2 8.8 13.5 2.5' }, { d: 'M11.2 4.8l1.8 1.8' }, { d: 'M9.6 6.4l1.4 1.4' }],
  /** Two ticks: "Select all" */
  checkAll: [{ d: 'M1.8 8.6l2.6 2.6L9.6 5' }, { d: 'M7.6 10.8l.6.6 6-6.6' }],
  /** A square with a minus: "Deselect all" */
  squareMinus: [{ x: 2.5, y: 2.5, w: 11, h: 11, rx: 2.5 }, { d: 'M5.5 8h5' }],
  /** A clock with a plus: "Include banks imported later" */
  clockPlus: [{ d: 'M13.4 7.2A5.5 5.5 0 1 0 8.6 13.5' }, { d: 'M8 5v3l1.8 1.2' }, { d: 'M12.5 10.5v4' }, { d: 'M10.5 12.5h4' }],
  /** An "i" in a circle: the explanation buttons and the administrator notice */
  info: [{ cx: 8, cy: 8, r: 6 }, { d: 'M8 7.3v3.7' }, { d: 'M8 5v.2' }],
  /** A four-point star and a small one: AI+ */
  sparkle: [{ d: 'M6.5 1.8c.4 2.7 1.6 3.9 4.3 4.3-2.7.4-3.9 1.6-4.3 4.3-.4-2.7-1.6-3.9-4.3-4.3 2.7-.4 3.9-1.6 4.3-4.3Z' }, { d: 'M12 10.2c.2 1.2.7 1.7 1.9 1.9-1.2.2-1.7.7-1.9 1.9-.2-1.2-.7-1.7-1.9-1.9 1.2-.2 1.7-.7 1.9-1.9Z' }],
  /** A speech bubble: a memory saved from a chat */
  chat: [{ d: 'M2.5 3.5h11v7.2H7.2L4.5 13v-2.3h-2z' }],
  /** A pencil: a memory added by hand, Edit */
  pencil: [{ d: 'M10.6 2.6l2.8 2.8-7.6 7.6H3v-2.8z' }, { d: 'M9.2 4l2.8 2.8' }],
  /** A bin: delete */
  trash: [{ d: 'M2.8 4.3h10.4' }, { d: 'M6.2 4.3V2.8h3.6v1.5' }, { d: 'M4.2 4.3l.6 8.9h6.4l.6-8.9' }],
  /** A plus: New chat, Add */
  plus: [{ d: 'M8 3v10' }, { d: 'M3 8h10' }],
  /** An arrow left: Back */
  back: [{ d: 'M9.8 3.5 5.3 8l4.5 4.5' }],
  /** An arrow down: Jump to latest */
  down: [{ d: 'M8 3v9.5' }, { d: 'M4 8.8l4 3.9 4-3.9' }],
  /** Two overlapping sheets: Copy */
  copy: [{ x: 5.5, y: 5.5, w: 8, h: 8, rx: 1.6 }, { d: 'M10.5 3.2V3a1.5 1.5 0 0 0-1.5-1.5H3A1.5 1.5 0 0 0 1.5 3v6A1.5 1.5 0 0 0 3 10.5h.2' }],
  /** A light bulb: AI hint */
  bulb: [{ d: 'M8 1.8a4.3 4.3 0 0 0-2.6 7.7c.5.4.8.9.8 1.5v.5h3.6V11c0-.6.3-1.1.8-1.5A4.3 4.3 0 0 0 8 1.8Z' }, { d: 'M6.3 13.1h3.4' }, { d: 'M6.9 14.6h2.2' }],
  /** An open book: AI explanation */
  book: [{ d: 'M8 4.3C6.6 3.2 4.6 2.8 2 2.9v9.5c2.6-.1 4.6.3 6 1.3 1.4-1 3.4-1.4 6-1.3V2.9c-2.6-.1-4.6.3-6 1.3Z' }, { d: 'M8 4.3v9.4' }],
  /** A chevron right: Next, "more on the next page" */
  fwd: [{ d: 'M6.2 3.5 10.7 8l-4.5 4.5' }],
  /** A chevron down (rotates when open): the AI+ panel, folded sections */
  chevDown: [{ d: 'M3.8 6l4.2 4.2L12.2 6' }],
  /** An arrow up: Back to top */
  up: [{ d: 'M8 13V3.5' }, { d: 'M4 7.2l4-3.9 4 3.9' }],
  /** A flag on a pole: Review later */
  flag: [{ d: 'M3.5 14V2.5' }, { d: 'M3.5 3h8l-1.7 2.7 1.7 2.8h-8' }],
  /** A door with an arrow out: Save and exit */
  exit: [{ d: 'M6.5 2.5h-3v11h3' }, { d: 'M6.5 8h7' }, { d: 'M11 5.5 13.5 8 11 10.5' }],
  /** A clock face: the timer */
  clock: [{ cx: 8, cy: 8, r: 6 }, { d: 'M8 4.8V8l2.2 1.4' }],
  /** Four squares: the answer sheet */
  grid: [{ x: 2.5, y: 2.5, w: 4.5, h: 4.5, rx: 1 }, { x: 9, y: 2.5, w: 4.5, h: 4.5, rx: 1 }, { x: 2.5, y: 9, w: 4.5, h: 4.5, rx: 1 }, { x: 9, y: 9, w: 4.5, h: 4.5, rx: 1 }],
  /** A tick: copied, saved */
  check: [{ d: 'M3 8.5l3.2 3.2L13 4.8' }],
  /** A cross: close */
  x: [{ d: 'M4 4l8 8' }, { d: 'M12 4 4 12' }],
  /** A cross in a circle: failed, errors */
  xCircle: [{ cx: 8, cy: 8, r: 6 }, { d: 'M5.9 5.9l4.2 4.2' }, { d: 'M10.1 5.9 5.9 10.1' }],
  /** A circular arrow: ask the model again, reload */
  refresh: [{ d: 'M13 8a5 5 0 1 1-1.6-3.7' }, { d: 'M13 2.5V5h-2.5' }],
  /** A circular arrow around a small play triangle: take it again (a new exam or practice) */
  retake: [{ d: 'M13 8a5 5 0 1 1-1.6-3.7' }, { d: 'M13 2.5V5h-2.5' }, { d: 'M6.8 6.2v3.6L10 8Z' }],
  /** A rounded square: Stop */
  stop: [{ x: 4, y: 4, w: 8, h: 8, rx: 1.8 }],
  /** A paper plane: Send */
  send: [{ d: 'M14 2.2 2.2 6.9l4.5 1.9 1.9 4.5Z' }, { d: 'M6.7 8.8 14 2.2' }],
  /** A bookmark: memories */
  memory: [{ d: 'M4.5 2.5h7v11L8 10.8 4.5 13.5Z' }],
  /** Three dots: more actions */
  more: [{ d: 'M3.5 8h.01' }, { d: 'M8 8h.01' }, { d: 'M12.5 8h.01' }],
  /** A key with a circular arrow: reset a password */
  keyRound: [{ cx: 5.5, cy: 10.5, r: 2.5 }, { d: 'M7.4 8.6 12.5 3.5' }, { d: 'M10.6 5.4l1.6 1.6' }, { d: 'M2.2 7A5.6 5.6 0 0 1 6.6 2.4' }, { d: 'M6.6 2.4v2' }],
  /** Pause in a circle */
  pauseCircle: [{ cx: 8, cy: 8, r: 6 }, { d: 'M6.5 5.8v4.4' }, { d: 'M9.5 5.8v4.4' }],
  /** Play in a circle */
  playCircle: [{ cx: 8, cy: 8, r: 6 }, { d: 'M6.8 5.6v4.8l3.8-2.4Z' }],
  /** A person with a slash: deactivate an account */
  userOff: [{ cx: 8, cy: 5.2, r: 2.4 }, { d: 'M3 13.5c.3-2.6 2.4-4 5-4s4.7 1.4 5 4' }, { d: 'M2.5 2.5l11 11' }],
  /** A person with a tick: activate an account */
  userOn: [{ cx: 6.5, cy: 5.2, r: 2.4 }, { d: 'M1.8 13.5c.3-2.6 2.2-4 4.7-4 1 0 1.9.2 2.6.6' }, { d: 'M9.8 11.2l1.6 1.6 3-3.3' }],
  /** An open padlock: unlock */
  unlock: [{ x: 3, y: 7.2, w: 10, h: 6.6, rx: 1.6 }, { d: 'M5.5 7.2V5a2.5 2.5 0 0 1 4.8-1' }],
  /** An eye: view */
  eye: [{ d: 'M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8Z' }, { cx: 8, cy: 8, r: 2.1 }],
  /** A tray with an arrow up: upload */
  upload: [{ d: 'M2.5 10v3h11v-3' }, { d: 'M8 10.5V2.8' }, { d: 'M4.8 5.8 8 2.6l3.2 3.2' }],
  /** A triangle with an exclamation mark: warnings */
  warn: [{ d: 'M8 2.2 14.2 13H1.8Z' }, { d: 'M8 6.4v3' }, { d: 'M8 11.3v.1' }],
  /** A magnifier: search */
  search: [{ cx: 7, cy: 7, r: 4.3 }, { d: 'M10.2 10.2l3.6 3.6' }],
  /** Two bars: pause live updates */
  pause: [{ d: 'M5.5 3.5v9' }, { d: 'M10.5 3.5v9' }],
  /** A triangle: resume live updates */
  play: [{ d: 'M5 3.2v9.6L13 8Z' }],
  /** A sun: light theme */
  sun: [{ cx: 8, cy: 8, r: 2.8 }, { d: 'M8 1.8v1.4' }, { d: 'M8 12.8v1.4' }, { d: 'M1.8 8h1.4' }, { d: 'M12.8 8h1.4' }, { d: 'M3.6 3.6l1 1' }, { d: 'M11.4 11.4l1 1' }, { d: 'M3.6 12.4l1-1' }, { d: 'M11.4 4.6l1-1' }],
  /** A crescent: dark theme */
  moon: [{ d: 'M13 9.6A5.5 5.5 0 0 1 6.4 3a5.5 5.5 0 1 0 6.6 6.6Z' }],
  /** A circle with its left half closed: follow the system theme */
  auto: [{ cx: 8, cy: 8, r: 5.5 }, { d: 'M8 2.5a5.5 5.5 0 0 0 0 11Z' }],
  /** A funnel: filter */
  filter: [{ d: 'M2.5 3h11l-4.2 5v4.8L6.7 14V8Z' }],
  /** Three lines with dots: a list of records */
  list: [{ d: 'M6 4h7.5' }, { d: 'M6 8h7.5' }, { d: 'M6 12h7.5' }, { d: 'M2.5 4h.01' }, { d: 'M2.5 8h.01' }, { d: 'M2.5 12h.01' }],
  /** A house: Home */
  home: [{ d: 'M2.5 7.5 8 2.8l5.5 4.7' }, { d: 'M3.8 6.6v6.9h8.4V6.6' }, { d: 'M6.6 13.5V9.6h2.8v3.9' }],
  /** A simple line-style GitHub mark (a cat head with a tail): the footer link */
  /** AI settings category tabs (v1.0, AS-9): AI+ function / AI+ users / AI usage / AI safety */
  plug: [{ d: 'M5.8 2v3' }, { d: 'M10.2 2v3' }, { d: 'M4 5h8v2.4A4 4 0 0 1 8 11.4a4 4 0 0 1-4-4Z' }, { d: 'M8 11.4V14' }],
  users: [{ cx: 6, cy: 5.4, r: 2.3 }, { d: 'M1.8 13.4c.2-2.5 1.9-3.8 4.2-3.8s4 1.3 4.2 3.8' }, { d: 'M10.4 3.4a2.3 2.3 0 0 1 0 4.2' }, { d: 'M12 9.9c1.4.5 2.1 1.6 2.2 3.5' }],
  chart: [{ d: 'M2.5 13.5h11' }, { d: 'M4.5 11.5V8' }, { d: 'M8 11.5V3.5' }, { d: 'M11.5 11.5V6' }],
  shield: [{ d: 'M8 1.8 13 3.6v4.2c0 3-2.1 5-5 6.4-2.9-1.4-5-3.4-5-6.4V3.6Z' }, { d: 'M5.8 8l1.6 1.6 3-3.2' }],
  github: [{ d: 'M6 13.6c-3 .9-3-1.6-4.2-2' }, { d: 'M10 14.5v-2.3c0-.8.1-1.2-.4-1.7 2-.2 4.1-1 4.1-4.4 0-.9-.3-1.6-.9-2.2.1-.5.1-1.4-.2-2.2 0 0-.7-.2-2.3.9a7.6 7.6 0 0 0-4.1 0C4.6 1.5 3.9 1.7 3.9 1.7c-.3.8-.3 1.7-.2 2.2-.6.6-.9 1.3-.9 2.2 0 3.4 2.1 4.2 4.1 4.4-.3.3-.4.7-.4 1.2v2.8' }],
  /** v1.1: an hourglass: the exam time limit */
  hourglass: [{ d: 'M4 2.5h8' }, { d: 'M4 13.5h8' }, { d: 'M4.8 2.5c0 3 3.2 3.6 3.2 5.5S4.8 10.5 4.8 13.5' }, { d: 'M11.2 2.5c0 3-3.2 3.6-3.2 5.5s3.2 2.5 3.2 5.5' }],
  /** v1.1: a sheet with a small sparkle: AI question generation */
  aiGen: [{ d: 'M9 1.8H4.2A1.2 1.2 0 0 0 3 3v10a1.2 1.2 0 0 0 1.2 1.2h6.6A1.2 1.2 0 0 0 12 13V7' }, { d: 'M5.4 10h3.2' }, { d: 'M5.4 12.2h5.2' }, { d: 'M11.5 1.6c.2 1.2.7 1.7 1.9 1.9-1.2.2-1.7.7-1.9 1.9-.2-1.2-.7-1.7-1.9-1.9 1.2-.2 1.7-.7 1.9-1.9Z' }],
  /** v1.1: a page with an up arrow and a small sparkle: AI file import */
  aiImport: [{ d: 'M8.5 14.2H4.2A1.2 1.2 0 0 1 3 13V3a1.2 1.2 0 0 1 1.2-1.2H8.5' }, { d: 'M7.5 11.2V6' }, { d: 'M5.3 8l2.2-2.2L9.7 8' }, { d: 'M12.2 8.4c.2 1.2.7 1.7 1.9 1.9-1.2.2-1.7.7-1.9 1.9-.2-1.2-.7-1.7-1.9-1.9 1.2-.2 1.7-.7 1.9-1.9Z' }],
  /** v1.1: a sheet with an up arrow: hand in the paper (the quiz top row's 交卷 / 完成) */
  submit: [{ d: 'M4.2 1.8h5l3 3v8a1.2 1.2 0 0 1-1.2 1.2H4.2A1.2 1.2 0 0 1 3 12.8V3a1.2 1.2 0 0 1 1.2-1.2Z' }, { d: 'M7.6 12V7' }, { d: 'M5.5 9l2.1-2.1L9.7 9' }],
} satisfies Record<string, Shape[]>;

export type MiniIcon = keyof typeof MINI;

/** Every icon name, in the order of the table */
export const MINI_NAMES = Object.keys(MINI) as MiniIcon[];
