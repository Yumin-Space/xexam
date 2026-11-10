[English](deployment.md) | **日本語** | [简体中文](deployment.zh-CN.md)

# デプロイ

> 新しいアーキテクチャ：`nginx`（外部に公開されるのはこれだけ）+ `backend`（Rust、プライベートネットワーク内）+ SQLite（名前付きボリューム `xexam_data`）。構成図は [architecture.ja.md](architecture.ja.md)、セキュリティ境界は [security.ja.md](security.ja.md) を参照してください。

## 1. 前提

- Docker（Compose v2 を含む）が入ったマシン。x86（amd64）でも ARM（arm64）でも構いません。
  - arm64 イメージは CI 上で QEMU を使って**ビルドして起動する**ところまで検証済みです（`/api/info` が `aarch64` を返し、Nginx の `/healthz` も正常でした）。ただし、**本物の ARM ハードウェア上では動かしていません**。
  - バックエンドは、x86 のビルドマシン上で zig を使って aarch64 版をクロスコンパイルしています（キャッシュが空の状態で CI は約 5 分かかります。QEMU 上で Rust をコンパイルする必要はありません）。
    zig のリンカーは、rustc が aarch64 向けに既定で付ける `--fix-cortex-a53-843419` を受け付けないため、aarch64 のバイナリには Cortex-A53 コア向けのこのエラッタ回避策が**含まれていません**。
    Oracle Cloud の Ampere A1 は Ampere Altra（Wikipedia によれば Arm Neoverse N1 コア）を使っており、Cortex-A53 ではないため影響を受けません。
    Cortex-A53 コアのボード（一部の古い開発ボード）で動かす場合は、このイメージを使わず、先に知らせてください。

## 2. 起動

### 公開済みのイメージを pull する（推奨）

必要なのは、リポジトリにある [`docker-compose.yml`](../docker-compose.yml) というこの 1 つのファイルだけです。

```bash
docker login ghcr.io -u Yumin-Space      # 初回のみ。イメージは非公開なので、パスワードには read:packages 権限を持つトークンを入力します
docker compose up -d
```

ブラウザで <http://localhost:8765> を開き、`题库管理`（問題バンク管理）から Excel をアップロードします。**2 つのコンテナ**が pull されて起動します：`xexam-frontend`（Nginx、外部に公開されるのはこれだけ）と `xexam-backend`（Rust、プライベートネットワーク内）です。

### ソースからビルドする（コードを変更するとき）

```bash
git clone git@github.com:Yumin-Space/xexam.git
cd xexam
docker compose -f docker-compose.build.yml up -d --build
```

初回のビルドには数分かかります（Rust とフロントエンドをコンパイルするため）。2 回目以降はキャッシュが効きます。

| 操作 | コマンド |
|---|---|
| 状態を見る | `docker compose ps`（2 つのコンテナとも `healthy` になっているはずです） |
| ログを見る | `docker compose logs -f` |
| 停止 / 再起動 | `docker compose stop` / `docker compose start` |
| 最新バージョンに更新 | `docker compose pull && docker compose up -d`（ソースからビルドした場合：`git pull && docker compose -f docker-compose.build.yml up -d --build`） |
| バージョンとアーキテクチャ | <http://localhost:8765/api/info> |

> ⚠️ `docker compose down -v` は実行しないでください。`-v` を付けるとデータボリューム `xexam_data` が削除され、問題バンクも学習記録もすべて失われます。

**`docker-compose.yml` と `docker-compose.build.yml` は、同じプロジェクト名（`xexam`）、同じボリューム（`xexam_data`）、同じコンテナ名を使います**。そのため、両者を同時に実行することはできません。切り替える前に `docker compose stop` を実行してください。データは共通です。詳しくは [migration.ja.md](migration.ja.md) を参照してください。

## 3. 設定（`.env`）

`docker-compose.yml` と同じ場所に `.env` を作成します（コミットされず、イメージにも含まれません。テンプレートは [`.env.example`](../.env.example) です）：

| 変数 | デフォルト | 役割 |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | ホスト側で公開する「アドレス:ポート」で、Nginx に対応します。`IP:ポート番号` の形式で指定する必要があります。`0.0.0.0:8765` = LAN 内から接続可能。⚠️ ポート番号だけを書くと = すべてのネットワークインターフェース |
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
2. Public Hostname：`xexam.お使いのドメイン.com` → Service は `HTTP`（HTTPS ではありません）→ `ホストIP:8765`。
3. Cloudflare Access にポリシーを 1 つ追加し、**自分だけ**を許可します。ポリシーを設定していないドメイン名は、全世界に公開しているのと同じです。

Host の代表的な 2 つのケース（`.env`）：

| トンネルの動作 | 設定 |
|---|---|
| Host がそのままあなたのドメイン名として転送される | `XEXAM_ALLOWED_HOSTS=xexam.お使いのドメイン.com` |
| `httpHostHeader` を設定していて、Host が `192.168.1.20:8765` に書き換えられる | `XEXAM_ALLOWED_HOSTS=192.168.1.20` + `XEXAM_ALLOWED_ORIGINS=https://xexam.お使いのドメイン.com` |
| 不明 | まず 1 行目のとおりに設定します。拒否（403）されると、ページに、サーバーが受け取った Host / Origin が表示されます。それを該当するオプションに追加して `up -d` を実行してください |

Nginx は、ブラウザ（またはトンネル）から届いた `Host` を**そのまま**バックエンドに渡し、`X-Forwarded-*` を空にします。バックエンドが読むのは実際の `Host` だけです。

⚠️ アプリ自体にはログイン機能がありません。cloudflared だけがこのポートに接続できるようにし、ルーターでポートフォワーディングをしないでください。Linux では、ホストのファイアウォールでは Docker が公開したポートを止められません。`DOCKER-USER` チェーンを使うか、具体的な LAN の IP を指定してください。

どこまで検証したか（ありのままに書きます）：

- 移行の際に、本物の Chrome + HTTPS リバースプロキシを使って、**旧 Python 版**に対し、Host の 2 つのケース（Host を保持 / Host を書き換え）でブラウザ操作の一連の流れを検証しました。そのスクリプトは旧 Python コードに依存していたため、旧コードとともに削除しました。
- **新しいスタック**については、`tests/integration/stack_check.py` がリクエストヘッダー単位の検査で「Nginx を通った後でも、Host / Origin ガードが引き続き有効で、`X-Forwarded-*` は信頼されない」ことを確認し、`nginx_check.py` が Host がそのまま透過されることを確認します。
- 本物の Cloudflare では**テストしていません**。初めて公開するときは、上の表の「不明」の行に従い、ページに表示される 403 のメッセージを確認してください。

## 5. データ

- 場所：名前付きボリューム `xexam_data` → コンテナ内の `/data/xexam.db`。更新、再作成、コンテナの削除をしても失われません。
- バックアップと復元：[database.ja.md](database.ja.md) の第 5 節を参照してください。
- Docker を使わずにバックエンドを直接実行する（開発）：[development.ja.md](development.ja.md) を参照してください。

## 6. ヘルスチェックと停止

- `xexam-frontend`：`GET /healthz`（Host の許可リストを経由しません）。`xexam-backend`：`xexam-backend healthcheck`（コンテナ内で自分自身に `Host: 127.0.0.1` を付けてアクセスします）。バックエンドが healthy になってから Nginx が起動します（`depends_on`）。
- バックエンドが落ちたり再起動したりしても、Nginx は終了しません。`/api/` が一時的に JSON の 502（`后端暂时连不上`。「バックエンドに一時的に接続できません」）を返すだけで、バックエンドが復旧すれば自動的に元に戻ります。
- `docker compose stop`：バックエンドは 8 秒以内に正常終了します。インポートのトランザクションは、コミットが完了するかロールバックされるかのどちらかになります。

## 7. リソースとイメージ

| | |
|---|---|
| `xexam-frontend` | nginx-unprivileged + 静的ファイル。ポート 8080 で待ち受け。ルートファイルシステムは読み取り専用 |
| `xexam-backend` | 静的リンクされた単一の実行ファイル + tzdata。uid 10001。ルートファイルシステムは読み取り専用で、書き込めるのは `/data` と `/tmp` だけ |
| ネットワーク | `internal`（バックエンド + Nginx。外部ネットワークにはアクセスできません）と `public`（Nginx だけ） |

## 8. リリース

`main` にプッシュすると、[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) がまず、フロントエンド、バックエンド、Nginx と全スタック、arm64 の 4 グループの検査を実行します。すべて通過すると、`xexam-frontend` と `xexam-backend` の 2 つのイメージ（amd64 + arm64）を `ghcr.io/yumin-space/` に公開します。タグは `latest`、`v<VERSION>`、`sha-<コミット>` で、対応する GitHub Release も作成します。プルリクエストとそれ以外のブランチでは、検査だけを行い、公開はしません。

新しいイメージ名を初めて公開するときは、GitHub のパッケージ設定で、このリポジトリを **Manage Actions access** に追加し、ロールとして **Admin** を選んでください。そうしないとプッシュが拒否されます（`permission_denied: write_package`）。
