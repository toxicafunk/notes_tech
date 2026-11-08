mod common;

use assert_cmd::Command;
use common::*;
use std::fs;

fn fixture_on_disk() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("backup.ntbackup");
    fs::write(&path, fixture_file()).unwrap();
    (dir, path)
}

fn ntbackup() -> Command {
    Command::cargo_bin("ntbackup").unwrap()
}

#[test]
fn inspect_needs_no_secret() {
    let (_d, file) = fixture_on_disk();
    let out = ntbackup().arg("inspect").arg(&file).assert().success();
    assert!(String::from_utf8_lossy(&out.get_output().stdout).contains("Argon2id, t=3, m=65536 KiB, p=1"));
}

#[test]
fn decrypt_writes_the_json() {
    let (dir, file) = fixture_on_disk();
    let json = dir.path().join("content.json");
    ntbackup()
        .args(["decrypt", "--secret-stdin", "-o"])
        .arg(&json)
        .arg(&file)
        .write_stdin(format!("{}\n", CODE.to_lowercase()))
        .assert()
        .success();
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&json).unwrap()).unwrap();
    assert_eq!(value["format"], "notes-tech-backup");
}

#[test]
fn a_wrong_secret_exits_4() {
    let (_d, file) = fixture_on_disk();
    ntbackup()
        .args(["decrypt", "--secret-stdin", "-o", "-"])
        .arg(&file)
        .write_stdin("not the code at all\n")
        .assert()
        .code(4);
}

#[test]
fn a_refused_header_exits_3_before_asking_anything() {
    let (dir, _) = fixture_on_disk();
    let mut bytes = fixture_file().to_vec();
    bytes[16] = 2; // m = 128 MiB
    let forged = dir.path().join("forged.ntbackup");
    fs::write(&forged, bytes).unwrap();
    // No stdin given: it must fail on the header, not wait for a secret.
    ntbackup()
        .args(["decrypt", "--secret-stdin", "-o", "-"])
        .arg(&forged)
        .assert()
        .code(3);

    let not_one = dir.path().join("notes.txt");
    fs::write(&not_one, "hello").unwrap();
    ntbackup().arg("inspect").arg(&not_one).assert().code(3);
}

#[test]
fn export_plain_and_logseq() {
    let (dir, file) = fixture_on_disk();
    let out = dir.path().join("md");
    ntbackup()
        .args(["export", "--secret-stdin", "--skip-vaults", "-o"])
        .arg(&out)
        .arg(&file)
        .write_stdin(format!("{CODE}\n"))
        .assert()
        .success();
    assert!(out.join("inbox/Hello.md").exists());

    let graph = new_graph();
    let run = ntbackup()
        .args(["export", "--logseq", "--secret-stdin", "--skip-vaults", "-o"])
        .arg(graph.path())
        .arg(&file)
        .write_stdin(format!("{CODE}\n"))
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&run.get_output().stdout).into_owned();
    assert!(stdout.contains("2 notes of locked vaults left out"), "{stdout}");
    assert!(graph.path().join("pages/Hello.md").exists());
}

#[test]
fn logseq_only_flags_need_logseq() {
    let (_d, file) = fixture_on_disk();
    // A usage error: clap's exit code 2, distinct from a wrong secret (4).
    let run = ntbackup()
        .args(["export", "--dry-run", "-o", "x"])
        .arg(&file)
        .assert()
        .code(2);
    assert!(String::from_utf8_lossy(&run.get_output().stderr).contains("--logseq"));
}
