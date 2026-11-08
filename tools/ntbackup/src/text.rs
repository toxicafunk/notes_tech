//! String semantics the app's export relies on, reproduced exactly.
//!
//! Source of truth: `app/src/main/java/com/filestech/notes_tech/core/text/DartTextSemantics.kt` and the
//! JVM's own string rules (UTF-16 lengths, simple case mapping). See `docs/09-VECTEURS-DE-PARITE.md`
//! for why "looks the same" is not enough.

/// `DartTextSemantics.isDartTrimmable`.
pub fn is_dart_trimmable(c: char) -> bool {
    matches!(c,
        '\u{09}'..='\u{0D}' | ' ' | '\u{85}' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' |
        '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
}

/// `DartTextSemantics.trim`.
pub fn dart_trim(s: &str) -> &str {
    s.trim_matches(is_dart_trimmable)
}

/// One character of `WHITESPACE_CLASS`: Java's `\s` (`[ \t\n\x0B\f\r]`, ASCII only) plus the listed
/// Unicode spaces.
pub fn is_whitespace_class(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `DartTextSemantics.WHITESPACE.replace(s, " ")`: every run of whitespace becomes one space.
pub fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_run = false;
    for c in s.chars() {
        if is_whitespace_class(c) {
            if !in_run {
                out.push(' ');
            }
            in_run = true;
        } else {
            out.push(c);
            in_run = false;
        }
    }
    out
}

/// `DartTextSemantics.lowercase`: code point by code point, with Java's SIMPLE case mapping.
pub fn java_lowercase(s: &str) -> String {
    s.chars()
        .map(|c| {
            let mut low = c.to_lowercase();
            match (low.next(), low.next()) {
                (Some(l), None) => l,
                // `İ` (U+0130) is the one character whose full mapping has two code points; Java's
                // simple mapping gives `i`.
                (Some(l), Some(_)) => l,
                _ => c,
            }
        })
        .collect()
}

/// `s.length` on the JVM: UTF-16 code units.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `NoteMarkdown.tronque`: at most `max` UTF-16 units, never splitting a surrogate pair.
pub fn truncate_utf16(s: &str, max: usize) -> &str {
    if utf16_len(s) <= max {
        return s;
    }
    let mut units = 0;
    for (byte_index, c) in s.char_indices() {
        if units + c.len_utf16() > max {
            return &s[..byte_index];
        }
        units += c.len_utf16();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_follows_dart() {
        assert_eq!(dart_trim("\u{85}\u{FEFF} a b \u{202F}"), "a b");
        // U+001F is NOT trimmed by Dart (Kotlin's own `trim()` would remove it).
        assert_eq!(dart_trim("\u{1F}a"), "\u{1F}a");
    }

    #[test]
    fn collapse() {
        assert_eq!(collapse_whitespace("a \t\u{A0} b\u{3000}c"), "a b c");
        // U+0085 is not in the class.
        assert_eq!(collapse_whitespace("a\u{85}b"), "a\u{85}b");
    }

    #[test]
    fn truncation_counts_utf16_and_keeps_pairs() {
        let s = "a".repeat(79) + "😀" + "b";
        assert_eq!(truncate_utf16(&s, 80), "a".repeat(79));
        let s = "a".repeat(78) + "😀" + "b";
        assert_eq!(truncate_utf16(&s, 80), "a".repeat(78) + "😀");
        assert_eq!(truncate_utf16("short", 80), "short");
    }

    #[test]
    fn lowercase_is_simple() {
        assert_eq!(java_lowercase("ÉCOLE İ"), "école i");
    }
}
