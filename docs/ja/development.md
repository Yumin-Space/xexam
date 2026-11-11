[English](../en/development.md) | **日本語** | [简体中文](../zh-CN/development.md)

# 開発

> 日常的にコードを変更したりテストを実行したりするためのコマンドと、各ファイルの置き場所をまとめています。アーキテクチャは [architecture.md](architecture.md)、当初のテスト方針は [test-strategy.md](../zh-CN/history/test-strategy.md) を参照してください。

## 1. ツール

| ツール | バージョン | 説明 |
|---|---|---|
| Rust | 1.99.1（`backend/rust-toolchain.toml` で固定） | `rustup` が自動でインストールします。`rusqlite` の `bundled` 機能には C コンパイラが必要です（Linux：`build-essential`、Windows：MinGW gcc または MSVC） |
| Node | 24 | フロントエンド。単体テストは Node 標準のテストランナーを `--experimental-strip-types` 付きで使います |
| Python | 3.8+ | `tests/integration/` の統合スクリプトと、`scripts/` の 2 つのスクリプトにのみ使います。**標準ライブラリのみ**を使います |
| Docker | Compose v2 | スタック全体のチェック、イメージ |

## 2. バックエンド

```bash
cd backend
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 302 件のテスト
cargo run -- serve --data ../data --no-browser     # 127.0.0.1:8765 で待ち受け
```

- テストは `tests/baseline/` にコミット済みの**旧版スナップショット**と照合するため、Python は不要です。
- すべてのテストは、一時ディレクトリ内の新規データベースか `tests/baseline/fixtures/sample.db` のコピーを使い、`data/` には**触れません**。`src/` の中の単体テストのほかに、次のテストファイルがあります：`api_basic.rs`、`api_import.rs`、`api_misc.rs`、`api_golden.rs`（ルート、ガード、旧版のレスポンス）、`db_golden.rs`（データベースを旧版のスナップショットと照合）、`parser_golden.rs`、`parser_hardening.rs`、`parser_four_columns.rs`、`parser_six_columns.rs`、`id_vectors.rs`（パーサー）、そしてバージョンごとのファイル：`v05_banks_records_settings.rs`（スキーマ v2、問題集、記録、設定）、`v06_users_and_sign_in.rs`（スキーマ v3、アカウント、セッション、ログインの制限）、`v065_bank_access_and_practice.rs`（スキーマ v4、問題集へのアクセス、問題集の管理権限、所有者、練習）、`v07_wal_and_logs.rs`（WAL モード、チェックポイント、サーバーの動作中の `reset-password`、システムログ）、`v07_log_floods.rs`（大量の拒否されたリクエストやログインの失敗をまとめること）。
- テストはヘルパーを通してログインします。ログインを無効にするスイッチはなく、テストのためのものもありません。
- よく使う環境変数：`XEXAM_DATA`、`XEXAM_PORT`、`XEXAM_ALLOWED_HOSTS`、`XEXAM_LOG`（ログレベル、例：`debug`）、`XEXAM_SQLITE_JOURNAL`（`delete` で WAL ではなくロールバックジャーナルのまま）、`XEXAM_COOKIE_SECURE`。全一覧は `xexam-backend --help` と [deployment.md](deployment.md) を参照してください。
- 空のデータディレクトリで最初に起動すると、管理者 `admin` / `admin` が作られます。最初のログインで、ページが新しいパスワードを求めます。忘れたときは：`cargo run -- reset-password admin --data ../data`（サーバーの動作中でも使えます）。
- コマンドラインからのインポート（取り込み先はレガシー問題集）：`cargo run -- import ../some.xlsx --data ../data`。
- 古いスキーマのデータベースをバックエンドで開くと移行が行われ、その隣に `backups/` が書き出されます。コピーを使ってください。
- データベースは WAL モードで動きます。バックエンドの動作中は、`xexam.db` の隣に `xexam.db-wal` と `xexam.db-shm` があります。3 つともコピーするか、先にバックエンドを停止してください（停止すると、すべてが `xexam.db` に入り、隣に `-wal` ファイルが残っていても空です）。`sqlite3` は、バックエンドの動作中でもデータベースを読めます。

## 3. フロントエンド

```bash
cd frontend
npm ci
npm run typecheck      # アプリを tsc でチェックし、続いてテストファイルをチェック（tsconfig.test.json）
npm test               # 119 件の単体テスト：採点とランダムな出題、練習の答え合わせ、保存した結果、言語の規則、記録の絞り込み、
                       # ロックの説明と待ち時間、取り込みの警告、システムログのバッファ、問題集へのアクセス、保存した試験、設定のキャッシュ、
                       # UI カタログ、サーバーメッセージの翻訳、更新履歴の解析
npm run build          # 型チェック + vite build → dist/
npm run dev            # http://127.0.0.1:5173、/api は http://127.0.0.1:8765 へプロキシ
```

- 開発時は `localhost` か `127.0.0.1` でページを開いてください。Vite プロキシの `changeOrigin` は `false` にする必要があります：バックエンドは `Host` と `Origin` を比較するため、`Host` が書き換えられると書き込みリクエストが 403 になります。
- バックエンドが既定のアドレスにない場合：`XEXAM_BACKEND=http://127.0.0.1:9000 npm run dev`。
- 「Xexam について」のページには、`README.md`、`README/README.ja.md`、`README/README.zh-CN.md` の更新履歴が表示されます。`vite.config.ts` のプラグイン `xexam-changelog` が、フロントエンドのビルド時に（`npm run dev` では「Xexam について」のページを初めて開いたときに）`frontend/` の 1 つ上のフォルダ（リポジトリのルート）からこれらを読むので、そこになければなりません。どれかに更新履歴の項目（`### vX.Y — 日付`）が 1 つもないと、ビルドはエラーで止まります。
- テストファイルを 1 つだけ実行するには：`node --experimental-strip-types --test src/i18n/server-messages.test.ts`。
- インポートパスには `.ts` 拡張子を使います（Node 標準のテストランナーの要件です）。
- TypeScript は `~5.9.3` に固定しています。

### 3.1 画面の文言

ユーザーに見える文言は、すべてカタログから取り出します。ページに直接書き込む文言はありません。

- **場所**：`src/i18n/catalog/<area>.ts`。領域ごとに 1 ファイルで、`common`、`home`、`banks`、`records`、`settings`、`quiz`、`result`、`about`、`auth`、`users`、`logs` があります。各ファイルの中身は `export default defineCatalog({ en: {…}, zh: {…}, ja: {…} })` です。
- **キー**は領域名で始まります：`banks.delete.title`、`records.deleteAll.ok`。共通の接頭辞（`app.`、`nav.`、`common.`、`err.`、`kind.`、`mode.`、`act.`、`role.`、`forbidden.` など）は `common.ts` にあります。
- **必ず 3 言語。** `zh` や `ja` のブロックに英語のキーが欠けていると、TypeScript がエラーにします。`_one` で終わるキーは英語の単数形で、`tn()` は件数が 1 のときにこれを使います。中国語と日本語では省略できます。
- **プレースホルダー**は `{name}` の形です：`t('records.deleteOne.body', { at, pct })`。件数には `tn(key, n)` を使います。これが `{n}` を埋めます。
- **チェック**：`src/i18n/catalog.test.ts` は次の点を確認します。3 言語のキーとプレースホルダーが完全に一致していること、空の文言がないこと、すべてのキーに領域の接頭辞が付いていて 1 つのカタログにだけあること、すべてのカタログファイルが `src/i18n/index.ts` に登録されていること。
- **カタログファイルを新しく作るとき**：`src/i18n/index.ts` でインポートし、`ALL` と `MsgKey` 型に追加します。
- 英語を先に書き、そのあと中国語と日本語を書きます。用語は UI ですでに使っているものに合わせてください（例：「問題集」「出題停止」「間違いノート」「問題集を管理」）。
- 問題と答えの言語の切り替えは例外です。その 3 つのボタンは、表示言語に関係なく常に **English**、**中文**、**日本語** と表示します（`src/components/lang-switch.ts`）。カタログから取るのは、スクリーンリーダー向けの名前とツールチップだけです。

### 3.2 バックエンドのメッセージを UI に表示する

バックエンドのエラーメッセージは中国語で、内容は変わりません（テスト済みの API の一部だからです）。フロントエンドがそれを翻訳します：

- `src/i18n/server-messages.ts`：`EXACT` はメッセージ全体を、`PATTERNS` はバックエンドの書式文字列を対応付けます（書式文字列の `{}` の部分は、取り出す値になります）。UI が中国語のときはメッセージをそのまま表示します。未知の文言もそのまま表示します。いくつかの低レベルなプロトコルエラーは英単語です（`not found`、`bad json` など）。これらには、中国語を含むすべての言語で文を用意しています。
- `src/i18n/errors.ts`：`errorText(e)` は、どんなエラーも現在の言語のテキストに変換します。ページはエラーを表示するとき、必ずこれを使います。ページ自身のエラーは `LocalError(key, params)` にするので、UI の言語を切り替えると一緒に切り替わります。
- **ずれの検出テスト**：`src/i18n/server-messages.test.ts` は、小さな字句解析器で `backend/src` の Rust ソースを読みます。中国語または日本語の文字を含む文字列リテラル（コメント、テストコード、アサーション、ログやコンソール出力を除く）には、英語と日本語の訳が必要です。訳にはプレースホルダーに入る値がそのまま残っていなければならず、中国語が含まれていてはいけません。対応するバックエンドの文言がもう存在しない訳があっても、テストは失敗します。
- **バックエンドのメッセージを追加・変更するとき**：同じ変更の中で `server-messages.ts` に訳を追加してください。ユーザーに表示されない文字列（たとえばパーサーが探す語）であれば、テストの `NOT_MESSAGES` に理由と一緒に追加します。
- **ログの行は英語のまま**にし、パスワード、セッションのトークン、Cookie の値、問題文を決して含めてはいけません（`backend/tests/v07_wal_and_logs.rs` が、一連の操作の後でログを確認します）。ログインせずに誰でも起こせる行は、`tracing` に直接ではなく `BurstLog`（`backend/src/logs.rs`）を通します。

### 3.3 ページやルートを追加する

1. **ルート**：`src/router.ts` で、`Route` に分岐を 1 つ追加し、`parseHash` に case を追加します。ハッシュが `#/<name>` の形でない場合は、`routeHash` にも追加します。メニューのどの項目を強調表示するかを `sectionOf` に書きます（`null` = なし）。管理者だけが開けるページは `adminOnly` に入れるか（ほかのユーザーはホームへ移ります）、`main.ts` から **権限がありません** のページを表示します。
2. **ページ**：`src/pages/<name>.ts` を作り、`HTMLElement` を返す `render<Name>()` 関数を書きます。DOM は `src/utils/dom.ts` の `h()` で組み立てます。スタイルは `src/styles/<name>.css` に書いてページの先頭でインポートし、色には `main.css` のカラートークンを使います。そうすればダークテーマでも正しく表示されます。
3. **つなぎ込み**：`src/main.ts` の `page()` に case を追加し、ブラウザのタブに出すタイトルを指定します。
4. **メニュー項目**（主要なセクションの場合のみ）：`src/router.ts` の `NavSection` に追加し、`src/components/shell.ts` の `NAV`（一部のユーザーにだけ見せるときは `need` 付きで）、`HREF`、アイコン用の `ICONS` にも追加します。`NAV` の項目は、メニューの区切り線より上に並びます。その下の「Xexam について」の項目は別に作ります。
5. **文言**：新しいカタログ `src/i18n/catalog/<name>.ts` を、3 言語すべてで作ります（3.1 節）。
6. **状態**：ページ同士は互いにインポートしません。共有する状態は `src/state/store.ts` に置き、変更は `src/state/actions.ts` の関数（`navigate(route)`、`render()` など）を通して行います。そのページだけが使う状態は、ページモジュールの変数に置きます。ユーザーがページを離れるときや開いたときに処理を実行するには、`onRouteChange()` を使います。
7. **API**：新しいエンドポイントには、`backend/src/api/auth.rs` のルート表 `ROUTES` に 1 行（誰が使えるか）、`src/api/` に型付きのラッパー、`src/types/api.ts` にその型を追加し、[api.md](api.md) と [openapi.yaml](../en/openapi.yaml) に記載します。

## 4. スタック全体

```bash
# 2 つのコンテナをビルドして起動（同じ xexam_data ボリュームを使います！自分のデータが使われるので、試す場合は別のプロジェクト名を使ってください：-p some-other-name）
docker compose -f docker/docker-compose.build.yml up -d --build

# チェック用スクリプトはすべて一時的なプロジェクト名と一時ボリュームを使い、xexam_data には触れません
docker build -f nginx/Dockerfile -t xexam-frontend:test .
python -I tests/integration/nginx_check.py xexam-frontend:test 18080   # 38 項目、ダミーのバックエンドを使用
python -I tests/integration/stack_check.py 18090                      # スタック全体を起動（2 つのイメージをビルドします）：ログイン、インポート、
                                                                      # 成績の記録、永続化、WAL、システムログ、reset-password、docker stop
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend   # progress.json の移行、WAL と DELETE への切り替え（先に cargo build が必要）
```

どのスクリプトも、確認した項目と、最後に `All N checks passed` を出力します。`nginx_check.py` と `stack_check.py` の最後の引数は、使うホストのポートです。お使いのマシンで空いているものを選んでください。

compose ファイルと `.env` は `docker/` にあり、`docker/docker-compose.build.yml` はリポジトリのルート（`..`）をビルドコンテキストにします。フロントエンドのイメージは、リポジトリのルートからビルドします。`nginx/Dockerfile` は、Node のビルド段階に 3 つの README をコピーします（`COPY README.md /` と `COPY README/README.ja.md README/README.zh-CN.md /README/`）。「Xexam について」のページの更新履歴が、これらから作られるためです。同じ理由で、`.dockerignore` は `README.md` と `README/` をビルドコンテキストに残します。完成したイメージに入るのはビルドした静的ファイルだけです。

## 5. 旧版との照合（回帰）

旧版の Python 実装はリポジトリから削除されましたが、その挙動は `tests/baseline/` に固定されています（すべて架空の合成データです）：

| スナップショット | 内容 | 使うもの |
|---|---|---|
| `golden/NNN_*.json`（172 件） | 旧版の各エンドポイント × 各状態の実際のレスポンス（ステータスコード、安定したレスポンスヘッダー、正規化後の本文） | `backend/tests/api_golden.rs` が順番に新しいルートへ再生し、項目ごとに比較する |
| `fixtures/*.xlsx` + `parser_golden/*.json`（49 件） | 合成ワークブックと、それぞれに対する旧パーサーの出力 | `backend/tests/parser_golden.rs`（3 列と 4 列形式はバイト単位で同じでなければならない） |
| `golden/id_vectors.json`（85 件のアサーション） | 問題 id のアサーション。各件にハッシュ対象のバイト列が付く | `backend/tests/id_vectors.rs` |
| `fixtures/sample.db` + `golden/store_sample_outputs.json` | 旧実装が書き出したサンプルデータベースと、その各インターフェースの出力 | `backend/tests/db_golden.rs`（現在のスキーマへの移行も確認する） |
| `golden/allowed_differences_rust.json`（23 項目）、`allowed_probe_differences_rust.json`（51 項目） | 旧版と意図的に異なる箇所。各項目に理由が 1 文付く | `api_golden.rs` で登録されていない差異 = 失敗 |

- 挙動を**変更**したい場合（たとえば意図的な差異を新しく加える）：コードを変更し、`allowed_differences_rust.json` に登録して理由を明記します。ゴールデンレスポンスを直接変更しないでください。
- 旧版のキャプチャ / 照合 / プローブ用スクリプトは旧 Python コードに依存していたため、コードと共に削除されました。必要な場合は、Git タグ `python-v0.2` とコミット履歴から取り戻してください。

## 6. データ保護のルール

- `data/`、`progress.json`、ご自身の問題集の Excel ファイルを、読み取る・出力する・コミットする・変更する、のいずれも**しないでください**。
- 試す操作は必ずコピーか一時ディレクトリで行ってください。本物の `xexam.db` に対して「どうなるか見てみる」ようなコマンドを実行しないでください。古いデータベースを新しいバージョンで開くと移行される（そして WAL に切り替わる）点にも注意してください。
- `docker compose down -v` を実行しないでください。
- コード、テスト、ドキュメントの例には架空の内容だけを使い、本物の試験問題は使わないでください（デモ用問題集「General knowledge」と「Kitchen basics」、または `frontend/src/pages/banks.ts` にある架空の 6 列形式の例）。

## 7. CI

パイプラインは 1 つだけです：[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。`main` と `dev` へのプッシュ、`main` へのプルリクエスト、手動での起動のときに実行されます。

| ジョブ | 内容 |
|---|---|
| `check` | `VERSION` に `CODE_VERSION=` と `RELEASE_VERSION=` の行が**ちょうど 1 つずつ**あり、どちらも `0.7` や `0.6.5` のような形で、`RELEASE_VERSION` が `CODE_VERSION` より新しくないこと。3 つの README それぞれの更新履歴の最新の項目 = `CODE_VERSION`。各 README の冒頭のリリースバッジ = `RELEASE_VERSION`。`backend/Cargo.toml` と `frontend/package.json` が `CODE_VERSION` であること。配置（ルートの README は `README.md` だけ、残り 2 つの README は `README/`、compose ファイルと `.env.example` は `docker/`）とドキュメントのチェック（8 節）。このジョブの出力 `version`（`CODE_VERSION`）を後のジョブが使います |
| `frontend` | 型チェック、119 件の単体テスト、ビルド |
| `backend` | fmt、clippy、302 件のテスト、および `progress.json` の移行チェック |
| `stack` | `xexam-frontend` イメージをビルドして 38 項目の Nginx チェックを実行。compose スタック全体を起動してエンドツーエンドのチェックを実行 |
| `arm64` | 2 つのイメージを arm64 向けにビルドし、QEMU 上で起動して、`/api/info` が `aarch64` と `CODE_VERSION` を返すことを確認 |
| `publish` | `main` のみ：2 つのイメージ（amd64 + arm64）をタグ `latest`、`v<CODE_VERSION>`、`sha-<コミット>` で Docker Hub（`docker.io/yuminspace/`、公開。`docker/docker-compose.yml` が pull するのはこちら）と ghcr.io（非公開のミラー）にプッシュし、両方でマニフェストとタグを検証。リポジトリのシークレット `DOCKERHUB_USERNAME` と `DOCKERHUB_TOKEN`（必要に応じて変数 `DOCKERHUB_NAMESPACE`）が必要で、ない場合は何もプッシュせずに止まる |
| `release` | `main` のみ：GitHub Release `v<CODE_VERSION>` を作成（または更新）。リリースノートは `README.md` の更新履歴から取る（`scripts/release-notes.py`）。`main` にすでに記録されたリリースより古いバージョンの実行では、その Release を最新（latest）にしない。最後のステップでリリースを記録する（7.1 節） |

`dev` へのプッシュと `main` へのプルリクエストでは最初の 5 つのジョブだけを実行し、**公開は一切行いません**。公開されるのは `main` へのプッシュだけです。手動で起動した場合は、実行対象のブランチへのプッシュと同じ動きになります。

### 7.1 リリースの後：ボットのコミット

`RELEASE_VERSION` を動かすのは、イメージが公開され、GitHub Release ができたときの `release` ジョブの最後のステップだけです。

1. 最新の `origin/main` の上で `scripts/set-release-version.py <バージョン>` を実行し、`VERSION` の `RELEASE_VERSION` と、3 つの README の冒頭のリリースバッジ（代替テキスト、URL、幅）を書き換えます。
2. それを **`Record release v<バージョン>: RELEASE_VERSION and the README badge [skip ci]`** としてコミットし、`main` にプッシュします。ワークフロー自身のトークンによるプッシュは新しい実行を起こさず、メッセージにも `[skip ci]` が入っています。何も変わらないとき（やり直しの実行）は、何もコミットしません。
3. `dev` に独自のコミットがなければ、`dev` を同じコミットまで早送り（fast-forward）します。`dev` が先に進んでいる場合（またはリリース中に進んだ場合）は `dev` に触れず、実行の中に **注意（notice）** を出します：`merge main into dev (git pull origin main on dev) before the next merge to main`。どちらの場合も、リリースは完了しています。

`set-release-version.py` は `RELEASE_VERSION` を戻すことはありません。古い実行の release ジョブをやり直しても何も変えず、0 で終了します。`CODE_VERSION` より新しいバージョンは拒否します。

そのため、リリースのたびに、`origin/main`（と、たいていは `origin/dev`）には、お手元のクローンより 1 つ多くコミットがあります。

- **次にプッシュする前に**、両方のブランチを更新してください：`git checkout main && git pull`、続いて `git checkout dev && git pull`。
- **実行に注意が出ていたときは**、次に `main` へマージする前に、`main` を `dev` にマージしてください：`dev` で `git pull origin main` を実行し、`dev` をプッシュします。そうしないと、次の `main` へのマージでボットのコミットが抜けたり、`VERSION` やバッジで衝突したりします。

## 8. ドキュメント

- 言語ごとに 1 つのフォルダがあります：`docs/en`（英語、原文）、`docs/ja`（日本語）、`docs/zh-CN`（簡体字中国語）。どのフォルダにも同じ 7 つのドキュメントがあります：`api`、`architecture`、`database`、`deployment`、`development`、`migration`、`security`。
- `docs/zh-CN/history/` には、Rust へ切り替えたときの古いドキュメント（監査、計画、リスク評価）があります。中国語のみで、今後は更新しません。
- `docs/en/openapi.yaml` はすべての言語で共有します。日本語と中国語のドキュメントは `../en/openapi.yaml` にリンクします。
- どのドキュメントも、ほかの 2 つのフォルダにある同じドキュメントへのリンクを並べた言語バーで始まります。リンクはファイル自身のフォルダからの相対パスで、リポジトリのルートは `../../` です。
- 先に英語のドキュメントを変更し、同じ変更の中で日本語と中国語のドキュメントも更新します。UI の項目名は、その言語のカタログのとおりに太字で書きます。問題の言語の切り替えは、常に **English** / **中文** / **日本語** です。
- CI の `check` ジョブは、次のどれかに当てはまると失敗します：7 つのドキュメントのどれかが言語フォルダにない、`docs/` に `en`、`ja`、`zh-CN` 以外のフォルダやファイルがある、README やドキュメントの相対リンクや画像のパスが存在しないファイルを指している。GitHub のページへのリンク（`../../releases` など）も確認します。これらは `../` でちょうどリポジトリのページまで戻る必要があります（`README/` の中では `../../../` で始まります）。

## 9. バージョン、更新履歴、ライセンス

- リポジトリのルートの **`VERSION`** には、`KEY=VALUE` の行が 2 つ、それぞれちょうど 1 回ずつあります（`#` で始まる行はコメントです）：
  - `CODE_VERSION`（現在は `0.7`）：このブランチのコードのバージョン。3 つの README すべての更新履歴の項目、`backend/Cargo.toml` と `frontend/package.json`（`0.7.0`。それぞれのロックファイルも）と一緒に、手で上げます。イメージはこのバージョンでビルドされてタグが付き、`/api/info` もこれを返します（バックエンドはイメージの `/VERSION` から読みます）。
  - `RELEASE_VERSION`（v0.7 のリリースジョブが動くまでは `0.6`）：実際に公開された最後のバージョン。**手で編集しないでください**。リリースジョブが、README の冒頭のリリースバッジと一緒に書き換えます（7.1 節）。プッシュやマージだけで変わることはありません。
- 更新するたびに、3 つの README（`README.md`、`README/README.ja.md`、`README/README.zh-CN.md`）すべての更新履歴の一番上に、`CODE_VERSION` と同じバージョン番号の項目を 1 つ追加します。CI がこれを確認します。GitHub Release のノートは英語版から取ります。
- 「Xexam について」のページには、フロントエンドに組み込んだこれらの更新履歴が表示されます（3 節）。項目は簡単な Markdown で書きます：段落、`- ` の箇条書き、`  - ` の入れ子の箇条書き、`**太字**`、`` `コード` ``。リポジトリ内のファイルへのリンクは、そのページでは文字として表示されます。更新履歴を変えても、アプリに表示されるのはフロントエンドをもう一度ビルドした後です。
- 作業は `dev` で行い、`main` にはマージで取り込みます。`main` へのプッシュでイメージと Release が公開されます。リリースのたびに `main` と `dev` を pull してください（7.1 節）。
- このプロジェクトは MIT ライセンスです（リポジトリのルートの `LICENSE`）。
