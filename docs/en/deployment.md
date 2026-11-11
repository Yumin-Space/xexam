**English** | [日本語](../ja/deployment.md) | [简体中文](../zh-CN/deployment.md)

# Deployment

> New architecture: `nginx` (the only service exposed to the outside) + `backend` (Rust, private network) + SQLite (named volume `xexam_data`). For the structure diagrams see [architecture.md](architecture.md); for the security boundaries see [security.md](security.md).

## 1. Prerequisites

- A machine with Docker installed (including Compose v2); either x86 (amd64) or ARM (arm64) works.
  - The arm64 images were verified in CI under QEMU: **built and started** (`/api/info` reports `aarch64` and the right version, and Nginx `/healthz` is healthy). **They have not been run on real ARM hardware.**
  - The backend is cross-compiled for aarch64 with zig on an x86 build machine (about 5 minutes in CI with a cold cache; Rust does not have to be compiled under QEMU).
    zig's linker does not accept `--fix-cortex-a53-843419`, which rustc adds for aarch64 by default, so the aarch64 binary **does not include** this workaround for the Cortex-A53 erratum.
    Oracle Cloud's Ampere A1 uses Ampere Altra (according to Wikipedia, Arm Neoverse N1 cores), not Cortex-A53, so it is not affected.
    If you want to run on a board with Cortex-A53 cores (some older development boards), please do not use this image, and let me know first.
- A host with `/etc/localtime` (every common Linux distribution has it). Without it, see section 3.1.

## 2. Starting

### Pull the prebuilt images (recommended)

You only need one file from the repository, [`docker-compose.yml`](../../docker/docker-compose.yml) in the `docker/` folder (download it into a folder of its own, for example with `curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml`, and run the commands in that folder). It pulls the two images from Docker Hub (`docker.io/yuminspace/xexam-backend` and `docker.io/yuminspace/xexam-frontend`), where they are public, so no `docker login` is needed. The same tags are also on `ghcr.io/yumin-space/` as a private mirror (that one needs `docker login ghcr.io` with a classic GitHub token that has `read:packages`).

```bash
# no docker login needed: the images are public on Docker Hub
docker compose up -d
```

`docker compose up -d` pulls and starts **two containers**: `xexam-frontend` (Nginx, the only one exposed to the outside) and `xexam-backend` (Rust, private network). Then open <http://localhost:8765> in a browser and sign in with `admin` / `admin`; Xexam asks you to choose your own password right away (section 10). Open the menu (the three-line button at the top right), choose **Question banks** and import your Excel file. The interface starts in English; you can switch it to Chinese or Japanese under **Settings**, or on the sign-in page.

### Build from source (when you change the code)

```bash
git clone git@github.com:Yumin-Space/xexam.git
cd xexam
docker compose -f docker/docker-compose.build.yml up -d --build
```

The first build takes a few minutes (it compiles Rust and the frontend); later builds use the cache. In a clone, the compose files and `.env` are in `docker/`; `docker/docker-compose.build.yml` builds with the repository root as its context. Run the commands below from the repository root with `-f docker/docker-compose.build.yml` added (for example `docker compose -f docker/docker-compose.build.yml logs -f`).

The frontend image needs the three READMEs: the **About Xexam** page shows their changelogs, and they are built into the frontend. `nginx/Dockerfile` therefore copies `README.md`, `README/README.ja.md` and `README/README.zh-CN.md` into its build stage, and `.dockerignore` keeps `README.md` and `README/` in the build context. Only the built static files go into the final image.

The commands below name the Compose **services**, `backend` and `frontend`. The containers are called `xexam-backend` and `xexam-frontend`; plain `docker` commands (`docker logs`, `docker exec`) take those names instead.

| Action | Command |
|---|---|
| Check status | `docker compose ps` (both containers should be `healthy`) |
| View logs | `docker compose logs -f` (only the backend: `docker compose logs -f backend`; see section 9) |
| Stop / start again | `docker compose stop` / `docker compose start` |
| Update to the latest version | `docker compose pull && docker compose up -d` (for a source build: `git pull && docker compose -f docker/docker-compose.build.yml up -d --build`). Read section 6 first when you come from an older version |
| Version and architecture | <http://localhost:8765/api/info>, or in the app: menu → **Xexam v0.7** (the About page) |

> ⚠️ Do not run `docker compose down -v`: `-v` deletes the data volume `xexam_data`, which holds the question banks, the accounts and all study records.

**`docker/docker-compose.yml` and `docker/docker-compose.build.yml` use the same project name (`xexam`), the same volume (`xexam_data`) and the same container names**, so the two cannot run at the same time; run `docker compose stop` before switching. The data is shared between them; see [migration.md](migration.md).

## 3. Configuration (`.env`)

Create a `.env` file next to `docker-compose.yml`; in a clone that is `docker/.env`, which Compose also reads when you run it from the repository root with `-f docker/...` (an older clone with `.env` at the repository root: move it to `docker/`). It is not committed and does not go into the images; the template is [`.env.example`](../../docker/.env.example):

| Variable | Default | Purpose |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | The "address:port" published on the host, for Nginx. It must be written as `IP:port`. `0.0.0.0:8765` = reachable from the LAN. ⚠️ A bare port number = all network interfaces |
| `XEXAM_ALLOWED_HOSTS` | empty | Which domain names / IPs may be used to access it, separated by commas, spaces or semicolons |
| `XEXAM_ALLOWED_ORIGINS` | empty | When a proxy rewrites Host to the origin server's address: enter the public domain (including `https://`) |
| `XEXAM_ALLOW_ANY_HOST` | `0` | `1` = do not check Host (only when authentication is already in front) |
| `XEXAM_PARSE_TIMEOUT_SECS` | `30` | Maximum time to parse one Excel file |
| `XEXAM_SQLITE_JOURNAL` | (unset: WAL) | `delete` keeps SQLite's old rollback journal instead of WAL. Set it only when the data volume is on a network filesystem or on a Windows folder bind-mounted into Docker Desktop; any other value means WAL. See section 2.1 of [database.md](database.md) |
| `TZ` | (unset: the host's time zone) | Time zone of the timestamps in the database; the compose file mounts the host's `/etc/localtime`, so set `TZ` only to use another zone (for example `UTC`) |

After changing `.env`, run `docker compose up -d` (it recreates the containers as needed). `TZ` and `XEXAM_SQLITE_JOURNAL` are passed into the container only when `.env` sets them.

The backend reads two more variables that the compose files do not pass on: `XEXAM_COOKIE_SECURE` (`1` = the session cookie always gets `Secure`, see section 10) and `XEXAM_LOG` (the log filter, default `info`, for example `debug`). Setting them in `.env` has no effect; to use one, add it under `environment:` of the `backend` service in your `docker-compose.yml`, for example `XEXAM_LOG: "debug"`. `xexam-backend --help` lists every variable.

### 3.1 Time zone

The backend writes local times without a zone marker (section 4.6 of [database.md](database.md)). The compose files mount the host's `/etc/localtime` into the backend container, read only, with the long volume syntax:

```yaml
      - type: bind
        source: /etc/localtime
        target: /etc/localtime
        read_only: true
        bind:
          create_host_path: false
```

So the timestamps follow the host's time zone; a `TZ` in `.env` overrides it. `create_host_path: false` makes Compose stop with an error ("bind source path does not exist") on a host without `/etc/localtime`, instead of creating an empty folder there that cannot be mounted over the image's file. **On such a host, remove those six lines from `docker-compose.yml` and set `TZ` in `.env`** (for example `TZ=UTC`), otherwise the backend container does not start. Without the mount and without `TZ` the image uses UTC.

On Docker Desktop (Windows, macOS) the host's `/etc/localtime` is the file of Docker's own virtual machine, not of your computer, and that machine usually runs on UTC. The mount works, but the times are then UTC whatever zone your computer uses: set `TZ` in `.env` to the zone you want.

Changing the time zone later does not rewrite stored times; they keep the zone they were written in.

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
3. Add a policy in Cloudflare Access that **allows only you** (and the people you invite). A domain with no policy is open to the whole world.

Two common Host situations (`.env`):

| What the tunnel does | Setting |
|---|---|
| Host is passed through unchanged as your domain | `XEXAM_ALLOWED_HOSTS=xexam.your-domain.com` |
| `httpHostHeader` is set, so Host is rewritten to `192.168.1.20:8765` | `XEXAM_ALLOWED_HOSTS=192.168.1.20` + `XEXAM_ALLOWED_ORIGINS=https://xexam.your-domain.com` |
| Not sure | Start with the first row. When a request is rejected (403), the page states the Host / Origin it saw; add them to the matching option and run `up -d` again |

Nginx passes the `Host` sent by the browser (or the tunnel) to the backend **unchanged** and clears `X-Forwarded-*`; the backend reads only the real `Host`.

⚠️ Xexam has its own sign-in since v0.6, but keep Cloudflare Access in front of it anyway: let only cloudflared reach this port, and do not set up port forwarding on the router. On Linux, the host firewall cannot block ports published by Docker; use the `DOCKER-USER` chain or a specific LAN IP.

How far this has been verified (stated honestly):

- During the migration, the complete browser flow for both Host situations (Host preserved / Host rewritten) was run against the **old Python version**, using a real Chrome and an HTTPS reverse proxy. That script depended on the old Python code and was removed along with it.
- For the **new stack**: `tests/integration/stack_check.py` uses header-level checks to verify that "after passing through Nginx, the Host / Origin guard still works and `X-Forwarded-*` is not trusted"; `nginx_check.py` verifies that Host is passed through unchanged.
- It has **not** been tested with real Cloudflare. For the first go-live, follow the "Not sure" row in the table above and read the 403 message the page gives.

## 5. Data

- Location: named volume `xexam_data` → `/data/xexam.db` inside the container. Updating, rebuilding or deleting containers does not lose it. The database holds the question banks, the accounts, the study records and the settings.
- While the backend runs, SQLite's `xexam.db-wal` and `xexam.db-shm` sit next to the database (WAL mode, since v0.7). On a normal stop the backend copies everything into `xexam.db` and empties `xexam.db-wal`; the two files may stay next to it, empty and harmless.
- The volume can also hold a folder `/data/backups/` with the copies written before schema migrations (section 6). The program never deletes these files. To see them: `docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups`.
- **Backups**: stop the backend first (`docker compose stop`), pack the volume, start again: the commands are in section 5 of [database.md](database.md). A copy of `xexam.db` alone taken while the backend runs can miss the latest changes, which may still be in `xexam.db-wal`; if you cannot stop it, copy `xexam.db`, `xexam.db-wal` and `xexam.db-shm` together. A volume backup includes `backups/`.
- Running the backend directly without Docker (development): see [development.md](development.md).

## 6. Upgrading

Every upgrade is `docker compose pull && docker compose up -d` (with the new `docker-compose.yml` where noted). On its first start the new backend migrates the database to its schema after writing a copy to `/data/backups/`; if that copy cannot be written (disk full, no permission), it exits with an error and the database is not changed. Docker keeps restarting it, and each start tries again; Nginx does not start, because it waits for a healthy backend. `docker compose logs backend` says why. Back up the volume yourself first as well (section 5): a copy outside the volume is safer.

The step-by-step checklists, the checks afterwards and the rollbacks are in [migration.md](migration.md):

- **From v0.6** (section 1 of migration.md): schema v3 → v4, backup `xexam-schema3-<date-time>.db`, WAL mode. Get the new `docker-compose.yml` as well: it no longer sets a fixed default time zone (it follows the host, section 3.1), limits the container logs (section 8) and passes `XEXAM_SQLITE_JOURNAL`. If your earlier timestamps were written in a zone other than the host's, set that zone as `TZ` in `.env` so new records continue in it. No variable has to be added.
- **From v0.5 or v0.5.1** (section 2 of migration.md): schema v2 → v4 in one step, backup `xexam-schema2-<date-time>.db`. Sign-in is new: sign in with `admin` / `admin`, which now owns all earlier data.
- **From v0.4** (section 3 of migration.md): schema v1 → v4 in one step, backup `xexam-schema1-<date-time>.db`. Since v0.5.1 the images come from Docker Hub (`docker.io/yuminspace/…`, public, no login) instead of `ghcr.io`: replace your `docker-compose.yml` with the new one (or change its two `image:` lines to `docker.io/yuminspace/xexam-backend:latest` and `docker.io/yuminspace/xexam-frontend:latest`). For a source build, the compose files are now in `docker/`: move a `.env` at the repository root to `docker/.env`.

After the upgrade, `/api/info` shows `"version": "0.7"`. Later starts do not migrate again and write nothing extra. **Rolling back** means restoring the copy from `backups/` and running the older images; never run an older version on the migrated database for longer than an emergency. The exact commands are in [migration.md](migration.md); the background is in section 6 of [database.md](database.md).

## 7. Health checks and stopping

- `xexam-frontend`: `GET /healthz` (it does not go through the Host allow-list). `xexam-backend`: `xexam-backend healthcheck` (inside the container it requests itself with `Host: 127.0.0.1`). Nginx starts only after the backend is healthy (`depends_on`).
- When the backend is down or restarting, Nginx does not exit; `/api/` just returns a JSON 502 for a while (`后端暂时连不上`, "the backend is temporarily unreachable") and recovers automatically once the backend is back.
- `docker compose stop`: the backend exits gracefully within 8 seconds (Docker waits up to 15); an import transaction either finishes committing or is rolled back. Then it runs the WAL checkpoint, so everything is in `xexam.db` and `xexam.db-wal` is empty (the log says `WAL checkpoint at shutdown: everything is in xexam.db, the -wal file is empty`). After a crash or a `docker kill` the `-wal` file stays; SQLite applies it at the next start, nothing committed is lost.

## 8. Resources and images

| | |
|---|---|
| `xexam-frontend` | nginx-unprivileged + static files (the changelog for the About page is part of them); listens on 8080; read-only root filesystem |
| `xexam-backend` | a single statically linked executable + tzdata (UTC `/etc/localtime` as a fallback) + `/VERSION`; uid 10001; read-only root filesystem, only `/data` and `/tmp` are writable |
| Networks | `internal` (backend + Nginx, no access to the outside network) and `public` (Nginx only) |
| Container logs | logging driver `json-file` with `max-size: 10m` and `max-file: 3` for each container, so Docker keeps at most about 30 MB of log per container (without a limit Docker keeps container logs forever) |

## 9. Logs: the System log page and `docker compose logs`

There are two places to read what the backend logged. Both show the same lines, filtered by the same `XEXAM_LOG` (default `info`).

| | **System log** (menu, administrators only) | `docker compose logs backend` |
|---|---|---|
| Where | In the app, page `#/logs`, updated every 1.5 seconds | On the server |
| How much | The last **2000** lines since the backend last started | Everything Docker still keeps: up to 3 files of 10 MB, across restarts |
| After a restart | Empty again (kept in memory only, never in the database) | Kept |
| Also | Level filter, search, **Pause** / **Resume**, **Jump to latest** | The Nginx access and error log: `docker compose logs frontend` |

Without Compose: `docker logs xexam-backend`. The page's note names `docker compose logs xexam-backend`; with the compose files of this repository the service is called `backend`, so use `docker compose logs backend` (or `docker logs xexam-backend`).

What the log contains: the start-up lines (version, data directory, journal mode, allowed hosts, the reminder about the default password), sign-ins and account changes (by user id: `user 3 signed in`), refused uploads and writes, warnings and errors. It never contains passwords, session tokens, cookie values, confirmation codes or question text. Requests rejected by the Host / Origin check, failed sign-ins and locks of usernames without an account are logged once and then summed up at most once a minute, so nobody can push the other lines out of the 2000 without signing in. Details: section 8 of [security.md](security.md).

## 10. First start, accounts and recovery

- After installing or upgrading, open Xexam and sign in with `admin` / `admin`, then choose your own password right away; until then anyone who can open the page can use the default password. While an administrator still has it, the sign-in page names that account, and each start of the backend prints a reminder (`docker compose logs backend`).
- Add the other people under **User management** (menu, administrators only), and choose which question banks each of them sees. A user with the **Bank manager** permission can import banks and update or delete their own.
- **Locked out of the password**: `docker compose exec backend xexam-backend reset-password admin` (without Compose: `docker exec xexam-backend xexam-backend reset-password admin`) prints a new temporary password, enables the account, signs it out everywhere and asks for a new password at the next sign-in. It works while the server runs.
- **Locked out by wrong passwords**: a sign-in lock lives in the backend's memory, so `reset-password` (another process) cannot lift it. Another administrator can press **Unlock** in User management, or restart the backend: `docker compose restart backend`.
- Behind Cloudflare Access you sign in twice (Access, then Xexam). Plain HTTP on your own network works: the session cookie gets `Secure` only when the page is opened over `https://`. `XEXAM_COOKIE_SECURE=1` (section 3) forces `Secure` always; set it only if the browser always reaches Xexam over HTTPS.

## 11. Releasing

On a push to `main`, [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) first runs the version, changelog and docs check, then four check groups (frontend, backend, Nginx and full stack, arm64). Only if all of them pass does it publish the two images `xexam-frontend` and `xexam-backend` (amd64 + arm64) to Docker Hub (`docker.io/yuminspace/`, public) and, as a private mirror, to `ghcr.io/yumin-space/`, both tagged `latest`, `v<CODE_VERSION>` and `sha-<commit>`, and create the matching GitHub Release (its notes list the Docker Hub pull commands first). Last, it records the release: `RELEASE_VERSION` in `VERSION` and the release badge of the READMEs, committed by the bot on `main` (and on `dev` when it can be fast-forwarded). Pushes to `dev` and pull requests into `main` run the same checks and never publish. Details: section 7 of [development.md](development.md).

Docker Hub needs two repository secrets, `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` (an access token with **Read & Write**), and optionally the repository variable `DOCKERHUB_NAMESPACE`; without the secrets, or if `docker/docker-compose.yml` does not name the Docker Hub images of that namespace, the publish job stops before it pushes anything. On ghcr.io, the first time a new image name is published, add this repository under **Manage Actions access** in the GitHub package settings with the role **Admin**, otherwise the push is rejected (`permission_denied: write_package`).
