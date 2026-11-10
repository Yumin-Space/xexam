[English](development.md) | **日本語** | [简体中文](development.zh-CN.md)

# 開発

> 日常的にコードを変更したりテストを実行したりするためのコマンド集です。アーキテクチャは [architecture.ja.md](architecture.ja.md)、テストの方針は [test-strategy.md](test-strategy.md) を参照してください。

## 1. ツール

| ツール | バージョン | 説明 |
|---|---|---|
| Rust | 1.99.1（`backend/rust-toolchain.toml` で固定） | `rustup` が自動でインストールします。`rusqlite` の `bundled` には C コンパイラが必要です（Linux：`build-essential`、Windows：MinGW gcc または MSVC） |
| Node | 24 | フロントエンド |
| Python | 3.8+ | テストの実行にのみ使います：ゴールデンレスポンスとの照合、パーサーの照合（「旧版が正解」）。**標準ライブラリのみ**を使います |
| Docker | Compose v2 | スタック全体のチェック、イメージ |

## 2. バックエンド

```bash
cd backend
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 151 件のテスト
cargo run -- serve --data ../data --no-browser     # 127.0.0.1:8765 で待ち受け
```

- テストは `tests/baseline/` にコミット済みの**旧版スナップショット**と照合するため、Python は不要です（`migrate_check.py` のような統合スクリプトだけが Python の標準ライブラリで動きます）。
- すべてのテストは、一時ディレクトリ内の新規データベースか `tests/baseline/fixtures/sample.db` のコピーを使い、`data/` には**触れません**。
- よく使う環境変数：`XEXAM_DATA`、`XEXAM_PORT`、`XEXAM_ALLOWED_HOSTS`、`XEXAM_LOG`（ログレベル、例：`debug`）。全一覧は `xexam-backend --help` と [deployment.ja.md](deployment.ja.md) を参照してください。
- コマンドラインからのインポート：`cargo run -- import ../some.xlsx --data ../data`。

## 3. フロントエンド

```bash
cd frontend
npm ci
npm run typecheck      # tsc（テストファイルを含む）
npm test               # 27 件の単体テスト（採点、誤答ルール、sessionStorage、言語ルール）
npm run build          # 型チェック + vite build → dist/
npm run dev            # http://127.0.0.1:5173、/api は http://127.0.0.1:8765 へプロキシ
```

- 開発時は `localhost` か `127.0.0.1` でページを開いてください。Vite プロキシの `changeOrigin` は `false` にする必要があります：バックエンドは `Host` と `Origin` を比較するため、`Host` が書き換えられると書き込みリクエストが 403 になります。
- バックエンドが既定のアドレスにない場合：`XEXAM_BACKEND=http://127.0.0.1:9000 npm run dev`。
- インポートパスには `.ts` 拡張子を使います（Node 標準のテストランナーの要件です）。
- TypeScript は `~5.9.3` に固定しています（`openapi-typescript` の peer 依存）。

## 4. スタック全体

```bash
# 2 つのコンテナをビルドして起動（同じ xexam_data ボリュームを使います！自分のデータが使われるので、試す場合は別のプロジェクト名を使ってください：-p 別の名前）
docker compose -f docker-compose.build.yml up -d --build

# チェック用スクリプトはすべて一時的なプロジェクト名と一時ボリュームを使い、xexam_data には触れません
docker build -f nginx/Dockerfile -t xexam-frontend:test .
python -I tests/integration/nginx_check.py xexam-frontend:test   # 38 項目、ダミーのバックエンドを使用
python -I tests/integration/stack_check.py                      # 34 項目、スタック全体を起動（2 つのイメージをビルドします）
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend         # 10 項目：旧 progress.json の移行（先に cargo build が必要）
```

## 5. 旧版との照合（回帰）

旧版の Python 実装はリポジトリから削除されましたが、その挙動は `tests/baseline/` に固定されています（すべて架空の合成データです）：

| スナップショット | 内容 | 使うもの |
|---|---|---|
| `golden/NNN_*.json`（172 件） | 旧版の各エンドポイント × 各状態の実際のレスポンス（ステータスコード、安定したレスポンスヘッダー、正規化後の本文） | `backend/tests/api_golden.rs` が順番に新しいルーターへ再生し、項目ごとに比較する |
| `fixtures/*.xlsx` + `parser_golden/*.json`（49 件） | 合成ワークブックと、それぞれに対する旧パーサーの出力 | `backend/tests/parser_golden.rs` |
| `golden/id_vectors.json`（85 件） | 問題 id のアサーション。各件にハッシュ対象のバイト列が付く | `backend/tests/id_vectors.rs` |
| `fixtures/sample.db` + `golden/store_sample_outputs.json` | 旧実装が書き出したサンプルデータベースと、その各インターフェースの出力 | `backend/tests/db_golden.rs` |
| `golden/allowed_differences_rust.json`（23 項目）、`allowed_probe_differences_rust.json`（51 項目） | 旧版と意図的に異なる箇所。各項目に理由が 1 文付く | `api_golden.rs` で登録されていない差異 = 失敗 |

- 挙動を**変更**したい場合（たとえば意図的な差異を新しく加える）：コードを変更し、`allowed_differences_rust.json` に登録して理由を明記します。ゴールデンレスポンスを直接変更しないでください。
- 旧版のキャプチャ / 照合 / プローブ用スクリプトは旧 Python コードに依存していたため、コードと共に削除されました。必要な場合は、Git タグ `python-v0.2` とコミット履歴から取り戻してください。

## 6. データ保護のルール

- `data/`、`progress.json`、ご自身の問題バンクの Excel を、読み取る・出力する・コミットする・変更する、のいずれも**しないでください**。
- 試す操作は必ずコピーか一時ディレクトリで行ってください。本物の `xexam.db` に対して「どうなるか見てみる」ようなコマンドを実行しないでください。
- `docker compose down -v` を実行しないでください。

## 7. CI

パイプラインは 1 つだけです：[`.github/workflows/ci.yml`](../.github/workflows/ci.yml)。

| ジョブ | 内容 |
|---|---|
| `check` | `VERSION` の形式、README の更新履歴の先頭の項目 = `VERSION` |
| `frontend` | 型チェック、27 件の単体テスト、ビルド |
| `backend` | fmt、clippy、151 件のテスト、および `progress.json` の移行チェック |
| `stack` | `xexam-frontend` イメージをビルドして 38 項目の Nginx チェックを実行。compose スタック全体を起動して 34 項目のエンドツーエンドチェックを実行 |
| `arm64` | 2 つのイメージを arm64 向けにビルドし、QEMU 上で起動して、`/api/info` が `aarch64` を返すことを確認 |
| `publish` | `main` のみ：2 つのイメージ（amd64 + arm64）を ghcr.io にプッシュし、マニフェストとタグを検証 |
| `release` | `main` のみ：`v<VERSION>` の GitHub Release を作成（または更新） |

`dev` へのプッシュ、`main` へのプルリクエスト、その他のブランチは最初の 5 つだけを実行し、**公開は一切行いません**。公開されるのは `main` へのプッシュだけです。

## 8. バージョン番号と更新履歴

- バージョン番号の唯一の情報源は、リポジトリのルートにある `VERSION` です。`Cargo.toml` と `package.json` にもそれぞれ 1 つずつ書かれているので、一致させておいてください。
- 更新するたびに、README の「更新履歴」の先頭に項目を 1 つ追加し、バージョン番号を `VERSION` と一致させます（CI が検証します）。
