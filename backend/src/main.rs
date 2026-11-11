//! Binary entry point: parses arguments and starts the server; the `healthcheck` subcommand is the container self-check.
//!
//! Subcommands: `serve` (default) | `healthcheck` | `import <xlsx>` | `migrate-progress [progress.json]` |
//! `reset-password <username>` (v0.6: locked-out recovery).
//! The legacy form `--import <xlsx>` is also accepted.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::config::Config;
use xexam_backend::database::{JournalMode, LegacyMigration, Store, SystemClock};
use xexam_backend::error::DbError;
use xexam_backend::services::{Engine, RealEngine};

#[derive(Debug, PartialEq, Eq)]
enum Cmd {
    Serve,
    Healthcheck,
    Import(PathBuf),
    MigrateProgress(Option<PathBuf>),
    ResetPassword(String),
    Help,
    ResetPasswordHelp,
}

#[derive(Debug, PartialEq, Eq)]
struct Args {
    cmd: Cmd,
    data: Option<PathBuf>,
    host: Option<String>,
    port: Option<u16>,
    allow_shutdown: bool,
}

const USAGE: &str = "Usage:
  xexam-backend [serve] [--data DIR] [--host ADDR] [--port N] [--allow-shutdown] [--no-browser]
  xexam-backend healthcheck [--port N]
  xexam-backend import <question-bank.xlsx> [--data DIR]       (also accepts --import <question-bank.xlsx>)
  xexam-backend migrate-progress [progress.json] [--data DIR]
  xexam-backend reset-password <username> [--data DIR]      (see: xexam-backend reset-password --help)
Environment variables: XEXAM_DATA XEXAM_HOST XEXAM_PORT XEXAM_ALLOWED_HOSTS XEXAM_ALLOWED_ORIGINS XEXAM_ALLOW_ANY_HOST XEXAM_COOKIE_SECURE XEXAM_PARSE_TIMEOUT_SECS XEXAM_LOG XEXAM_SQLITE_JOURNAL";

const RESET_PASSWORD_USAGE: &str = "Usage:
  xexam-backend reset-password <username> [--data DIR]

Sets a new random password (16 characters, printed once) for the account <username>, enables the account, deletes its
sessions (it is signed out everywhere) and makes it choose a new password at the next sign-in. Use it when nobody can sign
in any more, e.g. the administrator forgot the password. The data directory is found as for serve: --data, else XEXAM_DATA,
else ./data. It can run while the server is running.

In Docker (the Compose service is called backend; its container is xexam-backend):
  docker compose exec backend xexam-backend reset-password admin

Exit code: 0 on success; 1 when there is no such user or the database cannot be opened or written.";

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        cmd: Cmd::Serve,
        data: None,
        host: None,
        port: None,
        allow_shutdown: false,
    };
    let mut positional: Vec<String> = Vec::new();
    let mut import_flag: Option<PathBuf> = None;
    let mut it = argv.iter().peekable();
    // The first word that does not start with `-` is the subcommand
    let mut sub: Option<String> = None;
    if let Some(first) = it.peek() {
        if !first.starts_with('-') {
            sub = it.next().cloned();
        }
    }
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |what: &str| -> Result<String, String> {
            match inline.clone() {
                Some(v) => Ok(v),
                None => it
                    .next()
                    .cloned()
                    .ok_or_else(|| format!("{what} requires a value")),
            }
        };
        match name.as_str() {
            "--data" => a.data = Some(PathBuf::from(value("--data")?)),
            "--host" => a.host = Some(value("--host")?),
            "--port" => {
                a.port = Some(
                    value("--port")?
                        .trim()
                        .parse()
                        .map_err(|_| "--port must be an integer from 0 to 65535".to_string())?,
                )
            }
            "--import" => import_flag = Some(PathBuf::from(value("--import")?)),
            "--allow-shutdown" => a.allow_shutdown = true,
            "--no-browser" => {}
            "-h" | "--help" => a.cmd = Cmd::Help,
            s if s.starts_with('-') => return Err(format!("unknown option: {s}")),
            _ => positional.push(arg.clone()),
        }
    }
    if a.cmd == Cmd::Help {
        if sub.as_deref() == Some("reset-password") {
            a.cmd = Cmd::ResetPasswordHelp;
        }
        return Ok(a);
    }
    match (sub.as_deref(), import_flag) {
        (_, Some(p)) => a.cmd = Cmd::Import(p),
        (None, None) | (Some("serve"), None) => a.cmd = Cmd::Serve,
        (Some("healthcheck"), None) => a.cmd = Cmd::Healthcheck,
        (Some("import"), None) => {
            let p = positional
                .first()
                .ok_or_else(|| "import requires the path of an .xlsx file".to_string())?;
            a.cmd = Cmd::Import(PathBuf::from(p));
        }
        (Some("migrate-progress"), None) => {
            a.cmd = Cmd::MigrateProgress(positional.first().map(PathBuf::from))
        }
        (Some("reset-password"), None) => {
            let name = positional.first().ok_or_else(|| {
                "reset-password requires a username (see: xexam-backend reset-password --help)"
                    .to_string()
            })?;
            if positional.len() > 1 {
                return Err("reset-password takes exactly one username".to_string());
            }
            a.cmd = Cmd::ResetPassword(name.clone());
        }
        (Some("help"), None) => a.cmd = Cmd::Help,
        (Some(other), None) => return Err(format!("unknown subcommand: {other}")),
    }
    Ok(a)
}

/// Prints a status line to stdout and keeps it in the System log buffer too (v0.7), so the administrators' log page shows
/// the startup and shutdown lines. Never pass a secret: use [`say_private`] for that.
fn say(msg: &str) {
    say_private(msg);
    xexam_backend::logs::record(tracing::Level::INFO, "xexam_backend", msg);
}

/// Prints a status line to stdout only (for example the new password printed by `reset-password`).
fn say_private(msg: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
}

fn init_tracing() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let filter = tracing_subscriber::EnvFilter::try_from_env("XEXAM_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    // One filter for both outputs: stderr (what `docker compose logs` shows) and the in-memory buffer of the System log page.
    // No ANSI escapes: user-controlled strings written to the log must not inject terminal control sequences
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::io::stderr),
        )
        .with(xexam_backend::logs::BufferLayer::global())
        .try_init();
}

/// Container self-check: `GET http://127.0.0.1:<port>/api/info` with Host 127.0.0.1 (on the default allowlist); only a 200 counts as healthy.
fn healthcheck(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_secs(5)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
    if s.write_all(b"GET /api/info HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 256];
    let mut got = 0usize;
    while got < buf.len() {
        match s.read(buf.get_mut(got..).unwrap_or(&mut [])) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                got += n;
                if buf.get(..got).is_some_and(|b| b.contains(&b'\n')) {
                    break;
                }
            }
        }
    }
    let head = String::from_utf8_lossy(buf.get(..got).unwrap_or(&[]));
    let line = head.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    matches!((parts.next(), parts.next()), (Some(v), Some("200")) if v.starts_with("HTTP/1."))
}

/// Opens the database. Migration of the legacy `progress.json` matches the Python version: it is checked on every start, and the `legacy_migrated` flag in the database
/// plus the "never overwrite when study records already exist" guard make sure it migrates only once and never overwrites (see `Store::migrate_legacy_progress`).
/// This is what makes the documented flow actually work: start once to create the volume -> put progress.json into the volume -> restart.
///
/// `journal` is the journal mode asked for (`XEXAM_SQLITE_JOURNAL`); a mode SQLite refuses is logged as a warning by the store.
fn open_store(data_dir: &Path, journal: JournalMode) -> Result<Arc<Store>, String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| format!("cannot create data directory {}: {e}", data_dir.display()))?;
    let db_path = data_dir.join("xexam.db");
    let mut others: Vec<String> = std::fs::read_dir(data_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.to_lowercase().ends_with(".db"))
                .collect()
        })
        .unwrap_or_default();
    others.sort();
    if !db_path.exists() && !others.is_empty() {
        say(&format!(
            "Note: there is no xexam.db in the data directory, but there are other database files: {}. If that is a question bank from an earlier version, stop the program, rename it to xexam.db and start again; a new empty xexam.db will be created now.",
            others.join(", ")
        ));
    }
    let opened = Store::open_with(&db_path, Arc::new(SystemClock), journal).map_err(|e| match e {
        DbError::Backup(_) => format!(
            "cannot open database: {e}. The database was NOT upgraded and is unchanged. Make sure {} can be created and written (disk space, permissions), then start again.",
            data_dir.join(xexam_backend::database::BACKUP_DIR).display()
        ),
        other => format!("cannot open database: {other}"),
    })?;
    let store = Arc::new(opened);
    match (store.migration_backup(), store.converted_prerelease()) {
        (Some(backup), true) => say(&format!(
            "Converted the question bank access of a database written by a v0.7 pre-release to the released format. A copy of the previous version was saved first: {}.",
            backup.display()
        )),
        (Some(backup), false) => say(&format!(
            "Upgraded the database to schema {}. A copy of the previous version was saved first: {} (restore it if you ever go back to an older version).",
            xexam_backend::database::schema::SCHEMA_VERSION,
            backup.display()
        )),
        (None, true) => say(
            "Converted the question bank access of a database written by a v0.7 pre-release to the released format.",
        ),
        (None, false) => {}
    }
    if store.created_default_admin() {
        say("Created the default administrator 'admin' (password 'admin'); sign in and choose a new password.");
    }
    retire_old_layouts(&store)?;
    let legacy = data_dir.join("progress.json");
    if legacy.is_file() {
        match store.migrate_legacy_progress_outcome(&legacy) {
            Ok(done) if done.migrated() => say(&format!(
                "Migrated the old study records into the database: {} (from {})",
                done.english(),
                legacy.display()
            )),
            // "Already migrated earlier" is the normal case; no need to report it on every start
            Ok(LegacyMigration::AlreadyMigrated) => {}
            Ok(other) => say(&format!(
                "Did not migrate the old study records: {}. Fix progress.json and restart to try again, or run xexam-backend migrate-progress manually",
                other.english()
            )),
            Err(e) => say(&format!(
                "Failed to migrate the old study records (the database was not changed): {e}. Restart later to try again, or run xexam-backend migrate-progress manually"
            )),
        }
    }
    Ok(store)
}

/// v0.7: retires the questions of the old 3- / 4-column layouts (see `Store::retire_old_layout_questions`), on every start of
/// the server and of the commands that open the database, after the migrations. One log line per bank when something was
/// retired; nothing is written otherwise. A failure stops the start (nothing was changed: it is one transaction).
fn retire_old_layouts(store: &Store) -> Result<(), String> {
    let retired = store.retire_old_layout_questions().map_err(|e| {
        format!("cannot retire the questions of the old 3- / 4-column layouts: {e}")
    })?;
    for b in retired {
        let (n, they) = if b.retired == 1 {
            ("1 question".to_string(), "it uses")
        } else {
            (format!("{} questions", b.retired), "they use")
        };
        tracing::warn!(
            "retired {n} of bank {:?}: {they} the old 3- / 4-column layout; import the six-column version of the workbook",
            b.name
        );
    }
    Ok(())
}

fn run_import(path: &Path, data_dir: &Path, journal: JournalMode) -> i32 {
    let store = match open_store(data_dir, journal) {
        Ok(s) => s,
        Err(e) => {
            say(&e);
            return 1;
        }
    };
    let engine = RealEngine::new(store.clone());
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            say(&format!(
                "Import failed: cannot read {}: {e}",
                path.display()
            ));
            return 1;
        }
    };
    let wb = match engine.check_and_parse(&data) {
        Ok(w) => w,
        Err(e) => {
            say(&format!("Import failed: {}.", e.log_reason()));
            return 1;
        }
    };
    // v0.7: only the six-column layout can be imported (the parser still recognises the old layouts, to say so)
    if wb.has_old_layout() {
        say(&format!(
            "Import failed: {}. Rewrite it in the six-column layout (A-C: the question in English, Chinese, Japanese; D-F: the answer in English, Chinese, Japanese) and import it again. Nothing was imported.",
            xexam_backend::services::LOG_OLD_LAYOUT
        ));
        return 1;
    }
    if wb.questions.is_empty() {
        say("No usable questions were found in this file; nothing was imported.");
        return 1;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let plan = match store.apply_import(&name, &wb.sheets, &wb.questions) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("apply_import failed: {e}");
            say("Failed to write to the database; the question bank was not changed.");
            return 1;
        }
    };
    let n = |k: &str| plan.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    let count = store.count_active().unwrap_or(0);
    say(&format!(
        "Imported {name}: {} added, {} updated, {} restored, {} deactivated, {} unreadable; {count} questions in total now.",
        n("added"),
        n("updated"),
        n("restored"),
        n("removed"),
        n("skipped")
    ));
    for s in plan
        .get("sheets")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let sheet = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
        for k in s
            .get("skipped")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let row = k.get("row").and_then(|v| v.as_i64()).unwrap_or(0);
            let reason = k.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            say(&format!("  Not imported: {sheet} row {row}: {reason}"));
        }
    }
    0
}

/// `reset-password <username>`: a new random password for that account (printed once), which must be changed at the next
/// sign-in; the account is enabled and signed out everywhere.
fn run_reset_password(username: &str, data_dir: &Path, journal: JournalMode) -> i32 {
    // Recovery must change the database that serve uses: never create a new one (a wrong --data or working directory would
    // otherwise "succeed" on an empty database and leave the real one untouched)
    let db_path = data_dir.join("xexam.db");
    if !db_path.is_file() {
        let shown = std::path::absolute(&db_path).unwrap_or(db_path);
        say(&format!(
            "There is no database at {}. Pass --data <dir> or set XEXAM_DATA to the data directory that serve uses. Nothing was changed.",
            shown.display()
        ));
        return 1;
    }
    // With the server running in WAL mode this is a second connection to the same database (same container, same
    // filesystem): SQLite coordinates the two through the -wal and -shm files, and busy_timeout waits for a running write
    let store = match open_store(data_dir, journal) {
        Ok(s) => s,
        Err(e) => {
            say(&e);
            return 1;
        }
    };
    let password = xexam_backend::auth::random_password();
    let hash = match xexam_backend::auth::hash_password(&password) {
        Ok(h) => h,
        Err(e) => {
            say(&format!("Could not set a new password: {e}"));
            return 1;
        }
    };
    match store.cli_reset_password(username.trim(), &hash) {
        Ok(Some(name)) => {
            // The password goes to stdout only, never into a log
            say_private(&format!("New password for '{name}': {password}"));
            say("It is shown only this once. The account is enabled and signed out everywhere; it must choose a new password at the next sign-in.");
            0
        }
        Ok(None) => {
            say(&format!(
                "There is no user named '{}' in {}. Nothing was changed.",
                xexam_backend::security::short(username),
                data_dir.join("xexam.db").display()
            ));
            1
        }
        Err(e) => {
            tracing::error!("reset-password failed: {e}");
            say("Could not write to the database; nothing was changed.");
            1
        }
    }
}

fn run_migrate(path: Option<PathBuf>, data_dir: &Path, journal: JournalMode) -> i32 {
    let store = match open_store(data_dir, journal) {
        Ok(s) => s,
        Err(e) => {
            say(&e);
            return 1;
        }
    };
    let path = path.unwrap_or_else(|| data_dir.join("progress.json"));
    match store.migrate_legacy_progress_outcome(&path) {
        Ok(done) if done.migrated() => {
            say(&format!(
                "Migrated the old study records into the database: {} (from {})",
                done.english(),
                path.display()
            ));
            0
        }
        Ok(other) => {
            say(&format!("Not migrated: {}", other.english()));
            0
        }
        Err(e) => {
            tracing::error!("migrate_legacy_progress failed: {e}");
            say("Migration failed; the database was not changed.");
            1
        }
    }
}

/// Waits for SIGTERM / SIGINT (unix); on other platforms only Ctrl-C.
async fn os_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) {
            (Ok(mut term), Ok(mut int)) => {
                tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
            }
            _ => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn serve(cfg: Config) -> i32 {
    let store = match open_store(&cfg.data_dir, cfg.sqlite_journal) {
        Ok(s) => s,
        Err(e) => {
            say(&e);
            return 1;
        }
    };
    // Once per start: the journal mode in effect (a refused switch was already logged as a warning)
    if cfg.sqlite_journal == JournalMode::Delete {
        tracing::info!(
            "SQLite journal mode: {} (XEXAM_SQLITE_JOURNAL=delete)",
            store.journal_mode()
        );
    } else {
        tracing::info!("SQLite journal mode: {}", store.journal_mode());
    }
    let listener = match tokio::net::TcpListener::bind((cfg.host.as_str(), cfg.port)).await {
        Ok(l) => l,
        Err(e) => {
            say(&format!(
                "cannot listen on {}:{}: {e}. Close the program using this port, or pick another port with --port.",
                cfg.host, cfg.port
            ));
            return 1;
        }
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    let shutdown_tx = if cfg.allow_shutdown { Some(tx) } else { None };
    let engine: Arc<dyn Engine> = Arc::new(RealEngine::new(store.clone()));
    let any_host = cfg.allow_any_host;
    let mut extra: Vec<String> = cfg
        .allowed_hosts
        .iter()
        .filter(|h| !matches!(h.as_str(), "localhost" | "127.0.0.1" | "::1"))
        .cloned()
        .collect();
    extra.sort();
    let mut origins: Vec<String> = cfg
        .allowed_origins
        .iter()
        .filter(|h| !cfg.allowed_hosts.contains(*h))
        .cloned()
        .collect();
    origins.sort();
    say(&format!(
        "Xexam v{} started: http://localhost:{}/",
        cfg.version, cfg.port
    ));
    say(&format!(
        "Data directory: {}",
        std::path::absolute(&cfg.data_dir)
            .unwrap_or_else(|_| cfg.data_dir.clone())
            .display()
    ));
    if any_host {
        say("Note: XEXAM_ALLOW_ANY_HOST is on, so the Host header is no longer checked (no protection against DNS rebinding). Sign-in still applies to every request.");
        tracing::warn!("XEXAM_ALLOW_ANY_HOST is on: the Host header is not checked");
    } else if !extra.is_empty() {
        say(&format!("Allowed hosts/addresses: {}", extra.join(", ")));
    }
    if !origins.is_empty() {
        say(&format!(
            "Additional allowed Origins: {}",
            origins.join(", ")
        ));
    }
    if store.count_active().unwrap_or(0) == 0 {
        if store.bank_count().unwrap_or(0) == 0 {
            say("The question bank is still empty: open the page and upload an Excel file under 'Question banks'.");
        } else {
            say("No question bank has a question in use: open the page and, under 'Question banks', update the banks with six-column Excel files (or import new ones).");
        }
    }
    // Computed once now, so the first failed sign-in of an unknown user does not take longer than later ones
    let _ = xexam_backend::auth::dummy_hash();
    let now = xexam_backend::auth::unix_now();
    match store.purge_expired_sessions(now) {
        Ok(0) => {}
        Ok(n) => tracing::info!("purged {n} expired sessions"),
        Err(e) => tracing::warn!("could not purge expired sessions: {e}"),
    }
    // An administrator who still has the default password: remind the owner at every start (the line printed when the account
    // was created appears only once), and let the sign-in page say how to sign in for the first time
    let default_admin = match xexam_backend::api::auth::find_default_admin_hashes(&store) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("could not check for the default administrator password: {e}");
            Vec::new()
        }
    };
    // The account is named as it is called now (it may have been renamed in the database since it was created)
    if let (false, Ok(Some(name))) = (
        store.created_default_admin(),
        store.default_admin_pending(&default_admin),
    ) {
        say(&format!("Reminder: the administrator account '{name}' still has the default password 'admin' (sign in as {name} / admin). Sign in now and choose a new password."));
        tracing::warn!("an administrator still has the default password");
    }
    let state = AppState::new(cfg, store.clone(), engine, shutdown_tx);
    let _ = state.0.default_admin_hashes.set(default_admin);
    state
        .0
        .last_session_purge
        .store(now, std::sync::atomic::Ordering::Relaxed);
    let app = build_router(state.clone());
    // The tail of a burst of refused sign-ins is logged by this timer (the throttle never logs per attempt)
    let refusal_reports = xexam_backend::api::auth::spawn_refusal_reports(state.clone());

    let notify = Arc::new(tokio::sync::Notify::new());
    let n2 = notify.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { n2.notified().await })
            .await
    });
    let mut code = 0;
    tokio::select! {
        _ = os_signal() => say("Received a stop signal, exiting..."),
        _ = rx.recv() => say("Received a shutdown request from the page, exiting..."),
        r = &mut server => {
            tracing::error!("server stopped unexpectedly: {r:?}");
            code = 1;
        }
    }
    notify.notify_one();
    if !server.is_finished() {
        match tokio::time::timeout(Duration::from_secs(8), &mut server).await {
            Ok(_) => {}
            Err(_) => {
                tracing::warn!("graceful shutdown timed out after 8s; exiting anyway");
                server.abort();
            }
        }
    }
    refusal_reports.abort();
    drop(state);
    // WAL: copy everything into xexam.db and empty xexam.db-wal, so the volume holds just the database file at rest
    // (closing the last connection below also removes the -wal and -shm files when no other process has the database open)
    match store.checkpoint() {
        Ok(None) => {}
        Ok(Some(c)) if c.busy => tracing::warn!(
            "WAL checkpoint at shutdown could not finish (another process is using the database); SQLite completes it later"
        ),
        Ok(Some(_)) => {
            tracing::info!("WAL checkpoint at shutdown: everything is in xexam.db, the -wal file is empty")
        }
        Err(e) => tracing::warn!("WAL checkpoint at shutdown failed: {e}"),
    }
    drop(store);
    say("Exited.");
    code
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            std::process::exit(2);
        }
    };
    init_tracing();
    let mut cfg = Config::from_env();
    if let Some(d) = &args.data {
        cfg.data_dir = d.clone();
    }
    if let Some(h) = &args.host {
        cfg.host = h.clone();
    }
    if let Some(p) = args.port {
        cfg.port = p;
    }
    cfg.allow_shutdown = args.allow_shutdown;
    let code = match args.cmd {
        Cmd::Help => {
            println!("{USAGE}");
            0
        }
        Cmd::ResetPasswordHelp => {
            println!("{RESET_PASSWORD_USAGE}");
            0
        }
        Cmd::ResetPassword(name) => run_reset_password(&name, &cfg.data_dir, cfg.sqlite_journal),
        Cmd::Healthcheck => i32::from(!healthcheck(cfg.port)),
        Cmd::Import(path) => run_import(&path, &cfg.data_dir, cfg.sqlite_journal),
        Cmd::MigrateProgress(path) => run_migrate(path, &cfg.data_dir, cfg.sqlite_journal),
        Cmd::Serve => match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt.block_on(serve(cfg)),
            Err(e) => {
                eprintln!("cannot start the async runtime: {e}");
                1
            }
        },
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Result<Args, String> {
        parse_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn default_is_serve() {
        let a = p(&[]).unwrap();
        assert_eq!(a.cmd, Cmd::Serve);
        assert!(!a.allow_shutdown);
        assert_eq!(
            p(&[
                "serve",
                "--port",
                "9000",
                "--data=/d",
                "--host",
                "0.0.0.0",
                "--allow-shutdown",
                "--no-browser"
            ])
            .unwrap()
            .port,
            Some(9000)
        );
        assert!(p(&["--allow-shutdown"]).unwrap().allow_shutdown);
    }

    #[test]
    fn subcommands() {
        assert_eq!(p(&["healthcheck"]).unwrap().cmd, Cmd::Healthcheck);
        assert_eq!(
            p(&["import", "a.xlsx", "--data", "d"]).unwrap().cmd,
            Cmd::Import(PathBuf::from("a.xlsx"))
        );
        assert_eq!(
            p(&["--import", "a.xlsx"]).unwrap().cmd,
            Cmd::Import(PathBuf::from("a.xlsx"))
        );
        assert_eq!(
            p(&["migrate-progress"]).unwrap().cmd,
            Cmd::MigrateProgress(None)
        );
        assert_eq!(
            p(&["migrate-progress", "x.json"]).unwrap().cmd,
            Cmd::MigrateProgress(Some(PathBuf::from("x.json")))
        );
        assert_eq!(p(&["--help"]).unwrap().cmd, Cmd::Help);
        assert_eq!(
            p(&["reset-password", "admin", "--data", "d"]).unwrap().cmd,
            Cmd::ResetPassword("admin".to_string())
        );
        assert_eq!(
            p(&["reset-password", "--help"]).unwrap().cmd,
            Cmd::ResetPasswordHelp
        );
        assert_eq!(
            p(&["reset-password", "admin", "-h"]).unwrap().cmd,
            Cmd::ResetPasswordHelp
        );
    }

    #[test]
    fn errors() {
        assert!(p(&["import"]).is_err());
        assert!(p(&["reset-password"]).is_err());
        assert!(p(&["reset-password", "a", "b"]).is_err());
        assert!(p(&["bogus"]).is_err());
        assert!(p(&["--port", "abc"]).is_err());
        assert!(p(&["--port"]).is_err());
        assert!(p(&["--nope"]).is_err());
    }

    #[test]
    fn healthcheck_against_tiny_server() {
        use std::net::TcpListener;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            for (i, conn) in l.incoming().take(2).enumerate() {
                let mut c = conn.unwrap();
                let mut b = [0u8; 512];
                let n = c.read(&mut b).unwrap();
                let req = String::from_utf8_lossy(&b[..n]).to_string();
                assert!(
                    req.starts_with("GET /api/info HTTP/1.0\r\nHost: 127.0.0.1\r\n"),
                    "{req}"
                );
                let status = if i == 0 { "200 OK" } else { "403 Forbidden" };
                let _ = c.write_all(
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n").as_bytes(),
                );
            }
        });
        assert!(healthcheck(port));
        assert!(!healthcheck(port));
        h.join().unwrap();
        // A port nobody is listening on
        assert!(!healthcheck(port));
    }
}
