//! The backup secret: a generated code, or a phrase the user chose.
//!
//! Source of truth: `app/src/main/java/com/filestech/notes_tech/security/sauvegarde/SecretDeSauvegarde.kt`.
//!
//! One rule for creating and restoring: what has the shape of a code (24 signs of Crockford's base 32
//! once whitespace and dashes are dropped, case ignored, `O`→`0`, `I`/`L`→`1`) is used in its canonical
//! form; anything else is a phrase, used in Unicode NFC.
//!
//! ⚠️ This NFC applies to the BACKUP secret only. A vault passphrase is never normalised
//! (see [`crate::vault`]).

use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

pub const CODE_LENGTH: usize = 24;
const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Kotlin's `Char.isWhitespace()`: Java's `isWhitespace || isSpaceChar`, i.e. the space separators
/// (Zs, Zl, Zp) plus `\t \n \u000B \f \r` and `U+001C..U+001F`.
///
/// ⚠️ Not Rust's `char::is_whitespace`: that one includes `U+0085` and excludes `U+001C..U+001F`. A
/// code with one of those in it must canonicalise here exactly as it did on the phone.
pub fn kotlin_is_whitespace(c: char) -> bool {
    matches!(c,
        '\u{09}'..='\u{0D}' | '\u{1C}'..='\u{1F}' | ' ' | '\u{A0}' | '\u{1680}' |
        '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}')
}

/// Kotlin's `Char.uppercaseChar()`: Java's simple (one-to-one) case mapping. Where Rust's full mapping
/// gives several characters (`ß` → `SS`), the simple mapping leaves the character as it is.
fn simple_uppercase(c: char) -> char {
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

/// `SecretDeSauvegarde.codeCanonique`: `input` as a code, canonical, or `None` if it is not one.
pub fn canonical_code(input: &str) -> Option<String> {
    let mut signs = String::with_capacity(CODE_LENGTH);
    for c in input.chars() {
        if kotlin_is_whitespace(c) || c == '-' {
            continue;
        }
        match simple_uppercase(c) {
            'O' => signs.push('0'),
            'I' | 'L' => signs.push('1'),
            s if ALPHABET.contains(s) => signs.push(s),
            _ => return None,
        }
    }
    (signs.chars().count() == CODE_LENGTH).then_some(signs)
}

/// `SecretDeSauvegarde.octets`: the bytes the backup key is derived from.
pub fn secret_bytes(input: &str) -> Zeroizing<Vec<u8>> {
    match canonical_code(input) {
        Some(code) => Zeroizing::new(code.into_bytes()),
        None => Zeroizing::new(input.nfc().collect::<String>().into_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "K7QM-T2XB-9H4N-RC8V-WZ3P-6JDE";

    #[test]
    fn a_code_is_canonicalised() {
        let canonical = "K7QMT2XB9H4NRC8VWZ3P6JDE";
        assert_eq!(canonical_code(CODE).as_deref(), Some(canonical));
        assert_eq!(canonical_code(&CODE.to_lowercase()).as_deref(), Some(canonical));
        assert_eq!(
            canonical_code("k7qm t2xb 9h4n rc8v wz3p 6jde").as_deref(),
            Some(canonical)
        );
        // Copy-paste brings no-break and narrow no-break spaces along.
        assert_eq!(
            canonical_code("K7QM\u{A0}T2XB\u{202F}9H4N-RC8V-WZ3P-6JDE").as_deref(),
            Some(canonical)
        );
    }

    #[test]
    fn look_alikes_are_folded() {
        assert_eq!(
            canonical_code("OOOO-IIII-LLLL-oooo-iiii-llll").as_deref(),
            Some("000011111111000011111111")
        );
        // `ı` (dotless i) uppercases to `I` in Kotlin's simple mapping too.
        assert_eq!(
            canonical_code("ıııı-0000-0000-0000-0000-0000").as_deref(),
            Some("111100000000000000000000")
        );
    }

    #[test]
    fn kotlin_whitespace_not_rust_whitespace() {
        // U+001F is whitespace for Kotlin, not for Rust: still a code.
        assert!(canonical_code("K7QM\u{1F}T2XB-9H4N-RC8V-WZ3P-6JDE").is_some());
        // U+0085 is whitespace for Rust, not for Kotlin: not a code, a phrase.
        assert!(canonical_code("K7QM\u{85}T2XB-9H4N-RC8V-WZ3P-6JDE").is_none());
    }

    #[test]
    fn not_a_code() {
        assert!(canonical_code("K7QM-T2XB-9H4N-RC8V-WZ3P").is_none()); // 20 signs
        assert!(canonical_code("K7QM-T2XB-9H4N-RC8V-WZ3P-6JDEU").is_none()); // U is not in the alphabet
        assert!(canonical_code("correct horse battery staple").is_none());
    }

    #[test]
    fn a_phrase_is_nfc() {
        let nfd = "cafe\u{301} passe\u{301} e\u{301}te\u{301}";
        let nfc = "caf\u{e9} pass\u{e9} \u{e9}t\u{e9}";
        assert_eq!(&*secret_bytes(nfd), nfc.as_bytes());
        assert_eq!(&*secret_bytes(nfc), nfc.as_bytes());
    }
}
