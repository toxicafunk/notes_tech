//! A backup built the way the app writes one, with every case the export has to handle.
#![allow(dead_code)]

use ntbackup::{crypto, header, vault};
use serde_json::{json, Value};
use std::sync::OnceLock;

pub const CODE: &str = "K7QM-T2XB-9H4N-RC8V-WZ3P-6JDE";
pub const VAULT_PASSPHRASE: &str = "ma passphrase de coffre 2026";
pub const SECRET_FOLDER: &str = "f-secret";

const T0: i64 = 1_762_600_000_000;

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn note(id: &str, folder: &str, title: &str, content: &str, n: i64) -> Value {
    json!({
        "id": id, "folderId": folder, "title": title, "content": content, "sealed": null, "encV": 0,
        "tags": [], "pinned": false, "favorite": false, "archived": false, "trashedAt": null,
        "colorId": null, "createdAt": T0 + n * 1000, "updatedAt": T0 + n * 2000
    })
}

fn folder(id: &str, name: &str, parent: Option<&str>) -> Value {
    json!({
        "id": id, "name": name, "parentId": parent, "color": null, "icon": null,
        "createdAt": T0, "updatedAt": T0, "vault": null
    })
}

/// The JSON content, and the passphrase vault's key, so tests can seal more notes.
pub fn content() -> &'static Value {
    static CONTENT: OnceLock<Value> = OnceLock::new();
    CONTENT.get_or_init(|| {
        // A passphrase vault, wrapped as `FolderVaultService.createPassphraseVault` does.
        let secret_key = [0x42u8; 32];
        let salt = [0x0fu8; 16];
        let iv = [0x01u8; 12];
        let derived = crypto::derive_key(VAULT_PASSPHRASE.as_bytes(), &salt, 3, 65536).unwrap();
        let wrapped = crypto::seal(derived.as_ref(), &iv, &secret_key, SECRET_FOLDER.as_bytes());
        let mut secret_folder = folder(SECRET_FOLDER, "Secret", None);
        secret_folder["vault"] = json!({
            "mode": "passphrase", "salt": b64(&salt), "wrappedKey": b64(&wrapped), "iv": b64(&iv),
            "verifier": b64(&crypto::verifier_for(&secret_key))
        });

        // A PIN vault saved open: its key travels.
        let pin_key = [0x24u8; 32];
        let mut pin_folder = folder("f-pin", "PinBox", None);
        pin_folder["vault"] = json!({ "mode": "pin", "folderKey": b64(&pin_key) });

        let mut notes = vec![
            {
                let mut n = note(
                    "n-hello",
                    "inbox",
                    "Hello",
                    "- [ ] buy milk\n- [x] call Ana\n\nSee [[Plan: Q3?]]",
                    1,
                );
                n["tags"] = json!(["work", "a, b"]);
                n
            },
            {
                let mut n = note(
                    "n-plan",
                    "f-projects",
                    "Plan: Q3?",
                    "# Plan\n\n```\n- [ ] code, not a task\n```\n",
                    2,
                );
                n["pinned"] = json!(true);
                n["favorite"] = json!(true);
                n
            },
            note("n-dup1", "inbox", "Dup", "first", 3),
            note("n-dup2", "inbox", "dup", "second", 4),
            {
                let mut n = note("n-trash", "inbox", "Trashed", "gone", 5);
                n["trashedAt"] = json!(T0 + 99_000);
                n
            },
            note("n-con", "f-con", "CON.txt", "reserved", 6),
            note("n-bidi", "inbox", "note\u{202E}gpj", "spoof", 7),
            note("n-cycle", "f-cycle-a", "In a cycle", "loop", 8),
            note("n-orphan", "f-missing", "No folder row", "orphan", 9),
            note("n-untitled", "inbox", "   ", "no title", 10),
        ];
        // Vault notes: one of each envelope format, and one in the PIN vault.
        let mut v1 = note("n-secret-v1", SECRET_FOLDER, "Old secret title", "", 11);
        v1["sealed"] = json!(b64(&vault::seal_note(
            &secret_key,
            &[1; 12],
            "n-secret-v1",
            "",
            "secret v1 body",
            1
        )));
        v1["encV"] = json!(1);
        let mut v2 = note("n-secret-v2", SECRET_FOLDER, "", "", 12);
        v2["sealed"] = json!(b64(&vault::seal_note(
            &secret_key,
            &[2; 12],
            "n-secret-v2",
            "Secret v2",
            "secret v2 body",
            2
        )));
        v2["encV"] = json!(2);
        let mut pin = note("n-pin", "f-pin", "", "", 13);
        pin["sealed"] = json!(b64(&vault::seal_note(
            &pin_key, &[3; 12], "n-pin", "Pin note", "pin body", 2
        )));
        pin["encV"] = json!(2);
        notes.extend([v1, v2, pin]);

        json!({
            "format": "notes-tech-backup", "version": 1, "createdAt": T0 + 100_000,
            "folders": [
                folder("inbox", "Inbox", None),
                folder("f-work", "Work", None),
                folder("f-projects", "Projects", Some("f-work")),
                folder("f-con", "CON", None),
                folder("f-cycle-a", "Cycle A", Some("f-cycle-b")),
                folder("f-cycle-b", "Cycle B", Some("f-cycle-a")),
                secret_folder,
                pin_folder,
            ],
            "notes": notes,
        })
    })
}

/// Encrypts `json` into a `.ntbackup`, as `FichierDeSauvegarde.ecrire` does.
pub fn encrypt(json: &[u8], secret: &str) -> Vec<u8> {
    let salt: [u8; 16] = rand::random();
    let nonce: [u8; 12] = rand::random();
    let head = header::Header::build(&salt, &nonce);
    let key = crypto::derive_key(&ntbackup::secret::secret_bytes(secret), &salt, 3, 65536).unwrap();
    let mut file = head.to_vec();
    file.extend(crypto::seal(key.as_ref(), &nonce, json, &head));
    file
}

/// The fixture backup, encrypted under [`CODE`].
pub fn fixture_file() -> &'static [u8] {
    static FILE: OnceLock<Vec<u8>> = OnceLock::new();
    FILE.get_or_init(|| encrypt(serde_json::to_vec(content()).unwrap().as_slice(), CODE))
}

/// An empty Logseq graph.
pub fn new_graph() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("pages")).unwrap();
    std::fs::create_dir_all(dir.path().join("logseq")).unwrap();
    std::fs::write(dir.path().join("logseq/config.edn"), "{}").unwrap();
    dir
}
