mod common;

use common::*;
use ntbackup::export::{self, Resolution};
use ntbackup::model::{Backup, Folder};
use ntbackup::vault::VaultError;
use std::fs;
use std::path::Path;

fn backup() -> Backup {
    ntbackup::open_backup(fixture_file(), CODE).unwrap()
}

fn with_passphrase(backup: &Backup) -> Resolution {
    let mut ask = |_: &Folder, previous: Option<&VaultError>| previous.is_none().then(|| VAULT_PASSPHRASE.to_owned());
    export::resolve(backup, false, &mut ask).unwrap()
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn the_code_opens_it_however_it_is_typed() {
    for typed in [CODE, "k7qm t2xb 9h4n rc8v wz3p 6jde", "K7QMT2XB9H4NRC8VWZ3P6JDE"] {
        assert!(ntbackup::open_backup(fixture_file(), typed).is_ok(), "{typed}");
    }
    assert!(matches!(
        ntbackup::open_backup(fixture_file(), "K7QM-T2XB-9H4N-RC8V-WZ3P-6JDF"),
        Err(ntbackup::OpenError::WrongSecretOrDamaged)
    ));
}

#[test]
fn a_tampered_byte_is_refused() {
    let mut file = fixture_file().to_vec();
    let last = file.len() - 20;
    file[last] ^= 1;
    assert!(matches!(
        ntbackup::open_backup(&file, CODE),
        Err(ntbackup::OpenError::WrongSecretOrDamaged)
    ));
    // The header is the AAD: changing the salt is caught too (and gives another key anyway).
    let mut file = fixture_file().to_vec();
    file[25] ^= 1;
    assert!(matches!(
        ntbackup::open_backup(&file, CODE),
        Err(ntbackup::OpenError::WrongSecretOrDamaged)
    ));
}

#[test]
fn a_chosen_phrase_is_normalised_nfc() {
    let json = serde_json::to_vec(content()).unwrap();
    let file = encrypt(&json, "caf\u{e9} cr\u{e8}me br\u{fb}l\u{e9}e");
    // The same phrase typed in NFD opens it.
    assert!(ntbackup::open_backup(&file, "cafe\u{301} cre\u{300}me bru\u{302}le\u{301}e").is_ok());
}

#[test]
fn vaults_open_with_their_passphrase_and_stay_shut_without() {
    let b = backup();
    let r = with_passphrase(&b);
    let titles: Vec<&str> = r.notes.iter().map(|n| n.title.as_str()).collect();
    assert!(titles.contains(&"Old secret title"), "encV 1 keeps its clear title");
    assert!(titles.contains(&"Secret v2"), "encV 2 carries its title inside");
    assert!(
        titles.contains(&"Pin note"),
        "a PIN vault saved open travels with its key"
    );
    assert!(r.locked_ids.is_empty());
    assert_eq!(r.trashed, 1);

    let skipped = export::resolve(&b, true, &mut |_, _| panic!("must not ask")).unwrap();
    assert_eq!(skipped.locked_ids.len(), 2, "the two passphrase-vault notes");
    assert!(skipped.notes.iter().any(|n| n.title == "Pin note"));
}

#[test]
fn a_wrong_vault_passphrase_is_asked_again() {
    let b = backup();
    let mut attempts = 0;
    let mut ask = |_: &Folder, previous: Option<&VaultError>| {
        attempts += 1;
        match attempts {
            1 => Some("wrong".to_owned()),
            2 => {
                assert_eq!(previous, Some(&VaultError::WrongPassphrase));
                Some(VAULT_PASSPHRASE.to_owned())
            }
            _ => None,
        }
    };
    let r = export::resolve(&b, false, &mut ask).unwrap();
    assert!(r.locked_ids.is_empty());
}

#[test]
fn plain_export_mirrors_the_app() {
    let b = backup();
    let r = with_passphrase(&b);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let summary = export::write_plain(&b, &r, &out, false, 1_762_700_000_000).unwrap();
    assert_eq!(summary.written, r.notes.len());

    // Nested folders, the inbox's technical name, collisions disambiguated without case.
    assert!(read(&out, "Work/Projects/Plan Q3.md").contains("title: \"Plan: Q3?\"\nfolder: \"Projects\"\n"));
    assert!(read(&out, "inbox/Dup.md").ends_with("first\n"));
    assert!(read(&out, "inbox/dup-2.md").ends_with("second\n"));
    // Reserved and spoofing names, untitled notes.
    assert!(
        out.join("untitled-folder/note-ncon.md").exists(),
        "CON folder and CON.txt title"
    );
    assert!(out.join("inbox/notegpj.md").exists());
    assert!(out.join("inbox/note-nuntitle.md").exists());
    // A cycle falls back to the folder alone; a missing folder row keeps its id.
    assert!(out.join("Cycle A/In a cycle.md").exists());
    assert!(out.join("f-missing/No folder row.md").exists());
    // Vault notes are marked, as the app marks them.
    let secret = read(&out, "Secret/Secret v2 [unlocked].md");
    assert!(secret.contains("# Note from vault: Secret\n---\n\nsecret v2 body\n"));
    assert!(out.join("PinBox/Pin note [unlocked].md").exists());
    // The trash stays out; the README is the one the app's import recognises.
    assert!(!fs::read_dir(out.join("inbox"))
        .unwrap()
        .any(|e| e.unwrap().file_name() == "Trashed.md"));
    assert!(read(&out, "README.md").starts_with("# Notes Tech export\n"));

    let hello = read(&out, "inbox/Hello.md");
    let expected = format!(
        "---\ntitle: \"Hello\"\nfolder: \"Inbox\"\ntags: [\"work\", \"a, b\"]\ncreated: {}\nupdated: {}\n---\n\n\
         - [ ] buy milk\n- [x] call Ana\n\nSee [[Plan: Q3?]]\n",
        ntbackup::markdown::iso(1_762_600_001_000),
        ntbackup::markdown::iso(1_762_600_002_000),
    );
    assert_eq!(hello, expected);
}

#[test]
fn plain_export_refuses_a_non_empty_directory() {
    let b = backup();
    let r = with_passphrase(&b);
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("mine.txt"), "x").unwrap();
    assert!(export::write_plain(&b, &r, dir.path(), false, 0).is_err());
    assert!(export::write_plain(&b, &r, dir.path(), true, 0).is_ok());
}

#[cfg(unix)]
#[test]
fn plain_export_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let b = backup();
    let r = with_passphrase(&b);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    export::write_plain(&b, &r, &out, false, 0).unwrap();
    assert_eq!(
        fs::metadata(out.join("inbox")).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(out.join("inbox/Hello.md")).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
