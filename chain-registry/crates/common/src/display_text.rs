// crates/common/src/display_text.rs
// ECA-17 / eco-adv-07: bounded, terminal-safe rendering of untrusted strings.
//
// Package names, reject reasons, finding titles and descriptions, publisher
// keys and every other string that arrives from a node, an SSE stream or a
// package archive is attacker-influenced. Printing it raw lets it smuggle
// terminal control sequences (OSC 52 clipboard writes, OSC 8 hyperlinks,
// title rewrites), carriage-return overwrites of a "REJECTED" label, or
// bidirectional overrides that visually reorder text. `DisplayText` is the
// one place that neutralises them before a string reaches a terminal, a TUI
// cell or a log line.

use serde::{Serialize, Serializer};
use std::fmt;

/// Default upper bound, in Unicode scalar values, for one rendered field.
pub const DEFAULT_DISPLAY_MAX_CHARS: usize = 512;

/// Suffix appended when a value was truncated.
pub const TRUNCATION_MARKER: char = '\u{2026}';

/// A sanitised, length-bounded string that is safe to print to a terminal.
///
/// Construction removes C0 controls other than tab (CR, LF and the Unicode
/// line/paragraph separators become a space so words do not run together),
/// DEL, C1 controls (which include the 8-bit CSI/OSC introducers), ESC, and
/// bidirectional embedding/override/isolate/mark characters. The result is
/// then truncated to the requested number of Unicode scalar values.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct DisplayText(String);

impl DisplayText {
    /// Sanitise for a terminal with the default bound.
    pub fn for_terminal(input: &str) -> Self {
        Self::bounded(input, DEFAULT_DISPLAY_MAX_CHARS)
    }

    /// Sanitise for a terminal with an explicit bound (`max_chars >= 1`).
    pub fn bounded(input: &str, max_chars: usize) -> Self {
        Self(sanitize_for_terminal(input, max_chars))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for DisplayText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for DisplayText {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Serialize for DisplayText {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Characters that change visual ordering without being visible themselves:
/// ARABIC LETTER MARK, LRM, RLM, the embeddings/overrides LRE..RLO and the
/// isolates LRI..PDI.
fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// Characters rendered as a single space instead of being dropped.
fn is_line_break(ch: char) -> bool {
    matches!(
        ch,
        '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{2028}' | '\u{2029}'
    )
}

/// Remove terminal control and bidi characters and bound the length.
///
/// `max_chars` counts Unicode scalar values of the output, including the
/// truncation marker. A bound of zero is treated as one.
pub fn sanitize_for_terminal(input: &str, max_chars: usize) -> String {
    let max_chars = max_chars.max(1);
    let mut out = String::with_capacity(input.len().min(max_chars * 4));
    let mut written = 0usize;
    let mut truncated = false;
    for ch in input.chars() {
        let mapped = if ch == '\t' {
            Some('\t')
        } else if is_line_break(ch) {
            Some(' ')
        } else if ch.is_control() || is_bidi_control(ch) {
            // `char::is_control` covers C0 (incl. ESC), DEL and C1.
            None
        } else {
            Some(ch)
        };
        let Some(mapped) = mapped else { continue };
        if written == max_chars {
            truncated = true;
            break;
        }
        out.push(mapped);
        written += 1;
    }
    if truncated {
        // Replace the last kept character with the marker so the output never
        // exceeds `max_chars` scalar values.
        out.pop();
        out.push(TRUNCATION_MARKER);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_osc52_clipboard_write() {
        // eco-adv-07: ESC ] 52 ; c ; <base64> BEL
        let hostile = "reason\u{1b}]52;c;Y3VybCBldmlsLnNoIHwgc2g=\u{07}tail";
        let clean = sanitize_for_terminal(hostile, 256);
        assert!(!clean.contains('\u{1b}'));
        assert!(!clean.contains('\u{07}'));
        assert_eq!(clean, "reason]52;c;Y3VybCBldmlsLnNoIHwgc2g=tail");
    }

    #[test]
    fn carriage_return_cannot_overwrite_label() {
        let hostile = "REJECTED: malware\r APPROVED       ";
        let clean = sanitize_for_terminal(hostile, 256);
        assert!(!clean.contains('\r'));
        assert!(clean.starts_with("REJECTED: malware"));
    }

    #[test]
    fn strips_bidi_overrides_and_isolates() {
        let hostile = "evil\u{202E}gpj.exe\u{2066}x\u{2069}\u{200F}";
        let clean = sanitize_for_terminal(hostile, 256);
        assert_eq!(clean, "evilgpj.exex");
    }

    #[test]
    fn strips_c1_csi_and_del() {
        let hostile = "a\u{9b}31mb\u{7f}c\u{85}d";
        assert_eq!(sanitize_for_terminal(hostile, 256), "a31mbcd");
    }

    #[test]
    fn keeps_tab_and_maps_newlines_to_space() {
        assert_eq!(sanitize_for_terminal("a\tb\nc\u{2028}d", 256), "a\tb c d");
    }

    #[test]
    fn truncates_by_scalar_value_with_marker() {
        let clean = sanitize_for_terminal("abcdefgh", 4);
        assert_eq!(clean, "abc\u{2026}");
        assert_eq!(clean.chars().count(), 4);
        assert_eq!(sanitize_for_terminal("abcd", 4), "abcd");
        // Multi-byte input is never split inside a scalar value.
        assert_eq!(sanitize_for_terminal("ééééé", 3), "éé\u{2026}");
    }

    #[test]
    fn display_text_serializes_sanitized() {
        let text = DisplayText::for_terminal("x\u{1b}[2Jy");
        assert_eq!(text.as_str(), "x[2Jy");
        assert_eq!(serde_json::to_string(&text).unwrap(), "\"x[2Jy\"");
        assert_eq!(text.to_string(), "x[2Jy");
    }
}
