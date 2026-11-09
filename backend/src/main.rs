//! 二进制入口：解析参数，启动服务；`healthcheck` 子命令给容器自检用。
//!
//! 子命令：`serve`（默认）| `healthcheck` | `import <xlsx>` | `migrate-progress [progress.json]`。
//! 也接受旧版的写法 `--import <xlsx>`。
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

const USAGE: &str = "用法：
  xexam-backend [serve] [--data DIR] [--host ADDR] [--port N] [--allow-shutdown] [--no-browser]
  xexam-backend healthcheck [--port N]
  xexam-backend import <题库.xlsx> [--data DIR]       （也接受 --import <题库.xlsx>）
  xexam-backend migrate-progress [progress.json] [--data DIR]
环境变量：XEXAM_DATA XEXAM_HOST XEXAM_PORT XEXAM_ALLOWED_HOSTS XEXAM_ALLOWED_ORIGINS XEXAM_ALLOW_ANY_HOST XEXAM_PARSE_TIMEOUT_SECS XEXAM_LOG";

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
    // 第一个不带 `-` 的词是子命令
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
                    .ok_or_else(|| format!("{what} 需要一个值")),
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
                        .map_err(|_| "--port 必须是 0-65535 的整数".to_string())?,
                )
            }
            "--import" => import_flag = Some(PathBuf::from(value("--import")?)),
            "--allow-shutdown" => a.allow_shutdown = true,
            "--no-browser" => {}
            "-h" | "--help" => a.cmd = Cmd::Help,
            s if s.starts_with('-') => return Err(format!("不认识的参数：{s}")),
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
                .ok_or_else(|| "import 需要一个 .xlsx 文件路径".to_string())?;
            a.cmd = Cmd::Import(PathBuf::from(p));
        }
        (Some("migrate-progress"), None) => {
            a.cmd = Cmd::MigrateProgress(positional.first().map(PathBuf::from))
        }
        (Some("help"), None) => a.cmd = Cmd::Help,
        (Some(other), None) => return Err(format!("不认识的子命令：{other}")),
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
    // 不带 ANSI 转义：用户可控的字符串进日志时不能注入终端控制序列
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .try_init();
}

/// 容器自检：`GET http://127.0.0.1:<port>/api/info`，Host 为 127.0.0.1（在默认白名单里）；200 才算健康。
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

/// 打开数据库。旧版 `progress.json` 只有在数据目录里**没有任何** .db 文件时才迁移（迁移计划 Q18）。
fn open_store(data_dir: &Path) -> Result<Arc<Store>, String> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| format!("无法创建数据目录 {}：{e}", data_dir.display()))?;
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
    let fresh = others.is_empty();
    if !db_path.exists() && !others.is_empty() {
        say(&format!(
            "注意：数据目录里没有 xexam.db，但有别的数据库文件：{}。如果那是以前版本的题库，请先停止程序，把它改名为 xexam.db 再启动；现在会新建一个空的 xexam.db。",
            others.join("、")
        ));
    }
    let store = Arc::new(Store::open(&db_path).map_err(|e| format!("无法打开数据库：{e}"))?);
    let legacy = data_dir.join("progress.json");
    if fresh && legacy.is_file() {
        match store.migrate_legacy_progress(&legacy) {
            Ok((true, msg)) => say(&format!(
                "已把旧的学习记录迁移进数据库：{msg}（来自 {}）",
                legacy.display()
            )),
            Ok((false, msg)) => say(&format!("没有迁移旧的学习记录：{msg}")),
            Err(e) => say(&format!("迁移旧的学习记录失败（数据库没有改动）：{e}")),
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
            say(&format!("导入失败：读不了 {}：{e}", path.display()));
            return 1;
        }
    };
    let wb = match engine.check_and_parse(&data) {
        Ok(w) => w,
        Err(e) => {
            say(&format!("导入失败：{e}"));
            return 1;
        }
    };
    if wb.questions.is_empty() {
        say("这份文件里没有读到任何可用的题，没有导入。");
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
            say("写入数据库失败，题库没有任何改动。");
            return 1;
        }
    };
    let n = |k: &str| plan.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    let count = store.count_active().unwrap_or(0);
    say(&format!(
        "已导入 {name}：新增 {}，更新 {}，恢复 {}，下架 {}，读不出来 {}；现在共 {count} 题。",
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
            say(&format!("  没读入：{sheet} 第 {row} 行：{reason}"));
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
                "已把旧的学习记录迁移进数据库：{msg}（来自 {}）",
                path.display()
            ));
            0
        }
        Ok((false, msg)) => {
            say(&format!("没有迁移：{msg}"));
            0
        }
        Err(e) => {
            tracing::error!("migrate_legacy_progress failed: {e}");
            say("迁移失败，数据库没有改动。");
            1
        }
    }
}

/// 等 SIGTERM / SIGINT（unix），其它平台只有 Ctrl-C。
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
                "无法监听 {}:{}：{e}。请关掉占用这个端口的程序，或用 --port 换一个端口。",
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
        "Xexam v{} 已启动：http://localhost:{}/",
        cfg.version, cfg.port
    ));
    say(&format!(
        "数据目录：{}",
        std::path::absolute(&cfg.data_dir)
            .unwrap_or_else(|_| cfg.data_dir.clone())
            .display()
    ));
    if any_host {
        say("注意：XEXAM_ALLOW_ANY_HOST 已打开，不再检查 Host。这个程序自己没有登录，请确保前面有身份验证（例如 Cloudflare Access）。");
        tracing::warn!("XEXAM_ALLOW_ANY_HOST is on: the Host header is not checked");
    } else if !extra.is_empty() {
        say(&format!("允许的域名/地址：{}", extra.join("、")));
    }
    if !origins.is_empty() {
        say(&format!("额外允许的 Origin：{}", origins.join("、")));
    }
    if store.count_active().unwrap_or(0) == 0 {
        say("题库还是空的：打开页面后在「题库管理」里上传 Excel。");
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
        _ = os_signal() => say("收到停止信号，正在退出……"),
        _ = rx.recv() => say("收到页面上的关闭请求，正在退出……"),
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
    say("已退出。");
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
                eprintln!("无法启动异步运行时：{e}");
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
        // 没人监听的端口
        assert!(!healthcheck(port));
    }
}
