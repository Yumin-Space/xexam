//! Encryption of the provider and gateway keys (spec 3.1–3.3).
//!
//! The master key comes from `XEXAM_AI_SECRET_KEY` or the key file `<XEXAM_AI_DATA>/secret.key` (created on first start).
//! From it two values are derived: the AEAD key (`SHA-256("xexam-ai aead v1" || master)`) and the key id (the first 16 hex
//! digits of `SHA-256("xexam-ai key id v1" || master)`). Secrets are sealed with XChaCha20-Poly1305, a random 24-byte
//! nonce each, and the AAD `"xexam-ai-secret-v1:" + name`, so a value cannot be moved to another row.
//!
//! The three domain strings above are protocol constants and keep the service's former name on purpose: they must stay
//! byte-identical, because changing them changes the key id and the AAD, and every saved key would become unreadable.
//!
//! The master key, the derived key and every decrypted secret live in `Zeroizing` buffers. Nothing here is ever logged
//! except the path of the key file.
use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, URL_SAFE_NO_PAD};
use base64::engine::DecodePaddingMode;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use sha2::{Digest, Sha256};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// Name of the key file inside the AI data directory.
pub const KEY_FILE: &str = "secret.key";
/// Length of a nonce.
pub const NONCE_LEN: usize = 24;

/// The two secret slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretName {
    ApiKey,
    GatewayKey,
}

impl SecretName {
    pub const ALL: [SecretName; 2] = [SecretName::ApiKey, SecretName::GatewayKey];

    /// The row name in `secrets`.
    pub fn db(self) -> &'static str {
        match self {
            SecretName::ApiKey => "api_key",
            SecretName::GatewayKey => "gateway_key",
        }
    }

    /// The JSON name (`field` of the key errors).
    pub fn json(self) -> &'static str {
        match self {
            SecretName::ApiKey => "apiKey",
            SecretName::GatewayKey => "gatewayKey",
        }
    }

    pub fn from_db(s: &str) -> Option<SecretName> {
        match s {
            "api_key" => Some(SecretName::ApiKey),
            "gateway_key" => Some(SecretName::GatewayKey),
            _ => None,
        }
    }

    fn aad(self) -> Vec<u8> {
        format!("xexam-ai-secret-v1:{}", self.db()).into_bytes()
    }
}

/// The loaded master key, as the two derived values. No `Debug`.
pub struct MasterKey {
    aead_key: Zeroizing<[u8; 32]>,
    key_id: String,
}

/// A sealed secret.
pub struct Sealed {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

/// Why a secret could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecryptError;

impl MasterKey {
    /// Derives the AEAD key and the key id from 32 master-key bytes.
    pub fn from_master(master: &[u8; 32]) -> MasterKey {
        let mut h = Sha256::new();
        h.update(b"xexam-ai aead v1");
        h.update(master);
        let mut aead_key = Zeroizing::new([0u8; 32]);
        aead_key.copy_from_slice(&h.finalize());
        let mut h = Sha256::new();
        h.update(b"xexam-ai key id v1");
        h.update(master);
        let full = crate::util::hex(&h.finalize());
        MasterKey {
            aead_key,
            key_id: full.get(..16).unwrap_or_default().to_string(),
        }
    }

    /// The id of this key (16 hex digits), stored next to every secret it sealed.
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        // The key is always 32 bytes, so this cannot fail
        XChaCha20Poly1305::new_from_slice(self.aead_key.as_slice())
            .unwrap_or_else(|_| XChaCha20Poly1305::new(&Default::default()))
    }

    /// Seals `plain` for the slot `name` with a fresh random nonce.
    pub fn seal(&self, name: SecretName, plain: &[u8]) -> Option<Sealed> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).ok()?;
        let n = XNonce::try_from(&nonce[..]).ok()?;
        let aad = name.aad();
        let ciphertext = self
            .cipher()
            .encrypt(
                &n,
                Payload {
                    msg: plain,
                    aad: &aad,
                },
            )
            .ok()?;
        Some(Sealed { nonce, ciphertext })
    }

    /// Opens a sealed secret of the slot `name`. Never panics; any failure is a [`DecryptError`].
    pub fn open(
        &self,
        name: SecretName,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<Zeroizing<String>, DecryptError> {
        let n = XNonce::try_from(nonce).map_err(|_| DecryptError)?;
        let aad = name.aad();
        let plain = Zeroizing::new(
            self.cipher()
                .decrypt(
                    &n,
                    Payload {
                        msg: ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| DecryptError)?,
        );
        let text = std::str::from_utf8(&plain).map_err(|_| DecryptError)?;
        Ok(Zeroizing::new(text.to_string()))
    }
}

/// Where the master key came from (for the startup line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    Env,
    File(PathBuf),
    Created(PathBuf),
}

/// Why no master key could be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// `XEXAM_AI_SECRET_KEY` is set but not 32 bytes in hex or base64
    InvalidEnv,
    /// The key file exists but is not valid
    InvalidFile(PathBuf),
    /// The key file is missing and could not be created
    CannotCreate(PathBuf, ErrorKind),
}

fn lenient(alpha: &alphabet::Alphabet) -> GeneralPurpose {
    GeneralPurpose::new(
        alpha,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    )
}

fn to_32(bytes: Zeroizing<Vec<u8>>) -> Option<Zeroizing<[u8; 32]>> {
    if bytes.len() != 32 {
        return None;
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&bytes);
    Some(out)
}

/// Parses the value of `XEXAM_AI_SECRET_KEY`: 64 hex digits, or base64 / base64url (padding optional) of 32 bytes.
pub fn parse_env_key(value: &str) -> Option<Zeroizing<[u8; 32]>> {
    let v = value.trim();
    if v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()) {
        let mut out = Zeroizing::new([0u8; 32]);
        for (i, chunk) in v.as_bytes().chunks(2).enumerate() {
            let s = std::str::from_utf8(chunk).ok()?;
            *out.get_mut(i)? = u8::from_str_radix(s, 16).ok()?;
        }
        return Some(out);
    }
    for alpha in [&alphabet::STANDARD, &alphabet::URL_SAFE] {
        if let Ok(bytes) = lenient(alpha).decode(v) {
            if let Some(k) = to_32(Zeroizing::new(bytes)) {
                return Some(k);
            }
        }
    }
    None
}

/// Parses the content of the key file: 43 characters of base64url without padding, optionally one trailing newline.
pub fn parse_key_file(content: &[u8]) -> Option<Zeroizing<[u8; 32]>> {
    let text = std::str::from_utf8(content).ok()?;
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    if text.len() != 43 {
        return None;
    }
    to_32(Zeroizing::new(URL_SAFE_NO_PAD.decode(text).ok()?))
}

fn create_dir_private(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)
}

fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

/// Reads an existing key file; another process may still be writing it, so an incomplete file is re-read for a moment.
fn read_key_file(path: &Path) -> Result<Zeroizing<[u8; 32]>, LoadError> {
    let mut last = None;
    for _ in 0..50 {
        match std::fs::read(path) {
            Ok(bytes) => {
                let bytes = Zeroizing::new(bytes);
                if let Some(k) = parse_key_file(&bytes) {
                    return Ok(k);
                }
                if bytes.len() >= 43 {
                    return Err(LoadError::InvalidFile(path.to_path_buf()));
                }
                last = Some(());
            }
            Err(_) => last = Some(()),
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = last;
    Err(LoadError::InvalidFile(path.to_path_buf()))
}

fn create_key_file(path: &Path) -> Result<Option<Zeroizing<[u8; 32]>>, LoadError> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = match opts.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok(None),
        Err(e) => return Err(LoadError::CannotCreate(path.to_path_buf(), e.kind())),
    };
    let mut key = Zeroizing::new([0u8; 32]);
    if getrandom::fill(key.as_mut_slice()).is_err() {
        return Err(LoadError::CannotCreate(
            path.to_path_buf(),
            ErrorKind::Other,
        ));
    }
    let mut text = Zeroizing::new(URL_SAFE_NO_PAD.encode(key.as_slice()));
    text.push('\n');
    file.write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| LoadError::CannotCreate(path.to_path_buf(), e.kind()))?;
    if let Some(dir) = path.parent() {
        sync_dir(dir);
    }
    Ok(Some(key))
}

/// Loads the master key: `XEXAM_AI_SECRET_KEY` when set (never falls back to the file then), else the key file, which
/// is created when missing (mode 0600 on unix; the data directory 0700).
pub fn load_master(
    env: Option<&str>,
    data_dir: &Path,
) -> Result<(MasterKey, KeySource), LoadError> {
    if let Some(v) = env.filter(|v| !v.trim().is_empty()) {
        return match parse_env_key(v) {
            Some(k) => Ok((MasterKey::from_master(&k), KeySource::Env)),
            None => Err(LoadError::InvalidEnv),
        };
    }
    let path = data_dir.join(KEY_FILE);
    if path.exists() {
        let k = read_key_file(&path)?;
        return Ok((MasterKey::from_master(&k), KeySource::File(path)));
    }
    create_dir_private(data_dir).map_err(|e| LoadError::CannotCreate(path.clone(), e.kind()))?;
    match create_key_file(&path)? {
        Some(k) => Ok((MasterKey::from_master(&k), KeySource::Created(path))),
        None => {
            let k = read_key_file(&path)?;
            Ok((MasterKey::from_master(&k), KeySource::File(path)))
        }
    }
}

/// Key format rule (3.3): 1–400 characters, each printable ASCII without space.
pub fn valid_key(key: &str) -> bool {
    let n = key.len();
    (1..=400).contains(&n) && key.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// The hint shown for a saved key: `…` + the last 4 characters when the key has at least 12, else empty.
pub fn hint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() >= 12 {
        let tail: String = chars.get(chars.len() - 4..).unwrap_or(&[]).iter().collect();
        format!("…{tail}")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(b: u8) -> MasterKey {
        MasterKey::from_master(&[b; 32])
    }

    #[test]
    fn round_trip_and_aad_binding() {
        let k = key(7);
        let s = k
            .seal(SecretName::ApiKey, b"sk-test-fake-0000")
            .expect("seal");
        assert_eq!(s.ciphertext.len(), 17 + 16);
        let p = k
            .open(SecretName::ApiKey, &s.nonce, &s.ciphertext)
            .expect("open");
        assert_eq!(p.as_str(), "sk-test-fake-0000");
        // The same value presented as the other slot does not open
        assert_eq!(
            k.open(SecretName::GatewayKey, &s.nonce, &s.ciphertext)
                .map(|_| ()),
            Err(DecryptError)
        );
        // Another master key does not open it either, and nothing panics on garbage
        assert!(key(8)
            .open(SecretName::ApiKey, &s.nonce, &s.ciphertext)
            .is_err());
        assert!(k.open(SecretName::ApiKey, &[0u8; 3], b"x").is_err());
        assert!(k.open(SecretName::ApiKey, &s.nonce, b"").is_err());
        // Two seals of the same value use different nonces
        let s2 = k
            .seal(SecretName::ApiKey, b"sk-test-fake-0000")
            .expect("seal");
        assert_ne!(s.nonce, s2.nonce);
    }

    #[test]
    fn key_id_is_stable_and_differs_per_key() {
        assert_eq!(key(1).key_id(), key(1).key_id());
        assert_ne!(key(1).key_id(), key(2).key_id());
        assert!(crate::util::is_lower_hex(key(1).key_id(), 16));
    }

    #[test]
    fn env_key_formats() {
        let raw = [0x5au8; 32];
        let hex = crate::util::hex(&raw);
        assert_eq!(parse_env_key(&hex).map(|k| *k), Some(raw));
        assert_eq!(parse_env_key(&hex.to_uppercase()).map(|k| *k), Some(raw));
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        assert!(b64.ends_with('='));
        assert_eq!(parse_env_key(&b64).map(|k| *k), Some(raw));
        assert_eq!(
            parse_env_key(b64.trim_end_matches('=')).map(|k| *k),
            Some(raw)
        );
        let raw2 = [0xfbu8; 32];
        let url = URL_SAFE_NO_PAD.encode(raw2);
        assert!(url.contains('-') || url.contains('_'));
        assert_eq!(parse_env_key(&url).map(|k| *k), Some(raw2));
        assert!(parse_env_key("short").is_none());
        assert!(parse_env_key(&"zz".repeat(32)).is_none());
        assert!(parse_env_key(&URL_SAFE_NO_PAD.encode([1u8; 31])).is_none());
        assert!(parse_env_key(&URL_SAFE_NO_PAD.encode([1u8; 33])).is_none());
    }

    #[test]
    fn key_file_is_created_once_and_reused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data = dir.path().join("ai");
        let (k1, src1) = load_master(None, &data).expect("create");
        assert!(matches!(src1, KeySource::Created(_)));
        let (k2, src2) = load_master(None, &data).expect("reuse");
        assert!(matches!(src2, KeySource::File(_)));
        assert_eq!(k1.key_id(), k2.key_id());
        let content = std::fs::read_to_string(data.join(KEY_FILE)).expect("read");
        assert_eq!(content.trim_end().len(), 43);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(data.join(KEY_FILE))
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
            let dmode = std::fs::metadata(&data).expect("meta").permissions().mode();
            assert_eq!(dmode & 0o777, 0o700);
        }
        // The env variable wins and never falls back to the file
        assert!(matches!(
            load_master(Some("not a key"), &data),
            Err(LoadError::InvalidEnv)
        ));
        let hex = crate::util::hex(&[3u8; 32]);
        let (k3, src3) = load_master(Some(&hex), &data).expect("env");
        assert_eq!(src3, KeySource::Env);
        assert_ne!(k3.key_id(), k1.key_id());
    }

    #[test]
    fn malformed_key_file_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(KEY_FILE), "x".repeat(60)).expect("write");
        assert!(matches!(
            load_master(None, dir.path()),
            Err(LoadError::InvalidFile(_))
        ));
    }

    #[test]
    fn concurrent_creation_gives_one_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data = dir.path().join("race");
        let d1 = data.clone();
        let d2 = data.clone();
        let t1 =
            std::thread::spawn(move || load_master(None, &d1).map(|(k, _)| k.key_id().to_string()));
        let t2 =
            std::thread::spawn(move || load_master(None, &d2).map(|(k, _)| k.key_id().to_string()));
        let a = t1.join().expect("join").expect("key");
        let b = t2.join().expect("join").expect("key");
        assert_eq!(a, b);
    }

    #[test]
    fn key_rules_and_hints() {
        assert!(valid_key("sk-demo-0000-1234"));
        assert!(!valid_key(""));
        assert!(!valid_key("has space"));
        assert!(!valid_key("tab\there"));
        assert!(!valid_key("ключ"));
        assert!(valid_key(&"a".repeat(400)));
        assert!(!valid_key(&"a".repeat(401)));
        assert_eq!(hint("sk-demo-0000-1234"), "…1234");
        assert_eq!(hint("short-key"), "");
        assert_eq!(hint("123456789012"), "…9012");
    }
}
