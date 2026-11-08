//! The primitives: Argon2id, AES-256-GCM, and the vault verifier.
//!
//! Source of truth: `app/src/main/java/com/filestech/notes_tech/security/vault/VaultCrypto.kt`.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::anyhow;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

pub const KEY_LEN: usize = 32;

/// `VaultParams.VERIFIER_MESSAGE`. Domain-separated; it never changes.
pub const VERIFIER_MESSAGE: &[u8] = b"files-tech.notes_tech.vault.v1";

/// The authentication tag did not hold: wrong key, or damaged data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadTag;

/// `VaultCrypto.deriveKey`: Argon2id v1.3, parallelism 1, 32-byte output.
pub fn derive_key(
    secret: &[u8],
    salt: &[u8],
    iterations: u32,
    memory_kib: u32,
) -> anyhow::Result<Zeroizing<[u8; KEY_LEN]>> {
    let params =
        argon2::Params::new(memory_kib, iterations, 1, Some(KEY_LEN)).map_err(|e| anyhow!("argon2 parameters: {e}"))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(secret, salt, out.as_mut())
        .map_err(|e| anyhow!("argon2: {e}"))?;
    Ok(out)
}

/// `VaultCrypto.open`: decrypts `ciphertext ‖ tag(16)`.
pub fn open(key: &[u8], nonce: &[u8], sealed: &[u8], aad: &[u8]) -> Result<Zeroizing<Vec<u8>>, BadTag> {
    if key.len() != KEY_LEN || nonce.len() != 12 || sealed.len() < 16 {
        return Err(BadTag);
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| BadTag)?;
    cipher
        .decrypt(Nonce::from_slice(nonce), Payload { msg: sealed, aad })
        .map(Zeroizing::new)
        .map_err(|_| BadTag)
}

/// `VaultCrypto.seal`: returns `ciphertext ‖ tag(16)`. Used to write test fixtures only.
pub fn seal(key: &[u8], nonce: &[u8], plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    cipher
        .encrypt(Nonce::from_slice(nonce), Payload { msg: plaintext, aad })
        .expect("AES-GCM encryption does not fail")
}

/// `VaultCrypto.verifierFor`: HMAC-SHA256(folder key, `VERIFIER_MESSAGE`).
pub fn verifier_for(folder_key: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(folder_key).expect("HMAC takes any key length");
    mac.update(VERIFIER_MESSAGE);
    mac.finalize().into_bytes().into()
}

/// `VaultCrypto.matchesVerifier`, in constant time.
pub fn matches_verifier(folder_key: &[u8], expected: &[u8]) -> bool {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(folder_key).expect("HMAC takes any key length");
    mac.update(VERIFIER_MESSAGE);
    mac.verify_slice(expected).is_ok()
}
