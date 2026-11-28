//! The allow-list of push-service endpoints (contracts C.10). A subscription endpoint is attacker-controlled input that the
//! courier later POSTs to, so only the known push services are accepted.

/// The longest endpoint accepted (bytes).
const MAX_ENDPOINT: usize = 2048;
/// Hosts that match exactly.
const EXACT_HOSTS: [&str; 3] = [
    "fcm.googleapis.com",
    "updates.push.services.mozilla.com",
    "web.push.apple.com",
];
/// Host suffixes that match on a label boundary.
const SUFFIXES: [&str; 2] = [".push.apple.com", ".notify.windows.com"];

/// The normalised host (lower case, one trailing dot removed) of an allowed endpoint, `None` when it is refused.
pub(crate) fn allowed_host(endpoint: &str) -> Option<String> {
    if endpoint.len() > MAX_ENDPOINT || !endpoint.bytes().all(|b| (0x21..=0x7E).contains(&b)) {
        return None;
    }
    let rest = endpoint.strip_prefix("https://")?;
    // A path is required
    let (authority, _path) = rest.split_once('/')?;
    if authority.contains('@') {
        return None;
    }
    let host = match authority.split_once(':') {
        None => authority,
        Some((host, port)) => {
            if port != "443" {
                return None;
            }
            host
        }
    };
    let host = host.to_ascii_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host);
    if host.is_empty() {
        return None;
    }
    let mut all_numeric = true;
    for label in host.split('.') {
        if label.is_empty()
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || label.starts_with("xn--")
        {
            return None;
        }
        if !label.bytes().all(|b| b.is_ascii_digit()) {
            all_numeric = false;
        }
    }
    if all_numeric {
        return None;
    }
    let ok = EXACT_HOSTS.contains(&host) || SUFFIXES.iter().any(|s| host.ends_with(s));
    ok.then(|| host.to_string())
}

/// Whether `endpoint` is an `https://` address of a known push service (no userinfo, port 443 only, no IP literal).
pub fn endpoint_allowed(endpoint: &str) -> bool {
    allowed_host(endpoint).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_table() {
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
        for (endpoint, expected) in table {
            assert_eq!(endpoint_allowed(&endpoint), expected, "{endpoint}");
        }
    }

    #[test]
    fn exactly_the_limit_is_accepted() {
        let at_limit = format!("https://fcm.googleapis.com/{}", "a".repeat(2048 - 27));
        assert_eq!(at_limit.len(), 2048);
        assert!(endpoint_allowed(&at_limit));
    }

    #[test]
    fn odd_inputs_are_refused() {
        for endpoint in [
            "",
            "https://",
            "https:///x",
            "https://fcm.googleapis.com/ x",
            "https://fcm.googleapis.com/\u{e9}",
            "https://fcm.googleapis.com/x\n",
            "https://fcm.googleapis.com:/x",
            "https://fcm.googleapis.com:0443/x",
            "https://fcm..googleapis.com/x",
            "https://.fcm.googleapis.com/x",
            "https://fcm.googleapis.com../x",
            "https://fcm_googleapis.com/x",
            "https://push.apple.com/x",
            "https://notify.windows.com/x",
            "https://1.2.3.4/x",
            "HTTPS://fcm.googleapis.com/x",
            "https://fcm.googleapis.com\\@evil.example/x",
            "https://evil.example/@fcm.googleapis.com/x",
        ] {
            assert!(!endpoint_allowed(endpoint), "{endpoint:?}");
        }
    }

    #[test]
    fn host_is_normalised() {
        assert_eq!(
            allowed_host("https://FCM.googleapis.com./x").as_deref(),
            Some("fcm.googleapis.com")
        );
    }
}
