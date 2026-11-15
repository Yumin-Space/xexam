//! The URL template and the key header of a lookup provider (spec 5.1 / 5.2): validation of a draft and building of the
//! request for one address.
use super::mapping::{self, Mapping};
use crate::codes::ApiError;
use reqwest::header::{HeaderName, HeaderValue};
use std::net::IpAddr;

/// Headers the key header may not be (lower case).
const FORBIDDEN_HEADERS: [&str; 12] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "cookie",
    "accept",
    "user-agent",
    "upgrade",
    "te",
    "trailer",
    "proxy-authorization",
    "keep-alive",
];

/// The address and the key that stand in for `{ip}` / `{key}` when only the shape of a template is checked.
const DUMMY_IP: &str = "203.0.113.5";
const DUMMY_KEY: &str = "k";

/// How a provider is asked and read: the URL template, the optional key header and the JSON mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCfg {
    pub url_template: String,
    /// "" = the key is not sent in a header
    pub key_header_name: String,
    /// Contains `{key}` when the name is set
    pub key_header_value: String,
    pub mapping: Mapping,
}

/// The request for one address: the filled URL and the key header (a sensitive value), if any.
pub struct Built {
    pub url: reqwest::Url,
    pub header: Option<(HeaderName, HeaderValue)>,
}

/// A request that cannot be built (a template that is not valid any more; saved rows are validated, so this is a safety
/// net only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildError;

/// Every `{…}` in `s` is one of `allowed` (exactly `{name}`), and no `}` stands alone.
fn placeholders_ok(s: &str, allowed: &[&str]) -> bool {
    let mut rest = s;
    while let Some(pos) = rest.find(['{', '}']) {
        let tail = rest.get(pos..).unwrap_or("");
        if !tail.starts_with('{') {
            return false;
        }
        let Some(end) = tail.find('}') else {
            return false;
        };
        let name = tail.get(1..end).unwrap_or("");
        if !allowed.contains(&name) {
            return false;
        }
        rest = tail.get(end + 1..).unwrap_or("");
    }
    true
}

/// The text between `://` and the first `/`, `?` or `#`.
fn authority(t: &str) -> &str {
    let after = t.split_once("://").map_or("", |(_, r)| r);
    after.split(['/', '?', '#']).next().unwrap_or("")
}

/// The template as a URL with `ip` and `key` filled in; `None` when it does not parse or has no host.
fn parse_filled(template: &str, ip: &str, key: &str) -> Option<reqwest::Url> {
    let filled = template.replace("{ip}", ip).replace("{key}", key);
    let url = reqwest::Url::parse(&filled).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(())?;
    url.host_str().filter(|h| !h.is_empty())?;
    Some(url)
}

/// Rule `urlTemplate` of 5.1.
pub fn validate_url(t: &str) -> Result<(), ApiError> {
    let bad = ApiError::geo_bad_url;
    if t.is_empty() || t.chars().count() > 500 {
        return Err(bad());
    }
    if t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad());
    }
    if !(t.starts_with("http://") || t.starts_with("https://")) {
        return Err(bad());
    }
    if !t.contains("{ip}") || !placeholders_ok(t, &["ip", "key"]) {
        return Err(bad());
    }
    let auth = authority(t);
    if auth.is_empty()
        || auth.contains('{')
        || auth.contains('}')
        || auth.contains('@')
        || t.contains('#')
    {
        return Err(bad());
    }
    if parse_filled(t, DUMMY_IP, DUMMY_KEY).is_none() {
        return Err(bad());
    }
    Ok(())
}

/// Rules `keyHeaderName` and `keyHeaderValue` of 5.1.
pub fn validate_header(name: &str, value: &str) -> Result<(), ApiError> {
    if !name.is_empty() {
        let lower = name.to_ascii_lowercase();
        if name.len() > 64
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || FORBIDDEN_HEADERS.contains(&lower.as_str())
        {
            return Err(ApiError::geo_bad_header("keyHeaderName"));
        }
    }
    let bad = || ApiError::geo_bad_header("keyHeaderValue");
    if name.is_empty() {
        return if value.is_empty() { Ok(()) } else { Err(bad()) };
    }
    if value.is_empty()
        || value.len() > 500
        || !value.bytes().all(|b| (0x20..=0x7e).contains(&b))
        || !value.contains("{key}")
        || !placeholders_ok(value, &["key"])
    {
        return Err(bad());
    }
    Ok(())
}

/// The static name of a mapping field for the `field` of an error.
fn mapping_field(name: &str) -> &'static str {
    match name {
        "country" => "mapping.country",
        "countryCode" => "mapping.countryCode",
        "region" => "mapping.region",
        "city" => "mapping.city",
        "okPath" => "mapping.okPath",
        "okValue" => "mapping.okValue",
        "errorPath" => "mapping.errorPath",
        _ => "mapping.messagePath",
    }
}

/// Rule `mapping.*` of 5.1.
pub fn validate_mapping(m: &Mapping) -> Result<(), ApiError> {
    for name in mapping::FIELD_NAMES {
        if name == "okValue" {
            continue;
        }
        let v = m.get(name).unwrap_or_default();
        if !v.is_empty() && !mapping::valid_path(v) {
            return Err(ApiError::geo_bad_mapping(mapping_field(name)));
        }
    }
    if m.country.is_empty() && m.country_code.is_empty() && m.region.is_empty() && m.city.is_empty()
    {
        return Err(ApiError::geo_bad_mapping("mapping.country"));
    }
    if m.ok_value.chars().count() > 64 || m.ok_value.chars().any(char::is_control) {
        return Err(ApiError::geo_bad_mapping("mapping.okValue"));
    }
    if !m.ok_path.is_empty() && m.ok_value.is_empty() {
        return Err(ApiError::geo_bad_mapping("mapping.okValue"));
    }
    Ok(())
}

/// The template without the query parameters whose value is exactly `{key}` (used when there is no key).
fn strip_key_params(t: &str) -> String {
    let Some((base, query)) = t.split_once('?') else {
        return t.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| p.split_once('=').map(|(_, v)| v) != Some("{key}"))
        .collect();
    if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

/// `true` when the provider cannot be asked without a key: `{key}` stays in the template after the whole-value query
/// parameters are dropped (the path, or inside a longer value), or in the header value (5.1 `keyRequired`).
pub fn needs_key(t: &str, header_name: &str, header_value: &str) -> bool {
    strip_key_params(t).contains("{key}")
        || (!header_name.is_empty() && header_value.contains("{key}"))
}

/// The host of a (valid) template, for events and the settings view.
pub fn host_of(t: &str) -> String {
    parse_filled(t, DUMMY_IP, DUMMY_KEY)
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// The origin (scheme, host, port) of a template; `None` when it does not parse. A saved key must not follow a change.
pub fn origin(t: &str) -> Option<(String, String, Option<u16>)> {
    let u = parse_filled(t, DUMMY_IP, DUMMY_KEY)?;
    Some((
        u.scheme().to_string(),
        u.host_str()?.to_string(),
        u.port_or_known_default(),
    ))
}

/// `true` for a template that starts with `http://` (the visitor IP travels unencrypted).
pub fn is_insecure_http(t: &str) -> bool {
    t.starts_with("http://")
}

/// Percent-encodes everything but `A-Z a-z 0-9 - . _ ~`.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Builds the request for `ip` (5.2): `{ip}` filled, the key percent-encoded into the URL or the key parameters dropped,
/// the key header added (sensitive) only with a key, and the scheme, host and port checked against the template's.
pub fn build(cfg: &ProviderCfg, ip: IpAddr, key: Option<&str>) -> Result<Built, BuildError> {
    let key = key.filter(|k| !k.is_empty());
    let template = if key.is_some() {
        cfg.url_template.clone()
    } else {
        strip_key_params(&cfg.url_template)
    };
    if key.is_none() && template.contains("{key}") {
        return Err(BuildError);
    }
    let encoded = key.map(percent_encode).unwrap_or_default();
    let url = parse_filled(&template, &ip.to_string(), &encoded).ok_or(BuildError)?;
    let shape = parse_filled(&cfg.url_template, DUMMY_IP, DUMMY_KEY).ok_or(BuildError)?;
    if url.scheme() != shape.scheme()
        || url.host_str() != shape.host_str()
        || url.port() != shape.port()
    {
        return Err(BuildError);
    }
    let header = match key {
        Some(k) if !cfg.key_header_name.is_empty() => {
            let name =
                HeaderName::from_bytes(cfg.key_header_name.as_bytes()).map_err(|_| BuildError)?;
            let raw = cfg.key_header_value.replace("{key}", k);
            let mut value = HeaderValue::from_str(&raw).map_err(|_| BuildError)?;
            value.set_sensitive(true);
            Some((name, value))
        }
        _ => None,
    };
    Ok(Built { url, header })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(url: &str) -> ProviderCfg {
        ProviderCfg {
            url_template: url.to_string(),
            key_header_name: String::new(),
            key_header_value: String::new(),
            mapping: Mapping {
                city: "city".into(),
                ..Mapping::default()
            },
        }
    }

    fn url_ok(t: &str) -> bool {
        validate_url(t).is_ok()
    }

    #[test]
    fn url_rules_both_ways() {
        for good in [
            "https://provider.example/{ip}/json?token={key}",
            "http://provider.example/json/{ip}",
            "https://provider.example:8443/lookup/{ip}",
            "https://provider.example/{ip}/{ip}",
            "https://provider.example/lookup?ip={ip}&lang=en",
            "https://provider.example?ip={ip}",
            "http://127.0.0.1:9000/{ip}",
            "https://provider.example/{key}/{ip}",
            "https://[2001:db8::1]/{ip}",
        ] {
            assert!(url_ok(good), "{good}");
        }
        for (bad, why) in [
            ("", "empty"),
            ("provider.example/{ip}", "no scheme"),
            ("ftp://provider.example/{ip}", "other scheme"),
            ("HTTPS://provider.example/{ip}", "upper-case scheme"),
            ("https://provider.example/json", "no {ip}"),
            ("https://provider.example/{ip", "open brace"),
            ("https://provider.example/ip}", "stray close brace"),
            ("https://provider.example/{IP}", "other placeholder case"),
            ("https://provider.example/{ip}/{lang}", "other placeholder"),
            ("https://provider.example/{ip}/{}", "empty placeholder"),
            ("https://provider.example/{ip}/{{key}}", "nested braces"),
            ("https://{ip}.provider.example/", "placeholder in the host"),
            (
                "https://provider.example:{key}/{ip}",
                "placeholder in the port",
            ),
            ("{ip}://provider.example/x", "placeholder in the scheme"),
            ("https://user@provider.example/{ip}", "user info"),
            (
                "https://user:pw@provider.example/{ip}",
                "user info with password",
            ),
            (
                "https://provider.example@/{ip}",
                "empty host after user info",
            ),
            ("https://provider.example/{ip}#frag", "fragment"),
            ("https://provider.example/{ip} x", "space"),
            ("https://provider.example/{ip}\n", "newline"),
            ("https://provider.example/{ip}\t", "tab"),
            ("https:///{ip}", "no host"),
            ("https://", "nothing after the scheme"),
        ] {
            assert!(!url_ok(bad), "{bad:?} ({why})");
        }
        // length: 500 characters pass, 501 do not
        let pad = |n: usize| format!("https://provider.example/{{ip}}/{}", "a".repeat(n));
        let base = pad(0).len();
        assert!(url_ok(&pad(500 - base)));
        assert!(!url_ok(&pad(501 - base)));
    }

    #[test]
    fn header_rules_both_ways() {
        assert!(validate_header("", "").is_ok());
        assert!(validate_header("Authorization", "Bearer {key}").is_ok());
        assert!(validate_header("X-Api-Key", "{key}").is_ok());
        assert!(validate_header("X-Token-2", "key={key}; {key}").is_ok());
        let name = |e: ApiError| e.field;
        // a value without a name
        assert_eq!(
            name(validate_header("", "x").expect_err("value only")),
            Some("keyHeaderValue")
        );
        assert_eq!(
            name(validate_header("", "{key}").expect_err("value only")),
            Some("keyHeaderValue")
        );
        // names
        for bad in [
            "a b",
            "a_b",
            "a:b",
            "é",
            "Host",
            "content-length",
            "Transfer-Encoding",
            "CONNECTION",
            "Cookie",
            "accept",
            "User-Agent",
            "upgrade",
            "TE",
            "trailer",
            "Proxy-Authorization",
            "keep-alive",
        ] {
            assert_eq!(
                name(validate_header(bad, "{key}").expect_err(bad)),
                Some("keyHeaderName"),
                "{bad}"
            );
        }
        assert!(validate_header(&"a".repeat(64), "{key}").is_ok());
        assert!(validate_header(&"a".repeat(65), "{key}").is_err());
        // values
        for bad in [
            "",
            "Bearer",
            "{ip}",
            "{key} {other}",
            "{key",
            "}{key}",
            "tab\t{key}",
            "é{key}",
            "line\n{key}",
        ] {
            assert_eq!(
                name(validate_header("Authorization", bad).expect_err(bad)),
                Some("keyHeaderValue"),
                "{bad:?}"
            );
        }
        let pad = |n: usize| format!("{}{{key}}", "a".repeat(n));
        assert!(validate_header("X", &pad(500 - 5)).is_ok());
        assert!(validate_header("X", &pad(501 - 5)).is_err());
    }

    #[test]
    fn mapping_rules_both_ways() {
        let ok = |f: &dyn Fn(&mut Mapping)| {
            let mut m = Mapping {
                city: "city".into(),
                ..Mapping::default()
            };
            f(&mut m);
            validate_mapping(&m)
        };
        assert!(ok(&|_| {}).is_ok());
        assert!(ok(&|m| m.country_code = "loc.country".into()).is_ok());
        assert!(ok(&|m| {
            m.ok_path = "status".into();
            m.ok_value = "success".into();
        })
        .is_ok());
        // none of the four place fields
        let e = validate_mapping(&Mapping::default()).expect_err("empty");
        assert_eq!(e.code, "ai.geo.badMapping");
        let e = validate_mapping(&Mapping {
            ok_path: "a".into(),
            ok_value: "b".into(),
            ..Mapping::default()
        })
        .expect_err("no place field");
        assert_eq!(e.code, "ai.geo.badMapping");
        // bad paths name their field
        type Change = fn(&mut Mapping);
        let cases: [(&str, Change); 10] = [
            ("mapping.country", |m| m.country = "a b".into()),
            ("mapping.countryCode", |m| m.country_code = "a..b".into()),
            ("mapping.region", |m| m.region = "a.".into()),
            ("mapping.city", |m| m.city = ".a".into()),
            ("mapping.okPath", |m| {
                m.ok_path = "a[0]".into();
                m.ok_value = "x".into();
            }),
            ("mapping.errorPath", |m| m.error_path = "a/b".into()),
            ("mapping.messagePath", |m| m.message_path = "é".into()),
            ("mapping.okValue", |m| m.ok_path = "status".into()),
            ("mapping.okValue", |m| m.ok_value = "x".repeat(65)),
            ("mapping.okValue", |m| {
                m.ok_value = "a
b"
                .into()
            }),
        ];
        for (field, set) in cases {
            let e = ok(&set).expect_err(field);
            assert_eq!(e.field, Some(field));
        }
        assert!(ok(&|m| m.city = "a.b.c.d.e.f.g.h.i".into()).is_err());
        assert!(ok(&|m| {
            m.ok_path = "s".into();
            m.ok_value = "x".repeat(64);
        })
        .is_ok());
    }

    #[test]
    fn filling_ip_v4_and_v6_and_encoding_the_key() {
        let c = cfg("https://provider.example/{ip}/json?token={key}");
        let b = build(&c, "203.0.113.5".parse().expect("ip"), Some("a b/c+d&e=f")).expect("built");
        assert_eq!(
            b.url.as_str(),
            "https://provider.example/203.0.113.5/json?token=a%20b%2Fc%2Bd%26e%3Df"
        );
        let b = build(&c, "2606:4700:4700::1111".parse().expect("ip"), Some("k")).expect("built");
        assert_eq!(
            b.url.as_str(),
            "https://provider.example/2606:4700:4700::1111/json?token=k"
        );
        // unreserved characters stay
        assert_eq!(percent_encode("Az09-._~"), "Az09-._~");
        assert_eq!(percent_encode("é"), "%C3%A9");
        // {ip} twice and {key} in the path
        let c = cfg("https://provider.example/{key}/{ip}?again={ip}");
        let b = build(&c, "192.0.2.1".parse().expect("ip"), Some("K1")).expect("built");
        assert_eq!(
            b.url.as_str(),
            "https://provider.example/K1/192.0.2.1?again=192.0.2.1"
        );
    }

    #[test]
    fn without_a_key_the_whole_value_key_parameters_are_dropped() {
        let ip: IpAddr = "203.0.113.5".parse().expect("ip");
        for (tmpl, want) in [
            (
                "https://p.example/{ip}?token={key}",
                "https://p.example/203.0.113.5",
            ),
            (
                "https://p.example/{ip}?token={key}&lang=en",
                "https://p.example/203.0.113.5?lang=en",
            ),
            (
                "https://p.example/{ip}?lang=en&token={key}&fmt=json",
                "https://p.example/203.0.113.5?lang=en&fmt=json",
            ),
            (
                "https://p.example/{ip}?lang=en&token={key}",
                "https://p.example/203.0.113.5?lang=en",
            ),
            (
                "https://p.example/json/{ip}/?key={key}",
                "https://p.example/json/203.0.113.5/",
            ),
            (
                "https://p.example/{ip}?a={key}&b={key}",
                "https://p.example/203.0.113.5",
            ),
        ] {
            let b = build(&cfg(tmpl), ip, None).expect(tmpl);
            assert_eq!(b.url.as_str(), want, "{tmpl}");
            assert!(!needs_key(tmpl, "", ""), "{tmpl}");
        }
        // the same templates with a key keep the parameter
        let b = build(
            &cfg("https://p.example/{ip}?lang=en&token={key}"),
            ip,
            Some("K"),
        )
        .expect("b");
        assert_eq!(
            b.url.as_str(),
            "https://p.example/203.0.113.5?lang=en&token=K"
        );
    }

    #[test]
    fn a_key_is_required_for_the_path_the_header_and_embedded_values() {
        assert!(needs_key("https://p.example/{key}/{ip}", "", ""));
        assert!(needs_key("https://p.example/{ip}?token=a{key}", "", ""));
        assert!(needs_key("https://p.example/{ip}?{key}", "", ""));
        assert!(needs_key(
            "https://p.example/{ip}",
            "Authorization",
            "Bearer {key}"
        ));
        assert!(!needs_key("https://p.example/{ip}?token={key}", "", ""));
        assert!(!needs_key("https://p.example/{ip}", "", ""));
        // building without a key fails when one is needed (safety net)
        let ip: IpAddr = "203.0.113.5".parse().expect("ip");
        assert_eq!(
            build(&cfg("https://p.example/{key}/{ip}"), ip, None).err(),
            Some(BuildError)
        );
    }

    #[test]
    fn key_placement_url_versus_header() {
        let ip: IpAddr = "203.0.113.5".parse().expect("ip");
        let mut c = cfg("https://p.example/{ip}");
        c.key_header_name = "Authorization".into();
        c.key_header_value = "Bearer {key}".into();
        // with a key: the header, never in the URL
        let b = build(&c, ip, Some("TESTKEY-geo-0123456789abcdef")).expect("built");
        assert_eq!(b.url.as_str(), "https://p.example/203.0.113.5");
        let (name, value) = b.header.expect("header");
        assert_eq!(name.as_str(), "authorization");
        assert_eq!(
            value.to_str().expect("text"),
            "Bearer TESTKEY-geo-0123456789abcdef"
        );
        assert!(value.is_sensitive());
        assert!(!format!("{value:?}").contains("TESTKEY"));
        // without a key: no header
        let b = build(&c, ip, None).expect("built");
        assert!(b.header.is_none());
        // an empty key counts as none
        let b = build(&c, ip, Some("")).expect("built");
        assert!(b.header.is_none());
        // both: the key in the URL and in the header
        c.url_template = "https://p.example/{ip}?token={key}".into();
        let b = build(&c, ip, Some("K")).expect("built");
        assert_eq!(b.url.as_str(), "https://p.example/203.0.113.5?token=K");
        assert!(b.header.is_some());
        // a header name without a header: nothing is sent
        c.key_header_name.clear();
        c.key_header_value.clear();
        assert!(build(&c, ip, Some("K")).expect("built").header.is_none());
    }

    #[test]
    fn scheme_host_and_port_are_kept() {
        let ip: IpAddr = "198.51.100.7".parse().expect("ip");
        for (tmpl, host, port, scheme) in [
            (
                "http://p.example:8080/{ip}",
                "p.example",
                Some(8080),
                "http",
            ),
            ("https://p.example/{ip}", "p.example", None, "https"),
            ("http://127.0.0.1:9/{ip}", "127.0.0.1", Some(9), "http"),
        ] {
            let b = build(&cfg(tmpl), ip, None).expect(tmpl);
            assert_eq!(b.url.scheme(), scheme);
            assert_eq!(b.url.host_str(), Some(host));
            assert_eq!(b.url.port(), port);
            assert_eq!(host_of(tmpl), host);
        }
        assert!(is_insecure_http("http://p.example/{ip}"));
        assert!(!is_insecure_http("https://p.example/{ip}"));
    }
}
