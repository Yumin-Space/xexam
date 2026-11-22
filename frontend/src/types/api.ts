// Types of the backend REST API. Shapes come from real responses of the old version (tests/baseline/golden/); key names match the old version, do not change them.

/** GET /api/info: public (the sign-in page shows the version). Since v0.7.5 it no longer says how many questions exist. */
export interface Info {
  version: string;
  arch: string;
  canShutdown: boolean;
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
  /** v0.9: the id of the record this save created (always last in the answer) */
  recordId?: number;
  /** v1.3 (IMPROVE A1): the same attempt was already saved (a retry after a lost answer); nothing was written again */
  duplicate?: true;
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
  /** v0.9: the session key (32 lower-case hex) and start time (unix seconds) that tie a record to its AI chat */
  key?: string;
  startedAt?: number;
  /** v1.1 (TL-9): the time limit of a timed exam in seconds (60 ... 36000); ignored for practice */
  limit?: number;
  /** v1.1 (TL-9): how an exam ended; sent for every exam */
  end?: 'submit' | 'timeUp';
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
  /** v1.1 (API-M1): present exactly for AI-generated / AI-imported banks */
  ai?: BankAi;
}

/** v1.1: where an AI bank came from (`generate` / file import); `authorId` never leaves the server */
export interface BankAi {
  source: 'bank' | 'scope' | 'wrong' | 'file';
  from: string[];
  /** File imports only (P-37) */
  file?: string;
  mode?: 'questions' | 'material';
  skipped?: number[];
  author: string | null;
  made: number;
  goal: number;
  /** `YYYY-MM-DD HH:mm:ss`, server time */
  at: string;
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
  /** v0.9, `GET /api/record` only: the session key and start time of the record (null for records saved before v0.9) */
  sessionKey?: string | null;
  startedAt?: number | null;
  /** v1.1 (TL-9): the limit of a timed exam in seconds */
  limit?: number;
  /** v1.1 (TL-9): present = the record was saved under the not-completed rule (every exam since v1.1) */
  ended?: 'submit' | 'timeUp';
  /** v1.1 (TL-9): the number of not-completed questions (with `ended`) */
  unanswered?: number;
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
  /** v1.1 (TL-9): not completed (only on records with `ended`) */
  unanswered?: boolean;
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
  /** The login (an e-mail address; older installs may still have a plain name) */
  username: string;
  /** v2.6: the name shown in the app (用户名); null until the user has set one */
  displayName?: string | null;
  /** v2.6: no display name yet: the user must set one before anything else works (like `mustChangePassword`) */
  needsName?: boolean;
  role: Role;
  /** The default or initial password must be replaced before anything else works */
  mustChangePassword: boolean;
  /** v0.6.5: may open Question banks (import new banks, update and delete their own); always true for administrators */
  canManageBanks: boolean;
  /** v1.6: may download the banks they can see (Excel / Word); always true for administrators. Optional: an older server omits it; only `true` counts */
  canDownloadBanks?: boolean;
  /**
   * v0.8: may use AI Chat while AI is enabled in AI settings (administrators always, plain users with AI+). Whether AI is
   * enabled at all comes from the AI service (GET /api/ai/status), not from here.
   */
  aiAllowed: boolean;
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
  /**
   * v0.8 round 2, signed in: AI+ is available (ai-service reported to the main service less than 60 s ago and the AI+
   * service is on in AI settings)
   */
  aiAvailable?: boolean;
  /** v0.8 round 2, administrators only: the state of ai-service from its reports (AI settings shows it) */
  aiService?: AiService;
  /** v1.3, signed in: the session rule and its limits */
  session?: SessionInfo;
  /** v1.3, signed in: the page reports its visits (`POST /api/activity/visit`) */
  logVisits?: boolean;
}

export interface AuthUserResponse {
  user: AuthUser;
}

/** v1.3: `POST /api/auth/login` body */
export interface LoginRequest {
  username: string;
  password: string;
  remember?: boolean;
}

/** v1.3: `POST /api/auth/login` answer */
export interface LoginResponse extends AuthUserResponse {
  session: SessionInfo;
}

/** One row of `GET /api/users` (administrators only) */
export interface UserRow {
  id: number;
  username: string;
  /** v2.6: the display name the user chose; null while they have not set one */
  displayName?: string | null;
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
  /** v1.6: the stored "download banks" permission (administrators always may, whatever is stored); false when absent */
  canDownloadBanks: boolean;
  /**
   * v0.8: the stored AI+ service switch of User management (administrators always have AI while AI+ is available, whatever
   * is stored). Kept while AI+ is not available; the page then shows no switch.
   */
  ai: boolean;
  /** v0.6.5: ids of the banks this user owns, ascending (they move to the deleting administrator when the user is deleted) */
  ownedBanks: number[];
  /** The login throttle of this account (in the backend's memory): null at round 1 and not locked */
  lock: UserLock | null;
  /**
   * v0.8 round 2: an administrator suspended this user's AI+ function in AI settings (from ai-service's last report to the
   * main service; always false for administrators). The row then shows the dark badge.
   */
  aiSuspended: boolean;
  /** v1.3: the last request of this user (minute precision); null = never active */
  lastActiveAt?: string | null;
  /** v1.3: the last sign-in IP; null while the sign-in info eye is closed or nothing was recorded */
  lastLoginIp?: string | null;
  lastLoginLocation?: GeoLocation | null;
  /** v1.4: IP of the last activity; null while the sign-in info eye is closed or nothing was recorded */
  lastActiveIp?: string | null;
  lastActiveLocation?: GeoLocation | null;
  /** v1.4: User-Agent of the last activity (shown as a device label, also while the eye is closed) */
  lastActiveUa?: string | null;
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
  /**
   * v0.8 round 2: AI+ is available (ai-service reported to the main service less than 60 s ago and the AI+ service is on in
   * AI settings). Only then can a user's AI+ service be switched on or off (else 409 `ai.serviceOff`).
   */
  aiAvailable: boolean;
  /** v1.3: the sign-in info eye (IP + approximate location) of User management */
  signinInfo?: boolean;
  /** v1.4: where the place of a sign-in comes from (`cloudflare` when missing) */
  geoSource?: GeoSource;
  /** v1.4: ai-service is connected (fresh heartbeat), whether or not AI+ is on */
  aiServiceUp?: boolean;
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
  /** v1.6: the "download banks" permission */
  canDownloadBanks?: boolean;
  /** v0.8: the AI+ switch */
  ai?: boolean;
}

/** `POST /api/users/create` (v0.8: `ai` added) */
export interface UserCreate {
  username: string;
  password: string;
  role: Role;
  banks: BankAccess;
  newBanks: boolean;
  canManageBanks: boolean;
  /** v1.6: the "download banks" permission (false when left out) */
  canDownloadBanks?: boolean;
  ai: boolean;
}

/**
 * `"banks"` of `POST /api/users/create` / `update`: the existing banks a plain user sees, as ids. The server also takes "all"
 * (every existing bank, plus `newBanks: true` unless it is given); the page always sends the list. A bank manager also sees
 * the banks they own.
 */
export type BankAccess = 'all' | number[];

// ---------------------------------------------------------------- v0.8: the AI service (ai-service, /api/ai/*)

/**
 * v0.8 round 2: the state of ai-service as the main service knows it from its reports (`aiService` of `GET /api/auth/state`,
 * administrators only). ai-service reports every 25 s (and at once after a change); after 60 s without one it counts as not
 * connected.
 */
export interface AiService {
  /** A report younger than 60 s that was not a "stopping" report */
  connected: boolean;
  /** The last report said ai-service is shutting down */
  stopped: boolean;
  /** Unix seconds of the last accepted report since the main service started; null = none */
  lastSeen: number | null;
  /** The version of the last report; null = none */
  version: string | null;
  /** The AI+ service is on (only while connected; false otherwise) */
  enabled: boolean;
  /** The main service has an internal token loaded (the AI link is on) */
  link: boolean;
}

/** v1.0: the AI request cap that would refuse a new call now (`retryAt`: unix seconds; for the day cap the next 0:00 of the server) */
export interface AiCallBlock {
  cap: 'minute' | 'hour' | 'day';
  limit: number;
  retryAt: number;
}

/** v1.0: counted AI model calls of one user (minute / hour rolling, day since 0:00 of the server's time zone) */
export interface AiCallCounts {
  minute: number;
  hour: number;
  day: number;
}

/** v1.0: the per-user caps on AI model calls, with the user's own counts */
export interface AiCalls {
  used: AiCallCounts;
  caps: AiCallCounts;
  blocked: AiCallBlock | null;
  /** v1.3: an administrator while "administrators are exempt" is on (counts still sent, no limits) */
  exempt?: true;
}

/** `GET /api/ai/status`: what the signed-in user may do with AI right now */
export interface AiStatusJson {
  /** The AI+ service is on (and its settings passed a test) and ai-service reached the main service less than 60 s ago */
  enabled: boolean;
  /** This user may chat now: enabled, AI+ service (administrators always) and the AI+ function not suspended */
  allowed: boolean;
  /** v0.8 round 2: an administrator suspended this user's AI+ function in AI settings (always false for administrators) */
  suspended: boolean;
  /** This user has at least one chat or memory (reading and deleting them stays possible when AI is off) */
  hasData: boolean;
  /** The user's answer that is being written right now, if any */
  running: { chatId: number; messageId: number } | null;
  retentionDays: number;
  memoryLimit: number;
  memoryMaxTokens: number;
  maxMessageChars: number;
  /** Section 14.1 of the v0.8 spec names these two as well; the page reads either spelling */
  messageChars?: number;
  memoryTokens?: number;
  /** Administrators only (false for others): the saved keys cannot be decrypted */
  keyProblem: boolean;
  /** Administrators only (false for others): a model and the required keys are saved */
  configured: boolean;
  /** Administrators only (false for others): the free mode is on (every AI answer, in any chat, is a free-mode answer) */
  freeMode: boolean;
  /** v1.0: the per-user AI request caps and the user's counts; present exactly when `enabled` and the user has the AI+ service */
  calls?: AiCalls;
  /** v1.1 (BT-2): why AI is not enabled; null when enabled; absent on an older ai-service */
  why?: 'off' | 'link' | null;
  /** v1.1 (FP-3, P-11): present exactly when the user has the AI+ service; a missing `import` key counts as true */
  features?: Partial<Record<AiFeatureKey, boolean>>;
  /** v1.1 (API-U1): present exactly when the user has the AI+ service, also while AI is off */
  gen?: GenStatus;
  /** v1.3, administrators: the default model (the model of AI Chat when nothing is picked); null = none */
  chatDefault?: { connectionId: number; connectionName: string; model: string } | null;
}

export type AiFeatureKey = 'chat' | 'hint' | 'explain' | 'summary' | 'assess' | 'generate' | 'import';

export type GenSource = 'bank' | 'scope' | 'wrong' | 'file';
export type GenState = 'queued' | 'running' | 'done' | 'failed' | 'cancelled';
export type GenPhase = 'planning' | 'reading' | 'confirm' | 'writing' | 'saving';
export type ImportFileType = 'pdf' | 'docx' | 'xlsx' | 'pptx' | 'md' | 'txt' | 'csv';

/** v1.1 (API-U1, P-30): one generation / import job */
export interface GenJob {
  id: number;
  state: GenState;
  phase: GenPhase | null;
  source: GenSource;
  name: string;
  from: Array<{ id: number; name: string }>;
  fromCount: number;
  file: { name: string; type: ImportFileType; size: number } | null;
  mode: 'questions' | 'material' | null;
  /** v1.9: true while the job waits for the provider's quota */
  waiting?: boolean;
  /** A-OWNER-70: questions found in the file (the service's `total`) and how many of them the import takes (`taken`); null while unknown */
  total?: number | null;
  taken?: number | null;
  /** The count is an estimate (the recognition was not sure): shown as "about N" */
  approx?: boolean;
  /** Older name of `total` (kept readable) */
  found?: number | null;
  read: { done: number; total: number } | null;
  confirmBy: number | null;
  errorArg: number | null;
  skipped: number[] | null;
  made: number;
  goal: number;
  calls: number;
  createdAt: number;
  startedAt: number | null;
  finishedAt: number | null;
  bankId: number | null;
  error: string | null;
  charged: boolean;
  /** A-OWNER-67: exact sums of the job's own model calls (recognition included) */
  usage?: { calls: number; tokensIn: number; tokensOut: number; tokensCached?: number; tokensCacheWrite?: number };
  /** v1.3 (COST 3): chunks of the planned source and tokens of the shared prefix; set once planning is done */
  chunks?: number;
  prefixTokens?: number;
}

/** v1.1 (API-U1): `gen` of the status */
export interface GenStatus {
  perDay: number | null;
  used: number;
  left: number | null;
  unlimited: boolean;
  resetAt: number;
  minSource: number;
  minWrong: number;
  bank: { single: number; multi: number };
  wrong: { single: number; multi: number };
  import?: { maxMb: number; maxQuestions: number; types: string[] };
  job: GenJob | null;
  unseen: GenJob[];
}

/** v1.1: row of `GET /api/ai/admin/gen` */
export type GenAdminJob = GenJob & { userId: number; username: string; tokensIn: number; tokensOut: number };

export interface AiChatSummary {
  id: number;
  title: string;
  /** Unix seconds */
  createdAt: number;
  updatedAt: number;
  messageCount: number;
  running: boolean;
  /** v0.9: 'chat' (AI Chat) or the kind of study chat */
  kind?: AiChatKind;
  /** v0.9: the record a study chat belongs to, when known */
  recordId?: number | null;
  /** v1.3 (CHAT 3): the leaf of the viewed branch (null = empty chat) */
  leafId?: number | null;
  /** v1.3, administrators only: the picked model (null = default) */
  pick?: ModelRef | null;
  /** v2.1, administrators only: the picked thinking-strength level (null = none) */
  /** a level of the route's family, or 'default' (v2.2: Default, no strength parameter sent) */
  pickEffort?: string | null;
}

export type AiChatKind = 'chat' | 'practice' | 'exam' | 'summary' | 'assess';
export type AiMessageKind = 'chat' | 'hint' | 'explain' | 'summary' | 'assess';

export type AiMessageStatus = 'streaming' | 'done' | 'stopped' | 'error';

export interface AiMessage {
  id: number;
  role: 'user' | 'assistant';
  content: string;
  status: AiMessageStatus;
  /** An error code (`ai.…`) when the status is `error` */
  error: string | null;
  /** The answer reached the model's length limit */
  truncated: boolean;
  /** administrators only (v1.3 sprint 2): absent for everyone else */
  model?: string | null;
  createdAt: number;
  /** Written in the administrator's free mode (the chat draws a divider where it changes) */
  free: boolean;
  /** v0.9 */
  kind?: AiMessageKind;
  questionId?: string | null;
  questionNo?: number | null;
  source?: 'practice' | 'result' | null;
  lang?: 'en' | 'zh-CN' | 'ja' | null;
  recordId?: number | null;
  sample?: { used: number; usable: number; wrong: number } | null;
  /** v1.3 (CHAT 3): the previous message of the branch tree (null = first); sibling versions in `versions` (2+ only) */
  parentId?: number | null;
  versions?: number[];
  /** v1.3, administrators only: the name of the connection that answered */
  connection?: string | null;
  /** v1.3: the model's reasoning (AI Chat answers only; plain text, trimmed, <= 32 KiB); null / absent = none */
  reasoning?: string | null;
  /** v1.3: milliseconds the model reasoned before its first answer text; null while it still reasons or unknown */
  reasoningMs?: number | null;
  /** v1.3: the reasoning was cut at the server's cap (only its head is kept) */
  reasoningTruncated?: boolean;
}

/** `GET /api/ai/chat?id=N` (and the administrators' `GET /api/ai/admin/chat`) */
export interface AiChatDetail {
  chat: AiChatSummary;
  messages: AiMessage[];
}

export interface AiMemory {
  id: number;
  content: string;
  source: 'manual' | 'chat';
  createdAt: number;
  updatedAt: number;
}

/** `GET /api/ai/memories` */
export interface AiMemoriesAnswer {
  memories: AiMemory[];
  limit: number;
  maxTokens: number;
}

export type AiProvider = 'openai' | 'anthropic' | 'gateway';
export type AiFormat = 'openai' | 'anthropic' | 'responses';
export type AiGatewayAuth = 'bearer' | 'x-api-key' | 'cf-aig-authorization';
/** The two key slots of the AI settings */
export type AiKeySlot = 'apiKey' | 'gatewayKey';

export interface AiUsageCounts {
  calls: number;
  tokensIn: number;
  tokensOut: number;
  errors: number;
  /** Input tokens the provider served from its prompt cache */
  tokensCached: number;
  /** Input tokens the provider wrote to its prompt cache (A-OWNER-74) */
  tokensCacheWrite?: number;
}

/** `GET /api/ai/admin/usage?days=N` or `?period=12h|today` */
export interface AiUsage {
  /** null for the two short periods */
  days: number | null;
  /** `7d` | `30d` | `90d` | `365d` | `12h` | `today` */
  period: string;
  from: string;
  to: string;
  /** `hour` for the short periods: `byPeriod[].period` is the local `HH:00`, oldest first, zeros included */
  unit: 'day' | 'month' | 'hour';
  totals: AiUsageCounts;
  /** `userId: null` is the one row of deleted users */
  byUser: Array<AiUsageCounts & {
    userId: number | null;
    username: string | null;
    lastAt: number | null;
    /** v1.0: this user's counted calls since today's 0:00 of the server's time zone (the number the day cap compares) */
    today?: number;
  }>;
  byModel: Array<AiUsageCounts & { model: string; connectionId?: number | null; connection?: string | null }>;
  /** v1.3: usage by connection */
  byConnection?: Array<AiUsageCounts & { connectionId: number | null; connection: string | null }>;
  /** v1.3: the shared hint / explanation store */
  shared?: { rows: number; hits: number; hitsToday: number };
  /** v1.3: the site-wide daily budget (`limit` 0 = off) */
  budget?: { limit: number; usedToday: number };
  byPeriod: Array<AiUsageCounts & { period: string }>;
  /** v0.9: chat, hint, explain, summary in server order */
  byPurpose?: Array<AiUsageCounts & { purpose: 'chat' | 'hint' | 'explain' | 'summary' | 'assess' | 'generate' | 'import' }>;
}

/**
 * One row of `GET /api/ai/admin/users`: every administrator and every user with the AI+ service (round 2: users without it
 * are not listed, even with AI data)
 */
export interface AiAdminUserRow {
  id: number;
  username: string;
  role: Role;
  /** The stored AI+ service switch of User management */
  ai: boolean;
  aiAllowed: boolean;
  disabled: boolean;
  /** false: deleted in the main app, its AI data not removed yet (always true in the list of round 2) */
  exists: boolean;
  /** v0.8 round 2: the AI+ function is suspended in AI settings (always false for administrators) */
  suspended: boolean;
  chats: number;
  messages: number;
  memories: number;
  lastUsedAt: number | null;
  tokensIn: number;
  tokensOut: number;
  /** v1.0: the user's counted AI calls and whether a cap blocks a new call now (same shape as the status without `caps`) */
  calls?: Omit<AiCalls, 'caps'>;
  /** v1.1 (FP-4): the user's feature switches; missing = all on */
  features?: Partial<Record<AiFeatureKey, boolean>>;
  /** v1.1 (A-OWNER-21): administrator rows only: the AI Chat free mode of that account */
  freeMode?: boolean;
}

/** `GET /api/ai/admin/user?id=N` */
export interface AiAdminUserDetail {
  user: AiAdminUserRow;
  chats: AiChatSummary[];
  memories: AiMemory[];
}

/** `POST /api/ai/admin/clear` */
export type AiClear = { userId: number; what: 'chat'; chatId: number } | { userId: number; what: 'chats' | 'memories' };

/** The safety limits of the AI service (v0.8 spec 14.1): they protect the server; they are not usage limits */
export interface AiLimits {
  perUserRunning: number;
  globalRunning: number;
  chatsPerUser: number;
  messagesPerChat: number;
  memoriesPerUser: number;
  memoryTokens: number;
  messageChars: number;
  /** v1.0: AI model calls one user may make per minute / hour / day (everyone, administrators included) */
  callsPerMinute: number;
  callsPerHour: number;
  callsPerDay: number;
  /** v1.1 (GS-1 / IS-1): the AI banks tab */
  genPerDay?: number;
  genAdminsExempt?: number;
  genMinSource?: number;
  genMinWrong?: number;
  genBankSingle?: number;
  genBankMulti?: number;
  genWrongSingle?: number;
  genWrongMulti?: number;
  genCallTokens?: number;
  genRunning?: number;
  importMaxMb?: number;
  /** v1.3: 1..300 (stored as `import_file_questions`) */
  importMaxQuestions?: number;
  /** v1.3: optional until the pages read them; hintMaxTokens / explainMaxTokens / summaryMaxTokens are gone */
  callsAdminsExempt?: number;
  studyShare?: number;
  studyShareDays?: number;
  genMaxChunks?: number;
  summaryMaxItems?: number;
  dailyTokenBudget?: number;
  /** v1.9: one pacing setting for AI generation + import (requests / tokens per minute) */
  paceOn?: number;
  paceRpm?: number;
  paceTpm?: number;
  /** v2.6: AI study assessment: successful assessments per user and day (0 = none) and whether administrators are exempt (0 / 1) */
  assessPerDay?: number;
  assessAdminsExempt?: number;
}

/** `GET` / `POST /api/ai/admin/limits` */
export interface AiLimitsAnswer {
  limits: AiLimits;
  defaults: AiLimits;
  ranges: Record<keyof AiLimits, [number, number]>;
}

// ---------------------------------------------------------------- v1.3 additions (frozen contract: frontend-types.md)

export interface ModelRef { connectionId: number; model: string }
export type SessionRule = 'admin' | 'remember' | 'browser';
export interface SessionInfo { rule: SessionRule; idleSecs: number; hardEndsAt: string | null }
export interface GeoLocation { country: string | null; region: string | null; city: string | null }

// ---------------------------------------------------------------- v1.4: external IP-location lookup (admin routes of ai-service)

export type GeoSource = 'cloudflare' | 'api';
export type GeoPresetId = 'ipinfo' | 'ipapi' | 'ipapico' | 'ipwhois' | 'ipsb' | 'custom';
export type GeoFailureCode =
  | 'timeout' | 'unreachable' | 'redirect' | 'httpStatus' | 'notJson' | 'tooLarge' | 'providerFail' | 'noFields'
  | 'keyProblem' | 'rateLimited' | 'notConfigured' | 'refused';
/** The eight dot-path fields of the JSON mapping ("" = not used) */
export interface GeoMapping {
  country: string; countryCode: string; region: string; city: string;
  okPath: string; okValue: string; errorPath: string; messagePath: string;
}
/** The body of test / save: `key` absent = keep the saved key, null = no key, a string = this key */
export interface GeoDraft {
  preset: GeoPresetId;
  urlTemplate: string;
  keyHeaderName: string;
  keyHeaderValue: string;
  key?: string | null;
  mapping: GeoMapping;
}
export interface GeoSettings {
  preset: GeoPresetId;
  urlTemplate: string;
  keyHeaderName: string;
  keyHeaderValue: string;
  key: { set: boolean; hint: string; unreadable: boolean };
  mapping: GeoMapping;
  host: string;
  insecureHttp: boolean;
  testedAt: number;
  updatedAt: number;
}
export interface GeoState { settings: GeoSettings | null; testIp: string; perMinute: number; timeoutMs: number }
export interface GeoPlace { country: string | null; countryCode: string | null; region: string | null; city: string | null }
export interface GeoFailure { code: GeoFailureCode; status: number | null; message: string | null }
export type GeoTestAnswer =
  | { ok: true; testIp: string; place: GeoPlace; ms: number }
  | { ok: false; testIp: string; failure: GeoFailure; ms: number };
export type GeoSaveAnswer =
  | { ok: true; settings: GeoSettings; place: GeoPlace; ms: number }
  | { ok: false; failure: GeoFailure; ms: number };

export type ActivityActor = 'user' | 'anon' | 'system';
export type ActivityGroup = 'auth' | 'users' | 'banks' | 'study' | 'settings' | 'visit' | 'ai' | 'system';
export interface ActivityEntry {
  id: number; time: string; lastTime: string; count: number;
  actor: ActivityActor; userId: number | null; username: string | null; role: 'admin' | 'user' | null; deleted: boolean;
  action: string; object: string | null; detail: Record<string, unknown> | null;
  result: 'ok' | 'fail'; reason: string | null;
  ip: string | null; location: GeoLocation | null; ua: string | null; source: 'backend' | 'ai';
}
export interface ActivitySettings { retentionDays: number; userVisits: boolean; signinInfo: boolean }
export interface ActivityPage {
  entries: ActivityEntry[]; next: number | null; newest: number | null;
  facets?: { users: { id: number; username: string; deleted: boolean }[] };
  settings: ActivitySettings;
}
export type VisitPage = 'home' | 'study' | 'banks' | 'users' | 'logs' | 'records' | 'record' | 'wrong' | 'settings' | 'profile' | 'about'
  | 'quiz' | 'result' | 'ai' | 'aiSettings';

export type AiSendMode = 'send' | 'regenerate' | 'edit';

/** v1.9: the study slot is split into hint and explain */
export type AiSlot = 'default' | 'chat' | 'hint' | 'explain' | 'summary' | 'assess' | 'generate' | 'import';
export type AiState = 'setup' | 'off' | 'on' | 'paused';
export interface AiKeyState { set: boolean; hint: string; unreadable: boolean }
export interface AiConnection {
  id: number; rev: number; name: string; provider: 'openai' | 'anthropic' | 'gateway'; baseUrl: string; host: string;
  format: AiFormat; gatewayAuth: 'bearer' | 'x-api-key' | 'cf-aig-authorization';
  keys: { apiKey: AiKeyState; gatewayKey: AiKeyState };
  status: 'ok' | 'broken'; errorCode: string | null; testedAt: number | null; listsModels: boolean;
  insecureHttp: boolean; usedBy: AiSlot[];
  /** v1.9: the on / off switch of the connection (missing = on) */
  enabled?: boolean;
  /** v2.1: the thinking-strength family of the connection (one connection = one family) */
  effortFamily: AiEffortFamily;
}
export interface AiPoolModel {
  id: number; connectionId: number; connectionName: string; model: string; manual: boolean;
  status: 'ok' | 'untested' | 'failed'; errorCode: string | null; testedAt: number | null;
  latencyMs: number | null; context: number | null; usedBy: AiSlot[];
}
/** v1.6: the thinking-strength families and the choice of one slot (null / missing = default, nothing sent) */
export type AiEffortFamily = 'anthropic' | 'openai' | 'google' | 'responses';
export interface AiEffort { family: AiEffortFamily; level: string }
/** `effort` is optional: an older ai-service does not send it */
export interface AiSlotState { modelId: number | null; using: { modelId: number; fallback: 'none' | 'default' } | null; effort?: AiEffort | null;
  /** v2.2: what is sent now: the stored choice, or null when nothing is sent (Default) */
  effortEffective: AiEffort | null;
  /** v2.2: the strength family of the row's target (also when its connection is switched off); null when none; absent on older answers */
  effortFamily?: AiEffortFamily | null }
/** `GET /api/ai/admin/settings` (administrators; v1.3: connections, the model pool, six slots, global feature switches) */
export interface AiSettings {
  enabled: boolean; effective: boolean; state: AiState; pauseReason: 'keyProblem' | 'noRoute' | null;
  keyProblem: string | null; retentionDays: number; updatedAt: number; updatedBy: string;
  defaults: { openai: string; anthropic: string }; maxConnections: number;
  connections: AiConnection[]; pool: AiPoolModel[]; slots: Record<AiSlot, AiSlotState>;
  featureSwitches: Record<AiFeatureKey, boolean>;
  /** v1.6: the levels per family; missing on an older ai-service (then the thinking-strength row is hidden) */
  effortLevels?: Record<AiEffortFamily, string[]>;
}
export interface AiConnectionSave {
  id?: number; rev?: number; name: string; provider: string; baseUrl: string; format?: string; gatewayAuth?: string;
  apiKey?: string | null; gatewayKey?: string | null; reuseKeysFrom?: number; confirmHost?: string;
  /** v2.1: exact lowercase family; omitted = inferred on create, unchanged on update */
  effortFamily?: AiEffortFamily;
}
export interface AiTestAnswer {
  ok: boolean; saved?: boolean; connectionId?: number; latencyMs?: number | null; listed?: boolean; modelCount?: number;
  tests?: { model: string; ok: boolean; latencyMs?: number; code?: string }[];
  code?: string | null; httpStatus?: number | null; failedModel?: string | null; settings: AiSettings;
}
export interface AiModelList {
  connections: { id: number; name: string; provider: string; status: 'ok' | 'broken'; enabled?: boolean; listed: boolean;
    error: string | null; models: { id: string; context: number | null; poolId: number | null; status: string | null }[] }[];
}
export interface GenItem { no: number; kind: 'single' | 'multi' | 'multiple'; stem: string;
  options: { text: string; correct: boolean }[]; explanation: string }
export interface GenJobAnswer { job: GenJob; items?: GenItem[]; last?: number; gone?: number[] }
