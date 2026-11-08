//! Opening a vault folder's key, and a vault note's envelope.
//!
//! Sources of truth:
//! - `security/vault/FolderVaultService.kt` (`unlockWithPassphrase`)
//! - `security/vault/NoteEnvelope.kt`
//! - `security/vault/VaultParams.kt`

use crate::crypto;
use zeroize::Zeroizing;

/// `VaultParams.PASSPHRASE_ITERATIONS` / `PASSPHRASE_MEMORY_KIB`.
pub const PASSPHRASE_ITERATIONS: u32 = 3;
pub const PASSPHRASE_MEMORY_KIB: u32 = 64 * 1024;
pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
/// `NoteEnvelope.ENC_V_CONTENT_ONLY`: the blob holds the content; the title stayed in clear.
pub const ENC_V_CONTENT_ONLY: i64 = 1;
/// `NoteEnvelope.ENC_V_TITLE_AND_CONTENT`: the blob holds the title AND the content.
pub const ENC_V_TITLE_AND_CONTENT: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultError {
    /// The passphrase does not open this vault (the tag did not hold, or the verifier did not match).
    WrongPassphrase,
    /// The stored data is malformed: not a wrong secret.
    Malformed(&'static str),
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultError::WrongPassphrase => write!(f, "wrong passphrase"),
            VaultError::Malformed(what) => write!(f, "malformed vault data: {what}"),
        }
    }
}

impl std::error::Error for VaultError {}

/// A vault folder's 32-byte key.
pub type FolderKey = Zeroizing<Vec<u8>>;

/// Opens a passphrase vault's key, as `FolderVaultService.unlockWithPassphrase` does:
///
/// `k = Argon2id(passphrase, salt, t=3, m=64 MiB)` → `AES-GCM-open(k, iv, wrappedKey, AAD = folder id)`
/// → checked against the HMAC verifier.
///
/// ⚠️ The passphrase is used as typed, in UTF-8, **without** Unicode normalisation: the app never
/// normalises it, and normalising here would fail to open every vault whose passphrase has accents
/// typed in NFD.
pub fn unlock_passphrase(
    passphrase: &str,
    folder_id: &str,
    salt: &[u8],
    iv: &[u8],
    wrapped_key: &[u8],
    verifier: &[u8],
) -> Result<FolderKey, VaultError> {
    if salt.is_empty() {
        return Err(VaultError::Malformed("empty salt"));
    }
    if iv.len() != NONCE_LEN {
        return Err(VaultError::Malformed("vault iv is not 12 bytes"));
    }
    if wrapped_key.len() < TAG_LEN {
        return Err(VaultError::Malformed("wrapped key too short"));
    }
    let derived = crypto::derive_key(
        passphrase.as_bytes(),
        salt,
        PASSPHRASE_ITERATIONS,
        PASSPHRASE_MEMORY_KIB,
    )
    .map_err(|_| VaultError::Malformed("key derivation failed"))?;
    let folder_key = crypto::open(derived.as_ref(), iv, wrapped_key, folder_id.as_bytes())
        .map_err(|_| VaultError::WrongPassphrase)?;
    if folder_key.len() != crypto::KEY_LEN {
        return Err(VaultError::Malformed("folder key is not 32 bytes"));
    }
    if !crypto::matches_verifier(&folder_key, verifier) {
        return Err(VaultError::WrongPassphrase);
    }
    Ok(folder_key)
}

/// A vault note, opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedNote {
    pub title: String,
    pub content: String,
}

/// Opens a vault note's envelope: `nonce(12) ‖ ciphertext ‖ tag(16)`, AAD = the note id in UTF-8.
///
/// - `encV = 1`: the plaintext is the content; the title is the clear `title` column.
/// - `encV = 2`: the plaintext is `u32 big-endian title length ‖ title ‖ content`.
pub fn open_note(
    folder_key: &[u8],
    note_id: &str,
    blob: &[u8],
    enc_v: i64,
    clear_title: &str,
) -> Result<OpenedNote, VaultError> {
    if blob.len() < NONCE_LEN + TAG_LEN {
        return Err(VaultError::Malformed("note envelope too short"));
    }
    let (nonce, sealed) = blob.split_at(NONCE_LEN);
    let plain = crypto::open(folder_key, nonce, sealed, note_id.as_bytes()).map_err(|_| VaultError::WrongPassphrase)?;
    match enc_v {
        ENC_V_CONTENT_ONLY => Ok(OpenedNote {
            title: clear_title.to_owned(),
            content: utf8(&plain)?,
        }),
        ENC_V_TITLE_AND_CONTENT => {
            if plain.len() < 4 {
                return Err(VaultError::Malformed("note plaintext shorter than its length prefix"));
            }
            let title_len = u32::from_be_bytes(plain[..4].try_into().unwrap()) as usize;
            let rest = &plain[4..];
            if title_len > rest.len() {
                return Err(VaultError::Malformed("note title length past the end"));
            }
            Ok(OpenedNote {
                title: utf8(&rest[..title_len])?,
                content: utf8(&rest[title_len..])?,
            })
        }
        _ => Err(VaultError::Malformed("unknown encV")),
    }
}

fn utf8(bytes: &[u8]) -> Result<String, VaultError> {
    String::from_utf8(bytes.to_vec()).map_err(|_| VaultError::Malformed("note text is not UTF-8"))
}

/// Seals a note as the app does. Used to write test fixtures only.
pub fn seal_note(
    folder_key: &[u8],
    nonce: &[u8; NONCE_LEN],
    note_id: &str,
    title: &str,
    content: &str,
    enc_v: i64,
) -> Vec<u8> {
    let plain = if enc_v == ENC_V_TITLE_AND_CONTENT {
        let mut p = (title.len() as u32).to_be_bytes().to_vec();
        p.extend_from_slice(title.as_bytes());
        p.extend_from_slice(content.as_bytes());
        p
    } else {
        content.as_bytes().to_vec()
    };
    let mut blob = nonce.to_vec();
    blob.extend(crypto::seal(folder_key, nonce, &plain, note_id.as_bytes()));
    blob
}
