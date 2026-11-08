//! Reads Notes Tech `.ntbackup` files (format version 1) on a desktop.
//!
//! Every format detail here is a port of the app's Kotlin and must stay byte-for-byte identical to it.
//! Each module names its source of truth.

pub mod crypto;
pub mod export;
pub mod header;
pub mod logseq;
pub mod markdown;
pub mod model;
pub mod secret;
pub mod text;
pub mod vault;

use anyhow::Context;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Why a file could not be opened. Each maps to a distinct exit code in the CLI.
#[derive(Debug)]
pub enum OpenError {
    /// The header was refused before any key was derived (exit code 3).
    Refused(header::Refusal),
    /// Wrong secret or damaged file: AES-GCM cannot tell them apart, nor can we (exit code 4).
    WrongSecretOrDamaged,
    /// Anything else: I/O, malformed JSON after a valid tag, unknown content format (exit code 1).
    Other(anyhow::Error),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Refused(r) => write!(f, "file refused: {r}"),
            OpenError::WrongSecretOrDamaged => {
                write!(
                    f,
                    "wrong backup secret, or the file is damaged (they cannot be told apart)"
                )
            }
            OpenError::Other(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// Reads a backup file, refusing anything past [`header::MAX_FILE_BYTES`] by what is really read.
pub fn read_file(path: &Path) -> Result<Vec<u8>, OpenError> {
    let file = File::open(path)
        .with_context(|| format!("cannot open {}", path.display()))
        .map_err(OpenError::Other)?;
    let mut bytes = Vec::new();
    file.take(header::MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {}", path.display()))
        .map_err(OpenError::Other)?;
    if bytes.len() as u64 > header::MAX_FILE_BYTES {
        return Err(OpenError::Refused(header::Refusal::TooLarge));
    }
    Ok(bytes)
}

/// Decrypts a whole backup file with the backup secret as typed, and returns the JSON bytes.
///
/// The header is checked strictly BEFORE the (64 MiB) key derivation, as the app does (R-10).
pub fn decrypt(file: &[u8], typed_secret: &str) -> Result<zeroize::Zeroizing<Vec<u8>>, OpenError> {
    let header = header::Header::parse(file).map_err(OpenError::Refused)?;
    let secret = secret::secret_bytes(typed_secret);
    let key =
        crypto::derive_key(&secret, &header.salt, header::ITERATIONS, header::MEMORY_KIB).map_err(OpenError::Other)?;
    crypto::open(key.as_ref(), &header.nonce, &file[header::HEADER_LEN..], &header.raw)
        .map_err(|_| OpenError::WrongSecretOrDamaged)
}

/// Decrypts and parses a backup file.
pub fn open_backup(file: &[u8], typed_secret: &str) -> Result<model::Backup, OpenError> {
    let json = decrypt(file, typed_secret)?;
    model::Backup::parse(&json).map_err(OpenError::Other)
}
