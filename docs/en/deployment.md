**English** | [日本語](../ja/deployment.md) | [简体中文](../zh-CN/deployment.md)

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

You only need one file from the repository, [`docker-compose.yml`](../../docker-compose.yml):

```bash
docker login ghcr.io -u Yumin-Space      # first time only; the images are private, and the password is a token with read:packages permission
docker compose up -d
```

Open <http://localhost:8765> in a browser, open the menu (the three-line button at the top left), choose **Question banks** and import your Excel file. The interface starts in English; you can switch it to Chinese or Japanese under **Settings**. This pulls and starts **two containers**: `xexam-frontend` (Nginx, the only one exposed to the outside) and `xexam-backend` (Rust, private network).

### Build from source (when you change the code)

```bash
git clone git@github.com:Yumin-Space/xexam.git
cd xexam
docker compose -f docker-compose.build.yml up -d --build
```

The first build takes a few minutes (it compiles Rust and the frontend); later builds use the cache.

The frontend image needs the three READMEs: the **About Xexam** page shows their changelogs, and they are built into the frontend. `nginx/Dockerfile` therefore copies `README.md`, `README.ja.md` and `README.zh-CN.md` into its build stage, and `.dockerignore` no longer excludes `README.md`. Only the built static files go into the final image.

| Action | Command |
|---|---|
| Check status | `docker compose ps` (both containers should be `healthy`) |
| View logs | `docker compose logs -f` |
| Stop / start again | `docker compose stop` / `docker compose start` |
| Update to the latest version | `docker compose pull && docker compose up -d` (for a source build: `git pull && docker compose -f docker-compose.build.yml up -d --build`). Coming from v0.4: read section 6 first |
| Version and architecture | <http://localhost:8765/api/info>, or in the app: menu → **Xexam v0.5** (the About page) |

> ⚠️ Do not run `docker compose down -v`: `-v` deletes the data volume `xexam_data`, which holds the question bank and all study records.

**`docker-compose.yml` and `docker-compose.build.yml` use the same project name (`xexam`), the same volume (`xexam_data`) and the same container names**, so the two cannot run at the same time; run `docker compose stop` before switching. The data is shared between them; see [migration.md](migration.md).

## 3. Configuration (`.env`)

Create a `.env` file next to `docker-compose.yml` (it is not committed and does not go into the images; the template is [`.env.example`](../../.env.example)):

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

- Location: named volume `xexam_data` → `/data/xexam.db` inside the container. Updating, rebuilding or deleting containers does not lose it. The database holds the question banks, the study records and, since v0.5, the settings.
- Since v0.5 the volume can also hold a folder `/data/backups/` with the copy written before a schema migration (section 6). The program never deletes these files. To see them: `docker run --rm -v xexam_data:/data alpine ls -l /data/backups`.
- Backup and restore: see section 5 of [database.md](database.md). A volume backup includes `backups/`.
- Running the backend directly without Docker (development): see [development.md](development.md).

## 6. Upgrading from v0.4 to v0.5

For deployment, v0.5 changes nothing: the same two images, the same `docker-compose.yml`, the same port, the same volume, and no new environment variables. What changes is the interface (the menu, question banks, study records, settings, the About page) and the database schema. A source build keeps the same commands; the frontend image now also reads the three READMEs (section 2).

1. Back up the volume first, as in section 5 of [database.md](database.md). The program writes its own copy too, but a copy outside the volume is safer.
2. Update as usual: `docker compose pull && docker compose up -d`.
3. On its first start, the backend upgrades the database to schema v2:
   - It first writes a copy of the old database to `/data/backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db`. If that copy cannot be written (disk full, no permission), the backend exits with an error and the database is not changed. Docker keeps restarting it, and each start tries again; Nginx does not start, because it waits for a healthy backend. `docker compose logs backend` says why.
   - Then it migrates in one transaction. Your existing questions become one question bank, with the same question ids, so the wrong-answer notebook, the statistics and the exam history stay as they were.
   - The log line `Upgraded the database to schema 2. A copy of the previous version was saved first: …` names the copy.
4. Check: `/api/info` shows `"version": "0.5"` and the same `bankCount` as before; **Question banks** shows one bank with all your questions; **Study records** lists your earlier exams.

Later starts do not migrate again and write nothing extra.

**Rolling back** to v0.4 means restoring that copy and running the v0.4 images (tag `v0.4`). Do not run v0.4 on the migrated database once you have more than one bank: v0.4 sees all banks as one, and its next import would deactivate the questions of the others. The exact commands are in section 1.4 of [migration.md](migration.md); the background is in section 6 of [database.md](database.md).

## 7. Health checks and stopping

- `xexam-frontend`: `GET /healthz` (it does not go through the Host allow-list). `xexam-backend`: `xexam-backend healthcheck` (inside the container it requests itself with `Host: 127.0.0.1`). Nginx starts only after the backend is healthy (`depends_on`).
- When the backend is down or restarting, Nginx does not exit; `/api/` just returns a JSON 502 for a while (`后端暂时连不上`, "the backend is temporarily unreachable") and recovers automatically once the backend is back.
- `docker compose stop`: the backend exits gracefully within 8 seconds; an import transaction either finishes committing or is rolled back.

## 8. Resources and images

| | |
|---|---|
| `xexam-frontend` | nginx-unprivileged + static files (the changelog for the About page is part of them); listens on 8080; read-only root filesystem |
| `xexam-backend` | a single statically linked executable + tzdata; uid 10001; read-only root filesystem, only `/data` and `/tmp` are writable |
| Networks | `internal` (backend + Nginx, no access to the outside network) and `public` (Nginx only) |

## 9. Releasing

On a push to `main`, [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) first runs the version, changelog and docs check, then four check groups (frontend, backend, Nginx and full stack, arm64). Only if all of them pass does it publish the two images `xexam-frontend` and `xexam-backend` (amd64 + arm64) to `ghcr.io/yumin-space/`, tagged `latest`, `v<VERSION>` and `sha-<commit>`, and create the matching GitHub Release. Pushes to `dev` and pull requests into `main` run the same checks and never publish. Details: section 7 of [development.md](development.md).

The first time a new image name is published, add this repository under **Manage Actions access** in the GitHub package settings with the role **Admin**, otherwise the push is rejected (`permission_denied: write_package`).
