[English](../en/deployment.md) | **日本語** | [简体中文](../zh-CN/deployment.md)

# デプロイ

> 新しいアーキテクチャ：`nginx`（外部に公開されるのはこれだけ）+ `backend`（Rust、プライベートネットワーク内）+ SQLite（名前付きボリューム `xexam_data`）。構成図は [architecture.md](architecture.md)、セキュリティ境界は [security.md](security.md) を参照してください。

## 1. 前提

- Docker（Compose v2 を含む）が入ったマシン。x86（amd64）でも ARM（arm64）でも構いません。
  - arm64 イメージは CI 上で QEMU を使って**ビルドして起動する**ところまで検証済みです（`/api/info` が `aarch64` を返し、Nginx の `/healthz` も正常でした）。ただし、**本物の ARM ハードウェア上では動かしていません**。
  - バックエンドは、x86 のビルドマシン上で zig を使って aarch64 版をクロスコンパイルしています（キャッシュが空の状態で CI は約 5 分かかります。QEMU 上で Rust をコンパイルする必要はありません）。
    zig のリンカーは、rustc が aarch64 向けに既定で付ける `--fix-cortex-a53-843419` を受け付けないため、aarch64 のバイナリには Cortex-A53 コア向けのこのエラッタ回避策が**含まれていません**。
    Oracle Cloud の Ampere A1 は Ampere Altra（Wikipedia によれば Arm Neoverse N1 コア）を使っており、Cortex-A53 ではないため影響を受けません。
    Cortex-A53 コアのボード（一部の古い開発ボード）で動かす場合は、このイメージを使わず、先に知らせてください。

## 2. 起動

### 公開済みのイメージを pull する（推奨）

必要なのは、リポジトリの `docker/` フォルダにある [`docker-compose.yml`](../../docker/docker-compose.yml) というこの 1 つのファイルだけです。専用のフォルダにダウンロードして（例：`curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml`）、そのフォルダでコマンドを実行します。このファイルは 2 つのイメージを Docker Hub（`docker.io/yuminspace/xexam-backend` と `docker.io/yuminspace/xexam-frontend`）から pull します。そこでは公開されているので、`docker login` は必要ありません。同じタグは、非公開のミラーとして `ghcr.io/yumin-space/` にもあります（こちらは、`read:packages` を付けた classic の GitHub トークンで `docker login ghcr.io` する必要があります）。

```bash
# docker login は不要です：イメージは Docker Hub で公開されています
docker compose up -d
```

`docker compose up -d` で **2 つのコンテナ**が pull されて起動します：`xexam-frontend`（Nginx、外部に公開されるのはこれだけ）と `xexam-backend`（Rust、プライベートネットワーク内）です。次に、ブラウザで <http://localhost:8765> を開き、右上の三本線のボタンでメニューを開いて **Question banks** を選び、Excel ファイルをインポートします。画面は最初は英語で表示されます。同じメニューの **Settings** で日本語または中国語に切り替えられます（日本語表示では、この 2 つはそれぞれ「問題集の管理」「画面設定」です）。メニューの一番下の「Xexam v0.6」を選ぶと、バージョン、サーバーのアーキテクチャ、更新履歴を表示する「Xexam について」のページ（`#/about`）が開きます。

### ソースからビルドする（コードを変更するとき）

```bash
git clone git@github.com:Yumin-Space/xexam.git
cd xexam
docker compose -f docker/docker-compose.build.yml up -d --build
```

初回のビルドには数分かかります（Rust とフロントエンドをコンパイルするため）。2 回目以降はキャッシュが効きます。クローンしたリポジトリでは、compose ファイルと `.env` は `docker/` にあり、`docker/docker-compose.build.yml` はリポジトリのルートをビルドコンテキストにします。以下のコマンドは、リポジトリのルートで `-f docker/docker-compose.build.yml` を付けて実行してください（例：`docker compose -f docker/docker-compose.build.yml logs -f`）。

フロントエンドのイメージのビルドでは、3 つの README（`README.md`、`README/README.ja.md`、`README/README.zh-CN.md`）も使います。`nginx/Dockerfile` がビルド段階でこれらをコピーし（`COPY README.md /` と `COPY README/README.ja.md README/README.zh-CN.md /README/`）、その更新履歴を「Xexam について」のページに組み込みます。そのため `.dockerignore` は `README.md` と `README/` を除外していません。完成したイメージに入るのはビルドした静的ファイルだけです。

| 操作 | コマンド |
|---|---|
| 状態を見る | `docker compose ps`（2 つのコンテナとも `healthy` になっているはずです） |
| ログを見る | `docker compose logs -f` |
| 停止 / 再起動 | `docker compose stop` / `docker compose start` |
| 最新バージョンに更新 | `docker compose pull && docker compose up -d`（ソースからビルドした場合：`git pull && docker compose -f docker/docker-compose.build.yml up -d --build`）。お使いの `docker-compose.yml` にまだ `ghcr.io` と書かれている場合（v0.5.1 より前のファイル）は、先に新しいファイルに差し替えてください。データボリュームはそのまま残ります。v0.4 から更新する場合は、先に第 6 節を読んでください |
| バージョンとアーキテクチャ | <http://localhost:8765/api/info> |

> ⚠️ `docker compose down -v` は実行しないでください。`-v` を付けるとデータボリューム `xexam_data` が削除されます。問題集とすべての学習記録は、このボリュームに入っています。

**`docker/docker-compose.yml` と `docker/docker-compose.build.yml` は、同じプロジェクト名（`xexam`）、同じボリューム（`xexam_data`）、同じコンテナ名を使います**。そのため、両者を同時に実行することはできません。切り替える前に `docker compose stop` を実行してください。データは共通です。詳しくは [migration.md](migration.md) を参照してください。

## 3. 設定（`.env`）

`docker-compose.yml` と同じ場所に `.env` を作成します。クローンしたリポジトリでは `docker/.env` です。リポジトリのルートから `-f docker/...` を付けて実行しても、Compose はこのファイルを読みます（以前にクローンしたリポジトリでルートに `.env` がある場合は、`docker/` に移動してください）。コミットされず、イメージにも含まれません。テンプレートは [`.env.example`](../../docker/.env.example) です：

| 変数 | 既定 | 役割 |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | ホスト側で公開する「アドレス:ポート」で、Nginx に対応します。`IP:ポート番号` の形式で指定する必要があります。`0.0.0.0:8765` = LAN 内から接続可能。⚠️ ポート番号だけを書くと、すべてのネットワークインターフェースで公開されます |
| `XEXAM_ALLOWED_HOSTS` | 空 | アクセスを許可するドメイン / IP。カンマ / スペース / セミコロン区切り |
| `XEXAM_ALLOWED_ORIGINS` | 空 | プロキシが Host をオリジンサーバーのアドレスに書き換える場合：公開ドメイン（`https://` を含む）を指定します |
| `XEXAM_ALLOW_ANY_HOST` | `0` | `1` = Host を検査しません（前段にすでに認証がある場合のみ） |
| `XEXAM_PARSE_TIMEOUT_SECS` | `30` | Excel 1 ファイルの解析にかける最大時間 |
| `TZ` | `Asia/Tokyo` | データベース内のタイムスタンプに使うタイムゾーン |

`.env` を変更したら、`docker compose up -d` を実行します（必要に応じてコンテナが再作成されます）。

## 4. スマートフォン / 外部からアクセスする

### 同じ Wi-Fi 内の iPhone

**信頼できるネットワーク**でのみ行ってください。`.env`：

```ini
XEXAM_BIND=0.0.0.0:8765
XEXAM_ALLOWED_HOSTS=192.168.1.20      # パソコンの LAN 内 IP
```

iPhone の Safari で `http://192.168.1.20:8765` を開きます（iOS 15.4 以降が必要です）。

### Cloudflare Tunnel でインターネットからアクセスする

バックエンドはもともと外部に公開されていないので、トンネルの向き先は **Nginx**（`XEXAM_BIND` で公開したポート）だけで済みます。必要な作業は 3 つです：

1. cloudflared から Nginx に接続できるようにします。同じマシン上なら既定のままにして `http://localhost:8765` を指定します。LAN 内の別のマシンなら、`XEXAM_BIND` を `0.0.0.0:8765`（または具体的な LAN の IP）に設定します。
2. Public Hostname：`xexam.your-domain.com` → Service は `HTTP`（HTTPS ではありません）→ `host-IP:8765`。
3. Cloudflare Access にポリシーを 1 つ追加し、**自分だけ**を許可します。ポリシーを設定していないドメイン名は、全世界に公開しているのと同じです。

Host の代表的な 2 つのケース（`.env`）：

| トンネルの動作 | 設定 |
|---|---|
| Host がそのままあなたのドメイン名として転送される | `XEXAM_ALLOWED_HOSTS=xexam.your-domain.com` |
| `httpHostHeader` を設定していて、Host が `192.168.1.20:8765` に書き換えられる | `XEXAM_ALLOWED_HOSTS=192.168.1.20` + `XEXAM_ALLOWED_ORIGINS=https://xexam.your-domain.com` |
| 不明 | まず 1 行目のとおりに設定します。拒否（403）されると、ページに、サーバーが受け取った Host / Origin が表示されます。それを該当するオプションに追加して `up -d` を実行してください |

Nginx は、ブラウザ（またはトンネル）から届いた `Host` を**そのまま**バックエンドに渡し、`X-Forwarded-*` を空にします。バックエンドが読むのは実際の `Host` だけです。

⚠️ アプリ自体にはログイン機能がありません。cloudflared だけがこのポートに接続できるようにし、ルーターでポートフォワーディングをしないでください。Linux では、ホストのファイアウォールでは Docker が公開したポートを止められません。`DOCKER-USER` チェーンを使うか、具体的な LAN の IP を指定してください。

どこまで検証したか（ありのままに書きます）：

- 移行の際に、本物の Chrome + HTTPS リバースプロキシを使って、**旧 Python 版**に対し、Host の 2 つのケース（Host を保持 / Host を書き換え）でブラウザ操作の一連の流れを検証しました。そのスクリプトは旧 Python コードに依存していたため、旧コードとともに削除しました。
- **新しいスタック**については、`tests/integration/stack_check.py` がリクエストヘッダー単位の検査で「Nginx を通った後でも、Host / Origin ガードが引き続き有効で、`X-Forwarded-*` は信頼されない」ことを確認し、`nginx_check.py` が Host がそのまま透過されることを確認します。
- 本物の Cloudflare では**テストしていません**。初めて公開するときは、上の表の「不明」の行に従い、ページに表示される 403 のメッセージを確認してください。

## 5. データ

- 場所：名前付きボリューム `xexam_data` → コンテナ内の `/data/xexam.db`。更新、再作成、コンテナの削除をしても失われません。データベースには、問題集、学習記録、そして v0.5 からは設定も保存されます。
- v0.5 からは、ボリューム内に `/data/backups/` フォルダができることもあります。ここには、スキーマ移行の前に書き出したコピーが入ります（第 6 節）。プログラムがこれらのファイルを削除することはありません。中身を見るには：`docker run --rm -v xexam_data:/data alpine ls -l /data/backups`。
- バックアップと復元：[database.md](database.md) の第 5 節を参照してください。ボリュームをバックアップすると `backups/` も含まれます。
- Docker を使わずにバックエンドを直接実行する（開発）：[development.md](development.md) を参照してください。

## 6. v0.4 から v0.5 へのアップグレード

デプロイの面では、v0.5 でもイメージは同じ 2 つで、ポートとボリュームも同じです。新しい環境変数もありません。変わるのは画面（三本線のボタンで開くメニュー、問題集、学習記録、設定、「Xexam について」のページ）、データベースのスキーマ、そして v0.5.1 からはレジストリです。新しい `docker-compose.yml` は、`ghcr.io` ではなく Docker Hub（`docker.io/yuminspace/…`、公開、ログイン不要）から pull します。プロジェクト名とボリューム `xexam_data` は変わらないので、レジストリを切り替えてもデータはすべて残ります。ソースからビルドする場合は第 2 節のコマンドを使います。compose ファイルは `docker/` に移ったので、リポジトリのルートにある `.env` は `docker/.env` に移動してください（Compose はルートの `.env` を読まなくなりました）。フロントエンドのイメージは、3 つの README も読み込むようになりました（第 2 節）。

1. まずボリュームをバックアップします。手順は [database.md](database.md) の第 5 節のとおりです。プログラム自身もコピーを書き出しますが、ボリュームの外にもコピーがあるほうが安全です。
2. お使いの `docker-compose.yml` を新しいファイルに差し替えてから（または、その 2 つの `image:` の行を `docker.io/yuminspace/xexam-backend:latest` と `docker.io/yuminspace/xexam-frontend:latest` に変更してから）、いつもどおりに更新します：`docker compose pull && docker compose up -d`。
3. 初回の起動時に、バックエンドがデータベースをスキーマ v2 にアップグレードします。
   - まず、古いデータベースのコピーを `/data/backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db` に書き出します。このコピーを書き出せない場合（ディスクがいっぱい、権限がないなど）、バックエンドはエラーで終了し、データベースは変更されません。Docker が再起動を繰り返し、起動のたびにもう一度試みます。Nginx は healthy なバックエンドを待つため、起動しません。理由は `docker compose logs backend` で確認できます。
   - 次に、1 つのトランザクションで移行します。既存の問題は 1 つの問題集になり、問題 id もそのままなので、間違いノート、統計、成績履歴は以前のまま残ります。
   - ログの `Upgraded the database to schema 2. A copy of the previous version was saved first: …` という行に、コピーのファイル名が出ます。
4. 確認：`/api/info` に `"version": "0.6"` と以前と同じ `bankCount` が表示されること。**Question banks**（日本語表示では「問題集の管理」）に、すべての問題を含む問題集が 1 つあること。**Study records**（日本語表示では「学習記録」）に、以前の試験が並んでいること。

2 回目以降の起動では、移行は行われず、余分なものも書き出しません。

v0.4 への**ロールバック**は、そのコピーを復元して、v0.4 のイメージ（タグ `v0.4`。非公開のミラー `ghcr.io/yumin-space/` にしかないので、先に `docker login ghcr.io` を実行します。Docker Hub は v0.5.1 から）を動かすことを意味します。問題集が 2 つ以上になったら、移行済みのデータベースを v0.4 で動かさないでください。v0.4 はすべての問題集を 1 つとして扱うため、次にインポートしたときに、ほかの問題集の問題を無効にしてしまいます。具体的なコマンドは [migration.md](migration.md) の 1.4 節、背景は [database.md](database.md) の第 6 節にあります。

## 7. ヘルスチェックと停止

- `xexam-frontend`：`GET /healthz`（Host の許可リストを経由しません）。`xexam-backend`：`xexam-backend healthcheck`（コンテナ内で自分自身に `Host: 127.0.0.1` を付けてアクセスします）。バックエンドが healthy になってから Nginx が起動します（`depends_on`）。
- バックエンドが落ちたり再起動したりしても、Nginx は終了しません。`/api/` が一時的に JSON の 502（`后端暂时连不上`。「バックエンドに一時的に接続できません」）を返すだけで、バックエンドが復旧すれば自動的に元に戻ります。
- `docker compose stop`：バックエンドは 8 秒以内に正常終了します。インポートのトランザクションは、コミットが完了するかロールバックされるかのどちらかになります。

## 8. リソースとイメージ

| | |
|---|---|
| `xexam-frontend` | nginx-unprivileged + 静的ファイル。ポート 8080 で待ち受け。ルートファイルシステムは読み取り専用 |
| `xexam-backend` | 静的リンクされた単一の実行ファイル + tzdata。uid 10001。ルートファイルシステムは読み取り専用で、書き込めるのは `/data` と `/tmp` だけ |
| ネットワーク | `internal`（バックエンド + Nginx。外部ネットワークにはアクセスできません）と `public`（Nginx だけ） |

## 9. リリース

`main` にプッシュすると、[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) がまずバージョン、変更履歴、ドキュメントの検査を行い、続いて 4 グループの検査（フロントエンド、バックエンド、Nginx と全スタック、arm64）を実行します。すべて通過した場合にだけ、`xexam-frontend` と `xexam-backend` の 2 つのイメージ（amd64 + arm64）を Docker Hub（`docker.io/yuminspace/`、公開）と、非公開のミラーとして `ghcr.io/yumin-space/` に公開します。タグはどちらも `latest`、`v<VERSION>`、`sha-<コミット>` で、対応する GitHub Release も作成します（そのリリースノートには、Docker Hub からの pull コマンドが最初に載ります）。`dev` へのプッシュと `main` へのプルリクエストでは、同じ検査を行うだけで、公開はしません。詳しくは [development.md](development.md) の第 7 節を参照してください。

Docker Hub には、リポジトリのシークレット `DOCKERHUB_USERNAME` と `DOCKERHUB_TOKEN`（**Read & Write** の権限を持つアクセストークン）の 2 つが必要で、必要に応じてリポジトリ変数 `DOCKERHUB_NAMESPACE` も設定できます。シークレットがない場合や、`docker/docker-compose.yml` がその名前空間の Docker Hub のイメージを指していない場合、公開ジョブは何もプッシュせずに止まります。ghcr.io では、新しいイメージ名を初めて公開するときに、GitHub のパッケージ設定で、このリポジトリを **Manage Actions access** に追加し、ロールとして **Admin** を選んでください。そうしないとプッシュが拒否されます（`permission_denied: write_package`）。

## 初回起動とアカウント（v0.6）

- インストールまたはアップグレードの後、Xexam を開いて `admin` / `admin` でログインし、すぐに自分のパスワードを設定してください。それまでは、このページを開ける人なら誰でも既定のパスワードを使えます。既定のパスワードのままの間は、バックエンドの起動のたびにリマインダーが出力されます（`docker compose logs xexam-backend`）。
- ほかの人は **ユーザー管理**（メニュー、管理者のみ）で追加します。
- ログインできなくなったとき：`docker compose exec xexam-backend xexam-backend reset-password admin` で新しい仮パスワードが表示されます。パスワードの間違いによるロックは `docker compose restart xexam-backend`（またはほかの管理者の **ロック解除**）で解除されます。
- Cloudflare Access の背後では 2 回ログインします（Access、次に Xexam）。家庭内ネットワークの HTTP でも使えます。`XEXAM_COOKIE_SECURE=1` は、ブラウザが常に HTTPS で Xexam に接続する場合にだけ設定してください。
