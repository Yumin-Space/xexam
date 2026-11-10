**English** | [日本語](deployment.ja.md) | [简体中文](deployment.zh-CN.md)

# Deployment

> New architecture: `nginx` (the only service exposed to the outside) + `backend` (Rust, private network) + SQLite (named volume `xexam_data`). For the structure diagrams see [architecture.md](architecture.md); for the security boundaries see [security.md](security.md).

## 1. Prerequisites

- A machine with Docker installed (including Compose v2); either x86 (amd64) or ARM (arm64) works.
  - The arm64 images were verified in CI under QEMU: **built and started** (`/api/info` reports `aarch64`, and Nginx `/healthz` is healthy). **They have not been run on real ARM hardware.**
  - The backend is cross-compiled for aarch64 with zig on an x86 build machine (about 5 minutes in CI with a cold cache; Rust does not have to be compiled under QEMU).
    zig's linker does not accept `--fix-cortex-a53-843419`, which rustc adds for aarch64 by default, so the aarch64 binary **does not include** this workaround for the Cortex-A53 erratum.
    Oracle Cloud's Ampere A1 uses Ampere Altra (according to Wikipedia, Arm Neoverse N1 cores), not Cortex-A53, so it is not affected.
    If you want to run on a board with Cortex-A53 cores (some older development boards), please do not use this image, and let me know first.

## 2. Starting

### Pull the prebuilt images (recommended)

You only need one file from the repository, [`docker-compose.yml`](../docker-compose.yml):

```bash
docker login ghcr.io -u Yumin-Space      # first time only; the images are private, and the password is a token with read:packages permission
docker compose up -d
```

Open <http://localhost:8765> in a browser and upload your Excel file under `题库管理` ("Manage question bank"). This pulls and starts **two containers**: `xexam-frontend` (Nginx, the only one exposed to the outside) and `xexam-backend` (Rust, private network).

### Build from source (when you change the code)

```bash
git clone git@github.com:Yumin-Space/xexam.git
cd xexam
docker compose -f docker-compose.build.yml up -d --build
```

The first build takes a few minutes (it compiles Rust and the frontend); later builds use the cache.

| Action | Command |
|---|---|
| Check status | `docker compose ps` (both containers should be `healthy`) |
| View logs | `docker compose logs -f` |
| Stop / start again | `docker compose stop` / `docker compose start` |
| Update to the latest version | `docker compose pull && docker compose up -d` (for a source build: `git pull && docker compose -f docker-compose.build.yml up -d --build`) |
| Version and architecture | <http://localhost:8765/api/info> |

> ⚠️ Do not run `docker compose down -v`: `-v` deletes the data volume `xexam_data`, which holds the question bank and all study records.

**`docker-compose.yml` and `docker-compose.build.yml` use the same project name (`xexam`), the same volume (`xexam_data`) and the same container names**, so the two cannot run at the same time; run `docker compose stop` before switching. The data is shared between them; see [migration.md](migration.md).

## 3. Configuration (`.env`)

Create a `.env` file next to `docker-compose.yml` (it is not committed and does not go into the images; the template is [`.env.example`](../.env.example)):

| Variable | Default | Purpose |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | The "address:port" published on the host, for Nginx. It must be written as `IP:port`. `0.0.0.0:8765` = reachable from the LAN. ⚠️ A bare port number = all network interfaces |
| `XEXAM_ALLOWED_HOSTS` | empty | Which domain names / IPs may be used to access it, separated by commas, spaces or semicolons |
| `XEXAM_ALLOWED_ORIGINS` | empty | When a proxy rewrites Host to the origin server's address: enter the public domain (including `https://`) |
| `XEXAM_ALLOW_ANY_HOST` | `0` | `1` = do not check Host (only when authentication is already in front) |
| `XEXAM_PARSE_TIMEOUT_SECS` | `30` | Maximum time to parse one Excel file |
| `TZ` | `Asia/Tokyo` | Time zone used for timestamps in the database |

After changing `.env`, run `docker compose up -d` (it recreates the containers as needed).

## 4. Letting your phone / the outside connect

### An iPhone on the same Wi-Fi

Do this only on a **trusted network**. `.env`:

```ini
XEXAM_BIND=0.0.0.0:8765
XEXAM_ALLOWED_HOSTS=192.168.1.20      # the LAN IP of your computer
```

Open `http://192.168.1.20:8765` in Safari on the iPhone (iOS 15.4 or later is required).

### From the internet through Cloudflare Tunnel

The backend is not exposed in the first place, so the tunnel only needs to point at **Nginx** (the port published by `XEXAM_BIND`). Three things to do:

1. Make sure cloudflared can reach Nginx: on the same machine, keep the default and point it at `http://localhost:8765`; on another machine on the LAN, set `XEXAM_BIND` to `0.0.0.0:8765` (or to a specific LAN IP).
2. Public Hostname: `xexam.your-domain.com` → Service `HTTP` (not HTTPS) → `host-IP:8765`.
3. Add a policy in Cloudflare Access that **allows only you**. A domain with no policy is open to the whole world.

Two common Host situations (`.env`):

| What the tunnel does | Setting |
|---|---|
| Host is passed through unchanged as your domain | `XEXAM_ALLOWED_HOSTS=xexam.your-domain.com` |
| `httpHostHeader` is set, so Host is rewritten to `192.168.1.20:8765` | `XEXAM_ALLOWED_HOSTS=192.168.1.20` + `XEXAM_ALLOWED_ORIGINS=https://xexam.your-domain.com` |
| Not sure | Start with the first row. When a request is rejected (403), the page states the Host / Origin it saw; add them to the matching option and run `up -d` again |

Nginx passes the `Host` sent by the browser (or the tunnel) to the backend **unchanged** and clears `X-Forwarded-*`; the backend reads only the real `Host`.

⚠️ The app itself has no login: let only cloudflared reach this port, and do not set up port forwarding on the router. On Linux, the host firewall cannot block ports published by Docker; use the `DOCKER-USER` chain or a specific LAN IP.

How far this has been verified (stated honestly):

- During the migration, the complete browser flow for both Host situations (Host preserved / Host rewritten) was run against the **old Python version**, using a real Chrome and an HTTPS reverse proxy. That script depended on the old Python code and was removed along with it.
- For the **new stack**: `tests/integration/stack_check.py` uses header-level checks to verify that "after passing through Nginx, the Host / Origin guard still works and `X-Forwarded-*` is not trusted"; `nginx_check.py` verifies that Host is passed through unchanged.
- It has **not** been tested with real Cloudflare. For the first go-live, follow the "Not sure" row in the table above and read the 403 message the page gives.

## 5. Data

- Location: named volume `xexam_data` → `/data/xexam.db` inside the container. Updating, rebuilding or deleting containers does not lose it.
- Backup and restore: see section 5 of [database.md](database.md).
- Running the backend directly without Docker (development): see [development.md](development.md).

## 6. Health checks and stopping

- `xexam-frontend`: `GET /healthz` (it does not go through the Host allow-list). `xexam-backend`: `xexam-backend healthcheck` (inside the container it requests itself with `Host: 127.0.0.1`). Nginx starts only after the backend is healthy (`depends_on`).
- When the backend is down or restarting, Nginx does not exit; `/api/` just returns a JSON 502 for a while (`后端暂时连不上`, "the backend is temporarily unreachable") and recovers automatically once the backend is back.
- `docker compose stop`: the backend exits gracefully within 8 seconds; an import transaction either finishes committing or is rolled back.

## 7. Resources and images

| | |
|---|---|
| `xexam-frontend` | nginx-unprivileged + static files; listens on 8080; read-only root filesystem |
| `xexam-backend` | a single statically linked executable + tzdata; uid 10001; read-only root filesystem, only `/data` and `/tmp` are writable |
| Networks | `internal` (backend + Nginx, no access to the outside network) and `public` (Nginx only) |

## 8. Releasing

On a push to `main`, [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) first runs four check groups (frontend, backend, Nginx and full stack, arm64). Only if all of them pass does it publish the two images `xexam-frontend` and `xexam-backend` (amd64 + arm64) to `ghcr.io/yumin-space/`, tagged `latest`, `v<VERSION>` and `sha-<commit>`, and create the matching GitHub Release. Pull requests and other branches are only checked, never published.

The first time a new image name is published, add this repository under **Manage Actions access** in the GitHub package settings with the role **Admin**, otherwise the push is rejected (`permission_denied: write_package`).
