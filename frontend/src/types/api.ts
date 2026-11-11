// Types of the backend REST API. Shapes come from real responses of the old version (tests/baseline/golden/); key names match the old version, do not change them.

export interface Info {
  version: string;
  arch: string;
  canShutdown: boolean;
  bankCount: number;
}

export interface Opt {
  key: string;
  text: string;
}

export interface Item {
  text: string;
  choices: string[];
}

/**
 * Structure split out for Chinese / English (same shape): single / multiple choice carry options, hotspot questions carry items;
 * if every item of a hotspot question ends with options written in this language (Chinese yes / no, English Yes / No, ...), there are also choices (only when the count matches the Japanese).
 */
export interface Zh {
  stem: string;
  options?: string[];
  items?: string[];
  choices?: string[][];
}

/** Structure split out for the English version, same as for Chinese. */
export type En = Zh;

export type Kind = 'single' | 'multi' | 'hotspot';

/** One language's explanation of a six-column question (every key is present, maybe "") */
export interface ExplainText {
  explanation: string;
  mnemonic: string;
  note: string;
}

/** `Question.explain`: a language whose answer column has nothing is left out (the object can be empty) */
export interface Explain {
  en?: ExplainText;
  zh?: ExplainText;
  ja?: ExplainText;
}

/** What a study record is (v0.6.5): an exam, or a practice with immediate feedback */
export type AttemptKind = 'exam' | 'practice';

export interface Question {
  kind: Kind;
  tag: string;
  stem: string;
  options: Opt[];
  items: Item[];
  /** Single / multiple choice: letters; hotspot question: which choice was picked for each item */
  answer: string[] | number[];
  translation: string;
  zh: Zh | null;
  /** Only present in the 4-column format: the original English text of column A, and a version split along the Japanese structure (en is empty when it does not line up with the Japanese) */
  english?: string;
  en?: En | null;
  /**
   * Only present in the six-column format (v0.6.5): "en" = the base text (stem / options / items) is the English of column A,
   * and the Japanese is in `ja`. Missing = the base text is Japanese (3 / 4-column formats).
   */
  lang?: 'en';
  /** Six-column format only: the Japanese of column C, split along the English structure (null / missing when it does not line up) */
  ja?: Zh | null;
  /** Compatibility text (six-column: the Chinese one when present, else the Japanese, else the English) */
  explanation: string;
  mnemonic: string;
  note: string;
  /** Six-column format only: the explanation, memory aid and answer note of each language that has them */
  explain?: Explain;
  id: string;
  sheet: string;
  row: number;
  /** Id of the question bank (imported workbook) the question belongs to (v0.5+) */
  bank: number;
}

export interface SheetInfo {
  name: string;
  usable: number;
  inactive: number;
  total: number;
  skipped: SkippedRow[];
}

export interface Bank {
  file: string;
  mtime: string;
  error: string;
  questions: Question[];
  sheets: SheetInfo[];
}

export interface SkippedRow {
  row: number;
  reason: string;
}

export interface QuestionStat {
  attempts: number;
  correct: number;
  wrong: number;
  streak: number;
  last?: boolean;
  at?: string;
}

export type Mode = 'all' | 'weak' | 'retry';

export interface HistoryEntry {
  at: string;
  sheets: string[];
  mode: Mode | string;
  total: number;
  correct: number;
  percent: number;
  seconds: number;
  /** v0.6.5; always sent by the backend from then on */
  kind?: AttemptKind;
}

export interface Progress {
  version: number;
  questions: Record<string, QuestionStat>;
  history: HistoryEntry[];
  masterStreak: number;
}

/**
 * The answer of `POST /api/attempt`: the user's progress, plus (v0.6.5) the ids of the results that were not counted because
 * their question no longer exists (its bank was deleted after the exam started). Missing when every result was counted.
 */
export interface AttemptResponse extends Progress {
  notCounted?: string[];
}

export interface ImportRecord {
  id: number;
  at: string;
  filename: string;
  added: number;
  updated: number;
  removed: number;
  restored: number;
  skipped: number;
  total: number;
}

export interface SheetPlan {
  name: string;
  total: number;
  usable: number;
  skipped: SkippedRow[];
  added: number;
  updated: number;
  removed: number;
  restored: number;
  /** Preview only: how many entries of the truncated "unreadable" list are not listed */
  skippedMore?: number;
  skippedCount?: number;
  /**
   * v0.7, six-column files only (left out when a sheet has none): rows imported with the English answer although the
   * Chinese (E) or Japanese (F) answer differs, one entry per differing column; capped in the preview like `skipped`
   */
  warnings?: SkippedRow[];
  warningsMore?: number;
  warningsCount?: number;
}

export interface ImportPlan {
  sheets: SheetPlan[];
  added: number;
  updated: number;
  unchanged: number;
  restored: number;
  removed: number;
  skipped: number;
  usable: number;
  removed_samples: string[];
  removed_stats: number;
  /** v0.7: the number of preview warnings of a six-column file (left out when there is none) */
  warnings?: number;
}

/** Response of POST /api/upload: preview + one-time confirmation code */
export interface ImportPreview extends ImportPlan {
  filename: string;
  token: string;
}

/** Response of POST /api/import/confirm. bankId is present when the upload named a target (always, from the v0.5 UI). */
export interface ImportConfirmed {
  result: ImportPlan;
  bank: Bank;
  bankId?: number;
}

/** Where an upload goes: a new bank, or an existing bank (update / replace). */
export type ImportTarget = 'new' | number;

export interface AttemptRequest {
  results: { id: string; correct: boolean; answer?: Array<string | number | null> | null }[];
  sheets: string[];
  mode: Mode;
  seconds: number;
  /** v0.6.5: missing = "exam" */
  kind?: AttemptKind;
}

export interface ApiErrorBody {
  error: string;
}

// ---------------------------------------------------------------- v0.5: question banks

export interface BankSheet {
  name: string;
  usable: number;
  inactive: number;
}

/** One element of GET /api/banks */
export interface BankSummary {
  id: number;
  /** Display name (the first imported file name without .xlsx) */
  name: string;
  /** The file name of the latest import */
  filename: string;
  /** Holds the data from before v0.5 (unscoped question ids) */
  legacy: boolean;
  createdAt: string;
  updatedAt: string;
  /** Active questions */
  total: number;
  /** Retired questions */
  inactive: number;
  sheets: BankSheet[];
  /** How many of its questions have per-question study stats */
  studied: number;
  /** How many exam records include this bank */
  records: number;
  /**
   * v0.6.5: the user who owns the bank (imported it as a new bank; banks from before v0.6.5 belong to the first
   * administrator) and their username; null when unknown. Only administrators get both; anyone else gets `ownerId` only for
   * their own banks and `owner` never (v0.7: the bank list does not name other accounts)
   */
  ownerId: number | null;
  owner: string | null;
  /** Whether the signed-in user may update / replace and delete this bank (an administrator, or a bank manager who owns it) */
  manageable: boolean;
}

export interface BanksResponse {
  banks: BankSummary[];
}

/** Response of POST /api/banks/delete */
export interface BankDeleted {
  banks: BankSummary[];
  bank: Bank;
  progress: Progress;
}

// ---------------------------------------------------------------- v0.5: study records

export interface RecordBank {
  id: number;
  name: string;
  deleted: boolean;
}

/** One exam session (GET /api/records) */
export interface RecordSummary {
  id: number;
  at: string;
  banks: RecordBank[];
  sheets: string[];
  mode: Mode | string;
  total: number;
  correct: number;
  percent: number;
  seconds: number;
  /** Questions that had an answer; null for records saved before v0.5 (no per-question detail) */
  answered: number | null;
  /** Whether per-question detail exists */
  detail: boolean;
  /** Exam or practice (v0.6.5; read it through kindOfRecord(), which treats anything unknown as an exam) */
  kind?: AttemptKind;
}

export interface RecordsResponse {
  records: RecordSummary[];
  retention: number;
}

/** A question as stored now, plus whether it is still active */
export type StoredQuestion = Question & { active: boolean };

export interface RecordItem {
  seq: number;
  questionId: string;
  /** What was picked: letters for single / multi, choice indexes (or null) per item for hotspot; null = unanswered */
  answer: Array<string | number | null> | null;
  correct: boolean;
  /** Null when the question no longer exists (its bank was deleted) */
  question: StoredQuestion | null;
}

/** GET /api/record?id= */
export interface RecordDetail {
  record: RecordSummary;
  items: RecordItem[];
}

/** POST /api/records/delete */
export interface RecordsDeleted extends RecordsResponse {
  progress: Progress;
}

// ---------------------------------------------------------------- v0.5: settings (stored in the data directory)

export type Theme = 'system' | 'light' | 'dark';
export type UiLang = 'en' | 'zh' | 'ja';
export type QuestionLang = 'ja' | 'zh' | 'en';

export interface Settings {
  theme: Theme;
  uiLang: UiLang;
  /** After answering, go to the next question automatically */
  autoAdvance: boolean;
  /** Language the question text is shown in (Japanese is the original) */
  questionLang: QuestionLang;
}

// ---------------------------------------------------------------- v0.6: users and sign-in

export type Role = 'admin' | 'user';

/** The signed-in user (`GET /api/auth/state`, `POST /api/auth/login`, `POST /api/auth/password`) */
export interface AuthUser {
  id: number;
  username: string;
  role: Role;
  /** The default or initial password must be replaced before anything else works */
  mustChangePassword: boolean;
  /** v0.6.5: may open Question banks (import new banks, update and delete their own); always true for administrators */
  canManageBanks: boolean;
}

export interface AuthState {
  user: AuthUser | null;
  /** Signed in: an opaque stamp of the shared question banks; it changes when a bank is imported, replaced or deleted */
  library?: string;
  /**
   * Not signed in: the username of an enabled administrator that still has the default password `admin` (the sign-in page
   * says how to sign in for the first time, with that name), else null
   */
  defaultAdmin?: string | null;
}

export interface AuthUserResponse {
  user: AuthUser;
}

/** One row of `GET /api/users` (administrators only) */
export interface UserRow {
  id: number;
  username: string;
  role: Role;
  disabled: boolean;
  createdAt: string;
  lastLoginAt: string | null;
  mustChangePassword: boolean;
  /** Number of exam records of this user */
  records: number;
  /**
   * The existing question banks the user may see, ascending ids (the stored setting; administrators always see every bank).
   * v0.7: always a list (before: "all" or a list)
   */
  banks: number[];
  /** v0.7: "Include banks imported later": every bank imported from now on is added to `banks` */
  newBanks: boolean;
  /** v0.6.5: the stored "can manage question banks" permission (administrators always have it, whatever is stored) */
  canManageBanks: boolean;
  /** v0.6.5: ids of the banks this user owns, ascending (they move to the deleting administrator when the user is deleted) */
  ownedBanks: number[];
  /** The login throttle of this account (in the backend's memory): null at round 1 and not locked */
  lock: UserLock | null;
}

/**
 * `"lock"` of a user in `GET /api/users`. Locked: `until` (local time `YYYY-MM-DD HH:MM:SS`) and `retryAfter` > 0, `round`
 * = the round that ended in this lock (1 = the first lock). Not locked but past round 1: `until` null, `retryAfter` 0, `round`
 * = the round in progress, `triesLeft` = failures left before the next lock. `knownBrowser`: the state counts the sign-ins
 * from browsers where the user signed in before (else from every other browser; the two are counted separately).
 * `otherUntil` / `otherRetryAfter`: a lock that runs at the same time for the other browsers (the account is then locked in
 * every browser); null / 0 otherwise (always so when not locked).
 */
export interface UserLock {
  until: string | null;
  retryAfter: number;
  round: number;
  triesLeft: number;
  knownBrowser: boolean;
  otherUntil?: string | null;
  otherRetryAfter?: number;
}

/** `GET /api/users` and every `POST /api/users/*` answer with the whole list */
export interface UsersResponse {
  users: UserRow[];
}

/** `POST /api/users/update`: only the fields that change */
export interface UserUpdate {
  id: number;
  username?: string;
  role?: Role;
  disabled?: boolean;
  banks?: BankAccess;
  newBanks?: boolean;
  canManageBanks?: boolean;
}

/**
 * `"banks"` of `POST /api/users/create` / `update`: the existing banks a plain user sees, as ids. The server also takes "all"
 * (every existing bank, plus `newBanks: true` unless it is given); the page always sends the list. A bank manager also sees
 * the banks they own.
 */
export type BankAccess = 'all' | number[];
