//! Binary entry point: `xexam-ai-service serve` (default), `xexam-ai-service healthcheck`, `xexam-ai-service --help`, and
//! `xexam-ai-service extract …` (the file-extraction child of the AI import, run by the service itself).
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use xexam_ai_service::app::App;
use xexam_ai_service::config::{Config, USAGE};

#[derive(Debug, PartialEq, Eq)]
enum Cmd {
    Serve,
    Healthcheck,
    Help,
}

fn parse_args(argv: &[String]) -> Result<Cmd, String> {
    match argv {
        [] => Ok(Cmd::Serve),
        [one] => match one.as_str() {
            "serve" => Ok(Cmd::Serve),
            "healthcheck" => Ok(Cmd::Healthcheck),
            "--help" | "-h" | "help" => Ok(Cmd::Help),
            other => Err(format!("unknown argument: {other}")),
        },
        _ => Err("too many arguments".to_string()),
    }
}

/// `GET http://127.0.0.1:<port>/healthz` over HTTP/1.0 with a 3 s limit; only a 200 counts as healthy. Never depends on
/// the backend or a provider.
fn healthcheck(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let limit = Duration::from_secs(3);
    let started = std::time::Instant::now();
    let Ok(mut s) = TcpStream::connect_timeout(&addr, limit) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(limit));
    let _ = s.set_write_timeout(Some(limit));
    if s.write_all(b"GET /healthz HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 256];
    let mut got = 0usize;
    while got < buf.len() && started.elapsed() < limit {
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
    let host = cfg.host.clone();
    let port = cfg.port;
    let version = cfg.version.clone();
    let app = match App::open(cfg) {
        Ok(a) => a,
        Err(e) => {
            tracing::error!("{e}");
            return 1;
        }
    };
    let listener = match tokio::net::TcpListener::bind((host.as_str(), port)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("cannot listen on {host}:{port}: {}", e.kind());
            return 1;
        }
    };
    tracing::info!("Xexam AI service {version} listening on {host}:{port}");
    tracing::info!(
        "AI database: {} (journal: {})",
        app.0.store.path().display(),
        app.0.store.journal_mode()
    );
    let tasks = xexam_ai_service::retention::spawn_all(&app);
    let router = xexam_ai_service::api::build_router(app.clone());
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let n2 = notify.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move { n2.notified().await })
            .await
    });
    let mut code = 0;
    tokio::select! {
        _ = os_signal() => {}
        r = &mut server => {
            let what = if r.is_ok() { "server ended" } else { "server task failed" };
            tracing::error!("the AI service stopped unexpectedly: {what}");
            code = 1;
        }
    }
    // Every running answer ends with `stopped` and is saved, then the streams close
    let cut = app.0.begin_shutdown();
    notify.notify_one();
    // The last state report tells the main backend at once that AI+ is gone (2 s at most)
    app.0.heartbeat_stopping().await;
    if !server.is_finished()
        && tokio::time::timeout(Duration::from_secs(8), &mut server)
            .await
            .is_err()
    {
        tracing::warn!("graceful shutdown timed out after 8s; exiting anyway");
        server.abort();
    }
    for t in &tasks {
        t.abort();
    }
    tracing::info!("AI service stopped ({cut} answers were cut off)");
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        xexam_ai_service::logs::flush_once(&app.0.link, xexam_ai_service::logs::queue()),
    )
    .await;
    if let Err(e) = app.0.store.checkpoint_now() {
        tracing::warn!("AI database checkpoint at shutdown failed: {e}");
    }
    drop(app);
    code
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // The extraction child of the AI file import (IX-1): before any configuration, database or logging set-up, so it
    // works with an empty environment and never touches a data directory
    if argv.first().map(String::as_str) == Some("extract") {
        let code = xexam_ai_service::extract::child::child_main(
            argv.get(1..).unwrap_or(&[]),
            std::io::stdin().lock(),
            std::io::stdout().lock(),
        );
        std::process::exit(code);
    }
    let cmd = match parse_args(&argv) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let code = match cmd {
        Cmd::Help => {
            println!("{USAGE}");
            0
        }
        Cmd::Healthcheck => i32::from(!healthcheck(Config::healthcheck_port())),
        Cmd::Serve => {
            let cfg = Config::from_env();
            xexam_ai_service::logs::init(cfg.log_filter.as_deref());
            match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => {
                    let code = rt.block_on(serve(cfg));
                    // Do not wait for blocking work left behind (for example a DNS lookup of an unreachable backend)
                    rt.shutdown_timeout(Duration::from_millis(500));
                    code
                }
                Err(e) => {
                    eprintln!("cannot start the async runtime: {e}");
                    1
                }
            }
        }
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Result<Cmd, String> {
        parse_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn arguments() {
        assert_eq!(p(&[]), Ok(Cmd::Serve));
        assert_eq!(p(&["serve"]), Ok(Cmd::Serve));
        assert_eq!(p(&["healthcheck"]), Ok(Cmd::Healthcheck));
        assert_eq!(p(&["--help"]), Ok(Cmd::Help));
        assert!(p(&["bogus"]).is_err());
        assert!(p(&["serve", "x"]).is_err());
    }

    #[test]
    fn healthcheck_needs_a_200() {
        use std::net::TcpListener;
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = l.local_addr().expect("addr").port();
        let h = std::thread::spawn(move || {
            let answers: [&[u8]; 2] = [
                b"HTTP/1.1 200 OK\r\n\r\n{}",
                b"HTTP/1.1 503 Service Unavailable\r\n\r\n",
            ];
            for answer in answers {
                let (mut s, _) = l.accept().expect("accept");
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf);
                assert!(buf.starts_with(b"GET /healthz HTTP/1.0\r\n"));
                let _ = s.write_all(answer);
            }
        });
        assert!(healthcheck(port));
        assert!(!healthcheck(port));
        h.join().expect("join");
        // nothing listening any more
        assert!(!healthcheck(port));
    }
}
