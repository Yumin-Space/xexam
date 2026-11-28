//! The endpoint allow-list: a push subscription may only point at the push services of the big browser vendors.
//! The same rules (and the same test table) as the backend's `push/allow.rs`; the courier checks again so a
//! compromised or buggy backend cannot turn it into a request forwarder.

/// Longest endpoint accepted, in bytes.
pub const MAX_ENDPOINT: usize = 2048;

/// Hosts that match exactly.
const EXACT_HOSTS: [&str; 3] = [
    "fcm.googleapis.com",
    "updates.push.services.mozilla.com",
    "web.push.apple.com",
];
/// Host suffixes that match on a label boundary.
const SUFFIXES: [&str; 2] = [".push.apple.com", ".notify.windows.com"];

/// The lower-case host of an allowed endpoint (one trailing dot removed), or `None` when the endpoint is not allowed.
pub fn allowed_host(endpoint: &str) -> Option<String> {
    if endpoint.len() > MAX_ENDPOINT || !endpoint.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return None;
    }
    let rest = endpoint.strip_prefix("https://")?;
    // A path is required: the authority ends at the first `/`
    let (authority, _path) = rest.split_once('/')?;
    if authority.contains('@') {
        return None;
    }
    let host_part = match authority.split_once(':') {
        None => authority,
        Some((h, "443")) => h,
        Some(_) => return None,
    };
    let host = host_part.to_ascii_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host);
    if host.is_empty() {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let label_ok = |l: &&str| {
        !l.is_empty()
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !l.starts_with("xn--")
    };
    if !labels.iter().all(label_ok) {
        return None;
    }
    // No IP literal: an address has only numeric labels
    if labels.iter().all(|l| l.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let listed = EXACT_HOSTS.contains(&host) || SUFFIXES.iter().any(|s| host.ends_with(s));
    listed.then(|| host.to_string())
}

/// `true` when the courier may send to this endpoint.
pub fn endpoint_allowed(endpoint: &str) -> bool {
    allowed_host(endpoint).is_some()
}

/// What a log line may say about an endpoint: the host of an allowed one, never anything else (a rejected endpoint can
/// carry user information).
pub fn log_host(endpoint: &str) -> String {
    allowed_host(endpoint).unwrap_or_else(|| "(not allowed)".to_string())
}

/// The path and query of an endpoint that starts with `https://<authority>` (what a test target is given).
pub fn path_and_query(endpoint: &str) -> Option<&str> {
    let rest = endpoint.strip_prefix("https://")?;
    rest.find('/').map(|i| &rest[i..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_table_holds() {
        let long = format!("https://fcm.googleapis.com/{}", "a".repeat(2049 - 27));
        assert_eq!(long.len(), 2049);
        let table: Vec<(String, bool)> = vec![
            ("https://fcm.googleapis.com/fcm/send/abc".into(), true),
            ("https://FCM.googleapis.com./fcm/send/abc".into(), true),
            ("https://fcm.googleapis.com:443/x".into(), true),
            (
                "https://updates.push.services.mozilla.com/wpush/v2/abc".into(),
                true,
            ),
            ("https://web.push.apple.com/QAbc".into(), true),
            ("https://api.push.apple.com/x".into(), true),
            (
                "https://wns2-par02p.notify.windows.com/w/?token=abc".into(),
                true,
            ),
            ("http://fcm.googleapis.com/x".into(), false),
            ("https://fcm.googleapis.com:8443/x".into(), false),
            ("https://user@fcm.googleapis.com/x".into(), false),
            ("https://evilpush.apple.com/x".into(), false),
            ("https://push.apple.com.evil.example/x".into(), false),
            ("https://127.0.0.1/x".into(), false),
            ("https://[::1]/x".into(), false),
            ("https://xn--fcm-xyz.googleapis.com/x".into(), false),
            ("https://fcm.googleapis.com".into(), false),
            (long, false),
        ];
        for (endpoint, want) in table {
            assert_eq!(endpoint_allowed(&endpoint), want, "{endpoint}");
        }
    }

    #[test]
    fn more_edges() {
        for bad in [
            "",
            "https://",
            "https:///x",
            "https://fcm.googleapis.com:/x",
            "https://fcm.googleapis.com:0443/x",
            "https://fcm..googleapis.com/x",
            "https://.fcm.googleapis.com/x",
            "https://push.apple.com/x",
            "https://notify.windows.com/x",
            "https://fcm.googleapis.com?x=1/y",
            "https://fcm.googleapis.com/x y",
            "https://fcm.googleapis.com/\u{e9}",
            "https://fcm.googleapis.com/x\r\n",
            "https://1.2.3.4/x",
            "https://2130706433/x",
            "ftp://fcm.googleapis.com/x",
            "HTTPS://fcm.googleapis.com/x",
        ] {
            assert!(!endpoint_allowed(bad), "{bad:?}");
        }
        assert_eq!(
            allowed_host("https://FCM.googleapis.com./fcm/send/abc").as_deref(),
            Some("fcm.googleapis.com")
        );
        assert_eq!(log_host("https://user@evil.example/x"), "(not allowed)");
        assert_eq!(
            path_and_query("https://fcm.googleapis.com/fcm/send/abc?x=1"),
            Some("/fcm/send/abc?x=1")
        );
    }
}
