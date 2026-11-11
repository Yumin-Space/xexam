// Translates the backend's user-facing messages into the current UI language.
//
// The backend's messages are Chinese (canonical; pinned by its golden tests). Chinese UI → the message unchanged;
// English / Japanese UI → looked up in EXACT (whole strings), then in PATTERNS (the backend's format strings, whose
// `{}` / `{name}` parts become capture groups that are put back into the translation). Unknown text is returned unchanged.
// A few low-level protocol errors are English words ("not found", "bad json", ...); they get a sentence in every language.
//
// server-messages.test.ts reads the backend sources and fails when a Chinese message there has no translation here,
// so a new or changed backend message must be added to this file (or, if it is not user-facing, to the test's skip list).
import type { UiLang } from '../types/api.ts';

type Target = 'en' | 'ja';
type Texts = Record<Target, string>;

// Excel's own name of the file type in its "Save as type" list, as the user sees it in that language.
const XLSX_EN = '“Excel Workbook (*.xlsx)”';
const XLSX_JA = '「Excel ブック (*.xlsx)」';

/** Whole messages. Keys are the backend texts exactly (see the Rust constant named in each comment). */
const EXACT: ReadonlyMap<string, Texts> = new Map<string, Texts>([
  // ---- error.rs: ImportError (upload checks); MSG_UPLOAD_INCOMPLETE in api/response.rs has the same text as Empty
  ['文件是空的，或者没有上传完整。', {
    en: 'The file is empty, or it was not uploaded completely.',
    ja: 'ファイルが空か、アップロードが最後まで完了していません。',
  }],
  ['这是旧版 .xls 格式，或者是设置了密码的 Excel 文件。请在 Excel 里去掉密码，并另存为「Excel 工作簿(.xlsx)」再上传。', {
    en: `This is an old .xls file or a password-protected Excel file. Remove the password in Excel, save the file as ${XLSX_EN} and upload it again.`,
    ja: `旧形式の .xls ファイルか、パスワードが設定された Excel ファイルです。Excel でパスワードを解除し、${XLSX_JA}として保存し直してからアップロードしてください。`,
  }],
  ['这个文件不是 .xlsx（可能是 .csv、.pdf 或别的格式）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。', {
    en: `This file is not an .xlsx file (it may be a .csv, a .pdf or another format). Save it in Excel as ${XLSX_EN} and upload it again.`,
    ja: `このファイルは .xlsx ではありません（.csv や .pdf など、別の形式の可能性があります）。Excel で${XLSX_JA}として保存し直してからアップロードしてください。`,
  }],
  ['这个 .xlsx 文件已损坏，打不开。请用 Excel 重新保存一份再上传。', {
    en: 'This .xlsx file is damaged and cannot be opened. Save a new copy with Excel and upload that copy.',
    ja: 'この .xlsx ファイルは壊れていて開けません。Excel で保存し直してからアップロードしてください。',
  }],
  ['这个文件里没有 Excel 工作簿的内容，不是有效的 .xlsx。', {
    en: 'This file does not contain an Excel workbook, so it is not a valid .xlsx file.',
    ja: 'このファイルには Excel ブックの内容が含まれていないため、有効な .xlsx ではありません。',
  }],
  ['文件解压后太大，已拒绝。', {
    en: 'The file is too large once decompressed, so it was rejected.',
    ja: 'ファイルを展開したときのサイズが大きすぎるため、拒否しました。',
  }],
  ['文件里含有不被允许的内容（XML 实体声明），已拒绝。正常的 Excel 文件不会有这个。', {
    en: 'The file contains something that is not allowed (an XML entity declaration), so it was rejected. Normal Excel files never contain this.',
    ja: 'ファイルに許可されていない内容（XML のエンティティ宣言）が含まれているため、拒否しました。通常の Excel ファイルにこのような内容は含まれません。',
  }],
  // ImportError::Timeout and services MSG_PARSE_TIMEOUT
  ['解析这个文件用的时间太长，已经停止。请确认选对了文件，或者换一份小一点的再试。', {
    en: 'Reading this file took too long, so it was stopped. Check that you chose the right file, or try a smaller one.',
    ja: 'このファイルの読み取りに時間がかかりすぎたため、中止しました。正しいファイルを選んだか確認するか、もっと小さいファイルで試してください。',
  }],
  // ImportError::Unexpected (its Display; the HTTP layer sends the longer MSG_UNEXPECTED below)
  ['读取这个 Excel 时出错', {
    en: 'An error occurred while reading this Excel file.',
    ja: 'この Excel ファイルの読み取り中にエラーが発生しました。',
  }],
  // error.rs From<ImportError> and services MSG_UNEXPECTED
  ['读取这个 Excel 时出错：文件内容不符合预期', {
    en: 'An error occurred while reading this Excel file: its content is not what was expected.',
    ja: 'この Excel ファイルの読み取り中にエラーが発生しました：ファイルの内容が想定と異なります。',
  }],

  // ---- api/response.rs
  ['文件太大了（最大 20 MB）。题库通常只有 1–2 MB，请确认选对了文件。', {
    en: 'The file is too large (20 MB at most). A question bank is usually only 1–2 MB, so check that you chose the right file.',
    ja: 'ファイルが大きすぎます（最大 20 MB）。問題集は通常 1〜2 MB ほどです。正しいファイルを選んだか確認してください。',
  }],
  ['这次上传的预览已经过期或已被处理，请重新上传。', {
    en: 'The preview of this upload has expired or has already been handled. Please upload the file again.',
    ja: 'このアップロードのプレビューは有効期限が切れたか、すでに処理されています。もう一度アップロードしてください。',
  }],
  ['在 Docker 里请用 docker compose stop 停止容器。', {
    en: 'When Xexam runs in Docker, stop the containers with docker compose stop.',
    ja: 'Docker で動かしている場合は、docker compose stop でコンテナを停止してください。',
  }],
  ['服务器内部出错，请稍后再试。', {
    en: 'An internal server error occurred. Please try again later.',
    ja: 'サーバー内部でエラーが発生しました。しばらくしてから再試行してください。',
  }],
  // v0.5 (MSG_BANK_NOT_FOUND, MSG_RECORD_NOT_FOUND, MSG_BAD_TARGET, MSG_BAD_REQUEST, MSG_BAD_SETTINGS)
  ['这个题库不存在（可能已被删除），请刷新页面后再试。', {
    en: 'This question bank does not exist (it may have been deleted). Reload the page and try again.',
    ja: 'この問題集は存在しません（削除された可能性があります）。ページを再読み込みしてから再試行してください。',
  }],
  ['这条考试记录不存在（可能已被删除），请刷新页面后再试。', {
    en: 'This exam record does not exist (it may have been deleted). Reload the page and try again.',
    ja: 'この試験記録は存在しません（削除された可能性があります）。ページを再読み込みしてから再試行してください。',
  }],
  ['上传目标不对，请刷新页面后再试。', {
    en: 'The upload target is not valid. Reload the page and try again.',
    ja: 'アップロード先が正しくありません。ページを再読み込みしてから再試行してください。',
  }],
  ['请求的格式不对。', {
    en: 'The request is not in the expected format.',
    ja: 'リクエストの形式が正しくありません。',
  }],
  ['设置的值不对。', {
    en: 'A setting has an invalid value.',
    ja: '設定の値が正しくありません。',
  }],
  // v0.6, users and sign-in (MSG_SIGN_IN_REQUIRED … MSG_WRONG_PASSWORD)
  ['请先登录。', {
    en: 'Please sign in first.',
    ja: '先にログインしてください。',
  }],
  ['只有管理员可以进行这个操作。', {
    en: 'Only an administrator can do this.',
    ja: 'この操作は管理者だけが行えます。',
  }],
  ['用户名或密码不对。', {
    en: 'The username or password is not correct.',
    ja: 'ユーザー名またはパスワードが正しくありません。',
  }],
  ['登录失败次数太多，请稍后再试。', {
    en: 'Too many failed attempts. Please try again later.',
    ja: '失敗が多すぎます。しばらくしてから、もう一度お試しください。',
  }],
  ['请先设置新密码。', {
    en: 'Please choose a new password first.',
    ja: '先に新しいパスワードを設定してください。',
  }],
  ['新密码不能和原来的一样。', {
    en: 'The new password must be different from the old one.',
    ja: '新しいパスワードは以前のものと別にしてください。',
  }],
  ['用户名不对：1–40 个字符，只能用字母、数字和 . _ - @。', {
    en: 'The username is not valid: it needs 1–40 characters, using only letters, digits and . _ - @.',
    ja: 'ユーザー名が正しくありません。1〜40 文字で、英字・数字と . _ - @ だけが使えます。',
  }],
  ['密码需要 8–128 个字符。', {
    en: 'The password needs 8 to 128 characters.',
    ja: 'パスワードは 8〜128 文字にしてください。',
  }],
  ['这个用户名已经有人用了。', {
    en: 'This username is already taken.',
    ja: 'このユーザー名はすでに使われています。',
  }],
  // v0.8 round 2 (MSG_AI_SERVICE_OFF, 409 with code ai.serviceOff): a user's AI+ feature switched while AI+ is not available;
  // the same words as the catalog's ai.err.serviceOff
  ['请先在 AI 设置里开启 AI+ 功能。', {
    en: 'Turn on the AI+ feature in AI settings first.',
    ja: '先に AI 設定で AI+ 機能をオンにしてください。',
  }],
  ['这个用户不存在（可能已被删除），请刷新页面后再试。', {
    en: 'This user does not exist (it may have been deleted). Reload the page and try again.',
    ja: 'このユーザーは存在しません（削除された可能性があります）。ページを再読み込みしてから再試行してください。',
  }],
  ['至少要保留一个启用的管理员。', {
    en: 'At least one enabled administrator must remain.',
    ja: '有効な管理者を少なくとも 1 人残す必要があります。',
  }],
  ['不能对自己的账户做这个操作。', {
    en: 'You cannot do this to your own account.',
    ja: '自分のアカウントにはこの操作を行えません。',
  }],
  ['当前密码不对。', {
    en: 'The current password is not correct.',
    ja: '現在のパスワードが正しくありません。',
  }],
  // v0.6.5: MSG_EXAM_QUESTIONS_GONE (POST /api/attempt, 409: none of the exam's questions exist any more)
  ['这场考试的题目已经被删除，成绩没有保存。', {
    en: 'The questions of this exam have been deleted, so the result was not saved.',
    ja: 'この試験の問題は削除されたため、結果は保存されませんでした。',
  }],
  // v0.6.5: MSG_NO_BANK_PERMISSION (403: bank management routes, plain user without the "can manage question banks" permission)
  ['你没有管理题库的权限，请联系管理员。', {
    en: 'You do not have permission to manage question banks. Ask an administrator.',
    ja: '問題集を管理する権限がありません。管理者に依頼してください。',
  }],
  // v0.6.5: MSG_NOT_BANK_OWNER (403: a bank manager updating or deleting a bank someone else uploaded)
  ['这个题库不是你上传的，只能更新或删除你自己上传的题库。', {
    en: 'You did not upload this question bank. You can only update or delete question banks you uploaded yourself.',
    ja: 'この問題集はあなたがアップロードしたものではありません。更新や削除ができるのは、自分でアップロードした問題集だけです。',
  }],
  // v0.6.5: MSG_UNKNOWN_BANKS (400: "banks" of POST /api/users/create or /api/users/update names a bank that does not exist)
  ['选择的题库中有不存在的（可能已被删除），请刷新页面后再试。', {
    en: 'One of the chosen question banks does not exist (it may have been deleted). Refresh the page and try again.',
    ja: '選んだ問題集の中に存在しないものがあります（削除された可能性があります）。ページを再読み込みしてからもう一度お試しください。',
  }],

  // ---- services/mod.rs
  ['这份文件里没有读到任何可用的题，没有导入。', {
    en: 'No usable questions were found in this file, so nothing was imported.',
    ja: 'このファイルから使える問題を 1 問も読み取れなかったため、インポートしていません。',
  }],
  ['写入数据库失败，题库没有任何改动。', {
    en: 'Writing to the database failed. The question bank was not changed.',
    ja: 'データベースへの書き込みに失敗しました。問題集は一切変更されていません。',
  }],
  // v0.7: MSG_OLD_LAYOUT (400: upload or import confirm of a workbook in the old 3- or 4-column layout)
  ['只能导入六列格式（A–C 列：英文、中文、日文题目；D–F 列：英文、中文、日文答案）。这个文件是旧的三列或四列格式，请改成六列格式后再导入。', {
    en: 'Only the six-column layout can be imported (columns A–C: the question in English, Chinese and Japanese; columns D–F: the answer in English, Chinese and Japanese). This file uses the old three- or four-column layout. Rewrite it in the six-column layout and import it again.',
    ja: '取り込めるのは 6 列形式だけです（A〜C 列：英語・中国語・日本語の問題文、D〜F 列：英語・中国語・日本語の解答）。このファイルは古い 3 列または 4 列形式です。6 列形式に直してから、もう一度インポートしてください。',
  }],

  // ---- api/attempt.rs
  ['成绩的格式不对', {
    en: 'The exam result is not in the expected format.',
    ja: '成績の形式が正しくありません。',
  }],
  ['没有成绩可保存', {
    en: 'There is no result to save.',
    ja: '保存する成績がありません。',
  }],

  // ---- parser/workbook.rs
  ['文件里的 XML 读不了：格式不正确', {
    en: 'The XML inside the file cannot be read: it is malformed.',
    ja: 'ファイル内の XML を読み取れません：形式が正しくありません。',
  }],
  ['这个文件不是有效的 .xlsx（可能是 .xls、.csv，或者文件已损坏）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。', {
    en: `This file is not a valid .xlsx file (it may be an .xls or .csv file, or it may be damaged). Save it in Excel as ${XLSX_EN} and upload it again.`,
    ja: `このファイルは有効な .xlsx ではありません（.xls や .csv であるか、ファイルが壊れている可能性があります）。Excel で${XLSX_JA}として保存し直してからアップロードしてください。`,
  }],
  ['这个文件的内容太多或层级太深，已拒绝。', {
    en: 'This file has too much content or is nested too deeply, so it was rejected.',
    ja: 'このファイルは内容が多すぎるか、階層が深すぎるため、拒否しました。',
  }],

  // ---- parser/question.rs (reasons why a row of the workbook was skipped, and one fatal error)
  ['没有找到选项（A、B、C…）或带「はい / いいえ」的小项', {
    en: 'No options (A, B, C…) and no sub-items ending in choices such as “はい / いいえ” were found',
    ja: '選択肢（A、B、C…）も、「はい / いいえ」などの選択肢が付いた小問も見つかりません',
  }],
  ['没有找到答案（C 列要写「【答案】…」）', {
    en: 'No answer was found (column C must contain “【答案】…”)',
    ja: '解答が見つかりません（C 列に「【答案】…」と書いてください）',
  }],
  ['答案里有超长的数字（超过 4300 位），已拒绝。', {
    en: 'The answer contains an extremely long number (more than 4300 digits), so the file was rejected.',
    ja: '解答に極端に長い数字（4300 桁超）が含まれているため、拒否しました。',
  }],

  // ---- parser/question.rs: the six-column layout (English is authoritative), unreadable rows and preview warnings
  ['这一行没有英文题目（六列格式以 A 列的英文题目为准）', {
    en: 'This row has no English question (in the six-column layout, the English question in column A is authoritative)',
    ja: 'この行には英語の問題文がありません（6 列形式では A 列の英語の問題文が基準です）',
  }],
  ['A 列的英文题目里没有找到选项（A、B、C…）或带「Yes / No」等选项的小项', {
    en: 'The English question in column A has no options (A, B, C…) and no sub-items ending in choices such as “Yes / No”',
    ja: 'A 列の英語の問題文に、選択肢（A、B、C…）も「Yes / No」などの選択肢が付いた小問も見つかりません',
  }],
  ['D 列没有找到英文答案（D 列要写「【Answer】…」）', {
    en: 'No English answer was found in column D (column D must contain “【Answer】…”)',
    ja: 'D 列に英語の解答が見つかりません（D 列に「【Answer】…」と書いてください）',
  }],
  ['E 列的中文答案和 D 列的英文答案不一致（已按英文答案导入）', {
    en: 'The Chinese answer in column E differs from the English answer in column D (imported with the English answer)',
    ja: 'E 列の中国語の解答が D 列の英語の解答と一致しません（英語の解答で取り込みました）',
  }],
  ['F 列的日文答案和 D 列的英文答案不一致（已按英文答案导入）', {
    en: 'The Japanese answer in column F differs from the English answer in column D (imported with the English answer)',
    ja: 'F 列の日本語の解答が D 列の英語の解答と一致しません（英語の解答で取り込みました）',
  }],
]);

/** English list of values the backend joined with "、" (Japanese keeps "、"). */
function enList(joined: string): string {
  return joined.split('、').join(', ');
}

function isPlural(joined: string): boolean {
  return joined.includes('、');
}

interface Pattern {
  /** The backend's format string, with `{}` / `{name}` where it inserts a value */
  template: string;
  re: RegExp;
  en: (parts: string[]) => string;
  ja: (parts: string[]) => string;
}

function escapeRe(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

const PLACEHOLDER = /\{[A-Za-z0-9_]*\}/;

function pattern(template: string, texts: Pick<Pattern, 'en' | 'ja'>): Pattern {
  const re = new RegExp(`^${template.split(PLACEHOLDER).map(escapeRe).join('([\\s\\S]*?)')}$`);
  return { template, re, ...texts };
}

/** Messages with values inserted by the backend, in matching order (the first one that matches wins). */
const PATTERNS: readonly Pattern[] = [
  // ---- api/response.rs: the plain-text 403 for a GET from a host that is not allowed (the English prefix stays as is)
  pattern('Forbidden host: {seen}。通过别的地址访问（局域网 IP、你的域名）时，请把它加进环境变量 XEXAM_ALLOWED_HOSTS，或设 XEXAM_ALLOW_ANY_HOST=1（只在前面有身份验证时）。', {
    en: ([seen = '']) => `Forbidden host: ${seen}. To open Xexam through another address (a LAN IP or your own domain), add that address to the environment variable XEXAM_ALLOWED_HOSTS, or set XEXAM_ALLOW_ANY_HOST=1 (only when an authentication layer sits in front).`,
    ja: ([seen = '']) => `Forbidden host: ${seen}。別のアドレス（LAN の IP アドレスや独自ドメイン）からアクセスする場合は、そのアドレスを環境変数 XEXAM_ALLOWED_HOSTS に追加するか、XEXAM_ALLOW_ANY_HOST=1 を設定してください（前段に認証がある場合のみ）。`,
  }),

  // ---- api/guard.rs: why a POST was rejected (values come from the request headers)
  pattern('被拒绝：这个请求的 Host 是「{}」，不在允许的列表里。请把它加进 XEXAM_ALLOWED_HOSTS（或设 XEXAM_ALLOW_ANY_HOST=1，只在前面有身份验证时）。', {
    en: ([host = '']) => `Denied: the Host of this request is “${host}”, which is not in the allowed list. Add it to XEXAM_ALLOWED_HOSTS (or set XEXAM_ALLOW_ANY_HOST=1, but only when an authentication layer sits in front).`,
    ja: ([host = '']) => `拒否されました：このリクエストの Host「${host}」は許可リストに含まれていません。XEXAM_ALLOWED_HOSTS に追加してください（または XEXAM_ALLOW_ANY_HOST=1 を設定してください。前段に認証がある場合に限ります）。`,
  }),
  pattern('被拒绝：这个请求的 Origin 是「{}」，不是一个网址。', {
    en: ([origin = '']) => `Denied: the Origin of this request, “${origin}”, is not a web address.`,
    ja: ([origin = '']) => `拒否されました：このリクエストの Origin「${origin}」は URL ではありません。`,
  }),
  pattern('被拒绝：这个请求来自「{}」，和服务看到的 Host「{}」不是同一个主机。如果你在 Cloudflare Tunnel 等反向代理后面且代理改写了 Host，请把「{}」加进 XEXAM_ALLOWED_ORIGINS。', {
    en: ([origin = '', host = '', add = '']) => `Denied: this request comes from “${origin}”, which is not the same host as the Host the service sees (“${host}”). If Xexam runs behind a reverse proxy such as Cloudflare Tunnel that rewrites the Host, add “${add}” to XEXAM_ALLOWED_ORIGINS.`,
    ja: ([origin = '', host = '', add = '']) => `拒否されました：このリクエストの送信元「${origin}」は、サービスが受け取った Host「${host}」と同じホストではありません。Cloudflare Tunnel などのリバースプロキシの背後にあり、プロキシが Host を書き換えている場合は、「${add}」を XEXAM_ALLOWED_ORIGINS に追加してください。`,
  }),

  // ---- parser/workbook.rs: a part every .xlsx needs is missing (the quoted English text is the zip library's own wording)
  pattern('这个文件缺少 .xlsx 必需的内容（"There is no item named \'{name}\' in the archive"），可能已损坏。', {
    en: ([name = '']) => `This file is missing content that every .xlsx file needs ("There is no item named '${name}' in the archive"). It may be damaged.`,
    ja: ([name = '']) => `このファイルには .xlsx に必須の内容がありません（"There is no item named '${name}' in the archive"）。壊れている可能性があります。`,
  }),

  // ---- parser/question.rs: why a row was skipped (values come from the workbook)
  pattern('答案「{}」里没有找到选项字母', {
    en: ([answer = '']) => `No option letter was found in the answer “${answer}”`,
    ja: ([answer = '']) => `解答「${answer}」に選択肢の記号が見つかりません`,
  }),
  pattern('答案 {} 不在选项 {} 里', {
    en: ([bad = '', keys = '']) => isPlural(bad)
      ? `The answer letters ${enList(bad)} are not among the options ${enList(keys)}`
      : `The answer letter ${bad} is not among the options ${enList(keys)}`,
    ja: ([bad = '', keys = '']) => `解答の ${bad} は選択肢 ${keys} にありません`,
  }),
  pattern('第 {k} 小项没有答案', {
    en: ([k = '']) => `Sub-item ${k} has no answer`,
    ja: ([k = '']) => `小問 ${k} の解答がありません`,
  }),
  pattern('第 {k} 小项的答案「{v}」不在选项「{}」里', {
    en: ([k = '', value = '', choices = '']) => `The answer “${value}” to sub-item ${k} is not one of its choices “${choices}”`,
    ja: ([k = '', value = '', choices = '']) => `小問 ${k} の解答「${value}」は選択肢「${choices}」にありません`,
  }),
  pattern('答案里有第 {joined} 项，但题目只有 {} 个小项', {
    en: ([joined = '', n = '']) => {
      const given = isPlural(joined) ? `entries for sub-items ${enList(joined)}` : `an entry for sub-item ${joined}`;
      return `The answer has ${given}, but the question has only ${n} ${n === '1' ? 'sub-item' : 'sub-items'}`;
    },
    ja: ([joined = '', n = '']) => `解答に小問 ${joined} の答えがありますが、この問題の小問は ${n} 個だけです`,
  }),
];

/**
 * Low-level protocol errors whose backend text is a few English words (wrong Content-Type, body too large, broken JSON,
 * unknown path, wrong method). They get a sentence in every UI language, Chinese included.
 */
const PROTOCOL: ReadonlyMap<string, Record<UiLang, string>> = new Map<string, Record<UiLang, string>>([
  ['bad content type', {
    en: 'The request was sent with an unsupported content type.',
    zh: '请求的内容类型不受支持。',
    ja: 'リクエストのコンテンツタイプがサポートされていません。',
  }],
  ['too large', {
    en: 'The request is too large.',
    zh: '请求的内容太大了。',
    ja: 'リクエストの内容が大きすぎます。',
  }],
  ['bad json', {
    en: 'The request is not valid JSON.',
    zh: '请求不是有效的 JSON。',
    ja: 'リクエストが有効な JSON ではありません。',
  }],
  ['not found', {
    en: 'The service does not recognize this request. Reload the page; if this keeps happening, make sure the frontend and the backend are the same version.',
    zh: '服务不认识这个请求。请刷新页面；如果一直这样，请确认前端和后端是同一个版本。',
    ja: 'サービスがこのリクエストを認識できません。ページを再読み込みしてください。繰り返し起きる場合は、フロントエンドとバックエンドが同じバージョンか確認してください。',
  }],
  ['method not allowed', {
    en: 'The service does not accept this kind of request.',
    zh: '服务不接受这种请求。',
    ja: 'サービスはこの種類のリクエストを受け付けていません。',
  }],
]);

export function localizeServerMessage(msg: string, lang: UiLang): string {
  const text = msg.trim();
  const protocol = PROTOCOL.get(text);
  if (protocol) return protocol[lang];
  if (lang === 'zh') return msg;
  const exact = EXACT.get(text);
  if (exact) return exact[lang];
  for (const p of PATTERNS) {
    const m = p.re.exec(text);
    if (m) return p[lang](m.slice(1).map((part) => part ?? ''));
  }
  return msg;
}

/** For tests: the backend texts this file knows (exact messages and the format strings of the patterns). */
export function knownServerMessages(): { exact: string[]; templates: string[]; protocol: string[] } {
  return { exact: [...EXACT.keys()], templates: PATTERNS.map((p) => p.template), protocol: [...PROTOCOL.keys()] };
}
