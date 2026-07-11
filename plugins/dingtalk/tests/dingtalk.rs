use dingtalk::dingtalk::{
    build_reply_body, connection_url, decode_frame, registration_body, DingTalkConfig,
    GatewayResponse,
};
use serde_json::{json, Value};

#[test]
fn registration_to_connection_url() {
    let cfg =
        DingTalkConfig::from_json(r#"{"enabled":true,"client_id":"app","client_secret":"secret"}"#);
    assert!(cfg.is_configured());
    assert_eq!(registration_body(&cfg)["clientId"], "app");
    let gateway = GatewayResponse {
        endpoint: "wss://gateway.example/stream".into(),
        ticket: "ticket/1".into(),
    };
    assert_eq!(
        connection_url(&gateway).as_deref(),
        Some("wss://gateway.example/stream?ticket=ticket%2F1")
    );
}

#[test]
fn callback_to_session_reply_round_trip() {
    let frame = json!({
        "type": "CALLBACK",
        "headers": { "messageId": "message-1" },
        "data": {
            "conversationType": 2,
            "conversationId": "group-1",
            "senderStaffId": "user-1",
            "sessionWebhook": "https://example.invalid/session",
            "text": { "content": "hello" }
        }
    });
    let outcome = decode_frame(&frame.to_string());
    let ack: Value = serde_json::from_str(outcome.response.as_deref().unwrap()).unwrap();
    assert_eq!(ack["headers"]["messageId"], "message-1");
    let inbound = outcome.inbound.unwrap();
    assert_eq!(inbound.reply_target, "group-1");
    let webhook = outcome.session_webhook.unwrap();
    assert_eq!(webhook.url, "https://example.invalid/session");
    let reply = build_reply_body("world", Some("Subject"));
    assert_eq!(reply["markdown"]["title"], "Subject");
    assert_eq!(reply["markdown"]["text"], "world");
}
