//! RFC 8291 `aes128gcm` message encryption (`ring` HKDF and AES-GCM), contracts C.4. One record per message.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_128_GCM};
use ring::agreement::{agree_ephemeral, EphemeralPrivateKey, UnparsedPublicKey, ECDH_P256};
use ring::hkdf::{KeyType, Salt, HKDF_SHA256};
use ring::rand::{SecureRandom, SystemRandom};

/// The record size written into the header.
pub const RECORD_SIZE: u32 = 4096;
/// The longest plaintext accepted (bytes).
pub const MAX_PLAINTEXT: usize = 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum EceError {
    BadKey,
    TooLong,
    Crypto,
}

/// A HKDF output length.
struct Len(usize);

impl KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

/// HKDF-Extract(salt, secret) then HKDF-Expand(info) into `out`.
fn hkdf(salt: &[u8], secret: &[u8], info: &[u8], out: &mut [u8]) -> Result<(), EceError> {
    Salt::new(HKDF_SHA256, salt)
        .extract(secret)
        .expand(&[info], Len(out.len()))
        .map_err(|_| EceError::Crypto)?
        .fill(out)
        .map_err(|_| EceError::Crypto)
}

/// Whether `ua_public` is 65 bytes, starts with `0x04` and is accepted by a test key agreement.
pub fn check_public_key(ua_public: &[u8]) -> bool {
    if ua_public.len() != 65 || ua_public[0] != 0x04 {
        return false;
    }
    let rng = SystemRandom::new();
    let Ok(private) = EphemeralPrivateKey::generate(&ECDH_P256, &rng) else {
        return false;
    };
    agree_ephemeral(
        private,
        &UnparsedPublicKey::new(&ECDH_P256, ua_public),
        |_| (),
    )
    .is_ok()
}

/// Encrypts `plaintext` for the user agent with a fresh ephemeral key and salt.
pub fn encrypt(
    ua_public: &[u8; 65],
    auth: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, EceError> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(EceError::TooLong);
    }
    let rng = SystemRandom::new();
    let private = EphemeralPrivateKey::generate(&ECDH_P256, &rng).map_err(|_| EceError::Crypto)?;
    let mut as_public = [0u8; 65];
    as_public.copy_from_slice(
        private
            .compute_public_key()
            .map_err(|_| EceError::Crypto)?
            .as_ref(),
    );
    if ua_public[0] != 0x04 {
        return Err(EceError::BadKey);
    }
    // ring rejects a point that is not on the curve
    let secret = agree_ephemeral(
        private,
        &UnparsedPublicKey::new(&ECDH_P256, ua_public),
        |s| <[u8; 32]>::try_from(s).ok(),
    )
    .map_err(|_| EceError::BadKey)?
    .ok_or(EceError::Crypto)?;
    let mut salt = [0u8; 16];
    rng.fill(&mut salt).map_err(|_| EceError::Crypto)?;
    encrypt_with(&secret, &as_public, ua_public, auth, &salt, plaintext)
}

/// The deterministic core of [`encrypt`] (for the RFC 8291 test vector).
pub fn encrypt_with(
    ecdh_secret: &[u8; 32],
    as_public: &[u8; 65],
    ua_public: &[u8; 65],
    auth: &[u8; 16],
    salt: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, EceError> {
    if plaintext.len() > MAX_PLAINTEXT {
        return Err(EceError::TooLong);
    }
    if ua_public[0] != 0x04 || as_public[0] != 0x04 {
        return Err(EceError::BadKey);
    }
    // IKM = HKDF(auth, ecdh_secret, "WebPush: info\0" || ua_public || as_public, 32)
    let mut key_info = Vec::with_capacity(14 + 65 + 65);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    hkdf(auth, ecdh_secret, &key_info, &mut ikm)?;
    let mut cek = [0u8; 16];
    hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", &mut cek)?;
    let mut nonce = [0u8; 12];
    hkdf(salt, &ikm, b"Content-Encoding: nonce\0", &mut nonce)?;

    // One record: plaintext || 0x02 (the last-record delimiter), sealed
    let mut record = Vec::with_capacity(plaintext.len() + 1 + 16);
    record.extend_from_slice(plaintext);
    record.push(0x02);
    let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &cek).map_err(|_| EceError::Crypto)?);
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::empty(),
        &mut record,
    )
    .map_err(|_| EceError::Crypto)?;

    // Header: salt || record size (u32 BE) || key id length || key id (the sender's public key)
    let mut body = Vec::with_capacity(16 + 4 + 1 + 65 + record.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(65);
    body.extend_from_slice(as_public);
    body.extend_from_slice(&record);
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    fn b64(s: &str) -> Vec<u8> {
        URL_SAFE_NO_PAD.decode(s).expect("base64url")
    }

    fn arr<const N: usize>(s: &str) -> [u8; N] {
        b64(s).try_into().expect("length")
    }

    fn new_agent() -> (EphemeralPrivateKey, [u8; 65]) {
        let rng = SystemRandom::new();
        let private = EphemeralPrivateKey::generate(&ECDH_P256, &rng).expect("key");
        let public: [u8; 65] = private
            .compute_public_key()
            .expect("public")
            .as_ref()
            .try_into()
            .expect("65 bytes");
        (private, public)
    }

    #[test]
    fn rfc8291_appendix_a_vector() {
        let ecdh: [u8; 32] = arr("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs");
        let as_public: [u8; 65] = arr(
            "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
        );
        let ua_public: [u8; 65] = arr(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let auth: [u8; 16] = arr("BTBZMqHH6r4Tts7J_aSIgg");
        let salt: [u8; 16] = arr("DGv6ra1nlYgDCS1FRnbzlw");
        let body = encrypt_with(
            &ecdh,
            &as_public,
            &ua_public,
            &auth,
            &salt,
            b"When I grow up, I want to be a watermelon",
        )
        .expect("encrypts");
        let expected = b64(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN",
        );
        assert_eq!(body, expected);
    }

    /// Decrypts a body the way a user agent does (same HKDF steps; the agent's private key agrees with the header key).
    fn decrypt(
        ua_private: EphemeralPrivateKey,
        ua_public: &[u8; 65],
        auth: &[u8; 16],
        body: &[u8],
    ) -> Vec<u8> {
        let salt = &body[..16];
        assert_eq!(&body[16..20], &RECORD_SIZE.to_be_bytes());
        assert_eq!(body[20], 65);
        let as_public: [u8; 65] = body[21..86].try_into().expect("key id");
        let secret = agree_ephemeral(
            ua_private,
            &UnparsedPublicKey::new(&ECDH_P256, &as_public),
            |s| s.to_vec(),
        )
        .expect("agreement");
        let mut key_info = b"WebPush: info\0".to_vec();
        key_info.extend_from_slice(ua_public);
        key_info.extend_from_slice(&as_public);
        let mut ikm = [0u8; 32];
        hkdf(auth, &secret, &key_info, &mut ikm).expect("ikm");
        let mut cek = [0u8; 16];
        hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", &mut cek).expect("cek");
        let mut nonce = [0u8; 12];
        hkdf(salt, &ikm, b"Content-Encoding: nonce\0", &mut nonce).expect("nonce");
        let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &cek).expect("key"));
        let mut record = body[86..].to_vec();
        let plain = key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut record,
            )
            .expect("opens");
        let mut plain = plain.to_vec();
        assert_eq!(plain.pop(), Some(0x02));
        plain
    }

    #[test]
    fn round_trip_with_a_second_ephemeral_key() {
        let (ua_private, ua_public) = new_agent();
        let mut auth = [0u8; 16];
        SystemRandom::new().fill(&mut auth).expect("rng");
        assert!(check_public_key(&ua_public));
        let message = r#"{"v":1,"title":"Xexam","body":"hello","tag":"test","url":"/#/settings","lang":"en"}"#;
        let body = encrypt(&ua_public, &auth, message.as_bytes()).expect("encrypts");
        assert_eq!(body.len(), 86 + message.len() + 1 + 16);
        assert_eq!(
            decrypt(ua_private, &ua_public, &auth, &body),
            message.as_bytes()
        );
    }

    #[test]
    fn two_encryptions_differ() {
        let (_private, ua_public) = new_agent();
        let auth = [7u8; 16];
        let a = encrypt(&ua_public, &auth, b"x").expect("a");
        let b = encrypt(&ua_public, &auth, b"x").expect("b");
        assert_ne!(a, b);
    }

    #[test]
    fn length_limits_and_empty_plaintext() {
        let (_private, ua_public) = new_agent();
        let auth = [1u8; 16];
        assert_eq!(
            encrypt(&ua_public, &auth, &vec![b'a'; MAX_PLAINTEXT + 1]),
            Err(EceError::TooLong)
        );
        let full = encrypt(&ua_public, &auth, &vec![b'a'; MAX_PLAINTEXT]).expect("max");
        assert_eq!(full.len(), 86 + MAX_PLAINTEXT + 1 + 16);
        assert!(full.len() <= RECORD_SIZE as usize);
        let empty = encrypt(&ua_public, &auth, b"").expect("empty");
        assert_eq!(empty.len(), 86 + 1 + 16);
    }

    #[test]
    fn bad_public_keys_are_refused() {
        assert!(!check_public_key(&[]));
        assert!(!check_public_key(&[4u8; 64]));
        let mut wrong_prefix = [0u8; 65];
        wrong_prefix[0] = 0x02;
        assert!(!check_public_key(&wrong_prefix));
        // 0x04 prefix and the right length, but not a point on the curve
        let mut off_curve = [1u8; 65];
        off_curve[0] = 0x04;
        assert!(!check_public_key(&off_curve));
        assert_eq!(encrypt(&off_curve, &[0u8; 16], b"x"), Err(EceError::BadKey));
        assert_eq!(
            encrypt(&wrong_prefix, &[0u8; 16], b"x"),
            Err(EceError::BadKey)
        );
        assert_eq!(
            encrypt_with(
                &[0u8; 32],
                &off_curve,
                &wrong_prefix,
                &[0u8; 16],
                &[0u8; 16],
                b"x"
            ),
            Err(EceError::BadKey)
        );
        assert_eq!(
            encrypt_with(
                &[0u8; 32],
                &off_curve,
                &off_curve,
                &[0u8; 16],
                &[0u8; 16],
                &vec![0u8; MAX_PLAINTEXT + 1]
            ),
            Err(EceError::TooLong)
        );
    }
}
