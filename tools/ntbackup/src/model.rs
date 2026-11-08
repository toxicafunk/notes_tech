//! The decrypted content of a backup: JSON, folders then notes.
//!
//! Source of truth: `app/src/main/java/com/filestech/notes_tech/data/sauvegarde/ContenuDeSauvegarde.kt`.
//! Binary fields are standard base64 with padding (`Base64.NO_WRAP`).

use anyhow::{bail, Context};
use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const FORMAT: &str = "notes-tech-backup";
pub const CONTENT_VERSION: i64 = 1;
/// `Folder.INBOX_ID`.
pub const INBOX_ID: &str = "inbox";
/// `Folder.INBOX_DEFAULT_NAMES`: the names the inbox carries by default, as opposed to a user's choice.
pub const INBOX_DEFAULT_NAMES: [&str; 2] = ["Inbox", "Bo\u{ee}te de r\u{e9}ception"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub format: String,
    pub version: i64,
    pub created_at: i64,
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub color: Option<i64>,
    pub icon: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub vault: Option<Vault>,
}

impl Folder {
    /// `Folder.isDefaultInboxName`.
    pub fn has_default_inbox_name(&self) -> bool {
        self.id == INBOX_ID && INBOX_DEFAULT_NAMES.contains(&self.name.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum Vault {
    /// A passphrase vault's columns, as they are: only its passphrase opens them.
    #[serde(rename = "passphrase", rename_all = "camelCase")]
    Passphrase {
        #[serde(with = "b64")]
        salt: Vec<u8>,
        #[serde(with = "b64")]
        wrapped_key: Vec<u8>,
        #[serde(with = "b64")]
        iv: Vec<u8>,
        #[serde(with = "b64")]
        verifier: Vec<u8>,
    },
    /// A PIN vault saved while open: its folder key, protected by the backup's secret only.
    #[serde(rename = "pin", rename_all = "camelCase")]
    Pin {
        #[serde(with = "b64")]
        folder_key: Vec<u8>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub folder_id: String,
    pub title: String,
    pub content: String,
    /// A vault note's envelope, untouched; `None` for a note in clear.
    #[serde(with = "b64_opt")]
    pub sealed: Option<Vec<u8>>,
    pub enc_v: i64,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub favorite: bool,
    pub archived: bool,
    pub trashed_at: Option<i64>,
    pub color_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Backup {
    /// Parses the decrypted JSON and checks its format and version.
    pub fn parse(json: &[u8]) -> anyhow::Result<Backup> {
        let backup: Backup = serde_json::from_slice(json).context("decrypted content is not a valid backup")?;
        if backup.format != FORMAT || backup.version != CONTENT_VERSION {
            bail!(
                "unsupported content: format {:?}, version {} (expected {FORMAT:?}, {CONTENT_VERSION})",
                backup.format,
                backup.version
            );
        }
        Ok(backup)
    }

    pub fn folder(&self, id: &str) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }
}

mod b64 {
    use super::*;

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}

mod b64_opt {
    use super::*;

    pub fn serialize<S: Serializer>(bytes: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match bytes {
            Some(b) => super::b64::serialize(b, s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|text| {
                base64::engine::general_purpose::STANDARD
                    .decode(text)
                    .map_err(serde::de::Error::custom)
            })
            .transpose()
    }
}
