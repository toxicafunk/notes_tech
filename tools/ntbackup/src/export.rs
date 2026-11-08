//! From a decrypted backup to readable notes, and the plain Markdown export.

use crate::markdown::{self, NoteView};
use crate::model::{Backup, Folder, Note, Vault};
use crate::vault::{self, FolderKey, VaultError};
use anyhow::{bail, Context};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// `PlanDeRestauration`'s bound on folder depth.
pub const MAX_FOLDER_DEPTH: usize = 32;

/// A note in clear, ready to be written.
#[derive(Debug, Clone)]
pub struct ResolvedNote {
    pub id: String,
    pub folder_id: String,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub favorite: bool,
    pub created_at: i64,
    pub updated_at: i64,
    /// Its folder is a vault: the export marks it, as the app's does (` [unlocked]`, YAML comment).
    pub from_vault: bool,
}

impl ResolvedNote {
    pub fn view(&self) -> NoteView<'_> {
        NoteView {
            title: &self.title,
            content: &self.content,
            tags: &self.tags,
            pinned: self.pinned,
            favorite: self.favorite,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// What a backup holds once the vaults that could be opened are opened.
#[derive(Debug, Default)]
pub struct Resolution {
    /// Not in the trash, readable.
    pub notes: Vec<ResolvedNote>,
    /// Still sealed: their vault was skipped. They exist; they must not be treated as deleted.
    pub locked_ids: HashSet<String>,
    pub trashed: usize,
    /// Sealed notes that did not open with their folder's key: damaged data, reported.
    pub damaged: usize,
}

/// Asked for a passphrase vault's passphrase. `previous` is the error of the previous attempt on the
/// same folder, if any. `None` skips the folder.
pub type PassphrasePrompt<'a> = dyn FnMut(&Folder, Option<&VaultError>) -> Option<String> + 'a;

/// Opens what can be opened. Passphrase vaults are asked for only when they hold notes to export.
pub fn resolve(backup: &Backup, skip_vaults: bool, ask: &mut PassphrasePrompt) -> anyhow::Result<Resolution> {
    let mut resolution = Resolution::default();
    let alive: Vec<&Note> = backup
        .notes
        .iter()
        .filter(|n| {
            let trashed = n.trashed_at.is_some();
            if trashed {
                resolution.trashed += 1;
            }
            !trashed
        })
        .collect();

    let needs_key: HashSet<&str> = alive
        .iter()
        .filter(|n| n.sealed.is_some())
        .map(|n| n.folder_id.as_str())
        .collect();
    let mut keys: HashMap<&str, FolderKey> = HashMap::new();
    for folder in &backup.folders {
        if !needs_key.contains(folder.id.as_str()) {
            continue;
        }
        match &folder.vault {
            Some(Vault::Pin { folder_key }) => {
                keys.insert(&folder.id, zeroize::Zeroizing::new(folder_key.clone()));
            }
            Some(Vault::Passphrase {
                salt,
                wrapped_key,
                iv,
                verifier,
            }) if !skip_vaults => {
                let mut previous: Option<VaultError> = None;
                while let Some(passphrase) = ask(folder, previous.as_ref()) {
                    let passphrase = zeroize::Zeroizing::new(passphrase);
                    match vault::unlock_passphrase(&passphrase, &folder.id, salt, iv, wrapped_key, verifier) {
                        Ok(key) => {
                            keys.insert(&folder.id, key);
                            break;
                        }
                        Err(e @ VaultError::WrongPassphrase) => previous = Some(e),
                        Err(e) => bail!("folder {:?}: {e}", folder.name),
                    }
                }
            }
            _ => {}
        }
    }

    for note in alive {
        let from_vault = backup.folder(&note.folder_id).is_some_and(|f| f.vault.is_some());
        let (title, content) = match &note.sealed {
            None => (note.title.clone(), note.content.clone()),
            Some(blob) => match keys.get(note.folder_id.as_str()) {
                None => {
                    resolution.locked_ids.insert(note.id.clone());
                    continue;
                }
                Some(key) => match vault::open_note(key, &note.id, blob, note.enc_v, &note.title) {
                    Ok(opened) => (opened.title, opened.content),
                    Err(_) => {
                        resolution.damaged += 1;
                        resolution.locked_ids.insert(note.id.clone());
                        continue;
                    }
                },
            },
        };
        resolution.notes.push(ResolvedNote {
            id: note.id.clone(),
            folder_id: note.folder_id.clone(),
            title,
            content,
            tags: note.tags.clone(),
            pinned: note.pinned,
            favorite: note.favorite,
            created_at: note.created_at,
            updated_at: note.updated_at,
            from_vault,
        });
    }
    Ok(resolution)
}

/// The chain of folders from the root down to `folder_id` (included). A missing parent ends the chain;
/// a cycle or a chain deeper than [`MAX_FOLDER_DEPTH`] falls back to the folder alone, at the root.
pub fn folder_chain<'a>(backup: &'a Backup, folder_id: &str) -> Vec<Option<&'a Folder>> {
    let Some(folder) = backup.folder(folder_id) else {
        return vec![None];
    };
    let mut chain = vec![folder];
    let mut seen: HashSet<&str> = HashSet::from([folder.id.as_str()]);
    let mut current = folder;
    while let Some(parent_id) = current.parent_id.as_deref() {
        let Some(parent) = backup.folder(parent_id) else { break };
        if !seen.insert(parent.id.as_str()) || chain.len() >= MAX_FOLDER_DEPTH {
            return vec![Some(folder)];
        }
        chain.push(parent);
        current = parent;
    }
    chain.into_iter().rev().map(Some).collect()
}

/// What the plain export did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PlainSummary {
    pub written: usize,
}

/// Writes one Markdown file per note under `dir`, folders as nested directories, plus a `README.md`.
pub fn write_plain(
    backup: &Backup,
    resolution: &Resolution,
    dir: &Path,
    force: bool,
    exported_at_millis: i64,
) -> anyhow::Result<PlainSummary> {
    prepare_output_dir(dir, force)?;
    let root = dir
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", dir.display()))?;
    let mut used = HashMap::new();
    // The README is a name of the root: a note at the root never takes it.
    used.insert(crate::text::java_lowercase(markdown::README_NAME), 1);

    let mut summary = PlainSummary::default();
    for note in &resolution.notes {
        let chain = folder_chain(backup, &note.folder_id);
        let segments: Vec<String> = chain
            .iter()
            .map(|f| markdown::safe_folder_name(&markdown::folder_dir_raw(*f, f.map_or(&note.folder_id, |f| &f.id))))
            .collect();
        let file_name = markdown::safe_file_name(&note.title, &note.id, note.from_vault);
        let relative = markdown::disambiguate(&mut used, &format!("{}/{}", segments.join("/"), file_name));

        let folder = backup.folder(&note.folder_id);
        let label = markdown::folder_label(folder, &note.folder_id);
        let mention = note.from_vault.then(|| markdown::vault_mention(&label));
        let text = markdown::render(&note.view(), &label, mention.as_deref());
        write_private_file(&root, Path::new(&relative), text.as_bytes())?;
        summary.written += 1;
    }
    write_private_file(
        &root,
        Path::new(markdown::README_NAME),
        markdown::readme(summary.written, exported_at_millis).as_bytes(),
    )?;
    Ok(summary)
}

fn prepare_output_dir(dir: &Path, force: bool) -> anyhow::Result<()> {
    if dir.exists() {
        if !dir.is_dir() {
            bail!("{} exists and is not a directory", dir.display());
        }
        let not_empty = fs::read_dir(dir)?.next().is_some();
        if not_empty && !force {
            bail!("{} is not empty (use --force to write into it anyway)", dir.display());
        }
    } else {
        create_private_dir_all(dir)?;
    }
    Ok(())
}

/// Creates `dir` and its missing parents, owner-only on Unix.
pub fn create_private_dir_all(dir: &Path) -> anyhow::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .with_context(|| format!("cannot create {}", dir.display()))
}

/// Writes `bytes` at `root/relative`, refusing any path that would leave `root`. Owner-only on Unix.
pub fn write_private_file(root: &Path, relative: &Path, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    if relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("refusing to write outside the output directory: {}", relative.display());
    }
    // Both sides resolved: on macOS `/var` is a link to `/private/var`, and a resolved parent never
    // starts with an unresolved root.
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", root.display()))?;
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        create_private_dir_all(parent)?;
        let parent = parent.canonicalize()?;
        if !parent.starts_with(&root) {
            bail!("refusing to write outside the output directory: {}", relative.display());
        }
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    file.write_all(bytes)?;
    Ok(path)
}
