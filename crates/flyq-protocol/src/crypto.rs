//! IPMSG Blowfish-CBC encryption and RSA key exchange.
//!
//! Implements the cryptographic primitives required by the IPMSG protocol:
//!
//! - **Blowfish-CBC** with PKCS#7 padding for symmetric message encryption.
//! - **RSA** (PKCS#1 v1.5) for encrypting the per-message session key.
//! - **Key exchange**: public keys are serialised in the `EE-NNNNNN` hex
//!   format (exponent-modulus) used by the original IP Messenger.
//!
//! # Encryption flow (send)
//!
//! 1. Generate a random 16-byte Blowfish session key.
//! 2. Encrypt the session key with the recipient's RSA public key.
//! 3. Encrypt the message body with Blowfish-CBC using the session key
//!    and an all-zero IV (or a packet-number-derived IV).
//! 4. Format the extension area as `<capFlags>:<encKey>:<encBody>` in hex.
//!
//! # Decryption flow (receive)
//!
//! 1. Parse `capFlags` to determine the cipher suite.
//! 2. RSA-decrypt the session key with our private key.
//! 3. Blowfish-CBC decrypt the message body with the session key.
//! 4. Strip PKCS#7 padding to recover the plaintext.

use blowfish::Blowfish;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use rsa::pkcs1v15::{DecryptingKey, EncryptingKey};
use rsa::traits::{Decryptor, RandomizedEncryptor};
use rsa::{RsaPrivateKey, RsaPublicKey};

/// Blowfish block size in bytes.
const BF_BLOCK_SIZE: usize = 8;

/// Session key length in bytes (128 bits).
pub const SESSION_KEY_LEN: usize = 16;

/// All-zero IV for Blowfish-CBC (default when `IPMSG_PACKETNO_IV` is not set).
pub const ZERO_IV: [u8; BF_BLOCK_SIZE] = [0u8; BF_BLOCK_SIZE];

/// RSA key size for IPMSG RSA-1024 capability.
const RSA_KEY_BITS: usize = 1024;

// ── Type aliases for Blowfish-CBC ──
type BfCbcEnc = cbc::Encryptor<Blowfish>;
type BfCbcDec = cbc::Decryptor<Blowfish>;

// ── Capability flags (subset relevant to encryption) ──

/// IPMSG Blowfish 128-bit symmetric cipher.
pub const IPMSG_BLOWFISH_128: u32 = 0x0002_0000;
/// IPMSG RSA 1024-bit asymmetric cipher.
pub const IPMSG_RSA_1024: u32 = 0x0000_0002;
/// IPMSG packet-number-derived IV.
pub const IPMSG_PACKETNO_IV: u32 = 0x0080_0000;

/// Default capability flags advertised by this implementation.
pub const DEFAULT_CAP_FLAGS: u32 = IPMSG_RSA_1024 | IPMSG_BLOWFISH_128;

/// Errors that can occur during cryptographic operations.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("Blowfish encryption failed: invalid key length")]
    InvalidKeyLength,
    #[error("Blowfish decryption failed: padding error")]
    PaddingError,
    #[error("RSA operation failed: {0}")]
    RsaError(String),
    #[error("hex decode error: {0}")]
    HexDecode(#[from] hex::FromHexError),
    #[error("malformed public key format (expected EE-NNNN)")]
    MalformedPublicKey,
    #[error("unsupported capability flags: {0:#010x}")]
    UnsupportedCipher(u32),
}

// ── Blowfish-CBC primitives ──

/// Encrypt `plaintext` with Blowfish-CBC using PKCS#7 padding.
///
/// `key` must be 4–56 bytes. `iv` is the 8-byte initialisation vector
/// (pass `&ZERO_IV` for the default all-zero IV).
pub fn blowfish_cbc_encrypt(
    key: &[u8],
    iv: &[u8; BF_BLOCK_SIZE],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if key.is_empty() || key.len() > 56 {
        return Err(CryptoError::InvalidKeyLength);
    }
    let enc = BfCbcEnc::new_from_slices(key, iv)
        .map_err(|_| CryptoError::InvalidKeyLength)?;
    Ok(enc.encrypt_padded_vec_mut::<Pkcs7>(plaintext))
}

/// Decrypt `ciphertext` with Blowfish-CBC using PKCS#7 padding.
///
/// Returns the plaintext with padding removed.
pub fn blowfish_cbc_decrypt(
    key: &[u8],
    iv: &[u8; BF_BLOCK_SIZE],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if key.is_empty() || key.len() > 56 {
        return Err(CryptoError::InvalidKeyLength);
    }
    let dec = BfCbcDec::new_from_slices(key, iv)
        .map_err(|_| CryptoError::InvalidKeyLength)?;
    dec.decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| CryptoError::PaddingError)
}

/// Derive an IV from a packet number string (zero-padded to 8 bytes).
///
/// Used when the `IPMSG_PACKETNO_IV` capability flag is set.
pub fn iv_from_packet_no(packet_no: &str) -> [u8; BF_BLOCK_SIZE] {
    let bytes = packet_no.as_bytes();
    let mut iv = [0u8; BF_BLOCK_SIZE];
    let len = bytes.len().min(BF_BLOCK_SIZE);
    iv[..len].copy_from_slice(&bytes[..len]);
    iv
}

// ── RSA key management ──

/// An RSA key pair used for IPMSG key exchange.
///
/// Generated once at startup; the public key is shared with peers via
/// `ANSPUBKEY`, and the private key is used to decrypt incoming session keys.
pub struct IpmsgKeyPair {
    private: RsaPrivateKey,
    public: RsaPublicKey,
}

impl IpmsgKeyPair {
    /// Generate a new RSA-1024 key pair.
    pub fn generate() -> Result<Self, CryptoError> {
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, RSA_KEY_BITS)
            .map_err(|e| CryptoError::RsaError(e.to_string()))?;
        let public = RsaPublicKey::from(&private);
        Ok(Self { private, public })
    }

    /// Serialise the public key in the IPMSG `EE-NNNNNN` hex format.
    ///
    /// `EE` is the public exponent, `NNNNNN` is the modulus, both in
    /// lowercase hex without leading zeros (except a single `0` for zero).
    pub fn public_key_hex(&self) -> String {
        use rsa::traits::PublicKeyParts;
        let e = self.public.e().to_bytes_be();
        let n = self.public.n().to_bytes_be();
        let e_hex = hex::encode(&e);
        let n_hex = hex::encode(&n);
        format!("{}-{}", e_hex, n_hex)
    }

    /// Encrypt a session key with this key pair's public key (PKCS#1 v1.5).
    pub fn encrypt_session_key(&self, session_key: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut rng = rand::thread_rng();
        let enc_key = EncryptingKey::new(self.public.clone());
        enc_key
            .encrypt_with_rng(&mut rng, session_key)
            .map_err(|e| CryptoError::RsaError(e.to_string()))
    }

    /// Decrypt a session key with this key pair's private key (PKCS#1 v1.5).
    pub fn decrypt_session_key(&self, encrypted: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let dec_key = DecryptingKey::new(self.private.clone());
        dec_key
            .decrypt(encrypted)
            .map_err(|e| CryptoError::RsaError(e.to_string()))
    }
}

/// Parse a peer's RSA public key from the `EE-NNNNNN` hex format.
pub fn parse_public_key(hex_str: &str) -> Result<RsaPublicKey, CryptoError> {
    let parts: Vec<&str> = hex_str.splitn(2, '-').collect();
    if parts.len() != 2 {
        return Err(CryptoError::MalformedPublicKey);
    }
    let e_bytes = hex::decode(parts[0])?;
    let n_bytes = hex::decode(parts[1])?;

    use rsa::BigUint;
    let e = BigUint::from_bytes_be(&e_bytes);
    let n = BigUint::from_bytes_be(&n_bytes);
    RsaPublicKey::new(n, e).map_err(|e| CryptoError::RsaError(e.to_string()))
}

/// Encrypt a session key with a peer's public key.
pub fn encrypt_session_key_with(
    peer_key: &RsaPublicKey,
    session_key: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let mut rng = rand::thread_rng();
    let enc_key = EncryptingKey::new(peer_key.clone());
    enc_key
        .encrypt_with_rng(&mut rng, session_key)
        .map_err(|e| CryptoError::RsaError(e.to_string()))
}

// ── Session key generation ──

/// Generate a cryptographically random session key (16 bytes / 128 bits).
pub fn generate_session_key() -> [u8; SESSION_KEY_LEN] {
    use rand::RngCore;
    let mut key = [0u8; SESSION_KEY_LEN];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

// ── High-level encrypt / decrypt for IPMSG extension area ──

/// Encrypt a message for the IPMSG extension area.
///
/// Returns the formatted extension string: `<capFlags>:<encKey>:<encBody>`
/// where `encKey` and `encBody` are hex-encoded.
///
/// - `plaintext`: the message body (including trailing `\0` if applicable).
/// - `our_key`: our RSA key pair (used to get the public key for encryption
///   — in practice you'd encrypt with the *peer's* public key, but this
///   helper also supports encrypting for ourselves for testing).
/// - `peer_pub`: the recipient's RSA public key.
/// - `iv`: the initialisation vector (zero IV or packet-number-derived).
pub fn encrypt_extension(
    plaintext: &[u8],
    peer_pub: &RsaPublicKey,
    iv: &[u8; BF_BLOCK_SIZE],
) -> Result<String, CryptoError> {
    let session_key = generate_session_key();
    let enc_key = encrypt_session_key_with(peer_pub, &session_key)?;
    let enc_body = blowfish_cbc_encrypt(&session_key, iv, plaintext)?;

    Ok(format!(
        "{}:{}:{}",
        DEFAULT_CAP_FLAGS,
        hex::encode(&enc_key),
        hex::encode(&enc_body),
    ))
}

/// Decrypt an IPMSG extension area.
///
/// Parses `<capFlags>:<encKey>:<encBody>`, RSA-decrypts the session key,
/// then Blowfish-CBC decrypts the body.
///
/// Returns the decrypted plaintext bytes.
pub fn decrypt_extension(
    ext: &str,
    our_key: &IpmsgKeyPair,
    iv: &[u8; BF_BLOCK_SIZE],
) -> Result<Vec<u8>, CryptoError> {
    let parts: Vec<&str> = ext.splitn(3, ':').collect();
    if parts.len() < 3 {
        return Err(CryptoError::MalformedPublicKey);
    }

    let _cap_flags = u32::from_str_radix(parts[0], 16)
        .map_err(|_| CryptoError::UnsupportedCipher(0))?;

    let enc_key = hex::decode(parts[1])?;
    let enc_body = hex::decode(parts[2])?;

    let session_key = our_key.decrypt_session_key(&enc_key)?;
    blowfish_cbc_decrypt(&session_key, iv, &enc_body)
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blowfish_cbc_roundtrip() {
        let key = b"test-key-1234";
        let iv = ZERO_IV;
        let plaintext = b"Hello, IPMSG encryption!";

        let ciphertext = blowfish_cbc_encrypt(key, &iv, plaintext).unwrap();
        assert_ne!(&ciphertext, plaintext);

        let decrypted = blowfish_cbc_decrypt(key, &iv, &ciphertext).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn blowfish_cbc_empty_plaintext() {
        let key = b"short";
        let iv = ZERO_IV;
        let plaintext = b"";

        let ciphertext = blowfish_cbc_encrypt(key, &iv, plaintext).unwrap();
        let decrypted = blowfish_cbc_decrypt(key, &iv, &ciphertext).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn blowfish_cbc_pkcs7_padding() {
        let key = b"padding-test-key";
        let iv = ZERO_IV;
        // Exactly 8 bytes (one block) — padding adds another full block.
        let plaintext = b"12345678";

        let ciphertext = blowfish_cbc_encrypt(key, &iv, plaintext).unwrap();
        assert_eq!(ciphertext.len(), BF_BLOCK_SIZE * 2);

        let decrypted = blowfish_cbc_decrypt(key, &iv, &ciphertext).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn blowfish_cbc_invalid_key_length() {
        let iv = ZERO_IV;
        // Key too long (57 bytes, max is 56).
        let key = [0u8; 57];
        assert!(blowfish_cbc_encrypt(&key, &iv, b"test").is_err());
    }

    #[test]
    fn blowfish_cbc_wrong_key_fails() {
        let key1 = b"correct-key";
        let key2 = b"wrong-key-xx";
        let iv = ZERO_IV;
        let plaintext = b"secret message";

        let ciphertext = blowfish_cbc_encrypt(key1, &iv, plaintext).unwrap();
        let result = blowfish_cbc_decrypt(key2, &iv, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn iv_from_packet_no_short() {
        let iv = iv_from_packet_no("123");
        assert_eq!(&iv[..3], b"123");
        assert_eq!(&iv[3..], &[0, 0, 0, 0, 0]);
    }

    #[test]
    fn iv_from_packet_no_exact() {
        let iv = iv_from_packet_no("12345678");
        assert_eq!(&iv, b"12345678");
    }

    #[test]
    fn iv_from_packet_no_long() {
        let iv = iv_from_packet_no("1234567890");
        assert_eq!(&iv, b"12345678"); // truncated to 8 bytes
    }

    #[test]
    fn rsa_keypair_generation() {
        let kp = IpmsgKeyPair::generate().unwrap();
        let hex_str = kp.public_key_hex();
        // Should contain exactly one hyphen separating exponent and modulus.
        assert_eq!(hex_str.matches('-').count(), 1);
        // Both parts should be valid hex.
        let parts: Vec<&str> = hex_str.split('-').collect();
        assert!(hex::decode(parts[0]).is_ok());
        assert!(hex::decode(parts[1]).is_ok());
    }

    #[test]
    fn rsa_session_key_roundtrip() {
        let kp = IpmsgKeyPair::generate().unwrap();
        let session_key = generate_session_key();

        let encrypted = kp.encrypt_session_key(&session_key).unwrap();
        assert_ne!(encrypted, session_key.to_vec());

        let decrypted = kp.decrypt_session_key(&encrypted).unwrap();
        assert_eq!(&decrypted, &session_key);
    }

    #[test]
    fn parse_public_key_valid() {
        let kp = IpmsgKeyPair::generate().unwrap();
        let hex_str = kp.public_key_hex();
        let parsed = parse_public_key(&hex_str).unwrap();
        use rsa::traits::PublicKeyParts;
        let original = &kp.public;
        assert_eq!(parsed.n(), original.n());
        assert_eq!(parsed.e(), original.e());
    }

    #[test]
    fn parse_public_key_invalid() {
        assert!(parse_public_key("no-hyphen").is_err());
        assert!(parse_public_key("zz-badhex").is_err());
    }

    #[test]
    fn full_extension_encrypt_decrypt_roundtrip() {
        let kp = IpmsgKeyPair::generate().unwrap();
        let iv = ZERO_IV;
        let plaintext = b"Hello, encrypted LAN!\0";

        let ext = encrypt_extension(plaintext, &kp.public, &iv).unwrap();
        // Extension should have the format capFlags:encKey:encBody.
        assert_eq!(ext.matches(':').count(), 2);

        let decrypted = decrypt_extension(&ext, &kp, &iv).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn full_roundtrip_with_packet_no_iv() {
        let kp = IpmsgKeyPair::generate().unwrap();
        let iv = iv_from_packet_no("42");
        let plaintext = b"Packet IV test\0";

        let ext = encrypt_extension(plaintext, &kp.public, &iv).unwrap();
        let decrypted = decrypt_extension(&ext, &kp, &iv).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn cross_keypair_encrypt_decrypt() {
        // Alice generates a keypair, Bob encrypts with Alice's public key.
        let alice = IpmsgKeyPair::generate().unwrap();
        let iv = ZERO_IV;
        let plaintext = b"cross-key message\0";

        let ext = encrypt_extension(plaintext, &alice.public, &iv).unwrap();
        let decrypted = decrypt_extension(&ext, &alice, &iv).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let alice = IpmsgKeyPair::generate().unwrap();
        let bob = IpmsgKeyPair::generate().unwrap();
        let iv = ZERO_IV;
        let plaintext = b"for alice only\0";

        let ext = encrypt_extension(plaintext, &alice.public, &iv).unwrap();
        // Bob tries to decrypt — should fail because session key was
        // encrypted with Alice's public key.
        let result = decrypt_extension(&ext, &bob, &iv);
        assert!(result.is_err());
    }

    #[test]
    fn session_key_length() {
        let key = generate_session_key();
        assert_eq!(key.len(), SESSION_KEY_LEN);
    }
}
