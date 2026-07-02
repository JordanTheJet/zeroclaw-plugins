//! Pure redaction core. No wit-bindgen or wasm dependency so it compiles and
//! tests on the host with a plain `cargo test`, while the wasm component reuses
//! the exact same logic through `lib.rs`.

use std::collections::HashMap;

pub const DEFAULT_REPLACEMENT: &str = "[REDACTED]";

/// Redaction policy resolved from the plugin's own config section.
pub struct RedactConfig {
    pub replacement: String,
    pub redact_emails: bool,
    pub patterns: Vec<String>,
}

impl RedactConfig {
    /// Build from the flat `string -> string` section the host injects. Absent
    /// or empty keys fall back to defaults, which is also what an unprivileged
    /// (no `config_read`) plugin sees.
    pub fn from_section(section: &HashMap<String, String>) -> Self {
        let replacement = section
            .get("replacement")
            .filter(|v| !v.is_empty())
            .cloned()
            .unwrap_or_else(|| DEFAULT_REPLACEMENT.to_string());
        let redact_emails = section
            .get("redact_emails")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(true);
        let patterns = section
            .get("patterns")
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            replacement,
            redact_emails,
            patterns,
        }
    }
}

/// Apply email masking, bearer/API-token masking, and configured literal
/// patterns. Returns the scrubbed text and the number of redactions made. The
/// matched secret values are never logged or returned in the count.
pub fn redact(text: &str, cfg: &RedactConfig) -> (String, usize) {
    let mut out = text.to_string();
    let mut count = 0usize;

    if cfg.redact_emails {
        let (replaced, n) = mask_emails(&out, &cfg.replacement);
        out = replaced;
        count += n;
    }

    let (replaced, n) = mask_tokens(&out, &cfg.replacement);
    out = replaced;
    count += n;

    for pat in &cfg.patterns {
        let occurrences = out.matches(pat.as_str()).count();
        if occurrences > 0 {
            out = out.replace(pat.as_str(), &cfg.replacement);
            count += occurrences;
        }
    }

    (out, count)
}

fn mask_emails(text: &str, replacement: &str) -> (String, usize) {
    let mut result = String::with_capacity(text.len());
    let mut count = 0usize;
    for token in split_keep_delims(text) {
        if is_email(token) {
            result.push_str(replacement);
            count += 1;
        } else {
            result.push_str(token);
        }
    }
    (result, count)
}

fn mask_tokens(text: &str, replacement: &str) -> (String, usize) {
    let mut result = String::with_capacity(text.len());
    let mut count = 0usize;
    for token in split_keep_delims(text) {
        if is_secret_token(token) {
            result.push_str(replacement);
            count += 1;
        } else {
            result.push_str(token);
        }
    }
    (result, count)
}

fn is_email(token: &str) -> bool {
    let at = match token.find('@') {
        Some(i) => i,
        None => return false,
    };
    let (local, domain_with_at) = token.split_at(at);
    let domain = &domain_with_at[1..];
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c))
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
}

fn is_secret_token(token: &str) -> bool {
    const PREFIXES: [&str; 4] = ["sk-", "ghp_", "AKIA", "xoxb-"];
    if PREFIXES.iter().any(|p| token.starts_with(p)) && token.len() >= 8 {
        return true;
    }
    if token.len() >= 20 {
        let alnum = token.chars().all(|c| c.is_ascii_alphanumeric());
        let has_digit = token.chars().any(|c| c.is_ascii_digit());
        let has_alpha = token.chars().any(|c| c.is_ascii_alphabetic());
        return alnum && has_digit && has_alpha;
    }
    false
}

/// Split into alternating word / delimiter chunks, preserving every character
/// so the rejoined output is byte-identical except at redacted spans.
fn split_keep_delims(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut in_word = false;
    for (i, c) in text.char_indices() {
        let is_word = c.is_ascii_alphanumeric() || "@._%+-".contains(c);
        if i == 0 {
            in_word = is_word;
            continue;
        }
        if is_word != in_word {
            chunks.push(&text[start..i]);
            start = i;
            in_word = is_word;
        }
    }
    if start < text.len() {
        chunks.push(&text[start..]);
    }
    chunks
}
