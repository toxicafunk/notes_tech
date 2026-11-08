//! The 48-byte header of a `.ntbackup` file, version 1.
//!
//! Source of truth: `app/src/main/java/com/filestech/notes_tech/security/sauvegarde/FichierDeSauvegarde.kt`.
//!
//! ```text
//! 0   8   "NTBACKUP"          20  16  salt
//! 8   2   version = 1         36  12  GCM nonce
//! 10  1   KDF = 1, Argon2id   48  …   AES-256-GCM of the content, AAD = these 48 bytes, tag at the end
//! 11  4   t      15  4   m (KiB)      19  1   p
//! ```

pub const HEADER_LEN: usize = 48;
pub const MAGIC: &[u8; 8] = b"NTBACKUP";
pub const VERSION: u16 = 1;
pub const KDF_ARGON2ID: u8 = 1;
/// `VaultParams.PASSPHRASE_ITERATIONS`.
pub const ITERATIONS: u32 = 3;
/// `VaultParams.PASSPHRASE_MEMORY_KIB`.
pub const MEMORY_KIB: u32 = 64 * 1024;
/// `VaultParams.PARALLELISM`.
pub const PARALLELISM: u8 = 1;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
/// `FichierDeSauvegarde.TAILLE_MAX`: the whole file, header and tag included.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Why a file is refused before its tag is reached (`FichierDeSauvegarde.Refus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotABackup,
    UnknownVersion(u16),
    /// Other KDF parameters than the ones version 1 allows. Refused BEFORE any derivation, so that a
    /// forged header cannot make us allocate or compute more than a real backup does (R-10).
    UnknownParameters,
    Truncated,
    TooLarge,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotABackup => write!(f, "not a Notes Tech backup (.ntbackup)"),
            Refusal::UnknownVersion(v) => {
                write!(
                    f,
                    "backup format version {v} is not supported by this tool (expected {VERSION})"
                )
            }
            Refusal::UnknownParameters => write!(f, "unknown key-derivation parameters in the header"),
            Refusal::Truncated => write!(f, "file truncated"),
            Refusal::TooLarge => write!(f, "file larger than {} MiB", MAX_FILE_BYTES / 1024 / 1024),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
    /// The 48 header bytes as read: the AAD of the content.
    pub raw: [u8; HEADER_LEN],
}

impl Header {
    /// Parses and checks the header at the start of `file`, strictly — nothing is authenticated yet.
    pub fn parse(file: &[u8]) -> Result<Header, Refusal> {
        if file.len() as u64 > MAX_FILE_BYTES {
            return Err(Refusal::TooLarge);
        }
        if file.len() < HEADER_LEN {
            // A short file that does not even start with the magic is not a backup at all.
            let n = file.len().min(MAGIC.len());
            return Err(if file[..n] == MAGIC[..n] {
                Refusal::Truncated
            } else {
                Refusal::NotABackup
            });
        }
        let raw: [u8; HEADER_LEN] = file[..HEADER_LEN].try_into().expect("length checked");
        if &raw[0..8] != MAGIC {
            return Err(Refusal::NotABackup);
        }
        let version = u16::from_be_bytes([raw[8], raw[9]]);
        if version != VERSION {
            return Err(Refusal::UnknownVersion(version));
        }
        let kdf = raw[10];
        let t = u32::from_be_bytes(raw[11..15].try_into().unwrap());
        let m = u32::from_be_bytes(raw[15..19].try_into().unwrap());
        let p = raw[19];
        if kdf != KDF_ARGON2ID || t != ITERATIONS || m != MEMORY_KIB || p != PARALLELISM {
            return Err(Refusal::UnknownParameters);
        }
        // The GCM tag must at least be there.
        if file.len() < HEADER_LEN + TAG_LEN {
            return Err(Refusal::Truncated);
        }
        Ok(Header {
            salt: raw[20..36].try_into().unwrap(),
            nonce: raw[36..48].try_into().unwrap(),
            raw,
        })
    }

    /// The header bytes for `salt` and `nonce` (`FichierDeSauvegarde.EnTete.octets`). Used to write
    /// test fixtures.
    pub fn build(salt: &[u8; SALT_LEN], nonce: &[u8; NONCE_LEN]) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..8].copy_from_slice(MAGIC);
        out[8..10].copy_from_slice(&VERSION.to_be_bytes());
        out[10] = KDF_ARGON2ID;
        out[11..15].copy_from_slice(&ITERATIONS.to_be_bytes());
        out[15..19].copy_from_slice(&MEMORY_KIB.to_be_bytes());
        out[19] = PARALLELISM;
        out[20..36].copy_from_slice(salt);
        out[36..48].copy_from_slice(nonce);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Vec<u8> {
        let mut f = Header::build(&[7; SALT_LEN], &[9; NONCE_LEN]).to_vec();
        f.extend_from_slice(&[0; TAG_LEN]);
        f
    }

    #[test]
    fn round_trip() {
        let h = Header::parse(&valid()).unwrap();
        assert_eq!(h.salt, [7; SALT_LEN]);
        assert_eq!(h.nonce, [9; NONCE_LEN]);
    }

    #[test]
    fn refusals() {
        assert_eq!(Header::parse(b"hello").unwrap_err(), Refusal::NotABackup);
        assert_eq!(Header::parse(b"NTBACK").unwrap_err(), Refusal::Truncated);
        let mut f = valid();
        f[0] = b'X';
        assert_eq!(Header::parse(&f).unwrap_err(), Refusal::NotABackup);
        let mut f = valid();
        f[9] = 2;
        assert_eq!(Header::parse(&f).unwrap_err(), Refusal::UnknownVersion(2));
        // KDF id, t, m (byte 16: 64 MiB → 128 MiB), p.
        for (i, v) in [(10, 2u8), (14, 4), (16, 2), (19, 2)] {
            let mut f = valid();
            f[i] = v;
            assert_eq!(Header::parse(&f).unwrap_err(), Refusal::UnknownParameters, "byte {i}");
        }
        assert_eq!(
            Header::parse(&valid()[..HEADER_LEN + 3]).unwrap_err(),
            Refusal::Truncated
        );
    }
}
