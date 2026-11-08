//! The Markdown rendering and naming of the app's export, ported.
//!
//! Source of truth: `app/src/main/java/com/filestech/notes_tech/domain/export/NoteMarkdown.kt` and
//! `NoteArchive.kt`. The output must be what the app's own export writes, so that the app's import and
//! other tools (Obsidian, Logseq, Bear, Foam) read it the same way.

use crate::text::{collapse_whitespace, dart_trim, java_lowercase, truncate_utf16};
use chrono::{Local, TimeZone};
use std::collections::HashMap;

/// `NoteMarkdown.INBOX_DIR_NAME`: never translated, so two exports give the same tree.
pub const INBOX_DIR_NAME: &str = "inbox";
/// `NoteMarkdown.UNTITLED_FOLDER_DIR_NAME`.
pub const UNTITLED_FOLDER_DIR_NAME: &str = "untitled-folder";
/// `NoteMarkdown.README_HEADING`: how the app's import recognises the README, and leaves it out.
pub const README_HEADING: &str = "# Notes Tech export";
/// `NoteArchive.README_NAME`.
pub const README_NAME: &str = "README.md";
/// The inbox's label in English, as the app writes it for an English interface.
pub const INBOX_LABEL: &str = "Inbox";
/// `NoteMarkdown.MAX_FILE_NAME_LENGTH`, in UTF-16 units.
const MAX_FILE_NAME_LENGTH: usize = 80;

/// `FORBIDDEN`: `[<>:"/\\|?*\x00-\x1f\x7f]`.
fn is_forbidden(c: char) -> bool {
    matches!(
        c,
        '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\u{00}'..='\u{1F}' | '\u{7F}'
    )
}

/// `BIDI`: the writing-direction controls and the byte-order mark.
fn is_bidi(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// `YAML_CONTROL`: `[\x00-\x08\x0B\x0C\x0E-\x1F\x7F]`.
fn is_yaml_control(c: char) -> bool {
    matches!(c, '\u{00}'..='\u{08}' | '\u{0B}' | '\u{0C}' | '\u{0E}'..='\u{1F}' | '\u{7F}')
}

fn strip(s: &str, pred: fn(char) -> bool) -> String {
    s.chars().filter(|&c| !pred(c)).collect()
}

/// `WINDOWS_RESERVED`.
fn is_windows_reserved_stem(stem: &str) -> bool {
    match stem {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³" => true,
        _ => {
            let b = stem.as_bytes();
            b.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && b[3].is_ascii_digit()
        }
    }
}

/// `estReserveWindows`: a device name stays reserved whatever its extension, and Windows ignores
/// trailing dots and spaces.
pub fn is_windows_reserved(name: &str) -> bool {
    let trimmed = name.trim_end_matches([' ', '.']);
    let stem = trimmed.split('.').next().unwrap_or("");
    is_windows_reserved_stem(&stem.to_uppercase())
}

/// `NoteMarkdown.safeFileName`.
pub fn safe_file_name(title: &str, fallback_id: &str, from_unlocked_vault: bool) -> String {
    let mut clean = dart_trim(title).to_owned();
    clean = strip(&clean, is_forbidden);
    clean = strip(&clean, is_bidi);
    clean = dart_trim(&collapse_whitespace(&clean)).to_owned();
    clean = dart_trim(truncate_utf16(&clean, MAX_FILE_NAME_LENGTH)).to_owned();
    if clean == "." || clean == ".." {
        clean.clear();
    }
    if clean.is_empty() || is_windows_reserved(&clean) {
        clean = format!(
            "note-{}",
            fallback_id.replace('-', "").chars().take(8).collect::<String>()
        );
    }
    if from_unlocked_vault {
        clean.push_str(" [unlocked]");
    }
    clean + ".md"
}

/// `NoteMarkdown.safeFolderName`, given the raw name already chosen by the caller (see
/// [`folder_dir_raw`]).
pub fn safe_folder_name(raw: &str) -> String {
    let mut clean = strip(raw, is_forbidden);
    clean = strip(&clean, is_bidi);
    clean = dart_trim(&collapse_whitespace(&clean)).to_owned();
    clean = truncate_utf16(&clean, MAX_FILE_NAME_LENGTH).to_owned();
    clean = clean.trim_end_matches([' ', '.']).to_owned();
    if clean.trim_end_matches([' ', '.']).is_empty() || is_windows_reserved(&clean) {
        UNTITLED_FOLDER_DIR_NAME.to_owned()
    } else {
        clean
    }
}

/// The raw directory name of a folder, before cleaning (`safeFolderName`'s `brut`).
pub fn folder_dir_raw(folder: Option<&crate::model::Folder>, folder_id: &str) -> String {
    match folder {
        Some(f) if !f.has_default_inbox_name() => f.name.clone(),
        _ if folder_id == crate::model::INBOX_ID => INBOX_DIR_NAME.to_owned(),
        _ => folder_id.to_owned(),
    }
}

/// `NoteMarkdown.folderLabel`: the `folder:` value of the frontmatter.
pub fn folder_label(folder: Option<&crate::model::Folder>, folder_id: &str) -> String {
    match folder {
        Some(f) if !f.has_default_inbox_name() => f.name.clone(),
        _ if folder_id == crate::model::INBOX_ID => INBOX_LABEL.to_owned(),
        _ => folder_id.to_owned(),
    }
}

/// `NoteMarkdown.yamlString`: always double-quoted, escaped per YAML 1.2. The backslash goes first.
pub fn yaml_string(raw: &str) -> String {
    let escaped = raw
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    let mut out = String::with_capacity(escaped.len() + 2);
    out.push('"');
    for c in escaped.chars() {
        if is_yaml_control(c) {
            out.push_str(&format!("\\x{:02x}", c as u32));
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// `NoteMarkdown.iso`: local date-time with milliseconds and NO offset — what Dart's
/// `toIso8601String()` gives for a local date, which the app keeps.
pub fn iso(epoch_millis: i64) -> String {
    match Local.timestamp_millis_opt(epoch_millis).single() {
        Some(t) => t.format("%Y-%m-%dT%H:%M:%S%.3f").to_string(),
        None => epoch_millis.to_string(),
    }
}

/// What a rendered note is made of: its fields once opened.
#[derive(Debug, Clone)]
pub struct NoteView<'a> {
    pub title: &'a str,
    pub content: &'a str,
    pub tags: &'a [String],
    pub pinned: bool,
    pub favorite: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `NoteMarkdown.render`: frontmatter, then the body, with a final newline guaranteed.
pub fn render(note: &NoteView, folder_label: &str, vault_mention: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("title: {}\n", yaml_string(note.title)));
    out.push_str(&format!("folder: {}\n", yaml_string(folder_label)));
    if note.tags.is_empty() {
        out.push_str("tags: []\n");
    } else {
        let tags: Vec<String> = note.tags.iter().map(|t| yaml_string(t)).collect();
        out.push_str(&format!("tags: [{}]\n", tags.join(", ")));
    }
    out.push_str(&format!("created: {}\n", iso(note.created_at)));
    out.push_str(&format!("updated: {}\n", iso(note.updated_at)));
    if note.pinned {
        out.push_str("pinned: true\n");
    }
    if note.favorite {
        out.push_str("favorite: true\n");
    }
    if let Some(mention) = vault_mention.filter(|m| !m.is_empty()) {
        out.push_str(&format!("# {}\n", mention.replace('\n', " ")));
    }
    out.push_str("---\n\n");
    out.push_str(note.content);
    if !note.content.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// `R.string.export_note_from_vault` in English.
pub fn vault_mention(folder_label: &str) -> String {
    format!("Note from vault: {folder_label}")
}

/// `NoteMarkdown.readme`.
pub fn readme(note_count: usize, exported_at_millis: i64) -> String {
    format!(
        "{README_HEADING}\n\n- Exported: {}\n- Notes: {note_count}\n\n\
         Format: one Markdown file per note, with YAML frontmatter\n\
         (`title`, `folder`, `tags`, `created`, `updated`, `pinned`,\n\
         `favorite`). Works with Obsidian, Logseq, Bear, Foam.\n\n\
         The folder tree mirrors your folders at export time.\n\
         Notes in the trash are NOT included.\n\n\
         Notes Tech — https://www.files-tech.com\n",
        iso(exported_at_millis)
    )
}

/// `NoteArchive.desambigue`: `a.md`, `a-2.md`, `a-3.md`… compared case-insensitively.
pub fn disambiguate(used: &mut HashMap<String, usize>, path: &str) -> String {
    let key = java_lowercase(path);
    let seen = *used.get(&key).unwrap_or(&0);
    used.insert(key, seen + 1);
    if seen == 0 {
        return path.to_owned();
    }
    let (stem, ext) = match path.rfind('.') {
        Some(i) => (&path[..i], &path[i..]),
        None => (path, ""),
    };
    let mut n = seen + 1;
    let mut candidate = format!("{stem}-{n}{ext}");
    while used.contains_key(&java_lowercase(&candidate)) {
        n += 1;
        candidate = format!("{stem}-{n}{ext}");
    }
    used.insert(java_lowercase(&candidate), 1);
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(safe_file_name("  Hello: world? ", "id", false), "Hello world.md");
        assert_eq!(safe_file_name("a/b", "id", false), "ab.md");
        assert_eq!(safe_file_name("CON.txt", "1234-5678-9abc", false), "note-12345678.md");
        assert_eq!(safe_file_name("COM1", "abcdefgh-ij", false), "note-abcdefgh.md");
        assert_eq!(safe_file_name("..", "x", false), "note-x.md");
        assert_eq!(safe_file_name("note\u{202E}gpj", "x", false), "notegpj.md");
        assert_eq!(safe_file_name("Secret", "x", true), "Secret [unlocked].md");
        assert_eq!(safe_file_name("a \t b", "x", false), "a b.md");
    }

    #[test]
    fn folder_names() {
        assert_eq!(safe_folder_name("Work"), "Work");
        assert_eq!(safe_folder_name("Secret."), "Secret");
        assert_eq!(safe_folder_name(".."), UNTITLED_FOLDER_DIR_NAME);
        assert_eq!(safe_folder_name("CON"), UNTITLED_FOLDER_DIR_NAME);
        assert_eq!(safe_folder_name("a/b"), "ab");
    }

    #[test]
    fn yaml() {
        assert_eq!(yaml_string(r#"a "b" \c"#), r#""a \"b\" \\c""#);
        assert_eq!(yaml_string("l1\nl2\t\u{1}"), r#""l1\nl2\t\x01""#);
        assert_eq!(yaml_string("true"), "\"true\"");
    }

    #[test]
    fn disambiguation() {
        let mut used = HashMap::new();
        assert_eq!(disambiguate(&mut used, "inbox/a.md"), "inbox/a.md");
        assert_eq!(disambiguate(&mut used, "inbox/A.md"), "inbox/A-2.md");
        assert_eq!(disambiguate(&mut used, "inbox/a.md"), "inbox/a-3.md");
    }
}
