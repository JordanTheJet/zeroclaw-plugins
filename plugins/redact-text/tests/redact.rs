//! Integration test for the redaction core, exercised exactly as the wasm
//! `execute` entry point drives it: build a `RedactConfig` from a flat config
//! section, then redact. This runs on the host with a plain `cargo test` and
//! covers the same code path the component runs inside the wasmtime host.

use std::collections::HashMap;

use redact_text::redact::{redact, RedactConfig, DEFAULT_REPLACEMENT};

fn section(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn masks_email_by_default() {
    let cfg = RedactConfig::from_section(&HashMap::new());
    let (out, n) = redact("ping bob@corp.com now", &cfg);
    assert_eq!(n, 1);
    assert!(!out.contains("bob@corp.com"));
    assert!(out.contains(DEFAULT_REPLACEMENT));
}

#[test]
fn masks_known_token_prefixes() {
    let cfg = RedactConfig::from_section(&HashMap::new());
    for token in [
        "sk-abcdef0123456789",
        "ghp_abcd1234efgh5678",
        "xoxb-1-2-3abcXYZ",
    ] {
        let (out, n) = redact(&format!("key {token} end"), &cfg);
        assert!(n >= 1, "{token} should redact");
        assert!(!out.contains(token), "{token} should be masked");
    }
}

#[test]
fn masks_high_entropy_run() {
    let cfg = RedactConfig::from_section(&HashMap::new());
    let (out, n) = redact("token AKIA1234567890ABCDEF42 trailing", &cfg);
    assert_eq!(n, 1);
    assert!(!out.contains("AKIA1234567890ABCDEF42"));
}

#[test]
fn applies_configured_replacement_and_patterns() {
    let cfg = RedactConfig::from_section(&section(&[
        ("replacement", "<X>"),
        ("patterns", "project-zeus, internal-codename"),
    ]));
    let (out, n) = redact("re: project-zeus and internal-codename", &cfg);
    assert_eq!(n, 2);
    assert!(!out.contains("project-zeus"));
    assert!(!out.contains("internal-codename"));
    assert_eq!(out.matches("<X>").count(), 2);
}

#[test]
fn email_masking_disabled_by_config() {
    let cfg = RedactConfig::from_section(&section(&[("redact_emails", "false")]));
    let (out, n) = redact("mail a@b.com", &cfg);
    assert_eq!(n, 0);
    assert!(out.contains("a@b.com"));
}

#[test]
fn empty_config_is_the_unprivileged_jail_case() {
    // A plugin without the config_read permission receives an empty section.
    // It must still run, falling back to safe defaults.
    let cfg = RedactConfig::from_section(&HashMap::new());
    assert_eq!(cfg.replacement, DEFAULT_REPLACEMENT);
    assert!(cfg.redact_emails);
    assert!(cfg.patterns.is_empty());
}

#[test]
fn non_secret_text_passes_through_unchanged() {
    let cfg = RedactConfig::from_section(&HashMap::new());
    let input = "the quick brown fox jumps over 13 lazy dogs";
    let (out, n) = redact(input, &cfg);
    assert_eq!(n, 0);
    assert_eq!(out, input);
}
