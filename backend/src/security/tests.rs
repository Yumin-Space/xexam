use super::*;

fn set(items: &[&str]) -> HashSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn host_name_cases() {
    // 来自 tests/baseline/tools/host_origin_test.py
    for (h, want) in [
        (Some("localhost:8765"), "localhost"),
        (Some("LOCALHOST"), "localhost"),
        (Some("localhost."), "localhost"),
        (Some("[::1]:8765"), "::1"),
        (Some("xexam.example.com"), "xexam.example.com"),
        (Some("Xexam.Example.COM:443"), "xexam.example.com"),
        (Some("192.168.1.20:8765"), "192.168.1.20"),
        (Some(""), ""),
        (None, ""),
        // 来自 docs/audit/security.md C2 / api-contract 3.1
        (Some("localhost:1:2"), "localhost:1"),
        (Some("localhost:80@evil.com"), "localhost"),
        (Some("user@localhost"), "user@localhost"),
        (Some("[::1]x"), "::1"),
        (Some("localhost:abc"), "localhost"),
        (Some("127.0.0.1."), "127.0.0.1"),
        (Some("  Localhost:9  "), "localhost"),
        (Some("["), "["),
        (Some("[]"), ""),
        (Some("[::1"), "[::1"),
        (Some("LOCALHOST.:8765"), "localhost"),
        (Some("\u{3000}localhost\u{3000}"), "localhost"),
        (Some("İstanbul"), "i̇stanbul"), // 多字节字符不会在切片处 panic
    ] {
        assert_eq!(host_name(h), want, "host_name({h:?})");
    }
}

#[test]
fn origin_host_cases() {
    for (o, want) in [
        ("https://xexam.example.com", Some("xexam.example.com")),
        ("http://localhost:8765", Some("localhost")),
        ("HTTPS://X.Example.com:8443/", Some("x.example.com")),
        ("null", None),
        ("file://", None),
        ("", None),
        ("https://user@evil.com", None),
        ("ftp://x", None),
        ("https://", None),
        ("http://127.0.0.1/path", Some("127.0.0.1")),
        ("HTTP://127.0.0.1:5", Some("127.0.0.1")),
        ("http://[::1]:8765", Some("::1")),
        ("http://localhost:80@evil.com", None),
        ("  http://localhost  ", Some("localhost")),
        ("http:/localhost", None),
        ("evil.com", None),
        (
            "https://xexam.example.com.evil.com",
            Some("xexam.example.com.evil.com"),
        ),
        ("https://日本語.example/", Some("日本語.example")),
        ("İ://x", None),
    ] {
        assert_eq!(origin_host(Some(o)).as_deref(), want, "origin_host({o:?})");
    }
    assert_eq!(origin_host(None), None);
}

#[test]
fn parse_allowed_hosts_cases() {
    assert_eq!(
        parse_allowed_hosts(Some(
            "xexam.example.com, https://b.example.org:8443/path ,192.168.1.20:8765,,"
        )),
        set(&["xexam.example.com", "b.example.org", "192.168.1.20"])
    );
    assert_eq!(parse_allowed_hosts(None), HashSet::new());
    assert_eq!(parse_allowed_hosts(Some("")), HashSet::new());
    assert_eq!(parse_allowed_hosts(Some(" , ;; ")), HashSet::new());
    assert_eq!(
        parse_allowed_hosts(Some("a.com;b.com c.com\td.com\ne.com")),
        set(&["a.com", "b.com", "c.com", "d.com", "e.com"])
    );
    assert_eq!(
        parse_allowed_hosts(Some("HTTPS://Xexam.Example.COM/")),
        set(&["xexam.example.com"])
    );
    assert_eq!(parse_allowed_hosts(Some("[::1]:8080")), set(&["::1"]));
    assert_eq!(
        parse_allowed_hosts(Some("xexam.example.com.")),
        set(&["xexam.example.com"])
    );
    assert_eq!(parse_allowed_hosts(Some("http://")), HashSet::new());
    // 只剥第一个 "://"
    assert_eq!(parse_allowed_hosts(Some("a://b://c")), set(&["b"]));
}

#[test]
fn clean_filename_cases() {
    assert_eq!(clean_filename(Some("demo.xlsx")), "demo.xlsx");
    assert_eq!(clean_filename(None), "未命名.xlsx");
    assert_eq!(clean_filename(Some("")), "未命名.xlsx");
    assert_eq!(clean_filename(Some("   ")), "未命名.xlsx");
    // 捕获 126：テスト/dir\evil
    assert_eq!(
        clean_filename(Some("%E3%83%86%E3%82%B9%E3%83%88%2Fdir%5Cevil")),
        "evil"
    );
    assert_eq!(clean_filename(Some("..\\..\\x.xlsx")), "x.xlsx");
    assert_eq!(clean_filename(Some("a/b/c.xlsx")), "c.xlsx");
    assert_eq!(clean_filename(Some("dir/")), "未命名.xlsx");
    // 控制字符
    assert_eq!(clean_filename(Some("a%00b%0d%0a.xlsx")), "ab.xlsx");
    assert_eq!(clean_filename(Some("a\u{7f}b")), "ab");
    // 120 个字符（不是字节）
    let long = "题".repeat(200);
    let got = clean_filename(Some(&long));
    assert_eq!(got.chars().count(), 120);
    assert_eq!(clean_filename(Some(&"a".repeat(300))).len(), 120);
    // 无效 UTF-8 的百分号序列不 panic
    assert!(!clean_filename(Some("%ff%fe.xlsx")).is_empty());
    // 前后空白被去掉
    assert_eq!(clean_filename(Some("%20%20x.xlsx%20")), "x.xlsx");
}

#[test]
fn base64url_known_vectors() {
    // RFC 4648 测试向量（去掉填充）
    for (input, want) in [
        (&b""[..], ""),
        (b"f", "Zg"),
        (b"fo", "Zm8"),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg"),
        (b"fooba", "Zm9vYmE"),
        (b"foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64url_nopad(input), want);
    }
    assert_eq!(base64url_nopad(&[0xfb, 0xff, 0xfe]), "-__-");
    assert_eq!(base64url_nopad(&[0u8; 16]), "AAAAAAAAAAAAAAAAAAAAAA");
    assert_eq!(base64url_nopad(&[0xffu8; 16]), "_____________________w");
}

#[test]
fn tokens_are_22_urlsafe_chars_and_unique() {
    let mut seen = HashSet::new();
    for _ in 0..200 {
        let t = new_token();
        assert_eq!(t.len(), 22, "{t}");
        assert!(
            t.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{t}"
        );
        assert!(seen.insert(t));
    }
}

#[test]
fn token_eq_cases() {
    assert!(token_eq("abc", "abc"));
    assert!(!token_eq("abc", "abd"));
    assert!(!token_eq("abc", "abcd"));
    assert!(!token_eq("", "a"));
    assert!(token_eq("", ""));
    assert!(!token_eq("令牌", "令"));
    assert!(token_eq("令牌", "令牌"));
}

#[test]
fn py_helpers() {
    assert_eq!(py_strip("\u{a0} x \u{3000}"), "x");
    assert_eq!(strip_ctrl("a\u{0}b\u{1f}c\u{7f}d\u{80}"), "abcd\u{80}");
    assert_eq!(truncate_chars("あいうえお", 3), "あいう");
    assert_eq!(short(&"x".repeat(100)).len(), 80);
    assert_eq!(py_int_str("77"), Some(77));
    assert_eq!(py_int_str(" -5 "), Some(-5));
    assert_eq!(py_int_str("+3"), Some(3));
    assert_eq!(py_int_str("1_0"), Some(10));
    assert_eq!(py_int_str("1__0"), None);
    assert_eq!(py_int_str("_1"), None);
    assert_eq!(py_int_str("1_"), None);
    assert_eq!(py_int_str("12.7"), None);
    assert_eq!(py_int_str(""), None);
    assert_eq!(py_int_str("abc"), None);
    assert_eq!(py_int_str("-"), None);
    assert!(py_int_str(&"9".repeat(5000)).is_some_and(|v| v > 0));
    assert_eq!(latin1(&[0x41, 0xe9, 0xff]), "Aéÿ");
}

/// 用真实的 Python 生成期望值，保证 `py_json` 与 `json.dumps(ensure_ascii=False)` 字节一致。
/// 没有 python 时跳过（CI 里 python3 一般存在）。
#[test]
fn py_json_matches_python_for_tricky_strings() {
    use crate::api::json::py_json;
    use serde_json::json;
    let cases: Vec<String> = vec![
        "plain".into(),
        "quote\" backslash\\ slash/".into(),
        "nl\n cr\r tab\t bs\u{8} ff\u{c}".into(),
        "ctl\u{0}\u{1}\u{1f}\u{7f}\u{80}\u{9f}".into(),
        "日本語 中文 “引号” – 😀 \u{2028}\u{2029}\u{feff}".into(),
        "<script>&'\"</script>".into(),
        String::new(),
    ];
    let script = "import json,sys\nraw=sys.stdin.buffer.read().decode('utf-8')\nsys.stdout.buffer.write(json.dumps(json.loads(raw),ensure_ascii=False).encode('utf-8'))";
    for py in ["python", "python3"] {
        let probe = std::process::Command::new(py)
            .arg("-I")
            .arg("-c")
            .arg("print(1)")
            .output();
        if probe.map(|o| o.status.success()).unwrap_or(false) {
            for case in &cases {
                let value =
                    json!({ "k": case, "list": [case, 1, true, null], "nested": { "é": case } });
                let ours = py_json(&value);
                // 把我们的输出再交给 Python 重新序列化：必须逐字节相同（证明 Python 也会这样写）
                use std::io::Write;
                let mut child = std::process::Command::new(py)
                    .arg("-I")
                    .arg("-c")
                    .arg(script)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(&ours).unwrap();
                let out = child.wait_with_output().unwrap();
                assert!(out.status.success());
                assert_eq!(
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&ours),
                    "case {case:?}"
                );
            }
            return;
        }
    }
    eprintln!("python not available: skipped py_json differential test");
}
