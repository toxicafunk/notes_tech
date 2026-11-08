mod common;

use common::*;
use ntbackup::export::{self, Resolution};
use ntbackup::logseq::{self, Action, Options};
use ntbackup::model::Backup;
use std::fs;
use std::path::Path;

fn backup() -> Backup {
    ntbackup::open_backup(fixture_file(), CODE).unwrap()
}

fn resolve(b: &Backup, with_vaults: bool) -> Resolution {
    let mut ask = |_: &_, previous: Option<&_>| previous.is_none().then(|| VAULT_PASSPHRASE.to_owned());
    export::resolve(b, !with_vaults, &mut ask).unwrap()
}

fn page(graph: &Path, file: &str) -> String {
    fs::read_to_string(graph.join("pages").join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))
}

fn sync(b: &Backup, r: &Resolution, graph: &Path) -> logseq::Report {
    logseq::sync(b, r, graph, Options::default()).unwrap()
}

#[test]
fn refuses_what_is_not_a_graph() {
    let b = backup();
    let dir = tempfile::tempdir().unwrap();
    assert!(logseq::sync(&b, &resolve(&b, false), dir.path(), Options::default()).is_err());
}

#[test]
fn first_export_writes_one_page_per_note() {
    let b = backup();
    let r = resolve(&b, true);
    let graph = new_graph();
    let report = sync(&b, &r, graph.path());
    assert_eq!(report.count(|a| matches!(a, Action::Create { .. })), r.notes.len());

    let hello = page(graph.path(), "Hello.md");
    assert!(hello.starts_with("title:: Hello\nfolder:: [[Inbox]]\ntags:: work, [[a, b]]\ncreated:: "));
    assert!(hello.contains("ntbackup-id:: n-hello\nntbackup-hash:: "));
    // Tasks for Logseq; the app's [[link]] kept as it is.
    assert!(hello.ends_with("\n\n- TODO buy milk\n- DONE call Ana\n\nSee [[Plan: Q3?]]\n"));

    // The page is named after the title, so the link above resolves; the folder is a namespace.
    let plan = page(graph.path(), "Plan Q3.md");
    assert!(plan.starts_with("title:: Plan: Q3?\nfolder:: [[Work/Projects]]\n"));
    assert!(plan.contains("pinned:: true\nfavorite:: true\n"));
    assert!(
        plan.contains("```\n- [ ] code, not a task\n```"),
        "code blocks are left alone"
    );

    // Same title without case: the later note gets a suffix.
    assert!(page(graph.path(), "dup (2).md").starts_with("title:: dup (2)\n"));
    assert!(page(graph.path(), "Secret v2.md").contains("vault:: true\n"));
    assert!(!graph.path().join("pages/Trashed.md").exists());

    // Nothing is left behind by the atomic writes.
    assert!(fs::read_dir(graph.path().join("pages")).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with('.')));
}

#[test]
fn a_second_export_changes_nothing() {
    let b = backup();
    let r = resolve(&b, true);
    let graph = new_graph();
    sync(&b, &r, graph.path());
    let again = sync(&b, &r, graph.path());
    assert_eq!(
        again.count(|a| matches!(a, Action::Unchanged { .. })),
        r.notes.len(),
        "{:?}",
        again.actions
    );
}

#[test]
fn a_users_page_is_never_touched() {
    let b = backup();
    let r = resolve(&b, false);
    let graph = new_graph();
    fs::write(graph.path().join("pages/Hello.md"), "- my own page\n").unwrap();
    fs::write(graph.path().join("pages/other.md"), "title:: Plan: Q3?\n\n- mine too\n").unwrap();
    let report = sync(&b, &r, graph.path());

    assert_eq!(page(graph.path(), "Hello.md"), "- my own page\n");
    assert_eq!(page(graph.path(), "other.md"), "title:: Plan: Q3?\n\n- mine too\n");
    assert!(page(graph.path(), "Hello (notes-tech).md").starts_with("title:: Hello (notes-tech)\n"));
    assert!(page(graph.path(), "Plan Q3 (notes-tech).md").starts_with("title:: Plan: Q3? (notes-tech)\n"));
    assert!(report.warnings.iter().any(|w| w.contains("already named \"Hello\"")));
}

#[test]
fn an_edited_page_is_skipped_unless_asked() {
    let b = backup();
    let r = resolve(&b, false);
    let graph = new_graph();
    sync(&b, &r, graph.path());
    let edited = page(graph.path(), "Hello.md") + "- added in Logseq\n";
    fs::write(graph.path().join("pages/Hello.md"), &edited).unwrap();

    let report = sync(&b, &r, graph.path());
    assert!(report.actions.contains(&Action::SkipEdited {
        file: "Hello.md".into(),
        reason: "edited in Logseq"
    }));
    assert_eq!(page(graph.path(), "Hello.md"), edited);

    let forced = logseq::sync(
        &b,
        &r,
        graph.path(),
        Options {
            overwrite_edited: true,
            dry_run: false,
        },
    )
    .unwrap();
    assert!(forced.actions.contains(&Action::Update {
        file: "Hello.md".into()
    }));
    assert!(!page(graph.path(), "Hello.md").contains("added in Logseq"));
}

#[test]
fn a_note_gone_from_the_backup_loses_its_page_but_a_locked_one_keeps_it() {
    let b = backup();
    let graph = new_graph();
    sync(&b, &resolve(&b, true), graph.path());
    assert!(graph.path().join("pages/Secret v2.md").exists());

    // The vaults skipped this time: their pages stay.
    let report = sync(&b, &resolve(&b, false), graph.path());
    assert!(graph.path().join("pages/Secret v2.md").exists());
    assert_eq!(report.count(|a| matches!(a, Action::Delete { .. })), 0);

    // Hello deleted in the app.
    let mut fewer = b.clone();
    fewer.notes.retain(|n| n.id != "n-hello");
    let report = sync(&fewer, &resolve(&fewer, true), graph.path());
    assert!(report.actions.contains(&Action::Delete {
        file: "Hello.md".into()
    }));
    assert!(!graph.path().join("pages/Hello.md").exists());
}

#[test]
fn a_renamed_note_moves_its_page() {
    let b = backup();
    let graph = new_graph();
    sync(&b, &resolve(&b, false), graph.path());
    let mut renamed = b.clone();
    renamed.notes.iter_mut().find(|n| n.id == "n-hello").unwrap().title = "Hello again".into();
    let report = sync(&renamed, &resolve(&renamed, false), graph.path());
    assert!(report.actions.contains(&Action::Rename {
        from: "Hello.md".into(),
        to: "Hello again.md".into()
    }));
    assert!(!graph.path().join("pages/Hello.md").exists());
    assert!(page(graph.path(), "Hello again.md").starts_with("title:: Hello again\n"));
}

#[test]
fn dry_run_writes_nothing() {
    let b = backup();
    let graph = new_graph();
    let report = logseq::sync(
        &b,
        &resolve(&b, false),
        graph.path(),
        Options {
            overwrite_edited: false,
            dry_run: true,
        },
    )
    .unwrap();
    assert!(report.count(|a| matches!(a, Action::Create { .. })) > 0);
    assert_eq!(fs::read_dir(graph.path().join("pages")).unwrap().count(), 0);
}
