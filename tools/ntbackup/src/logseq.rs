//! `--logseq`: notes as pages of an existing Logseq graph, kept in sync one way.
//!
//! - One page per note, named after the note's title, so the app's `[[Title]]` links and their
//!   backlinks resolve in Logseq. The folder goes in a `folder::` property, as a namespace page.
//! - Pages written here carry `ntbackup-id::` (the note's id) and `ntbackup-hash::` (a SHA-256 of the
//!   page as written, that line excluded). Nothing else is needed to sync: no state file.
//! - Only pages carrying `ntbackup-id::` are ever rewritten, renamed or deleted. A page edited in
//!   Logseq since it was written (its hash no longer holds) is left alone unless `overwrite_edited`.
//! - Checklists are turned into Logseq tasks: `- [ ]` → `- TODO`, `- [x]` → `- DONE`, outside code.

use crate::export::{folder_chain, Resolution, ResolvedNote};
use crate::markdown;
use crate::model::Backup;
use crate::text::{collapse_whitespace, dart_trim, java_lowercase, truncate_utf16};
use anyhow::{bail, Context};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const ID_PROPERTY: &str = "ntbackup-id";
pub const HASH_PROPERTY: &str = "ntbackup-hash";
/// Appended to a note's page name when a page of the user's already has that name.
pub const COLLISION_SUFFIX: &str = " (notes-tech)";
const MAX_FILE_NAME_LENGTH: usize = 80;

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub overwrite_edited: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Create {
        file: String,
    },
    Update {
        file: String,
    },
    Rename {
        from: String,
        to: String,
    },
    Delete {
        file: String,
    },
    /// Edited in Logseq since it was written: left as it is.
    SkipEdited {
        file: String,
        reason: &'static str,
    },
    Unchanged {
        file: String,
    },
}

#[derive(Debug, Default)]
pub struct Report {
    pub actions: Vec<Action>,
    pub warnings: Vec<String>,
}

impl Report {
    pub fn count(&self, pred: fn(&Action) -> bool) -> usize {
        self.actions.iter().filter(|a| pred(a)).count()
    }
}

/// A page found in `pages/`.
#[derive(Debug)]
struct ExistingPage {
    file: String,
    /// The page's name, lowercased: its `title::`, or the name its file stands for.
    name_key: String,
    managed: Option<Managed>,
}

#[derive(Debug)]
struct Managed {
    id: String,
    edited: bool,
    hash: String,
}

/// Syncs the notes of `resolution` into the graph at `graph`.
pub fn sync(backup: &Backup, resolution: &Resolution, graph: &Path, options: Options) -> anyhow::Result<Report> {
    let pages_dir = graph.join("pages");
    if !pages_dir.is_dir() || !graph.join("logseq").join("config.edn").is_file() {
        bail!(
            "{} does not look like a Logseq graph (expected pages/ and logseq/config.edn)",
            graph.display()
        );
    }
    let existing = scan(&pages_dir)?;
    let mut report = Report::default();

    // Pages of the user's: never touched, and their names and files are taken.
    let mut taken_names: HashSet<String> = HashSet::new();
    let mut taken_files: HashSet<String> = HashSet::new();
    let mut user_names: HashSet<String> = HashSet::new();
    // Ours, by note id. A second page with the same id is an orphan.
    let mut ours: BTreeMap<String, &ExistingPage> = BTreeMap::new();
    let mut orphans: Vec<&ExistingPage> = Vec::new();
    for page in &existing {
        match &page.managed {
            None => {
                user_names.insert(page.name_key.clone());
                taken_names.insert(page.name_key.clone());
                taken_files.insert(java_lowercase(&page.file));
            }
            Some(m) => {
                if ours.contains_key(&m.id) {
                    orphans.push(page);
                } else {
                    ours.insert(m.id.clone(), page);
                }
            }
        }
    }

    let wanted: HashSet<&str> = resolution.notes.iter().map(|n| n.id.as_str()).collect();
    let keep_as_is = |page: &ExistingPage| -> bool {
        let m = page.managed.as_ref().expect("managed");
        (m.edited && !options.overwrite_edited) || resolution.locked_ids.contains(&m.id)
    };
    // Pages we keep as they are still hold their names and files.
    for page in ours.values().filter(|p| keep_as_is(p)) {
        taken_names.insert(page.name_key.clone());
        taken_files.insert(java_lowercase(&page.file));
    }

    let mut notes: Vec<&ResolvedNote> = resolution.notes.iter().collect();
    notes.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));

    let mut writes: Vec<(String, String)> = Vec::new();
    let mut deletes: Vec<String> = Vec::new();
    for note in notes {
        let current = ours.get(&note.id).copied();
        if let Some(page) = current.filter(|p| keep_as_is(p)) {
            report.actions.push(Action::SkipEdited {
                file: page.file.clone(),
                reason: "edited in Logseq",
            });
            report.warnings.push(format!(
                "pages/{}: edited in Logseq since the last export, left as it is",
                page.file
            ));
            continue;
        }

        let base = page_name(note);
        let mut name = base.clone();
        if user_names.contains(&java_lowercase(&name)) {
            name = format!("{base}{COLLISION_SUFFIX}");
            report.warnings.push(format!(
                "a page of yours is already named {base:?}: the note is written as {name:?}"
            ));
        }
        if taken_names.contains(&java_lowercase(&name)) {
            let stem = name.clone();
            let mut n = 2;
            while taken_names.contains(&java_lowercase(&format!("{stem} ({n})"))) {
                n += 1;
            }
            name = format!("{stem} ({n})");
            report.warnings.push(format!(
                "two notes are titled {stem:?}: the later one is written as {name:?}"
            ));
        }
        taken_names.insert(java_lowercase(&name));

        let mut file = page_file_name(&name, &note.id);
        if taken_files.contains(&java_lowercase(&file)) {
            let stem = file.trim_end_matches(".md").to_owned();
            let mut n = 2;
            while taken_files.contains(&java_lowercase(&format!("{stem}-{n}.md"))) {
                n += 1;
            }
            file = format!("{stem}-{n}.md");
        }
        taken_files.insert(java_lowercase(&file));

        let text = render(backup, note, &name);
        match current {
            None => {
                report.actions.push(Action::Create { file: file.clone() });
                writes.push((file, text));
            }
            Some(page) if page.file == file => {
                if page.managed.as_ref().map(|m| m.hash.as_str()) == Some(hash_of_page(&text).as_str())
                    && !page.managed.as_ref().unwrap().edited
                {
                    report.actions.push(Action::Unchanged { file });
                } else {
                    report.actions.push(Action::Update { file: file.clone() });
                    writes.push((file, text));
                }
            }
            Some(page) => {
                report.actions.push(Action::Rename {
                    from: page.file.clone(),
                    to: file.clone(),
                });
                writes.push((file, text));
                deletes.push(page.file.clone());
            }
        }
    }

    // Our pages whose note is gone from the backup — deleted, or in the trash, in the app.
    for page in ours.values().copied().chain(orphans) {
        let m = page.managed.as_ref().unwrap();
        let is_current = ours.get(&m.id).is_some_and(|p| std::ptr::eq(*p, page));
        if is_current && (wanted.contains(m.id.as_str()) || resolution.locked_ids.contains(&m.id)) {
            continue;
        }
        if m.edited && !options.overwrite_edited {
            report.actions.push(Action::SkipEdited {
                file: page.file.clone(),
                reason: "note gone, page edited",
            });
            report.warnings.push(format!(
                "pages/{}: its note is no longer in the backup, but the page was edited in Logseq: left as it is",
                page.file
            ));
        } else {
            report.actions.push(Action::Delete {
                file: page.file.clone(),
            });
            deletes.push(page.file.clone());
        }
    }

    if !options.dry_run {
        for (file, text) in &writes {
            write_atomically(&pages_dir, file, text)?;
        }
        // Compared without case: on macOS and Windows, `Note.md` and `note.md` are one file, and a rename
        // between them has already written the new page on top of the old one.
        let written: HashSet<String> = writes.iter().map(|(f, _)| java_lowercase(f)).collect();
        for file in &deletes {
            if written.contains(&java_lowercase(file)) {
                continue;
            }
            fs::remove_file(pages_dir.join(file)).with_context(|| format!("cannot delete pages/{file}"))?;
        }
    }
    Ok(report)
}

/// The page's name: the note's title on one line; an untitled note gets its id.
pub fn page_name(note: &ResolvedNote) -> String {
    let one_line = property_value(&note.title);
    if one_line.is_empty() {
        format!(
            "Untitled note {}",
            note.id.replace('-', "").chars().take(8).collect::<String>()
        )
    } else {
        one_line
    }
}

/// A property value on one line, trimmed.
fn property_value(raw: &str) -> String {
    dart_trim(&collapse_whitespace(&raw.replace(['\r', '\n'], " "))).to_owned()
}

/// The file of a page: Logseq's triple-lowbar format (`/` → `___`), then the export's own cleaning.
pub fn page_file_name(name: &str, note_id: &str) -> String {
    let encoded = name.replace('/', "___");
    let cleaned: String = encoded
        .chars()
        .filter(|&c| {
            !matches!(
                c,
                '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*' | '\u{00}'..='\u{1F}' | '\u{7F}'
            ) && !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
        })
        .collect();
    let mut clean = dart_trim(truncate_utf16(dart_trim(&cleaned), MAX_FILE_NAME_LENGTH)).to_owned();
    clean = clean.trim_start_matches('.').trim_end_matches([' ', '.']).to_owned();
    if clean.is_empty() || markdown::is_windows_reserved(&clean) {
        clean = format!("note-{}", note_id.replace('-', "").chars().take(8).collect::<String>());
    }
    clean + ".md"
}

/// The `folder::` value: the folder path as a Logseq namespace, `[[Work/Projects]]`.
fn folder_property(backup: &Backup, note: &ResolvedNote) -> String {
    let segments: Vec<String> = folder_chain(backup, &note.folder_id)
        .into_iter()
        .map(|f| {
            let label = markdown::folder_label(f, f.map_or(&note.folder_id, |f| &f.id));
            let clean = property_value(&label).replace('/', "-").replace(['[', ']'], "");
            if clean.is_empty() {
                markdown::UNTITLED_FOLDER_DIR_NAME.to_owned()
            } else {
                clean
            }
        })
        .collect();
    format!("[[{}]]", segments.join("/"))
}

/// The `tags::` value: comma-separated; a tag that has a comma of its own is bracketed.
fn tags_property(tags: &[String]) -> String {
    tags.iter()
        .map(|t| property_value(t).replace("[[", "").replace("]]", ""))
        .filter(|t| !t.is_empty())
        .map(|t| if t.contains(',') { format!("[[{t}]]") } else { t })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The whole page, `ntbackup-hash::` included.
pub fn render(backup: &Backup, note: &ResolvedNote, name: &str) -> String {
    let mut props = String::new();
    props.push_str(&format!("title:: {name}\n"));
    props.push_str(&format!("folder:: {}\n", folder_property(backup, note)));
    let tags = tags_property(&note.tags);
    if !tags.is_empty() {
        props.push_str(&format!("tags:: {tags}\n"));
    }
    props.push_str(&format!("created:: {}\n", markdown::iso(note.created_at)));
    props.push_str(&format!("updated:: {}\n", markdown::iso(note.updated_at)));
    if note.pinned {
        props.push_str("pinned:: true\n");
    }
    if note.favorite {
        props.push_str("favorite:: true\n");
    }
    if note.from_vault {
        props.push_str("vault:: true\n");
    }
    props.push_str(&format!("{ID_PROPERTY}:: {}\n", note.id));

    let mut body = convert_tasks(&note.content);
    if !body.ends_with('\n') {
        body.push('\n');
    }
    let without_hash = format!("{props}\n{body}");
    let hash = sha256_hex(without_hash.as_bytes());
    format!("{props}{HASH_PROPERTY}:: {hash}\n\n{body}")
}

/// The hash a page carries: SHA-256 of its text without its `ntbackup-hash::` line.
pub fn hash_of_page(text: &str) -> String {
    sha256_hex(remove_hash_line(text).as_bytes())
}

fn remove_hash_line(text: &str) -> String {
    let marker = format!("{HASH_PROPERTY}:: ");
    let mut out = String::with_capacity(text.len());
    let mut removed = false;
    for line in text.split_inclusive('\n') {
        if !removed && line.starts_with(&marker) {
            removed = true;
            continue;
        }
        out.push_str(line);
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// `- [ ] x` → `- TODO x`, `- [x] x` → `- DONE x` (any of `-*+`, any indent), outside fenced code.
pub fn convert_tasks(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut fence: Option<(char, usize)> = None;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let fence_char = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        if let Some(c) = fence_char {
            let run = trimmed.chars().take_while(|x| *x == c).count();
            if run >= 3 {
                match fence {
                    None => fence = Some((c, run)),
                    Some((open, len)) if open == c && run >= len => fence = None,
                    _ => {}
                }
                out.push_str(line);
                continue;
            }
        }
        if fence.is_some() {
            out.push_str(line);
            continue;
        }
        let indent = &line[..line.len() - trimmed.len()];
        let converted = ["- ", "* ", "+ "].iter().find_map(|bullet| {
            let rest = trimmed.strip_prefix(bullet)?;
            if let Some(after) = rest.strip_prefix("[ ] ") {
                Some(format!("{indent}- TODO {after}"))
            } else {
                rest.strip_prefix("[x] ")
                    .or_else(|| rest.strip_prefix("[X] "))
                    .map(|after| format!("{indent}- DONE {after}"))
            }
        });
        out.push_str(converted.as_deref().unwrap_or(line));
    }
    out
}

/// The pages directly in `pages/`, with what we need to know of each.
fn scan(pages_dir: &Path) -> anyhow::Result<Vec<ExistingPage>> {
    let mut pages = Vec::new();
    for entry in fs::read_dir(pages_dir).with_context(|| format!("cannot read {}", pages_dir.display()))? {
        let entry = entry?;
        let file = entry.file_name().to_string_lossy().into_owned();
        if !file.to_ascii_lowercase().ends_with(".md") || file.starts_with('.') || !entry.file_type()?.is_file() {
            continue;
        }
        let text = fs::read(entry.path()).map(|b| String::from_utf8_lossy(&b).into_owned())?;
        let props = leading_properties(&text);
        let name_key = java_lowercase(props.get("title").map(String::as_str).unwrap_or(&name_of_file(&file)));
        let managed = props.get(ID_PROPERTY).map(|id| {
            let hash = props.get(HASH_PROPERTY).cloned().unwrap_or_default();
            Managed {
                id: id.clone(),
                edited: hash != hash_of_page(&text),
                hash,
            }
        });
        pages.push(ExistingPage {
            file,
            name_key,
            managed,
        });
    }
    pages.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(pages)
}

/// The `key:: value` lines at the top of a page.
fn leading_properties(text: &str) -> HashMap<String, String> {
    let mut props = HashMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(":: ") else {
            break;
        };
        if key.is_empty() || key.contains(char::is_whitespace) {
            break;
        }
        props
            .entry(key.to_ascii_lowercase())
            .or_insert_with(|| value.trim().to_owned());
    }
    props
}

/// The page name a file stands for in Logseq's triple-lowbar format.
fn name_of_file(file: &str) -> String {
    let stem = &file[..file.len() - 3];
    percent_decode(&stem.replace("___", "/"))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &bytes[i + 1..i + 3];
            if let Some(b) = std::str::from_utf8(hex)
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Writes next to the target under a hidden name, then renames: Logseq never sees half a page.
fn write_atomically(pages_dir: &Path, file: &str, text: &str) -> anyhow::Result<PathBuf> {
    let tmp_name = format!(
        ".ntbackup-{}-{}.tmp",
        std::process::id(),
        sha256_hex(file.as_bytes()).get(..12).unwrap_or("")
    );
    let tmp = crate::export::write_private_file(pages_dir, Path::new(&tmp_name), text.as_bytes())?;
    let target = pages_dir.join(file);
    fs::rename(&tmp, &target).with_context(|| format!("cannot write pages/{file}"))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks() {
        let src = "- [ ] buy milk\n  * [x] done\n+ [X] also\n- [] not a task\n```\n- [ ] code\n```\n- [ ] after\n";
        let want = "- TODO buy milk\n  - DONE done\n- DONE also\n- [] not a task\n```\n- [ ] code\n```\n- TODO after\n";
        assert_eq!(convert_tasks(src), want);
    }

    #[test]
    fn tilde_fences_and_longer_fences() {
        let src = "~~~~\n- [ ] in\n```\n- [ ] still in\n~~~~\n- [ ] out";
        let want = "~~~~\n- [ ] in\n```\n- [ ] still in\n~~~~\n- TODO out";
        assert_eq!(convert_tasks(src), want);
    }

    #[test]
    fn file_names() {
        assert_eq!(page_file_name("Work/Plan: Q3?", "id"), "Work___Plan Q3.md");
        assert_eq!(page_file_name("CON", "abcdefgh-1"), "note-abcdefgh.md");
        assert_eq!(page_file_name("...", "abcdefgh-1"), "note-abcdefgh.md");
        assert_eq!(name_of_file("Work___Plan%3A Q3.md"), "Work/Plan: Q3");
    }

    #[test]
    fn hash_round_trip() {
        let text = "title:: a\nntbackup-id:: x\nntbackup-hash:: 00\n\nbody\n";
        let h = hash_of_page(text);
        assert_eq!(h, sha256_hex(b"title:: a\nntbackup-id:: x\n\nbody\n"));
    }

    #[test]
    fn properties() {
        let p = leading_properties("title:: Hello\nntbackup-id:: abc\n\nbody:: not a property\n");
        assert_eq!(p.get("title").map(String::as_str), Some("Hello"));
        assert_eq!(p.get("ntbackup-id").map(String::as_str), Some("abc"));
        assert!(!p.contains_key("body"));
        assert_eq!(
            tags_property(&["a".into(), "b, c".into(), "[[d]]".into()]),
            "a, [[b, c]], d"
        );
    }
}
