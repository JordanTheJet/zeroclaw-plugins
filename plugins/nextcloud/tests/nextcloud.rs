//! Host tests for the pure Nextcloud Talk core — the same signature/parse/send
//! logic the wasm component runs, exercised with plain `cargo test` (no token,
//! no network, no wasm).

use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;

use nextcloud::nextcloud::{
    build_send_body, chat_url, parse_webhook, truncate_to_nc_limit, verify_signature,
    NextcloudConfig, NC_MAX_MESSAGE_LENGTH,
};

type HmacSha256 = Hmac<Sha256>;

fn sign(secret: &str, random: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(random.as_bytes());
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn as2_body(actor_id: &str, actor_name: &str, token: &str, text: &str) -> String {
    let content = serde_json::to_string(&json!({ "message": text, "parameters": [] })).unwrap();
    json!({
        "type": "Create",
        "actor": { "type": "Person", "id": actor_id, "name": actor_name },
        "object": { "type": "Note", "name": "message", "id": "42", "content": content },
        "target": { "type": "Collection", "id": token, "name": "Room" }
    })
    .to_string()
}

fn cfg(secret: Option<&str>, bot_name: Option<&str>) -> NextcloudConfig {
    NextcloudConfig {
        base_url: "https://cloud.example.com".into(),
        app_token: Some("app-token".into()),
        webhook_secret: secret.map(str::to_string),
        bot_name: bot_name.map(str::to_string),
    }
}

#[test]
fn end_to_end_verified_webhook_maps_message() {
    let body = as2_body("users/alice", "Alice", "room-token-123", "hello from talk");
    let c = cfg(Some("shared-secret"), None);
    let random = "nonce-xyz";
    let sig = sign("shared-secret", random, body.as_bytes());
    let headers = vec![
        ("x-nextcloud-talk-random".to_string(), random.to_string()),
        ("x-nextcloud-talk-signature".to_string(), sig),
    ];

    let msgs = parse_webhook(&headers, body.as_bytes(), &c).expect("valid signature accepted");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].sender, "alice");
    assert_eq!(msgs[0].reply_target, "room-token-123");
    assert_eq!(msgs[0].content, "hello from talk");
    assert_eq!(msgs[0].id, "42");
}

#[test]
fn signature_prefix_variants_are_accepted() {
    let body = as2_body("users/a", "A", "r", "hi");
    let sig = sign("s", "n", body.as_bytes());
    // bare hex, `sha256=` prefixed, and uppercase all verify.
    assert!(verify_signature("s", "n", body.as_bytes(), &sig));
    assert!(verify_signature(
        "s",
        "n",
        body.as_bytes(),
        &format!("sha256={sig}")
    ));
    assert!(verify_signature(
        "s",
        "n",
        body.as_bytes(),
        &sig.to_ascii_uppercase()
    ));
}

#[test]
fn tampered_body_is_rejected_401() {
    let body = as2_body("users/alice", "Alice", "room", "original");
    let c = cfg(Some("shared-secret"), None);
    let sig = sign("shared-secret", "n", body.as_bytes());
    let headers = vec![
        ("x-nextcloud-talk-random".to_string(), "n".to_string()),
        ("x-nextcloud-talk-signature".to_string(), sig),
    ];
    let tampered = as2_body("users/alice", "Alice", "room", "tampered");
    assert!(parse_webhook(&headers, tampered.as_bytes(), &c).is_err());
}

#[test]
fn missing_secret_accepts_unsigned() {
    let body = as2_body("users/bob", "Bob", "room-2", "unsigned");
    let c = cfg(None, None);
    let msgs = parse_webhook(&[], body.as_bytes(), &c).expect("unsigned accepted with no secret");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].sender, "bob");
}

#[test]
fn bot_authored_messages_are_dropped() {
    let c = cfg(None, Some("mybot"));
    // configured bot name
    let by_name = as2_body("users/x", "MyBot", "room", "loop");
    assert!(parse_webhook(&[], by_name.as_bytes(), &c)
        .unwrap()
        .is_empty());
    // bots/ id prefix
    let by_prefix = as2_body("bots/mybot", "MyBot", "room", "loop");
    assert!(parse_webhook(&[], by_prefix.as_bytes(), &c)
        .unwrap()
        .is_empty());
}

#[test]
fn send_url_and_body_match_native_ocs_shape() {
    assert_eq!(
        chat_url("https://cloud.example.com", "room-token-123"),
        "https://cloud.example.com/ocs/v2.php/apps/spreed/api/v1/chat/room-token-123?format=json"
    );
    assert_eq!(
        build_send_body("reply text"),
        json!({ "message": "reply text" })
    );
}

#[test]
fn outbound_text_truncated_to_ocs_limit() {
    let long = "x".repeat(NC_MAX_MESSAGE_LENGTH + 500);
    assert_eq!(
        truncate_to_nc_limit(&long).chars().count(),
        NC_MAX_MESSAGE_LENGTH
    );
}

#[test]
fn config_reads_native_section_fields() {
    let c = NextcloudConfig::from_json(
        r#"{"base_url":"https://cloud.example.com/","app_token":"tok","webhook_secret":"sec","bot_name":"Bot"}"#,
    );
    assert_eq!(c.base_url(), "https://cloud.example.com");
    assert_eq!(c.app_token(), "tok");
    assert_eq!(c.webhook_secret(), "sec");
    assert_eq!(c.bot_name(), "bot");
}
