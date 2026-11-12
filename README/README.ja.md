[English](../README.md) | **日本語** | [简体中文](README.zh-CN.md)

<!-- The release badge shows RELEASE_VERSION from VERSION, the last published release. Do not edit it by hand: the release job of .github/workflows/ci.yml writes it (scripts/set-release-version.py) after the images and the GitHub Release are published. GitHub rounds an image's corners only when it has both width and height, so every badge carries its exact size (in px, from shields.io). -->
<p>
  <a href="../../../releases/latest"><img width="145" height="28" alt="リリース v0.9.1" src="https://img.shields.io/badge/release-v0.9.1-green?style=for-the-badge"></a>
</p>

<p align="center">
  <img width="120" height="120" alt="Xexam のロゴ" src="../frontend/public/icon.svg">
</p>

<p align="center">
  <a href="https://hub.docker.com/r/yuminspace/xexam-frontend"><img width="360" height="28" alt="Docker image yuminspace/xexam-frontend" src="https://img.shields.io/badge/Docker%20Image-yuminspace%2Fxexam--frontend-blue?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-frontend"><img width="150" height="28" alt="Docker stars" src="https://img.shields.io/docker/stars/yuminspace/xexam-frontend?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-frontend"><img width="148" height="28" alt="Docker pulls" src="https://img.shields.io/docker/pulls/yuminspace/xexam-frontend?style=for-the-badge"></a>
</p>
<p align="center">
  <a href="https://hub.docker.com/r/yuminspace/xexam-backend"><img width="352" height="28" alt="Docker image yuminspace/xexam-backend" src="https://img.shields.io/badge/Docker%20Image-yuminspace%2Fxexam--backend-blue?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-backend"><img width="150" height="28" alt="Docker stars" src="https://img.shields.io/docker/stars/yuminspace/xexam-backend?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-backend"><img width="148" height="28" alt="Docker pulls" src="https://img.shields.io/docker/pulls/yuminspace/xexam-backend?style=for-the-badge"></a>
</p>
<p align="center">
  <a href="https://hub.docker.com/r/yuminspace/xexam-ai-service"><img width="369" height="28" alt="Docker image yuminspace/xexam-ai-service" src="https://img.shields.io/badge/Docker%20Image-yuminspace%2Fxexam--ai--service-blue?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-ai-service"><img width="150" height="28" alt="Docker stars" src="https://img.shields.io/docker/stars/yuminspace/xexam-ai-service?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-ai-service"><img width="148" height="28" alt="Docker pulls" src="https://img.shields.io/docker/pulls/yuminspace/xexam-ai-service?style=for-the-badge"></a>
</p>

# Xexam

Xexam は、ブラウザで使うセルフホスト型の問題演習アプリです。ご自身の Excel 問題集を取り込んで試験や練習に使い、間違えた問題を復習できます。管理者が AI プロバイダーを接続し、**AI+ 機能** を付与すれば、ユーザーは **AI チャット** で AI モデルに質問できるほか、学習中の AI サポートも使えます：練習中のヒントと解説、結果ページのすべての問題の AI 解析、間違えた問題の AI まとめ。パソコンでもスマートフォン（iPhone）でも使えます。

![試験ページ：日本語で表示したホットスポット問題、English / 中文 / 日本語 の切り替え、提出する ボタンのある解答用紙](../docs/ja/screenshots/desktop-2-quiz.png)

リポジトリには**問題は含まれていません**。Xexam が提供するのは、問題を取り込み、整理し、練習するための機能だけです。問題集はご自身の Excel ファイルであり、コミットされません（`.gitignore` で除外済み）。スクリーンショットには架空のデモ用問題集を使っており、実際の試験の内容ではありません。

| | |
|---|---|
| 現在のバージョン | **v1.0**（[更新履歴](../docs/ja/changelog.md)、またはリポジトリの [Releases](../../../releases) ページを参照。アプリでは：メニュー → **Xexam v1.0**） |
| イメージ | 3 つ。Docker Hub（公開、ログイン不要）：`docker.io/yuminspace/xexam-frontend`、`docker.io/yuminspace/xexam-backend`、`docker.io/yuminspace/xexam-ai-service`。タグは `latest` / `v1.0` / `sha-<コミット>`（`sha-` のタグは問題の調査用）。ミラー：`ghcr.io/yumin-space/xexam-frontend`、`ghcr.io/yumin-space/xexam-backend`、`ghcr.io/yumin-space/xexam-ai-service`（非公開、タグは同じ） |
| CPU アーキテクチャ | `linux/amd64`、`linux/arm64`（同じタグの中から自動的に適切なものが選ばれます） |
| ポート | `8765`（既定ではローカルマシンのみにバインド） |
| ライセンス | [MIT](../LICENSE) |

---

## 目次

- [できること](#できること)
- [クイックスタート](#クイックスタート)
- [環境設定](#環境設定)
- [アップグレード](#アップグレード)
- [ドキュメント](#ドキュメント)
- [ライセンス](#ライセンス)
- [更新履歴](#更新履歴)

---

## できること

- **試験**：提出するまでは採点されません。提出すると、得点と、間違えた問題（またはすべての問題）ごとに、あなたの回答、正解、解説、覚え方が表示されます。問題の間を自由に移動でき、あとで見直したい問題に印を付けたり、保存して中断し、あとで続けたりできます。
- **練習**：答えるたびにすぐ答え合わせをして、正解と解説を表示します。
- **自分の問題集**：取り込んだ Excel ブックは、それぞれが独立した問題集になり、ブック内の各ワークシートは、選んで出題できる問題グループになります。ブックは 6 列形式で書きます：問題と答えを英語で、必要なら中国語と日本語でも書きます。保存する前に、取り込みで何が変わるかをプレビューで確認できます。
- **3 つの言語**：画面は英語・中国語・日本語で使え、問題、答え、解説はどれも **English**、**中文**、**日本語** に切り替えられます。
- **学習記録と間違いノート**：試験と練習はすべて回答と一緒に残ります。間違えた問題は、2 回続けて正解するまで間違いノートに残ります。
- **1 つの Xexam を複数人で**：全員がログインして使い、学習記録と設定は人ごとに分かれます。どのユーザーがどの問題集を見られるか、誰が問題集を取り込めるか（問題集の管理権限）は管理者が決め、管理者は **システムログ** も確認できます。
- **AI チャット**（任意。管理者が設定するまではオフ）：管理者は **AI 設定** で OpenAI 互換 API、Anthropic、Cloudflare AI Gateway のいずれかを接続し、テストして **AI+ 機能** をオンにし、使ってよいユーザーごとに **ユーザー管理** で **AI+ 機能** を付与します。**AI 設定** では、機能を取り上げずに、ユーザーの **AI+ 機能** を **一時停止** したり **再開** したりできます。回答は書かれるそばから表示され（推論モデルが働いている間は **考え中…** と表示）、ページを閉じても書き続けられます。AI は **設定** で選んだ言語で答えます。チャットは保存され（既定では 90 日後に自動で削除）、**メモリー** には、AI がその後のチャットで参考にする短いメモが残ります。管理者は **設定** の **AI 対話モード** で **フリーモード** を選ぶと、学習アシスタントの指示なしにモデルと直接話せます。管理者は使用量の統計を見られ、AI+ 機能を持つユーザーのチャットとメモを閲覧・削除できます。このことは AI チャットがすべてのユーザーに伝えます。AI は専用のデータベースを持つ専用のコンテナ `ai-service` で動き、プロバイダーのキーは暗号化して保存されます。
- **学習中の AI サポート**（**AI+ 機能** のあるユーザー向け。試験の最中は何も表示されません）：練習では、問題の下の **AI+ パネル** に **AI ヒント**（答えを言わずに解き方を示します）と **AI 解説**（答えとその理由）が表示されます。結果ページと学習記録の詳細ページでは **AI 解析**、学習記録と **ホーム** では間違えた問題の **AI まとめ** があります。回答は **AI チャット** に保存され、AI パネルからは、どのページへも移動しません。ユーザーごとに、AI リクエストの 1 分・1 時間・1 日あたりの上限があり、管理者が設定します。
- **スマートフォンを優先**：どのページも、スマートフォンの幅で横にスクロールせずに使えます。アイコンのボタンは、押し続けると名前が表示されます。タッチ領域は 44 px 以上です。フッターの中央には、プロジェクトへのリンク付きの 1 行があり、問題と回答がいちばん多くの場所を使います。パソコンのページは、広い画面では中央に寄せられます。

各ページの使い方、練習と結果での AI サポート、AI チャットと AI 設定、Excel の形式、取り込みの規則：[docs/ja/usage.md](../docs/ja/usage.md)。

---

## クイックスタート

必要なのは、**Docker**（Compose v2 を含む）が入ったマシン（Windows / macOS / Linux のいずれでも、**x86 でも ARM でも**構いません）とブラウザです。

**1. [`docker-compose.yml`](../docker/docker-compose.yml) をダウンロードします**（このリポジトリの `docker/` フォルダにあります）。新しいフォルダ（例：`xexam`）に保存してください。必要なファイルはこれだけです。いちばん簡単なのは、GitHub でこのファイルを開いて **Download raw file** をクリックする方法です。コマンドラインなら次のとおりです。

```bash
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml
```

リポジトリは非公開です。`curl` でファイルをダウンロードする（または `git clone` する）には、オーナーに頼んで、あなたの GitHub アカウントをコラボレーターに追加してもらい、スコープ `repo` を付けた classic の個人用アクセストークンを作成し（GitHub → **Settings** → **Developer settings** → **Personal access tokens** → **Tokens (classic)**）、`-fLO` の後に `-H "Authorization: token YOUR_TOKEN"` を加えてください。オーナーにファイルを送ってもらうだけでも構いません。Windows PowerShell では、`curl` の代わりに `curl.exe` と入力します。イメージそのものは Docker Hub で公開されているので、pull するのにアカウントも `docker login` も必要ありません。

**2. 起動します。** `docker-compose.yml` があるフォルダで、次を実行します。

```bash
docker compose up -d
```

このコマンドで 3 つのイメージが pull され、**3 つのコンテナ** が起動します：`xexam-frontend`（Nginx、外部から接続できるのはこれだけ）、`xexam-backend`（Rust、インターネットに接続できないプライベートネットワーク内）、`ai-service`（Rust、AI サービス。インターネットに接続できる唯一のコンテナで、接続するのは管理者が設定した AI プロバイダーを呼び出すときだけです）。25 秒ごとにバックエンドへ報告するので、Xexam は AI が使えるかどうかを把握します。`ai-service` がなくても Xexam は動き、AI チャットが使えないだけで、**AI 設定** には ai-service が動いていないと表示されます。

**3. ログインします。** 同じマシンのブラウザで <http://localhost:8765> を開き、ユーザー名 `admin`、パスワード `admin` でログインします。すると、自分のパスワードを設定するよう求められます。すぐに設定してください。それまでは、このページを開ける人なら誰でも既定のパスワードを使えてしまいます。

**4. 問題集を取り込んで、試験を受けます。** メニュー（右上の三本線のボタン）を開いて **Question banks** を選び、Excel ブックを取り込んでから、**Home** で試験を始めます：[最初の試験](../docs/ja/usage.md#1-最初の試験) を参照してください。画面は最初は英語で表示されます。**Settings** → **Language** で中国語や日本語に切り替えられます（日本語では、これらは **問題集の管理**、**ホーム**、**設定** → **言語** と表示されます）。

**5. AI チャット（任意）。** AI は設定するまでオフです：メニュー → **AI 設定**（ai-service が接続されているかが表示されます）でプロバイダーを選び、そのアドレスとキーを入力し、一覧からモデルを選んで **テスト** し、**AI+ 機能** をオンにします。管理者はすぐに AI チャットを使えます。ほかのユーザーは、**ユーザー管理** でそのユーザーの **AI+ 機能** をオンにすると使えます（そのスイッチは、ai-service が接続されていて AI+ 機能がオンの間だけ表示されます）。手順は [docs/ja/usage.md の第 17 節](../docs/ja/usage.md#17-ai-設定) にあります。

| 操作 | コマンド |
|---|---|
| 最新バージョンに更新 | `docker compose pull` の後に `docker compose up -d`（先に [アップグレード](#アップグレード) を読んでください） |
| 特定のバージョンに固定 | `docker-compose.yml` 内の 3 つのイメージの `:latest` をすべて `:v1.0` に変更してから、`docker compose up -d` を実行 |
| 状態を確認 | `docker compose ps`（3 つのコンテナがすべて `healthy` になっていれば正常） |
| 停止 / 再起動 | `docker compose stop` / `docker compose start` |
| ログを見る | `docker compose logs -f backend` と `docker compose logs -f ai-service`（管理者はメニューの **システムログ** でも見られます。AI サービスの行も表示されます） |
| 動いているバージョンとアーキテクチャを確認 | ブラウザで <http://localhost:8765/api/info> を開く |

> ⚠️ **`docker compose down -v` は実行しないでください**：`-v` を付けると 2 つのデータボリュームも一緒に削除されます。`xexam_data`（問題集、ユーザー、学習記録、設定）と `xexam_ai_data`（AI 設定、暗号化したキーとそのキーファイル、チャットとメモ）です。

- **スマートフォンや別のパソコンから**、または **Cloudflare Tunnel でインターネットから** 開く：[docs/ja/deployment.md](../docs/ja/deployment.md) の第 4 節。
- **ソースからイメージをビルドする**（コードを変更するとき）：クローンしたリポジトリで `docker compose -f docker/docker-compose.build.yml up -d --build`。[docs/ja/deployment.md](../docs/ja/deployment.md) の第 2 節を参照してください。
- **バックアップ**：Xexam を停止して 2 つのデータボリュームをまとめます。コマンドは [docs/ja/database.md](../docs/ja/database.md) の第 5 節にあります。AI のボリュームには、保存した AI のキーを復号するキーファイルも入っているので、そのバックアップはキーそのものと同じように大切に保管してください。

---

## 環境設定

既定のままなら、何も設定せずに 1 台のパソコンで使えます。変更したいときは、`docker-compose.yml` と同じ場所に `.env` ファイルを作成し（クローンしたリポジトリでは `docker/.env`。テンプレートは [`.env.example`](../docker/.env.example)）、もう一度 `docker compose up -d` を実行してください。compose ファイル自体を編集する必要はありません。

| `.env` の変数名 | 既定 | 役割 |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | 待ち受けるアドレスとポート。`IP:ポート番号` の形式で書きます。既定では本機からのみ接続できます。`0.0.0.0:8765` = ネットワーク内の他のマシンからも接続できます。⚠️ ポート番号だけ（例：`8766`）= すべてのネットワークインターフェースから接続できます |
| `XEXAM_ALLOWED_HOSTS` | 空 | Xexam を開くのに使ってよいドメイン名 / IP。カンマ、スペース、セミコロンのいずれかで区切ります。例：`xexam.example.com,192.168.1.20` |
| `XEXAM_ALLOWED_ORIGINS` | 空 | リバースプロキシが Host をオリジンサーバーのアドレスに書き換える場合にのみ必要です。公開ドメインを `https://` 付きで指定します |
| `XEXAM_ALLOW_ANY_HOST` | `0` | `1` = Host を検査しなくなります（前段にすでに認証がある場合にのみ） |
| `XEXAM_PARSE_TIMEOUT_SECS` | `30` | Excel 1 ファイルの解析にかける最大時間（秒） |
| `XEXAM_SQLITE_JOURNAL` | 未設定（WAL） | `delete` = SQLite の従来の DELETE ジャーナルのまま。データボリュームが特殊なストレージ（ネットワークファイルシステムや、Docker にバインドマウントした Windows のフォルダ）にある場合にだけ使います |
| `XEXAM_COOKIE_SECURE` | 未設定 | `1` = セッションの Cookie に常に `Secure` を付けます。Xexam に常に HTTPS で接続する場合にだけ使います |
| `XEXAM_LOG` | 未設定（`info`） | バックエンドのログのフィルター。例：`debug`。空なら `info` |
| `TZ` | 未設定（ホストのタイムゾーン） | Xexam が表示する日時と、AI の使用量の統計の日付のタイムゾーン。例：`TZ=UTC` |
| `XEXAM_AI_SECRET_KEY` | 未設定（AI ボリューム内のキーファイル） | 保存した AI プロバイダーのキーを暗号化するキー。32 バイトを 16 進数 64 桁か base64 で書きます。例：`openssl rand -base64 32` の出力。設定するとキーを AI ボリュームから切り離せるので、そのボリュームのコピーだけではキーを復号できません。変更したり失ったりすると、AI のキーを **AI 設定** で入力し直す必要があります |
| `XEXAM_INTERNAL_TOKEN` | 未設定（リンクボリューム内のランダムなトークン） | バックエンドと AI サービスが共有するトークン。compose ファイルは同じ値を両方に渡します。空白を含まない表示可能な ASCII 文字で 32 文字以上。例：`openssl rand -hex 32` の出力。AI サービス自身のバックエンドへの呼び出し（状態の報告など）の認証にも使われます。この規則に合わない値では AI チャットがオフになります：**AI 設定** には ai-service が接続できないと表示され、ログの行がその理由を伝えます |
| `XEXAM_AI_LOG` | 未設定（`info`） | AI サービスのログのフィルター。`XEXAM_LOG` と同じです |
| `XEXAM_AI_SQLITE_JOURNAL` | 未設定（WAL） | `delete` = AI のデータベースを従来の DELETE ジャーナルのままにします。`XEXAM_SQLITE_JOURNAL` と同じです |

compose ファイルがホストの `/etc/localtime` をバックエンドと AI サービスにマウントするので、日時はホストのタイムゾーンに従います。このファイルのないホストや、仮想マシンがたいてい UTC で動いている Docker Desktop（Windows、macOS）では、[docs/ja/deployment.md](../docs/ja/deployment.md) の 3.1 節を読んでください。すべてのオプションの説明は、同じドキュメントの第 3 節にあります。AI プロバイダー、そのアドレス、キー、モデルはここではなく、アプリの **AI 設定** で設定します。

---

## アップグレード

どのアップグレードでも、ポートとボリューム `xexam_data` は同じままなので、データはそのまま残ります：`docker compose pull` の後に `docker compose up -d` です。v0.8 では 3 つ目のイメージと 2 つのボリュームが加わったので、v0.7.5 以前からの場合は新しい `docker-compose.yml` も必要です。v0.9 には新しい compose ファイルは不要です。データベースのスキーマが変わる場合、新しいバージョンは初回の起動時に、まずデータベースのバックアップのコピーをボリューム内の `/data/backups/` に書き出し（AI サービスも、自分のデータベースについて、自分のボリュームで同じことをします）、それから 1 つのトランザクションで移行します。コピーを書き出せない場合（ディスクがいっぱい、権限がないなど）、そのサービスは起動せず、データベースも変更しません。理由は `docker compose logs backend` と `docker compose logs ai-service` で確認できます。バックアップのコピーはデータベースとほぼ同じ大きさなので、アップグレードの前にそれだけの空き容量を確保してください。バックアップのコピーは自動では削除されません。

以下の節は、それぞれが行き着くバージョンに向けて書かれています。古いバージョンから現在のイメージ（v1.0）を pull すると、すべての手順が一度に行われます：それまでのスキーマにちなんだ名前の 1 つのバックアップが取られ、データベースはスキーマ 6 で終わります（そのため、ログには、ある節がスキーマ 5 と書いている箇所で `schema 6` と出て、`/api/info` は `"version": "1.0"` を示します）。

### Xexam v0.9.1 から v1.0 へ

v1.0 では、スマートフォンの表示と AI サポートが変わります。3 つのイメージ、ボリューム、`docker-compose.yml` はそのままです：**新しい compose ファイルは不要です**。先に 2 つのデータボリュームをバックアップしてください（コマンドは下の v0.9 の節にあります）。そのあと pull して起動します。

- **AI サービスの初回起動** は、AI のボリュームに `/data/backups/xexam-ai-schema3-<YYYYmmdd-HHMMSS>.db` のコピーを書き出し、そのデータベースをスキーマ 4 に移行します：ユーザーごとの 3 つのリクエスト上限（1 分あたり 5、1 時間あたり 20、1 日あたり 100）と、回答の長さ（1000 文字）です。チャット、メモリー、設定、キーはそのまま残ります。古い既定値の 300 文字のままのメモリー上限は 200 になります。ほかの値はそのままです。ログには `Upgraded the AI database to schema 4` と出ます。
- **メインのデータベースは変わりません**（スキーマ 6 のまま）。
- **新しいページのために設定するものはありません**。上限は **AI 設定 › AI 安全** にあります。1 日の数は AI サービスのタイムゾーン（その `TZ` 設定。例：`TZ=UTC`）の 0:00 に再び 0 から始まります。
- <http://localhost:8765/api/info> が `"version": "1.0"` を示し、`docker compose ps` が 3 つの正常なコンテナを示すことを確認してください。

ロールバック（v0.9.1 へ）：メインのデータベースは復元が不要ですが、v0.9.1 の AI サービスはスキーマ 4 を拒否するので、コピー `xexam-ai-schema3-….db` を元に戻す必要があり、アップグレード以降に作られた AI のチャット、設定、上限の変更、使用量の行はすべて失われます。[docs/ja/migration.md](../docs/ja/migration.md) の第 0 節を参照してください。


### Xexam v0.9 から v0.9.1 へ

することはありません：`docker compose pull` の後に `docker compose up -d` を実行します。データベースの変更も新しい設定もなく、3 つのイメージ、ボリューム、`docker-compose.yml`、`.env` はそのままです。変更点は[更新履歴](../docs/ja/changelog.md)にあります（更新履歴は今は `docs/<lang>/changelog.md` にあります）。

### Xexam v0.8 から v0.9 へ

v0.9 では、学習のための AI サポートが加わります（[更新履歴](../docs/ja/changelog.md) を参照）。3 つのイメージ、ボリューム、`docker-compose.yml` はそのままです：**新しい compose ファイルは不要** です。先に 2 つのデータボリュームの両方をバックアップしてから、pull して起動してください。

```bash
# 1. 2 つのボリュームをバックアップ（先に停止。バックアップファイルにはデータベース全体が入るので、コミットしない）
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v08.tgz -C /data .
docker run --rm -v xexam_ai_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-ai-data-v08.tgz -C /data .

# 2. 3 つのイメージを pull して起動し（down -v は使わない）、アップグレードの行を読む
docker compose pull
docker compose up -d
docker compose logs backend
docker compose logs ai-service
```

- **バックエンドの初回の起動** でコピー `/data/backups/xexam-schema5-<YYYYmmdd-HHMMSS>.db` を書き出し、データベースをスキーマ v6 に移行します：`attempts` に 2 つの新しい列、`session_key` と `started_at` が加わり、すでにあるすべての記録では空です。ほかは何も変わりません。ログには `Upgraded the database to schema 6` と出ます。
- **AI サービスの初回の起動** で、AI のボリュームにコピー `/data/backups/xexam-ai-schema2-<YYYYMMDD-HHMMSS>.db` を書き出し、そのデータベースをスキーマ 3 に移行します（学習のチャットとそのメッセージ、使用量の行の `purpose`）。チャット、メモリー、設定、キーはそのままです。ログには `Upgraded the AI database to schema 3` と出ます。
- **設定するものはありません**：**AI+ 機能** はこれまでどおりに働きます。AI を使えるユーザーには新しいコントロールが表示され、**AI+ 機能** のないユーザーには新しいものは何も表示されません。v0.9 より前に作られた記録にはセッションキーがないので、その最初の AI 解析は、その記録専用のチャットを作ります（[docs/ja/usage.md](../docs/ja/usage.md) の第 16 節）。
- <http://localhost:8765/api/info> に `"version": "0.9"` と表示され、`docker compose ps` に healthy なコンテナが 3 つ表示されることを確認します。

v0.8 へのロールバック：メインのデータベースは復元が不要です（v0.8 はスキーマ 6 のデータベースをそのまま開け、そのあとで保存された記録はキーを持たないだけです）。ただし、v0.8 の AI サービスはスキーマ 3 の AI のデータベースを拒否するので、そのコピー `xexam-ai-schema2-….db` を元に戻す必要があり、アップグレード以降の AI のチャット、設定、使用量の行はすべて失われます。[docs/ja/migration.md](../docs/ja/migration.md) の第 0b 節を参照してください。

### Xexam v0.7.5（または v0.7）から v0.8 へ

v0.8 では AI サービスが加わります：3 つ目のイメージ `xexam-ai-service`（コンテナと Compose のサービスの名前は `ai-service`）、2 つの新しいボリューム `xexam_ai_data`（AI のデータベースとそのキーファイル）と `xexam_link`（バックエンドと AI サービスが共有するトークンだけ）、そしてネットワーク `egress`（AI サービスだけが使うインターネット接続）です。**新しい `docker-compose.yml` をダウンロードしてください**。古いファイルのままでも AI 以外はすべて動き、**AI 設定** には ai-service が動いていないと表示されます。

```bash
# 1. ボリュームをバックアップ（先に停止。バックアップファイルにはデータベース全体が入るので、コミットしない）
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v075.tgz -C /data .

# 2. docker-compose.yml を新しいファイルに差し替える（.env はそのまま。古いファイルに加えた自分の変更は引き継ぐ）
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

# 3. 3 つのイメージを pull して起動し（down -v は使わない）、アップグレードの行を読む
docker compose pull
docker compose up -d
docker compose logs backend
```

- **初回の起動** でコピー `/data/backups/xexam-schema4-<YYYYmmdd-HHMMSS>.db` を書き出し、データベースをスキーマ v5 に移行します：`users` に 2 つの新しい列、**AI+** のスイッチ（全員オフ）と識別用のスタンプが加わります。ほかは何も変わらず、問題集、学習記録、ユーザー、設定はそのままです。ログには `Upgraded the database to schema 5` と出ます。
- **AI はオフ** のままで、管理者が **AI 設定** で設定するまで使えません（[docs/ja/usage.md](../docs/ja/usage.md) の第 17 節）。AI について移行するものはありません。管理者は AI+ 機能がオンになるとすぐに AI チャットを使えます。ほかのユーザーには **ユーザー管理** での **AI+ 機能** が必要で、そのスイッチは、ai-service が接続されていて AI+ 機能がオンの間だけオンにできます。AI のデータベース `xexam-ai.db` は ai-service の最初の起動で作られ、v0.8 のプレリリース版のものであれば、足りない列は自動で追加されます。
- **これからバックアップするもの**：2 つのデータボリューム `xexam_data` と `xexam_ai_data` の両方です。AI のボリュームには AI のデータベースの隣にキーファイル `secret.key` があるので、そのバックアップがあれば保存したキーを復号できます。キーを切り離しておくには `XEXAM_AI_SECRET_KEY` を設定してください。`xexam_link` はバックアップ不要です：空ならバックエンドが新しいトークンを作ります。
- <http://localhost:8765/api/info> にバージョンが表示され（`"0.8"`。現在のイメージでは `"1.0"`）、`docker compose ps` に healthy なコンテナが 3 つ（`xexam-frontend`、`xexam-backend`、`ai-service`）表示されることを確認します。

v0.7.5 へのロールバックとは、`backups/` のコピーと古い `docker-compose.yml` を元に戻すことです。[docs/ja/migration.md](../docs/ja/migration.md) の第 1 節を参照してください。

### Xexam v0.6 から v0.7 以降へ

> ⚠️ v0.7 からは **6 列形式の Excel しかインポートできず**、以前の版の 3 列・4 列形式から取り込んだ問題は、**起動のたびに出題停止** になります。v0.6 の問題集はどれもこれらの形式なので、アップグレードの後は、6 列形式のワークブックで更新するまで何も練習できません。問題集、学習記録、統計は残ります。

1. **アップグレードの前に**、各ワークブックの 6 列形式の版を用意し（形式：[docs/ja/usage.md](../docs/ja/usage.md) の第 12 節）、未提出の試験は提出するか破棄してください。その問題は出題停止になるので、その後は続きから回答できません。
2. **バックアップしてからアップグレードします**：

   ```bash
   # 1. ボリュームをバックアップ（先に停止。バックアップファイルにはデータベース全体が入るので、コミットしない）
   docker compose stop
   docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v06.tgz -C /data .

   # 2. docker-compose.yml を新しいファイルに差し替える（.env はそのまま）
   curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

   # 3. pull して起動し（down -v は使わない）、アップグレードの行を読む
   docker compose pull
   docker compose up -d
   docker compose logs backend
   ```

   新しい `docker-compose.yml` は、ホストのタイムゾーンに従い、コンテナのログを制限し、`.env` の `XEXAM_SQLITE_JOURNAL`、`XEXAM_COOKIE_SECURE`、`XEXAM_LOG` を渡し、v0.8 の AI サービスとそのボリュームを加えます。
3. **初回の起動で行われること**（リクエストに応える前）：コピー `/data/backups/xexam-schema3-<YYYYmmdd-HHMMSS>.db` を書き出し、データベースをスキーマ v5 に移行して（v0.7 と v0.7.5 は v4 まで）WAL モードに切り替えます。既存のユーザーは、既存のすべての問題集と **今後インポートする問題集も含める** をそのまま持ちます。問題集はすべて最初の管理者のものになり、以前の記録はすべて **試験** になり、問題集の管理権限を持つユーザーはまだいません。その後、古い形式の問題を出題停止にし、ログに問題集ごとに警告を 1 行出します。たとえば `retired 12 questions of bank "Kitchen basics": they use the old 3- / 4-column layout; import the six-column version of the workbook` です。
4. **古い問題集を、それぞれの 6 列形式のファイルで更新します**：**問題集の管理** → **Excel の取り込み** → **既存を更新** で問題集を選び、ファイルをアップロードして確定します。問題集の名前、所有者、それを見られるユーザーはそのままです。問題はこれから英語の文で識別されるので、回答履歴なしで始まります。古い問題は出題停止のままで、古い記録には引き続き表示されます。6 列形式のファイルを **新しい問題集** として取り込んでも構いません。
5. **その後**：誰を **問題集を管理できる** ユーザーにするか、各ユーザーがどの問題集を見られるかを決め（これから追加するユーザーは、選ぶまで問題集を見られません）、<http://localhost:8765/api/info> に `"version": "1.0"` と表示されることを確認します。AI は設定するまでオフです（前の節を参照）。

v0.6 へのロールバックとは、`backups/` のコピーを元に戻すことです。コマンドは [docs/ja/migration.md](../docs/ja/migration.md) の 2.6 節にあります。

### さらに古いバージョンから

v0.5、v0.4、Python 版 v0.2.x からも同じ手順ですが、いくつか手順が加わります（ログインが加わる、v0.5.1 でイメージの取得元が変わった、古い単一のコンテナを削除する必要がある）：[docs/ja/migration.md](../docs/ja/migration.md) を参照してください。すべてのロールバックの方法もそこにあります。

---

## ドキュメント

| ドキュメント | 内容 |
|---|---|
| [使い方](../docs/ja/usage.md) | すべてのページ、試験と練習、学習記録、設定、Excel の形式、取り込み、ユーザーとログイン、システムログ、AI チャットと AI 設定、iPhone |
| [デプロイ](../docs/ja/deployment.md) | 起動、環境設定、タイムゾーン、スマートフォンと LAN からのアクセス、Cloudflare Tunnel、AI プロバイダー、バックアップ、ヘルスチェック、ログ、復旧 |
| [移行とロールバック](../docs/ja/migration.md) | すべてのアップグレードの方法と、その確認とロールバック |
| [データベース](../docs/ja/database.md) | メインのデータベースと AI のデータベースのテーブル、問題 id、バックアップと復元、スキーマの移行 |
| [セキュリティ](../docs/ja/security.md) | 脅威モデル、Host / Origin ガード、ログインとロック、権限、アップロードの検査、Nginx の制限。AI サービス：本人の確認、暗号化したキー、管理者に見えるもの |
| [アーキテクチャ](../docs/ja/architecture.md) | フロントエンド、Nginx、バックエンド、AI サービスの組み合わせ方、データの境界、ディレクトリ構成、リクエストの流れ |
| [API](../docs/ja/api.md)（[OpenAPI](../docs/en/openapi.yaml)、英語のみ） | すべてのエンドポイント、AI のルートとそのイベントストリーム、呼び出せる人、エラー |
| [開発](../docs/ja/development.md) | ビルドとテスト（バックエンド、AI サービス、フロントエンド）、スクリーンショットとアイコンのツール、CI とリリース |

---

## ライセンス

Xexam は [MIT ライセンス](../LICENSE) で公開されています。Copyright (c) 2026 Yumin-Space.

このライセンスの対象は、このリポジトリ内のソフトウェアです。リポジトリに試験問題は含まれていません。取り込む問題集はご自身のファイルであり、それを使用してよいかどうかの確認はご自身の責任で行ってください。

---

## 更新履歴

バージョンごとの変更点（新しい順）は [docs/ja/changelog.md](../docs/ja/changelog.md) にあります。
