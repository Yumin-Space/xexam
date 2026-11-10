[English](../en/development.md) | **日本語** | [简体中文](../zh-CN/development.md)

# 開発

> 日常的にコードを変更したりテストを実行したりするためのコマンドと、各ファイルの置き場所をまとめています。アーキテクチャは [architecture.md](architecture.md)、当初のテスト方針は [test-strategy.md](../zh-CN/history/test-strategy.md) を参照してください。

## 1. ツール

| ツール | バージョン | 説明 |
|---|---|---|
| Rust | 1.99.1（`backend/rust-toolchain.toml` で固定） | `rustup` が自動でインストールします。`rusqlite` の `bundled` 機能には C コンパイラが必要です（Linux：`build-essential`、Windows：MinGW gcc または MSVC） |
| Node | 24 | フロントエンド。単体テストは Node 標準のテストランナーを `--experimental-strip-types` 付きで使います |
| Python | 3.8+ | `tests/integration/` の統合スクリプトにのみ使います。**標準ライブラリのみ**を使います |
| Docker | Compose v2 | スタック全体のチェック、イメージ |

## 2. バックエンド

```bash
cd backend
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 181 件のテスト
cargo run -- serve --data ../data --no-browser     # 127.0.0.1:8765 で待ち受け
```

- テストは `tests/baseline/` にコミット済みの**旧版スナップショット**と照合するため、Python は不要です。
- すべてのテストは、一時ディレクトリ内の新規データベースか `tests/baseline/fixtures/sample.db` のコピーを使い、`data/` には**触れません**。v0.5 の挙動（スキーマの移行とバックアップ、問題集、学習記録、設定、新しいルートのガード）のテストは `backend/tests/v05_banks_records_settings.rs` にあります。
- よく使う環境変数：`XEXAM_DATA`、`XEXAM_PORT`、`XEXAM_ALLOWED_HOSTS`、`XEXAM_LOG`（ログレベル、例：`debug`）。全一覧は `xexam-backend --help` と [deployment.md](deployment.md) を参照してください。
- コマンドラインからのインポート（取り込み先はレガシー問題集）：`cargo run -- import ../some.xlsx --data ../data`。
- スキーマ 1 のデータベース（v0.4 以前）をバックエンドで開くと移行が行われ、その隣に `backups/` が書き出されます。試すときはコピーを使ってください。

## 3. フロントエンド

```bash
cd frontend
npm ci
npm run typecheck      # アプリを tsc でチェックし、続いてテストファイルをチェック（tsconfig.test.json）
npm test               # 44 件の単体テスト：採点と出題する問題の選び方、言語ルール、保存した試験、UI カタログ、サーバーメッセージの翻訳、README の更新履歴の解析
npm run build          # 型チェック + vite build → dist/
npm run dev            # http://127.0.0.1:5173、/api は http://127.0.0.1:8765 へプロキシ
```

- `npm run build` と `npm run dev` は、リポジトリのルートから 3 つの README（`README.md`、`README/README.ja.md`、`README/README.zh-CN.md`）を読みます。`vite.config.ts` のプラグイン `xexam-changelog` が、その更新履歴を「Xexam について」のページ（`#/about`）に組み込むためです。README のどれかに `### vX.Y — 日付` の項目が 1 つもないと、ビルドは失敗します。`npm run dev` の実行中は、README の変更も監視します。
- 開発時は `localhost` か `127.0.0.1` でページを開いてください。Vite プロキシの `changeOrigin` は `false` にする必要があります：バックエンドは `Host` と `Origin` を比較するため、`Host` が書き換えられると書き込みリクエストが 403 になります。
- バックエンドが既定のアドレスにない場合：`XEXAM_BACKEND=http://127.0.0.1:9000 npm run dev`。
- テストファイルを 1 つだけ実行するには：`node --experimental-strip-types --test src/i18n/server-messages.test.ts`。
- インポートパスには `.ts` 拡張子を使います（Node 標準のテストランナーの要件です）。
- TypeScript は `~5.9.3` に固定しています。

### 3.1 画面の文言

ユーザーに見える文言は、すべてカタログから取り出します。ページに直接書き込む文言はありません。

- **場所**：`src/i18n/catalog/<area>.ts`。領域ごとに 1 ファイルで、`common`、`home`、`banks`、`records`、`settings`、`quiz`、`result`、`about` があります。各ファイルの中身は `export default defineCatalog({ en: {…}, zh: {…}, ja: {…} })` です。
- **キー**は領域名で始まります：`banks.delete.title`、`records.deleteAll.ok`。共通の接頭辞（`app.`、`nav.`、`common.`、`err.`、`kind.`、`mode.`、`act.` など）は `common.ts` にあります。
- **必ず 3 言語。** `zh` や `ja` のブロックに英語のキーが欠けていると、TypeScript がエラーにします。`_one` で終わるキーは英語の単数形で、`tn()` は件数が 1 のときにこれを使います。中国語と日本語では省略できます。
- **プレースホルダー**は `{name}` の形です：`t('records.deleteOne.body', { at, pct })`。件数には `tn(key, n)` を使います。これが `{n}` を埋めます。
- **チェック**：`src/i18n/catalog.test.ts` は次の点を確認します。3 言語のキーとプレースホルダーが完全に一致していること、空の文言がないこと、すべてのキーに領域の接頭辞が付いていて 1 つのカタログにだけあること、すべてのカタログファイルが `src/i18n/index.ts` に登録されていること。
- **カタログファイルを新しく作るとき**：`src/i18n/index.ts` でインポートし、`ALL` と `MsgKey` 型に追加します。
- 英語を先に書き、そのあと中国語と日本語を書きます。用語は UI ですでに使っているものに合わせてください（例：「問題集」「出題停止」「間違いノート」）。

### 3.2 バックエンドのメッセージを UI に表示する

バックエンドのエラーメッセージは中国語で、内容は変わりません（テスト済みの API の一部だからです）。フロントエンドがそれを翻訳します：

- `src/i18n/server-messages.ts`：`EXACT` はメッセージ全体を、`PATTERNS` はバックエンドの書式文字列を対応付けます（書式文字列の `{}` の部分は、取り出す値になります）。UI が中国語のときはメッセージをそのまま表示します。未知の文言もそのまま表示します。いくつかの低レベルなプロトコルエラーは英単語です（`not found`、`bad json` など）。これらには、中国語を含むすべての言語で文を用意しています。
- `src/i18n/errors.ts`：`errorText(e)` は、どんなエラーも現在の言語のテキストに変換します。ページはエラーを表示するとき、必ずこれを使います。ページ自身のエラーは `LocalError(key, params)` にするので、UI の言語を切り替えると一緒に切り替わります。
- **ずれの検出テスト**：`src/i18n/server-messages.test.ts` は、小さな字句解析器で `backend/src` の Rust ソースを読みます。中国語または日本語の文字を含む文字列リテラル（コメント、テストコード、アサーション、ログやコンソール出力を除く）には、英語と日本語の訳が必要です。訳にはプレースホルダーに入る値がそのまま残っていなければならず、中国語が含まれていてはいけません。対応するバックエンドの文言がもう存在しない訳があっても、テストは失敗します。
- **バックエンドのメッセージを追加・変更するとき**：同じ変更の中で `server-messages.ts` に訳を追加してください。ユーザーに表示されない文字列（たとえばパーサーが探す語）であれば、テストの `NOT_MESSAGES` に理由と一緒に追加します。

### 3.3 ページやルートを追加する

1. **ルート**：`src/router.ts` で、`Route` に分岐を 1 つ追加し、`parseHash` に case を追加します。ハッシュが `#/<name>` の形でない場合は、`routeHash` にも追加します。メニューのどの項目を強調表示するかを `sectionOf` に書きます（`null` = なし）。
2. **ページ**：`src/pages/<name>.ts` を作り、`HTMLElement` を返す `render<Name>()` 関数を書きます。DOM は `src/utils/dom.ts` の `h()` で組み立てます。スタイルは `src/styles/<name>.css` に書いてページの先頭でインポートし、色には `main.css` のカラートークンを使います。そうすればダークテーマでも正しく表示されます。
3. **つなぎ込み**：`src/main.ts` の `page()` に case を追加し、ブラウザのタブに出すタイトルを指定します。ページ自身の見出しは、ページの中で `pageHead()`（`src/components/common.ts`）を使って書きます。
4. **メニュー項目**（主要なセクションの場合のみ）：`src/router.ts` の `NavSection` に追加し、`src/components/shell.ts` の `NAV`、`HREF`、アイコン用の `ICONS` にも追加します。`NAV` の項目は、三本線のボタンで開くドロップダウンメニューの、区切り線より上に並びます。区切り線の下の「Xexam vX.Y」（`#/about`）は `NAV` には入っていません。
5. **文言**：新しいカタログ `src/i18n/catalog/<name>.ts` を、3 言語すべてで作ります（3.1 節）。
6. **状態**：ページ同士は互いにインポートしません。共有する状態は `src/state/store.ts` に置き、変更は `src/state/actions.ts` の関数（`navigate(route)`、`render()` など）を通して行います。そのページだけが使う状態は、ページモジュールの変数に置きます。ユーザーがページを離れるときや開いたときに処理を実行するには、`onRouteChange()` を使います。
7. **API**：新しいエンドポイントには、`src/api/` に型付きのラッパーを、`src/types/api.ts` にその型を追加し、[api.md](api.md) と [openapi.yaml](../en/openapi.yaml) に記載します。

## 4. スタック全体

```bash
# 2 つのコンテナをビルドして起動（同じ xexam_data ボリュームを使います！自分のデータが使われるので、試す場合は別のプロジェクト名を使ってください：-p some-other-name）
docker compose -f docker/docker-compose.build.yml up -d --build

# チェック用スクリプトはすべて一時的なプロジェクト名と一時ボリュームを使い、xexam_data には触れません
docker build -f nginx/Dockerfile -t xexam-frontend:test .
python -I tests/integration/nginx_check.py xexam-frontend:test   # 38 項目、ダミーのバックエンドを使用
python -I tests/integration/stack_check.py                      # 34 項目、スタック全体を起動（2 つのイメージをビルドします）
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend         # 10 項目：旧 progress.json の移行（先に cargo build が必要）
```

- compose ファイルと `.env` は `docker/` にあり、`docker/docker-compose.build.yml` はリポジトリのルート（`..`）をビルドコンテキストにします。フロントエンドのイメージは、リポジトリのルートをビルドコンテキストにしてビルドします。`nginx/Dockerfile` は、ビルド段階で `COPY README.md /` と `COPY README/README.ja.md README/README.zh-CN.md /README/` を実行します。`vite.config.ts` が、「Xexam について」のページの更新履歴をそこから読むためです。`.dockerignore` は `docs/` などを除外しますが、この 3 つの README（`README.md` と `README/`）は除外しません。README を除外に戻すと、イメージのビルドが失敗します。

## 5. 旧版との照合（回帰）

旧版の Python 実装はリポジトリから削除されましたが、その挙動は `tests/baseline/` に固定されています（すべて架空の合成データです）：

| スナップショット | 内容 | 使うもの |
|---|---|---|
| `golden/NNN_*.json`（172 件） | 旧版の各エンドポイント × 各状態の実際のレスポンス（ステータスコード、安定したレスポンスヘッダー、正規化後の本文） | `backend/tests/api_golden.rs` が順番に新しいルートへ再生し、項目ごとに比較する |
| `fixtures/*.xlsx` + `parser_golden/*.json`（49 件） | 合成ワークブックと、それぞれに対する旧パーサーの出力 | `backend/tests/parser_golden.rs` |
| `golden/id_vectors.json`（85 件のアサーション） | 問題 id のアサーション。各件にハッシュ対象のバイト列が付く | `backend/tests/id_vectors.rs` |
| `fixtures/sample.db` + `golden/store_sample_outputs.json` | 旧実装が書き出したサンプルデータベースと、その各インターフェースの出力 | `backend/tests/db_golden.rs`（スキーマ v2 への移行も確認する） |
| `golden/allowed_differences_rust.json`（23 項目）、`allowed_probe_differences_rust.json`（51 項目） | 旧版と意図的に異なる箇所。各項目に理由が 1 文付く | `api_golden.rs` で登録されていない差異 = 失敗 |

- 挙動を**変更**したい場合（たとえば意図的な差異を新しく加える）：コードを変更し、`allowed_differences_rust.json` に登録して理由を明記します。ゴールデンレスポンスを直接変更しないでください。
- 旧版のキャプチャ / 照合 / プローブ用スクリプトは旧 Python コードに依存していたため、コードと共に削除されました。必要な場合は、Git タグ `python-v0.2` とコミット履歴から取り戻してください。

## 6. データ保護のルール

- `data/`、`progress.json`、ご自身の問題集の Excel ファイルを、読み取る・出力する・コミットする・変更する、のいずれも**しないでください**。
- 試す操作は必ずコピーか一時ディレクトリで行ってください。本物の `xexam.db` に対して「どうなるか見てみる」ようなコマンドを実行しないでください。古いデータベースを v0.5 で開くと移行される点にも注意してください。
- `docker compose down -v` を実行しないでください。
- コード、テスト、ドキュメントの例には架空の内容だけを使い、本物の試験問題は使わないでください。

## 7. CI

パイプラインは 1 つだけです：[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。`main` と `dev` へのプッシュ、`main` へのプルリクエスト、手動での起動のときに実行されます。

| ジョブ | 内容 |
|---|---|
| `check` | `VERSION` の形式。3 つの README それぞれの更新履歴の最新の項目 = `VERSION`。`backend/Cargo.toml` と `frontend/package.json` のバージョンが同じであること。配置（ルートの README は `README.md` だけ、残り 2 つの README は `README/`、compose ファイルと `.env.example` は `docker/`）とドキュメントのチェック（8 節） |
| `frontend` | 型チェック、44 件の単体テスト、ビルド |
| `backend` | fmt、clippy、181 件のテスト、および `progress.json` の移行チェック |
| `stack` | `xexam-frontend` イメージをビルドして 38 項目の Nginx チェックを実行。compose スタック全体を起動して 34 項目のエンドツーエンドチェックを実行 |
| `arm64` | 2 つのイメージを arm64 向けにビルドし、QEMU 上で起動して、`/api/info` が `aarch64` と正しいバージョンを返すことを確認 |
| `publish` | `main` のみ：2 つのイメージ（amd64 + arm64）を同じタグで Docker Hub（`docker.io/yuminspace/`、公開。`docker/docker-compose.yml` が pull するのはこちら）と ghcr.io（非公開のミラー）にプッシュし、両方でマニフェストとタグを検証。リポジトリのシークレット `DOCKERHUB_USERNAME` と `DOCKERHUB_TOKEN`（必要に応じて変数 `DOCKERHUB_NAMESPACE`）が必要で、ない場合は何もプッシュせずに止まる |
| `release` | `main` のみ：`v<VERSION>` の GitHub Release を作成（または更新）。リリースノートは `README.md` の更新履歴から取る |

`dev` へのプッシュと `main` へのプルリクエストでは最初の 5 つのジョブだけを実行し、**公開は一切行いません**。公開されるのは `main` へのプッシュだけです。手動で起動した場合は、実行対象のブランチへのプッシュと同じ動きになります。

## 8. ドキュメント

- 言語ごとに 1 つのフォルダがあります：`docs/en`（英語、原文）、`docs/ja`（日本語）、`docs/zh-CN`（簡体字中国語）。どのフォルダにも同じ 7 つのドキュメントがあります：`api`、`architecture`、`database`、`deployment`、`development`、`migration`、`security`。
- `docs/zh-CN/history/` には、Rust へ切り替えたときの古いドキュメント（監査、計画、リスク評価）があります。中国語のみで、今後は更新しません。
- `docs/en/openapi.yaml` はすべての言語で共有します。日本語と中国語のドキュメントは `../en/openapi.yaml` にリンクします。
- どのドキュメントも、ほかの 2 つのフォルダにある同じドキュメントへのリンクを並べた言語バーで始まります。リンクはファイル自身のフォルダからの相対パスで、リポジトリのルートは `../../` です。
- 先に英語のドキュメントを変更し、同じ変更の中で日本語と中国語のドキュメントも更新します。
- CI の `check` ジョブは、次のどれかに当てはまると失敗します：7 つのドキュメントのどれかが言語フォルダにない、`docs/` に `en`、`ja`、`zh-CN` 以外のフォルダやファイルがある、README やドキュメントの相対リンクや画像のパスが存在しないファイルを指している。GitHub のページへのリンク（`../../releases` など）も確認します。これらは `../` でちょうどリポジトリのページまで戻る必要があります（`README/` の中では `../../../` で始まります）。

## 9. バージョン、更新履歴、ライセンス

- バージョン番号の唯一の情報源は、リポジトリのルートにある `VERSION`（`0.5.1`）です。`backend/Cargo.toml` と `frontend/package.json` にもそれぞれコピー（`0.5.1`）があり、CI が一致を確認します。
- 更新するたびに、3 つの README（`README.md`、`README/README.ja.md`、`README/README.zh-CN.md`）すべての更新履歴の一番上に項目を 1 つ追加し、バージョン番号を `VERSION` と一致させます。CI がこれを確認します（各 README で最初に `### v` と数字で始まる行を読みます）。GitHub Release のノートは英語版から取ります。
- 同じ更新履歴は、「Xexam について」のページにも表示言語ごとに表示されます（ビルド時に組み込みます。3 節）。そのため、項目は簡単な Markdown で書きます：段落、`- ` の箇条書き、行頭を 2 文字下げた `- ` による入れ子の箇条書き、`**太字**`、インラインコード。リポジトリ内への相対リンクは、このページでは文字として表示されます。見出しは `### vX.Y — 日付` の形にします。
- 作業は `dev` で行い、`main` にはマージで取り込みます。`main` へのプッシュでイメージと Release が公開されます。
- このプロジェクトは MIT ライセンスです（リポジトリのルートの `LICENSE`）。
