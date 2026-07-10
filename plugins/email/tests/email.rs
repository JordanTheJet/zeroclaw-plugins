use email::email::{has_config, send_unavailable, SourceOnlyConfig, CHANNEL, PLUGIN_NAME};

#[test]
fn parses_common_self_handle_fields() {
    let cfg = SourceOnlyConfig::from_json(r#"{"bot_username":"botty"}"#);
    assert_eq!(cfg.self_handle.as_deref(), Some("botty"));
}

#[test]
fn empty_or_invalid_config_is_not_considered_configured() {
    assert!(!has_config("{}"));
    assert!(!has_config("not json"));
    assert!(!has_config(r#"{"enabled":true}"#));
}

#[test]
fn non_empty_config_is_detected() {
    assert!(has_config(r#"{"token":"secret"}"#));
}

#[test]
fn send_error_names_the_plugin_and_channel() {
    let err = send_unavailable();
    assert!(err.contains(PLUGIN_NAME));
    assert!(err.contains(CHANNEL));
}
