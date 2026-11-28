//! VAPID key and Authorization header (RFC 8292, ES256 through `ring`), contracts C.3.

use crate::config::Config;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::net::IpAddr;
use std::path::Path;

/// The key file in the data directory, next to `xexam.db`.
pub const KEY_FILE: &str = "push-vapid.key";
/// Overrides the file; the same text format.
pub const KEY_ENV: &str = "XEXAM_PUSH_VAPID_KEY";

/// The most bytes read from a key file.
const MAX_KEY_FILE: u64 = 4096;
/// The longest Authorization header accepted (bytes).
const MAX_HEADER: usize = 1024;

/// A loaded VAPID signing key. `Debug` prints the key id only, never key material.
pub struct Vapid {
    pair: EcdsaKeyPair,
    public: [u8; 65],
}

impl std::fmt::Debug for Vapid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Vapid({})", self.key_id())
    }
}

/// Where the loaded key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Env,
    File,
    Created,
}

impl Vapid {
    /// Env value set: it alone decides (valid → used, invalid → error, the file is never consulted). Else the key file in
    /// `data_dir`: read when present (an unreadable one is an error and is never overwritten), created when missing.
    pub fn load_or_create(
        data_dir: &Path,
        env_value: Option<&str>,
    ) -> Result<(Vapid, KeySource), String> {
        if let Some(value) = env_value.filter(|v| !v.trim().is_empty()) {
            let key = Vapid::from_text(value)
                .map_err(|e| format!("push: {KEY_ENV} does not hold a valid key ({e})"))?;
            return Ok((key, KeySource::Env));
        }
        let path = data_dir.join(KEY_FILE);
        match read_key_file(&path) {
            Ok(text) => Vapid::from_text(&text)
                .map(|k| (k, KeySource::File))
                .map_err(|_| unreadable()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let (key, text) = Vapid::generate()?;
                match create_key_file(&path, &text) {
                    Ok(true) => Ok((key, KeySource::Created)),
                    // Someone else created the file in the meantime: use theirs
                    Ok(false) => {
                        let text = read_key_file(&path).map_err(|_| unreadable())?;
                        Vapid::from_text(&text)
                            .map(|k| (k, KeySource::File))
                            .map_err(|_| unreadable())
                    }
                    Err(e) => Err(format!(
                        "push: cannot write the key file {KEY_FILE}: {}",
                        e.kind()
                    )),
                }
            }
            Err(_) => Err(unreadable()),
        }
    }

    /// Base64url (no padding) of the PKCS#8 v1 DER, surrounding whitespace ignored.
    pub fn from_text(text: &str) -> Result<Vapid, String> {
        let der = URL_SAFE_NO_PAD
            .decode(text.trim())
            .map_err(|_| "the key is not base64url".to_string())?;
        let rng = SystemRandom::new();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &rng)
            .map_err(|_| "the key is not a P-256 PKCS#8 key".to_string())?;
        let public: [u8; 65] = pair
            .public_key()
            .as_ref()
            .try_into()
            .map_err(|_| "unexpected public key length".to_string())?;
        Ok(Vapid { pair, public })
    }

    /// A new key and its text form.
    pub fn generate() -> Result<(Vapid, String), String> {
        let rng = SystemRandom::new();
        let der = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|_| "cannot generate a key".to_string())?;
        let text = URL_SAFE_NO_PAD.encode(der.as_ref());
        let key = Vapid::from_text(&text)?;
        Ok((key, text))
    }

    pub fn public_key(&self) -> &[u8; 65] {
        &self.public
    }

    /// The public key as base64url without padding (87 characters).
    pub fn public_b64(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.public)
    }

    /// The first 16 lower-case hex digits of SHA-256 of the public key.
    pub fn key_id(&self) -> String {
        let digest = Sha256::digest(self.public);
        digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
    }

    /// The value of the `Authorization` header for a push service of origin `aud`:
    /// `vapid t=<jwt>, k=<public key>`. The JWT expires one hour after `now`.
    pub fn authorization(&self, aud: &str, subject: &str, now: i64) -> Result<String, String> {
        let claims = serde_json::json!({"aud": aud, "exp": now + 3600, "sub": subject}).to_string();
        let signing_input = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#),
            URL_SAFE_NO_PAD.encode(claims)
        );
        let rng = SystemRandom::new();
        let signature = self
            .pair
            .sign(&rng, signing_input.as_bytes())
            .map_err(|_| "cannot sign".to_string())?;
        let header = format!(
            "vapid t={signing_input}.{}, k={}",
            URL_SAFE_NO_PAD.encode(signature.as_ref()),
            self.public_b64()
        );
        if header.len() > MAX_HEADER || header.contains(['\r', '\n']) {
            return Err("the Authorization header is not acceptable".to_string());
        }
        Ok(header)
    }
}

fn unreadable() -> String {
    format!(
        "push: the key file {KEY_FILE} cannot be read; delete it to create a new key (devices then enable notifications again)"
    )
}

fn read_key_file(path: &Path) -> std::io::Result<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_KEY_FILE + 1)
        .read_to_end(&mut buf)?;
    if buf.len() as u64 > MAX_KEY_FILE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "key file too large",
        ));
    }
    String::from_utf8(buf)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "not UTF-8"))
}

/// Writes `value` + newline into a new file (`create_new`, mode 0600 on unix) and syncs it; a file that could not be written
/// completely is removed again.
fn write_new_file(path: &Path, value: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    if let Err(e) = file
        .write_all(value.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// Creates the key file the way the internal token file is created: a temporary file in the same directory (mode 0600), then
/// a hard link under the final name, which fails when that name exists, so a reader never sees a half-written file and two
/// creators cannot overwrite each other. Where hard links do not work the file is created in place with `create_new`.
/// `Ok(false)` when the file exists already.
fn create_key_file(path: &Path, text: &str) -> std::io::Result<bool> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    if !dir.is_dir() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir)?;
    }
    let tmp = dir.join(format!(".{KEY_FILE}.{}.tmp", crate::security::new_token()));
    write_new_file(&tmp, text)?;
    let linked = std::fs::hard_link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(_) => match write_new_file(path, text) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e),
        },
    }
}

/// `https://<host>` of an allowed endpoint (no path, no port).
pub fn audience_of(endpoint: &str) -> Option<String> {
    super::allow::allowed_host(endpoint).map(|host| format!("https://{host}"))
}

/// A host name usable in an `https:` subject: dotted, not an IP literal, not `localhost`.
fn subject_host(entry: &str) -> Option<String> {
    let rest = entry
        .strip_prefix("https://")
        .or_else(|| entry.strip_prefix("http://"))
        .unwrap_or(entry);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.starts_with('[') || authority.contains('@') {
        return None;
    }
    let host = authority
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    let valid = host.contains('.')
        && host != "localhost"
        && host.parse::<IpAddr>().is_err()
        && host
            .split('.')
            .all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    valid.then(|| host.to_string())
}

/// `(subject, derived_from_config)`: the `sub` claim derived from the configuration (an explicit `XEXAM_PUSH_SUBJECT` is the
/// caller's business): the first usable name among the sorted allowed origins and hosts becomes `https://<name>`, else the
/// `mailto:admin@localhost` default (the second value is then `false` and the caller logs the warning).
pub fn default_subject(cfg: &Config) -> (String, bool) {
    let mut entries: Vec<&String> = cfg
        .allowed_origins
        .iter()
        .chain(cfg.allowed_hosts.iter())
        .collect();
    entries.sort();
    entries.dedup();
    for entry in entries {
        if let Some(host) = subject_host(entry) {
            return (format!("https://{host}"), true);
        }
    }
    ("mailto:admin@localhost".to_string(), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};

    #[test]
    fn generate_text_round_trip() {
        let (key, text) = Vapid::generate().unwrap();
        assert!(!text.contains('=') && !text.contains('\n'));
        let again = Vapid::from_text(&format!("  {text}\n")).unwrap();
        assert_eq!(key.public_key(), again.public_key());
        assert_eq!(key.key_id(), again.key_id());
        assert_eq!(key.public_b64().len(), 87);
        assert_eq!(key.public_key()[0], 0x04);
    }

    #[test]
    fn key_id_shape_and_debug() {
        let (key, text) = Vapid::generate().unwrap();
        let id = key.key_id();
        assert_eq!(id.len(), 16);
        assert!(id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        let shown = format!("{key:?}");
        assert_eq!(shown, format!("Vapid({id})"));
        assert!(!shown.contains(&text));
        let (other, _) = Vapid::generate().unwrap();
        assert_ne!(other.key_id(), id);
    }

    #[test]
    fn bad_texts_are_refused() {
        for text in ["", "  ", "not base64 !!", "AAAA", "x".repeat(5000).as_str()] {
            assert!(Vapid::from_text(text).is_err(), "{text:.20}");
        }
    }

    #[test]
    fn authorization_verifies_and_has_the_exact_json() {
        let (key, _) = Vapid::generate().unwrap();
        let header = key
            .authorization(
                "https://fcm.googleapis.com",
                "mailto:admin@xexam.example.com",
                1_800_000_000,
            )
            .unwrap();
        assert!(header.len() <= 1024);
        assert!(!header.contains('\r') && !header.contains('\n'));
        let rest = header.strip_prefix("vapid t=").unwrap();
        let (jwt, k) = rest.split_once(", k=").unwrap();
        assert_eq!(k, key.public_b64());
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let head = URL_SAFE_NO_PAD.decode(parts[0]).unwrap();
        assert_eq!(head, br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = URL_SAFE_NO_PAD.decode(parts[1]).unwrap();
        assert_eq!(
            String::from_utf8(claims).unwrap(),
            r#"{"aud":"https://fcm.googleapis.com","exp":1800003600,"sub":"mailto:admin@xexam.example.com"}"#
        );
        let sig = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
        assert_eq!(sig.len(), 64);
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, key.public_key())
            .verify(signing_input.as_bytes(), &sig)
            .expect("signature verifies");
        // A tampered input does not verify
        assert!(
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, key.public_key())
                .verify(b"other", &sig)
                .is_err()
        );
    }

    #[test]
    fn oversized_subject_is_refused() {
        let (key, _) = Vapid::generate().unwrap();
        let long = format!("https://{}", "a".repeat(1100));
        assert!(key
            .authorization("https://fcm.googleapis.com", &long, 1)
            .is_err());
        // CR / LF in a claim is base64url encoded, so the header itself never contains one
        let h = key
            .authorization("https://fcm.googleapis.com", "mailto:a\r\nb@x.y", 1)
            .unwrap();
        assert!(!h.contains(['\r', '\n']));
    }

    #[test]
    fn creates_then_reuses_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (key, source) = Vapid::load_or_create(dir.path(), None).unwrap();
        assert_eq!(source, KeySource::Created);
        let path = dir.path().join(KEY_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with('\n') && text.trim().len() > 60);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let (again, source) = Vapid::load_or_create(dir.path(), None).unwrap();
        assert_eq!(source, KeySource::File);
        assert_eq!(again.key_id(), key.key_id());
        // No temporary file is left behind
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![KEY_FILE.to_string()]);
    }

    #[test]
    fn creates_a_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        let (_, source) = Vapid::load_or_create(&nested, None).unwrap();
        assert_eq!(source, KeySource::Created);
        assert!(nested.join(KEY_FILE).is_file());
    }

    #[test]
    fn a_broken_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(KEY_FILE);
        std::fs::write(&path, "garbage").unwrap();
        let err = Vapid::load_or_create(dir.path(), None).unwrap_err();
        assert_eq!(
            err,
            "push: the key file push-vapid.key cannot be read; delete it to create a new key (devices then enable notifications again)"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "garbage");
        // A file of more than 4096 bytes is treated the same way
        std::fs::write(&path, "A".repeat(5000)).unwrap();
        assert!(Vapid::load_or_create(dir.path(), None).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap().len(), 5000);
    }

    #[test]
    fn env_value_wins_and_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (env_key, text) = Vapid::generate().unwrap();
        let (key, source) = Vapid::load_or_create(dir.path(), Some(&text)).unwrap();
        assert_eq!(source, KeySource::Env);
        assert_eq!(key.key_id(), env_key.key_id());
        assert!(!dir.path().join(KEY_FILE).exists());
        // Empty / blank env value counts as unset
        let (_, source) = Vapid::load_or_create(dir.path(), Some("  ")).unwrap();
        assert_eq!(source, KeySource::Created);
    }

    #[test]
    fn invalid_env_value_never_falls_back_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (file_key, _) = Vapid::load_or_create(dir.path(), None).unwrap();
        let err = Vapid::load_or_create(dir.path(), Some("definitely not a key")).unwrap_err();
        assert!(err.contains(KEY_ENV));
        assert!(!err.contains("definitely"));
        let (again, _) = Vapid::load_or_create(dir.path(), None).unwrap();
        assert_eq!(again.key_id(), file_key.key_id());
    }

    #[test]
    fn audience_of_allowed_endpoints() {
        assert_eq!(
            audience_of("https://FCM.googleapis.com:443/fcm/send/abc").as_deref(),
            Some("https://fcm.googleapis.com")
        );
        assert_eq!(
            audience_of("https://wns2-par02p.notify.windows.com/w/?token=abc").as_deref(),
            Some("https://wns2-par02p.notify.windows.com")
        );
        assert_eq!(audience_of("https://evil.example/x"), None);
        assert_eq!(audience_of("http://fcm.googleapis.com/x"), None);
    }

    fn cfg_with(origins: &[&str], hosts: &[&str]) -> Config {
        let mut cfg = Config::from_env_with(&|_| None, "3.1.0".to_string());
        cfg.allowed_origins = origins.iter().map(|s| s.to_string()).collect();
        cfg.allowed_hosts = hosts.iter().map(|s| s.to_string()).collect();
        cfg
    }

    #[test]
    fn subject_derivation() {
        let local = cfg_with(&[], &["localhost", "127.0.0.1", "::1"]);
        assert_eq!(
            default_subject(&local),
            ("mailto:admin@localhost".to_string(), false)
        );
        let named = cfg_with(
            &["http://localhost:8000", "https://xexam.example.com:8443"],
            &["localhost", "xexam.example.com", "10.0.0.5"],
        );
        assert_eq!(
            default_subject(&named),
            ("https://xexam.example.com".to_string(), true)
        );
        let only_host = cfg_with(&[], &["localhost", "192.168.1.2", "quiz.example.com"]);
        assert_eq!(
            default_subject(&only_host),
            ("https://quiz.example.com".to_string(), true)
        );
        // Sorted: the first usable name wins
        let two = cfg_with(&[], &["b.example.com", "a.example.com"]);
        assert_eq!(default_subject(&two).0, "https://a.example.com");
        let v6 = cfg_with(&["http://[::1]:8000"], &[]);
        assert_eq!(
            default_subject(&v6),
            ("mailto:admin@localhost".to_string(), false)
        );
    }
}
