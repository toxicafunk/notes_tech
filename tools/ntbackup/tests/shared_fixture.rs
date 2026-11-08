//! A `.ntbackup` written by this tool, read back by the APP's own code in a JVM unit test
//! (`app/src/test/.../security/sauvegarde/FichierEcritParNtbackupTest.kt`). The reverse direction of the
//! parity vectors: if the two sides ever disagree on the header, the KDF, the secret's canonical form or
//! the AAD, one of the two suites goes red.
//!
//! The fixture is committed. `NTBACKUP_WRITE_FIXTURE=1 cargo test --test shared_fixture` rewrites it.

use ntbackup::{crypto, header};
use std::path::PathBuf;

/// Typed in lowercase, with spaces: the app must canonicalise it the same way.
pub const TYPED_CODE: &str = "k7qm t2xb 9h4n rc8v wz3p 6jde";
pub const CONTENT: &str = r#"{"format":"notes-tech-backup","version":1,"createdAt":1762600000000,"folders":[{"id":"inbox","name":"Inbox","parentId":null,"color":null,"icon":null,"createdAt":1762600000000,"updatedAt":1762600000000,"vault":null}],"notes":[{"id":"n-1","folderId":"inbox","title":"Écrit par ntbackup","content":"- [ ] relu par l'appli\n","sealed":null,"encV":0,"tags":["parité"],"pinned":false,"favorite":false,"archived":false,"trashedAt":null,"colorId":null,"createdAt":1762600000000,"updatedAt":1762600000000}]}"#;

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../app/src/test/resources/sauvegarde/ecrit-par-ntbackup.ntbackup")
}

#[test]
fn the_shared_fixture_opens() {
    let path = path();
    if std::env::var_os("NTBACKUP_WRITE_FIXTURE").is_some() || !path.exists() {
        // Fixed salt and nonce: the committed bytes do not change from one run to the next.
        let salt = [0x5au8; header::SALT_LEN];
        let nonce = [0xa5u8; header::NONCE_LEN];
        let head = header::Header::build(&salt, &nonce);
        let key = crypto::derive_key(
            &ntbackup::secret::secret_bytes(TYPED_CODE),
            &salt,
            header::ITERATIONS,
            header::MEMORY_KIB,
        )
        .unwrap();
        let mut file = head.to_vec();
        file.extend(crypto::seal(key.as_ref(), &nonce, CONTENT.as_bytes(), &head));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, file).unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    let json = ntbackup::decrypt(&bytes, "K7QM-T2XB-9H4N-RC8V-WZ3P-6JDE").unwrap();
    assert_eq!(&*json, CONTENT.as_bytes());
}
