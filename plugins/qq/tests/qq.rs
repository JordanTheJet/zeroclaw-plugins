use qq::qq::{
    auth_body, build_send_body, decode_gateway_frame, identify_frame, send_url, GatewayEvent,
    QQConfig,
};
use serde_json::{json, Value};

#[test]
fn config_to_identify_frame() {
    let cfg = QQConfig::from_json(
        r#"{"enabled":true,"app_id":"app","app_secret":"secret","proxy_url":"http://ignored"}"#,
    );
    assert!(cfg.is_configured());
    assert_eq!(auth_body(&cfg)["clientSecret"], "secret");
    let identify: Value = serde_json::from_str(&identify_frame("token")).unwrap();
    assert_eq!(identify["op"], 2);
    assert_eq!(identify["d"]["token"], "QQBot token");
}

#[test]
fn inbound_to_text_reply_round_trip() {
    let frame = json!({
        "op": 0,
        "s": 8,
        "t": "GROUP_AT_MESSAGE_CREATE",
        "d": {
            "id": "message-1",
            "content": "hello",
            "group_openid": "group-1",
            "author": { "member_openid": "user-1" }
        }
    });
    let decoded = decode_gateway_frame(&frame.to_string());
    let GatewayEvent::Message(message) = decoded.event else {
        panic!("expected message");
    };
    assert_eq!(message.reply_target, "group:group-1");
    assert_eq!(
        send_url(&message.reply_target).as_deref(),
        Some("https://api.sgroup.qq.com/v2/groups/group%2D1/messages")
    );
    let body = build_send_body("world", 9);
    assert_eq!(body["markdown"]["content"], "world");
}
