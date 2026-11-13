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
  username: string;
  role: Role;
  /** The default or initial password must be replaced before anything else works */
  mustChangePassword: boolean;
  /** v0.6.5: may open Question banks (import new banks, update and delete their own); always true for administrators */
  canManageBanks: boolean;
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
}

export type AiFeatureKey = 'chat' | 'hint' | 'explain' | 'summary' | 'generate' | 'import';

export type GenSource = 'bank' | 'scope' | 'wrong' | 'file';
export type GenState = 'queued' | 'running' | 'done' | 'failed' | 'cancelled';
export type GenPhase = 'reading' | 'confirm' | 'writing' | 'saving';
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
}

export type AiChatKind = 'chat' | 'practice' | 'exam' | 'summary';
export type AiMessageKind = 'chat' | 'hint' | 'explain' | 'summary';

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
  model: string | null;
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
export type AiFormat = 'openai' | 'anthropic';
export type AiGatewayAuth = 'bearer' | 'x-api-key' | 'cf-aig-authorization';
/** The two key slots of the AI settings */
export type AiKeySlot = 'apiKey' | 'gatewayKey';

/** A saved key as the settings show it: never the key itself */
export interface AiKeyInfo {
  set: boolean;
  /** "…1234" for keys of 12 characters or more, else "" */
  hint: string;
  /** Saved, but it cannot be decrypted with the current encryption key */
  unreadable: boolean;
}

export type AiKeyProblem = null | 'mismatch' | 'invalidEnv' | 'invalidFile';

/** `GET /api/ai/admin/settings` (administrators) */
export interface AiSettings {
  provider: AiProvider;
  baseUrl: string;
  format: AiFormat;
  gatewayAuth: AiGatewayAuth;
  model: string;
  manualModel: boolean;
  enabled: boolean;
  verified: boolean;
  verifiedAt: number | null;
  retentionDays: number;
  keys: Record<AiKeySlot, AiKeyInfo>;
  keyProblem: AiKeyProblem;
  updatedAt: number;
  updatedBy: string;
  defaults: { openai: string; anthropic: string };
  /** The answer of a save only: the change switched AI off until the new settings pass a test */
  switchedOff?: boolean;
}

/** `POST /api/ai/admin/settings`: every field optional (absent = keep); a key: string = replace, null = remove */
export interface AiSettingsUpdate {
  provider?: AiProvider;
  baseUrl?: string;
  format?: AiFormat;
  gatewayAuth?: AiGatewayAuth;
  apiKey?: string | null;
  gatewayKey?: string | null;
  model?: string;
  manualModel?: boolean;
  retentionDays?: number;
}

/** `POST /api/ai/admin/models` */
export interface AiModels {
  models: string[];
  /** false: the service does not list its models (the model id is entered by hand) */
  listed: boolean;
}

/** `POST /api/ai/admin/verify` */
export interface AiVerify {
  ok: boolean;
  code: string | null;
  httpStatus: number | null;
  latencyMs: number;
  model: string;
  verifiedAt: number | null;
  enabled: boolean;
  switchedOff: boolean;
}

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

/** `GET /api/ai/admin/usage?days=N` */
export interface AiUsage {
  days: number;
  from: string;
  to: string;
  unit: 'day' | 'month';
  totals: AiUsageCounts;
  /** `userId: null` is the one row of deleted users */
  byUser: Array<AiUsageCounts & {
    userId: number | null;
    username: string | null;
    lastAt: number | null;
    /** v1.0: this user's counted calls since today's 0:00 of the server's time zone (the number the day cap compares) */
    today?: number;
  }>;
  byModel: Array<AiUsageCounts & { model: string }>;
  byPeriod: Array<AiUsageCounts & { period: string }>;
  /** v0.9: chat, hint, explain, summary in server order */
  byPurpose?: Array<AiUsageCounts & { purpose: 'chat' | 'hint' | 'explain' | 'summary' | 'generate' | 'import' }>;
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
  importMaxQuestions?: number;
}

/** `GET` / `POST /api/ai/admin/limits` */
export interface AiLimitsAnswer {
  limits: AiLimits;
  defaults: AiLimits;
  ranges: Record<keyof AiLimits, [number, number]>;
}
