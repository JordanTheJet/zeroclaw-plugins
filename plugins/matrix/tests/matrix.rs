use matrix::matrix::{build_send_body, parse_sync, send_url, sync_url, whoami_url, MatrixConfig};
use serde_json::json;

#[test]
fn access_token_config_is_runnable() {
    let cfg = MatrixConfig::from_json(
        r#"{"enabled":true,"homeserver":"https://matrix.example","access_token":"token","user_id":"@bot:example"}"#,
    );
    assert!(cfg.is_configured());
    assert_eq!(cfg.user_id(), Some("@bot:example"));
    assert_eq!(
        whoami_url(cfg.homeserver()),
        "https://matrix.example/_matrix/client/v3/account/whoami"
    );
    assert_eq!(
        sync_url(cfg.homeserver(), Some("next/1")),
        "https://matrix.example/_matrix/client/v3/sync?timeout=0&since=next%2F1"
    );
    assert!(send_url(cfg.homeserver(), "!room:example", 1).contains("%21room%3Aexample"));
}

#[test]
fn sync_to_threaded_reply_round_trip() {
    let cfg = MatrixConfig::from_json(
        r#"{"homeserver":"https://matrix.example","access_token":"token","reply_in_thread":true}"#,
    );
    let sync = json!({
        "next_batch": "s2",
        "rooms": { "join": { "!room:example": { "timeline": { "events": [{
            "type": "m.room.message",
            "event_id": "$event",
            "sender": "@user:example",
            "origin_server_ts": 1234,
            "content": { "msgtype": "m.text", "body": "hello" }
        }] } } } }
    });
    let batch = parse_sync(&sync, &cfg, "@bot:example");
    assert_eq!(batch.next_batch, "s2");
    assert_eq!(batch.messages.len(), 1);
    let inbound = &batch.messages[0];
    assert_eq!(inbound.reply_target, "!room:example");
    let reply = build_send_body("world", inbound.thread_ts.as_deref(), cfg.reply_in_thread);
    assert_eq!(reply["m.relates_to"]["event_id"], "$event");
}
