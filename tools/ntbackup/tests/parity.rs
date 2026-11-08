//! The vault crypto against the app's parity vectors: produced by RUNNING the Flutter app's Dart code,
//! cross-checked against reference C (argon2-cffi) and OpenSSL, and replayed by the Kotlin tests
//! (`PariteCoffreAvecFlutterTest`). Same files, read in place: a vector cannot drift between the two.

use ntbackup::{crypto, vault};
use std::path::PathBuf;

fn vectors(name: &str) -> Vec<Vec<String>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../app/src/test/resources/parite")
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect()
}

fn h(s: &str) -> Vec<u8> {
    hex::decode(s).unwrap()
}

#[test]
fn argon2id_matches_dart() {
    let cases = vectors("coffre_argon2.tsv");
    assert_eq!(cases.len(), 16);
    for c in cases {
        let (t, m) = (c[4].parse().unwrap(), c[5].parse().unwrap());
        let key = crypto::derive_key(&h(&c[2]), &h(&c[3]), t, m).unwrap();
        assert_eq!(hex::encode(key.as_ref()), c[6], "{}/{}", c[0], c[1]);
    }
}

#[test]
fn verifier_matches() {
    for c in vectors("coffre_verifier.tsv") {
        assert_eq!(hex::encode(crypto::verifier_for(&h(&c[0]))), c[1]);
        assert!(crypto::matches_verifier(&h(&c[0]), &h(&c[1])));
        let mut neighbour = h(&c[0]);
        neighbour[0] ^= 1;
        assert!(!crypto::matches_verifier(&neighbour, &h(&c[1])));
    }
}

#[test]
fn aes_gcm_matches_dart_byte_for_byte() {
    for c in vectors("coffre_wrap.tsv") {
        let (key, iv, plain, aad, wrapped) = (h(&c[1]), h(&c[2]), h(&c[3]), h(&c[4]), &c[5]);
        assert_eq!(&hex::encode(crypto::seal(&key, &iv, &plain, &aad)), wrapped, "{}", c[0]);
        assert_eq!(*crypto::open(&key, &iv, &h(wrapped), &aad).unwrap(), plain, "{}", c[0]);
    }
}

#[test]
fn note_envelopes_open() {
    for c in vectors("coffre_note.tsv") {
        let enc_v: i64 = c[1].parse().unwrap();
        let id = String::from_utf8(h(&c[3])).unwrap();
        let title = String::from_utf8(h(&c[4])).unwrap();
        let content = String::from_utf8(h(&c[5])).unwrap();
        // encV 1: the title stays in clear, in the column.
        let clear_title = if enc_v == 1 { title.as_str() } else { "" };
        let opened = vault::open_note(&h(&c[2]), &id, &h(&c[6]), enc_v, clear_title).unwrap();
        assert_eq!(opened.title, title, "{}", c[0]);
        assert_eq!(opened.content, content, "{}", c[0]);
        // The AAD binds the envelope to its note.
        assert!(vault::open_note(&h(&c[2]), "another-note", &h(&c[6]), enc_v, "").is_err());
    }
}

#[test]
fn a_whole_passphrase_vault_opens_from_its_passphrase_alone() {
    let c = vectors("coffre_complet.tsv").remove(0);
    let passphrase = String::from_utf8(h(&c[0])).unwrap();
    let folder_id = String::from_utf8(h(&c[1])).unwrap();
    let key = vault::unlock_passphrase(&passphrase, &folder_id, &h(&c[2]), &h(&c[3]), &h(&c[4]), &h(&c[5])).unwrap();
    assert_eq!(hex::encode(&*key), c[7]);

    let note_id = String::from_utf8(h(&c[8])).unwrap();
    let opened = vault::open_note(&key, &note_id, &h(&c[11]), 2, "").unwrap();
    assert_eq!(opened.title, String::from_utf8(h(&c[9])).unwrap());
    assert_eq!(opened.content, String::from_utf8(h(&c[10])).unwrap());

    assert_eq!(
        vault::unlock_passphrase(
            "not the passphrase",
            &folder_id,
            &h(&c[2]),
            &h(&c[3]),
            &h(&c[4]),
            &h(&c[5])
        )
        .unwrap_err(),
        vault::VaultError::WrongPassphrase
    );
}
