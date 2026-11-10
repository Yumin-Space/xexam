//! Binary entry point: parses arguments and starts the server; the `healthcheck` subcommand is the container self-check.
//!
//! Subcommands: `serve` (default) | `healthcheck` | `import <xlsx>` | `migrate-progress [progress.json]`.
//! The legacy form `--import <xlsx>` is also accepted.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::config::Config;
use xexam_backend::database::Store;
use xexam_backend::services::{Engine, RealEngine};

#[derive(Debug, PartialEq, Eq)]
enum Cmd {
    Serve,
    Healthcheck,
    Import(PathBuf),
    MigrateProgress(Option<PathBuf>),
    Help,
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
Environment variables: XEXAM_DATA XEXAM_HOST XEXAM_PORT XEXAM_ALLOWED_HOSTS XEXAM_ALLOWED_ORIGINS XEXAM_ALLOW_ANY_HOST XEXAM_PARSE_TIMEOUT_SECS XEXAM_LOG";

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
        (Some("help"), None) => a.cmd = Cmd::Help,
        (Some(other), None) => return Err(format!("unknown subcommand: {other}")),
    }
    Ok(a)
}

fn say(msg: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{msg}");
    let _ = out.flush();
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("XEXAM_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    // No ANSI escapes: user-controlled strings written to the log must not inject terminal control sequences
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
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
fn open_store(data_dir: &Path) -> Result<Arc<Store>, String> {
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
    let store = Arc::new(Store::open(&db_path).map_err(|e| format!("cannot open database: {e}"))?);
    let legacy = data_dir.join("progress.json");
    if legacy.is_file() {
        match store.migrate_legacy_progress(&legacy) {
            Ok((true, msg)) => say(&format!(
                "Migrated the old study records into the database: {msg} (from {})",
                legacy.display()
            )),
            // "Already migrated earlier" (the literal Chinese message 「之前已经迁移过」) is the normal case; no need to report it on every start
            Ok((false, msg)) if msg.contains("之前已经迁移过") => {}
            Ok((false, msg)) => say(&format!(
                "Did not migrate the old study records: {msg}. Fix progress.json and restart to try again, or run xexam-backend migrate-progress manually"
            )),
            Err(e) => say(&format!(
                "Failed to migrate the old study records (the database was not changed): {e}. Restart later to try again, or run xexam-backend migrate-progress manually"
            )),
        }
    }
    Ok(store)
}

fn run_import(path: &Path, data_dir: &Path) -> i32 {
    let store = match open_store(data_dir) {
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
            say(&format!("Import failed: {e}"));
            return 1;
        }
    };
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

fn run_migrate(path: Option<PathBuf>, data_dir: &Path) -> i32 {
    let store = match open_store(data_dir) {
        Ok(s) => s,
        Err(e) => {
            say(&e);
            return 1;
        }
    };
    let path = path.unwrap_or_else(|| data_dir.join("progress.json"));
    match store.migrate_legacy_progress(&path) {
        Ok((true, msg)) => {
            say(&format!(
                "Migrated the old study records into the database: {msg} (from {})",
                path.display()
            ));
            0
        }
        Ok((false, msg)) => {
            say(&format!("Not migrated: {msg}"));
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
    let store = match open_store(&cfg.data_dir) {
        Ok(s) => s,
        Err(e) => {
            say(&e);
            return 1;
        }
    };
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
        say("Note: XEXAM_ALLOW_ANY_HOST is on, so the Host header is no longer checked. This program has no login of its own; make sure authentication (for example Cloudflare Access) sits in front of it.");
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
        say("The question bank is still empty: open the page and upload an Excel file under '题库管理' (Question bank management).");
    }
    let state = AppState::new(cfg, store.clone(), engine, shutdown_tx);
    let app = build_router(state.clone());

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
    drop(state);
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
        Cmd::Healthcheck => i32::from(!healthcheck(cfg.port)),
        Cmd::Import(path) => run_import(&path, &cfg.data_dir),
        Cmd::MigrateProgress(path) => run_migrate(path, &cfg.data_dir),
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
    }

    #[test]
    fn errors() {
        assert!(p(&["import"]).is_err());
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
