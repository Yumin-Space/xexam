//! The extraction child process (IX-1, SEC-5, SEC-6). ai-service runs its own binary as `xexam-ai-service extract …`:
//! the file bytes arrive on stdin, one result line leaves on stdout, and the parent kills the child after 30 s. A
//! malformed file can therefore panic, loop, overflow the stack or blow up memory without touching the service.
//!
//! Child: (1) Linux: `oom_score_adj` 1000, so an out-of-memory kill picks the child; (2) a watchdog thread polls every
//! 50 ms: the resident size above 768 MiB ends the process with exit code 3, 25 s of wall time with exit code 4;
//! (3) stdin is read up to `max-bytes + 1`; (4) the extraction runs on a thread with a 64 MiB stack inside
//! `catch_unwind` (a panic is `corrupt`); (5) exactly one line `XEXAM-EXTRACT-RESULT <json>` is written.
//!
//! Parent: `env_clear()` (no secret reaches the child), stdin and stdout piped, stderr discarded, working directory the
//! system temp directory; the bytes are fed by a writer thread (then dropped), stdout is read up to 8 MiB, the child is
//! killed and reaped on a timeout and on every early return.
use super::{Doc, ExtractError, LangHint, Limits};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The prefix of the result line.
pub const RESULT_PREFIX: &str = "XEXAM-EXTRACT-RESULT ";
/// The child's resident size limit.
pub const CHILD_MAX_RSS: u64 = 768 << 20;
/// Most stdout the parent reads.
pub const CHILD_OUTPUT_MAX: usize = 8 << 20;
/// The parent kills the child after this many seconds.
pub const EXTRACT_KILL_SECS: u64 = 30;
/// The child's cooperative deadline and its hard watchdog (seconds).
pub const CHILD_SOFT_SECS: u64 = 24;
pub const CHILD_HARD_SECS: u64 = 25;
const STACK_BYTES: usize = 64 << 20;

/// The command line of the child (`extract --ext <ext> --lang <lang> --max-bytes <n> --max-chars <n>`; tests add
/// `--max-rss-mb <n>` and `--no-watchdog`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildArgs {
    pub ext: String,
    pub lang: LangHint,
    pub max_bytes: u64,
    pub max_chars: usize,
    pub max_rss_mb: Option<u64>,
    pub no_watchdog: bool,
}

impl ChildArgs {
    pub fn new(ext: &str, lang: LangHint, max_bytes: u64) -> ChildArgs {
        ChildArgs {
            ext: ext.to_string(),
            lang,
            max_bytes,
            max_chars: Limits::default().max_chars,
            max_rss_mb: None,
            no_watchdog: false,
        }
    }

    /// The flags after the `extract` word.
    pub fn to_args(&self) -> Vec<String> {
        let mut v = vec![
            "--ext".to_string(),
            self.ext.clone(),
            "--lang".to_string(),
            self.lang.as_str().to_string(),
            "--max-bytes".to_string(),
            self.max_bytes.to_string(),
            "--max-chars".to_string(),
            self.max_chars.to_string(),
        ];
        if let Some(n) = self.max_rss_mb {
            v.push("--max-rss-mb".into());
            v.push(n.to_string());
        }
        if self.no_watchdog {
            v.push("--no-watchdog".into());
        }
        v
    }

    /// Parses the flags (any order); `None` when one is missing or malformed.
    pub fn parse(args: &[String]) -> Option<ChildArgs> {
        let mut ext = None;
        let mut lang = LangHint::Zh;
        let mut max_bytes = None;
        let mut max_chars = Limits::default().max_chars;
        let mut max_rss_mb = None;
        let mut no_watchdog = false;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--ext" => ext = Some(it.next()?.to_ascii_lowercase()),
                "--lang" => lang = LangHint::parse(it.next()?).unwrap_or(LangHint::Zh),
                "--max-bytes" => max_bytes = Some(it.next()?.parse::<u64>().ok()?),
                "--max-chars" => max_chars = it.next()?.parse::<usize>().ok()?,
                "--max-rss-mb" => max_rss_mb = Some(it.next()?.parse::<u64>().ok()?),
                "--no-watchdog" => no_watchdog = true,
                _ => return None,
            }
        }
        Some(ChildArgs {
            ext: ext?,
            lang,
            max_bytes: max_bytes?,
            max_chars,
            max_rss_mb,
            no_watchdog,
        })
    }
}

/// The resident set size in bytes (Linux: `/proc/self/statm`, 4 KiB pages); `None` elsewhere.
fn rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let s = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
        Some(pages * 4096)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn spawn_watchdog(max_rss: u64) {
    let start = Instant::now();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(50));
        if rss_bytes().is_some_and(|r| r > max_rss) {
            std::process::exit(3);
        }
        if start.elapsed() >= Duration::from_secs(CHILD_HARD_SECS) {
            std::process::exit(4);
        }
    });
}

fn result_line(r: &Result<Doc, ExtractError>) -> String {
    let v = match r {
        Ok(doc) => serde_json::json!({"ok": true, "doc": doc}),
        Err(e) => serde_json::json!({"ok": false, "error": e.code()}),
    };
    format!("{RESULT_PREFIX}{v}\n")
}

/// The child's whole job; returns the exit code (0 after a result line).
pub fn child_main(args: &[String], mut stdin: impl Read, mut stdout: impl Write) -> i32 {
    let Some(a) = ChildArgs::parse(args) else {
        return 2;
    };
    #[cfg(target_os = "linux")]
    {
        let _ = std::fs::write("/proc/self/oom_score_adj", "1000");
    }
    if !a.no_watchdog {
        spawn_watchdog(a.max_rss_mb.map_or(CHILD_MAX_RSS, |m| m << 20));
    }
    let started = Instant::now();
    let mut bytes = Vec::new();
    let read = stdin
        .by_ref()
        .take(a.max_bytes.saturating_add(1))
        .read_to_end(&mut bytes);
    let result: Result<Doc, ExtractError> = if read.is_err() {
        Err(ExtractError::Corrupt)
    } else if bytes.len() as u64 > a.max_bytes {
        Err(ExtractError::TooLarge)
    } else {
        let ext = a.ext.clone();
        let hint = a.lang;
        let limits = Limits {
            max_chars: a.max_chars,
            deadline: Some(started + Duration::from_secs(CHILD_SOFT_SECS)),
            ..Limits::default()
        };
        let worker = std::thread::Builder::new()
            .stack_size(STACK_BYTES)
            .spawn(move || {
                if cfg!(debug_assertions)
                    && std::env::var("XEXAM_AI_EXTRACT_TEST_HANG").as_deref() == Ok("1")
                {
                    loop {
                        std::thread::sleep(Duration::from_secs(3600));
                    }
                }
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    super::extract(&bytes, &ext, hint, &limits)
                }))
                .unwrap_or(Err(ExtractError::Corrupt))
            });
        match worker {
            Ok(h) => h.join().unwrap_or(Err(ExtractError::Corrupt)),
            Err(_) => Err(ExtractError::Corrupt),
        }
    };
    let line = result_line(&result);
    if stdout.write_all(line.as_bytes()).is_err() || stdout.flush().is_err() {
        return 1;
    }
    0
}

/// Kills and reaps the child when dropped (every early return of the parent).
struct ChildGuard(Option<std::process::Child>);

impl ChildGuard {
    fn child(&mut self) -> &mut std::process::Child {
        self.0.as_mut().expect("child is present until drop")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Parses the stdout of a child: the last line with the result prefix.
pub fn parse_output(stdout: &[u8]) -> Result<Doc, ExtractError> {
    let text = String::from_utf8_lossy(stdout);
    let Some(line) = text
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix(RESULT_PREFIX))
    else {
        return Err(ExtractError::Corrupt);
    };
    let v: serde_json::Value = serde_json::from_str(line).map_err(|_| ExtractError::Corrupt)?;
    match v.get("ok").and_then(|o| o.as_bool()) {
        Some(true) => serde_json::from_value::<Doc>(v.get("doc").cloned().unwrap_or_default())
            .map_err(|_| ExtractError::Corrupt),
        Some(false) => Err(v
            .get("error")
            .and_then(|e| e.as_str())
            .and_then(ExtractError::from_code)
            .unwrap_or(ExtractError::Corrupt)),
        None => Err(ExtractError::Corrupt),
    }
}

/// Runs the child and waits (call it inside `tokio::task::spawn_blocking`). `bytes` is fed to the child and dropped as
/// soon as stdin is closed. `hang`: test hook, honoured by debug builds only.
pub fn run_child_with(
    exe: &Path,
    args: &ChildArgs,
    bytes: Vec<u8>,
    timeout: Duration,
    hang: bool,
    stop: &AtomicBool,
) -> Result<Doc, ExtractError> {
    let mut cmd = Command::new(exe);
    cmd.arg("extract")
        .args(args.to_args())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir(std::env::temp_dir());
    if hang {
        cmd.env("XEXAM_AI_EXTRACT_TEST_HANG", "1");
    }
    let mut guard = ChildGuard(Some(cmd.spawn().map_err(|_| ExtractError::Corrupt)?));
    let stdin = guard.child().stdin.take();
    let stdout = guard.child().stdout.take();
    let writer = std::thread::spawn(move || {
        if let Some(mut s) = stdin {
            let _ = s.write_all(&bytes);
            // `s` and `bytes` drop here: the child sees end of input and the parent's copy is gone
        }
    });
    let overflow = Arc::new(AtomicBool::new(false));
    let over = overflow.clone();
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(mut s) = stdout {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        out.extend_from_slice(&buf[..n]);
                        if out.len() > CHILD_OUTPUT_MAX {
                            over.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
            }
        }
        out
    });
    let started = Instant::now();
    let status = loop {
        if overflow.load(Ordering::SeqCst) {
            drop(guard);
            let _ = writer.join();
            let _ = reader.join();
            return Err(ExtractError::TooMuchText);
        }
        match guard.child().try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(_) => return Err(ExtractError::Corrupt),
        }
        if stop.load(Ordering::SeqCst) {
            // a cancel: the child is killed and reaped, the bytes are gone
            drop(guard);
            let _ = writer.join();
            let _ = reader.join();
            return Err(ExtractError::Timeout);
        }
        if started.elapsed() >= timeout {
            drop(guard);
            let _ = writer.join();
            let _ = reader.join();
            return Err(ExtractError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = writer.join();
    let out = reader.join().unwrap_or_default();
    drop(guard);
    if overflow.load(Ordering::SeqCst) || out.len() > CHILD_OUTPUT_MAX {
        return Err(ExtractError::TooMuchText);
    }
    match status.code() {
        Some(0) => parse_output(&out),
        Some(3) => Err(ExtractError::TooComplex),
        Some(4) => Err(ExtractError::Timeout),
        _ => Err(ExtractError::Corrupt),
    }
}

/// [`run_child_with`] without the test hook (what production calls).
pub fn run_child_blocking(
    exe: &Path,
    args: &ChildArgs,
    bytes: Vec<u8>,
    timeout: Duration,
) -> Result<Doc, ExtractError> {
    run_child_with(exe, args, bytes, timeout, false, &AtomicBool::new(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::testgen;

    fn run(args: &ChildArgs, input: &[u8]) -> (i32, String) {
        let mut out = Vec::new();
        let code = child_main(&args.to_args(), input, &mut out);
        (code, String::from_utf8_lossy(&out).into_owned())
    }

    fn quiet(ext: &str, max: u64) -> ChildArgs {
        ChildArgs {
            no_watchdog: true,
            ..ChildArgs::new(ext, LangHint::En, max)
        }
    }

    #[test]
    fn args_round_trip_and_refuse_junk() {
        let a = ChildArgs {
            max_rss_mb: Some(64),
            no_watchdog: true,
            ..ChildArgs::new("pdf", LangHint::Ja, 1000)
        };
        assert_eq!(ChildArgs::parse(&a.to_args()), Some(a));
        assert_eq!(ChildArgs::parse(&["--ext".to_string()]), None);
        assert_eq!(ChildArgs::parse(&["--bogus".to_string()]), None);
        assert_eq!(
            ChildArgs::parse(&["--ext".into(), "txt".into()]),
            None,
            "max-bytes is required"
        );
    }

    #[test]
    fn a_text_file_round_trip() {
        let text = "Which tool keeps a stock pot at a steady simmer? A thermostat does.";
        let (code, out) = run(&quiet("txt", 1 << 20), text.as_bytes());
        assert_eq!(code, 0);
        let doc = parse_output(out.as_bytes()).unwrap();
        assert_eq!(doc.sections[0].blocks, vec![text]);
    }

    #[test]
    fn the_result_is_the_last_prefixed_line_and_library_noise_is_ignored() {
        let doc_line = result_line(&Err(ExtractError::NoText));
        let ok_line = {
            let (_, o) = run(
                &quiet("txt", 1 << 20),
                b"plain text with enough letters to count",
            );
            o
        };
        let mixed = format!("noise\n{doc_line}more noise\n{ok_line}trailing\n");
        assert!(parse_output(mixed.as_bytes()).is_ok());
        let mixed = format!("{ok_line}{doc_line}");
        assert_eq!(parse_output(mixed.as_bytes()), Err(ExtractError::NoText));
        assert_eq!(parse_output(b"no result here"), Err(ExtractError::Corrupt));
        assert_eq!(
            parse_output(format!("{RESULT_PREFIX}{{bad json").as_bytes()),
            Err(ExtractError::Corrupt)
        );
    }

    #[test]
    fn input_over_max_bytes_is_too_large() {
        let (code, out) = run(&quiet("txt", 10), b"this is longer than ten bytes");
        assert_eq!(code, 0);
        assert_eq!(parse_output(out.as_bytes()), Err(ExtractError::TooLarge));
        // exactly max is fine
        let (_, out) = run(&quiet("txt", 29), b"this is longer than ten bytes");
        assert_ne!(parse_output(out.as_bytes()), Err(ExtractError::TooLarge));
    }

    #[test]
    fn errors_travel_as_their_code() {
        let (_, out) = run(&quiet("pdf", 1 << 20), b"not a pdf");
        assert_eq!(parse_output(out.as_bytes()), Err(ExtractError::Corrupt));
        let (_, out) = run(&quiet("doc", 1 << 20), b"x");
        assert_eq!(parse_output(out.as_bytes()), Err(ExtractError::Legacy));
        let (_, out) = run(&quiet("txt", 1 << 20), b"");
        assert_eq!(parse_output(out.as_bytes()), Err(ExtractError::Empty));
        let zip = testgen::docx(&testgen::DocxSpec::paragraphs(&[
            "Which tool keeps a stock pot at a steady simmer?",
        ]));
        let (_, out) = run(&quiet("docx", 1 << 20), &zip);
        assert!(parse_output(out.as_bytes()).is_ok());
    }

    #[test]
    fn bad_args_are_exit_2() {
        let mut out = Vec::new();
        assert_eq!(child_main(&["--nope".to_string()], &b""[..], &mut out), 2);
        assert!(out.is_empty());
    }

    #[test]
    fn the_error_codes_are_the_im9_suffixes() {
        let want = [
            "empty",
            "tooLarge",
            "type",
            "legacy",
            "image",
            "protected",
            "corrupt",
            "tooComplex",
            "noText",
            "tooMuchText",
            "encoding",
            "readTimeout",
        ];
        let got: Vec<&str> = crate::extract::ALL_ERRORS
            .iter()
            .map(|e| e.code())
            .collect();
        assert_eq!(got, want);
        for e in crate::extract::ALL_ERRORS {
            assert_eq!(ExtractError::from_code(e.code()), Some(e));
        }
        assert_eq!(ExtractError::from_code("nope"), None);
    }

    #[test]
    fn doc_json_round_trip() {
        let doc = Doc {
            kind: crate::extract::FileKind::Pptx,
            sections: vec![crate::extract::Section {
                label: "Slide 1".into(),
                blocks: vec!["a".into(), "b\nc".into()],
            }],
            chars: 4,
            meaningful: 3,
            pages: 1,
            encoding: None,
        };
        let v = serde_json::to_string(&doc).unwrap();
        assert_eq!(serde_json::from_str::<Doc>(&v).unwrap(), doc);
        assert!(v.contains("\"pptx\""));
    }
}
