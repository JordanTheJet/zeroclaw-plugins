use serde_json::{json, Value};
use wecom_ws::wecom_ws::{
    access_decision, build_ping_frame, build_proactive_frame, build_respond_frame,
    build_subscribe_frame, decode_server_frame, markdown_chunks, split_stream_content,
    AccessDecision, MessageIdCache, ServerFrame, StreamMode, WeComEvent, WeComWsConfig,
    MARKDOWN_CHUNK_BYTES, MARKDOWN_MAX_BYTES,
};

fn config() -> WeComWsConfig {
    WeComWsConfig::from_json(
        r#"{
            "enabled": true,
            "bot_id": "bot-1",
            "secret": "secret-1",
            "allowed_users": ["user-1"],
            "allowed_groups": ["group-1"],
            "bot_name": "@helper",
            "stream_mode": "partial"
        }"#,
    )
    .expect("valid fixture config")
}

fn callback(body: Value, request_id: &str) -> String {
    json!({
        "cmd": "aibot_msg_callback",
        "headers": { "req_id": request_id },
        "body": body,
    })
    .to_string()
}

#[test]
fn config_uses_native_fields_and_rejects_incomplete_enabled_channel() {
    let config = config();
    assert!(config.is_active());
    assert_eq!(config.bot_name.as_deref(), Some("helper"));
    assert_eq!(config.self_mention().as_deref(), Some("@helper"));
    assert_eq!(config.stream_mode, StreamMode::Partial);

    let error = WeComWsConfig::from_json(r#"{"enabled":true,"bot_id":"bot"}"#)
        .expect_err("enabled config without secret must fail");
    assert!(error.contains("bot_id and secret"));
    assert!(WeComWsConfig::from_json(r#"{"stream_mode":"multi_message"}"#).is_err());
}

#[test]
fn subscribe_and_ping_frames_match_wecom_commands() {
    let subscribe: Value =
        serde_json::from_str(&build_subscribe_frame(&config(), "req-sub")).unwrap();
    assert_eq!(subscribe["cmd"], "aibot_subscribe");
    assert_eq!(subscribe["headers"]["req_id"], "req-sub");
    assert_eq!(subscribe["body"]["bot_id"], "bot-1");
    assert_eq!(subscribe["body"]["secret"], "secret-1");

    let ping: Value = serde_json::from_str(&build_ping_frame("req-ping")).unwrap();
    assert_eq!(ping["cmd"], "ping");
    assert_eq!(ping["headers"]["req_id"], "req-ping");
}

#[test]
fn command_ack_preserves_correlation_and_error() {
    let frame = decode_server_frame(
        r#"{"headers":{"req_id":"req-1"},"errcode":93001,"errmsg":"session denied"}"#,
    )
    .unwrap();
    let ServerFrame::CommandAck(ack) = frame else {
        panic!("expected command ack");
    };
    assert_eq!(ack.request_id, "req-1");
    assert_eq!(ack.errcode, 93001);
    assert_eq!(ack.errmsg, "session denied");
    assert!(!ack.is_success());
}

#[test]
fn direct_text_callback_maps_reply_scope_and_request_thread() {
    let frame = callback(
        json!({
            "msgid": "msg-1",
            "msgtype": "text",
            "chattype": "single",
            "from": { "userid": "user-1" },
            "text": { "content": "  hello  " }
        }),
        "req-message",
    );
    let ServerFrame::Text(message) = decode_server_frame(&frame).unwrap() else {
        panic!("expected text callback");
    };
    assert_eq!(message.id, "msg-1");
    assert_eq!(message.sender, "user-1");
    assert_eq!(message.reply_target, "user--user-1");
    assert_eq!(message.content, "hello");
    assert_eq!(message.request_id, "req-message");
    assert_eq!(
        access_decision(&config(), &message),
        AccessDecision::Allowed
    );
}

#[test]
fn group_text_callback_uses_exact_group_allowlist() {
    let frame = callback(
        json!({
            "msgid": "msg-2",
            "msgtype": "text",
            "chattype": "group",
            "chatid": "group-1",
            "from": { "userid": "another-user" },
            "text": { "content": "@helper status" }
        }),
        "req-group",
    );
    let ServerFrame::Text(message) = decode_server_frame(&frame).unwrap() else {
        panic!("expected group text callback");
    };
    assert_eq!(message.reply_target, "group--group-1");
    assert_eq!(
        access_decision(&config(), &message),
        AccessDecision::Allowed
    );

    let mut denied = config();
    denied.allowed_groups = vec!["GROUP-1".to_string()];
    assert_eq!(access_decision(&denied, &message), AccessDecision::Denied);
    denied.allowed_groups.clear();
    denied.allowed_users.clear();
    assert_eq!(
        access_decision(&denied, &message),
        AccessDecision::MissingAllowlist
    );
}

#[test]
fn quoted_text_is_preserved_without_accepting_media_path() {
    let frame = callback(
        json!({
            "msgid": "msg-quote",
            "msgtype": "text",
            "from": { "userid": "user-1" },
            "text": { "content": "new text" },
            "quote": {
                "msgtype": "text",
                "text": { "content": "old text" }
            }
        }),
        "req-quote",
    );
    let ServerFrame::Text(message) = decode_server_frame(&frame).unwrap() else {
        panic!("expected quoted text callback");
    };
    assert!(message.content.contains("[WECOM_QUOTE]"));
    assert!(message.content.contains("old text"));
    assert!(message.content.ends_with("new text"));
}

#[test]
fn non_text_callbacks_are_explicitly_unsupported() {
    let frame = callback(
        json!({
            "msgid": "msg-image",
            "msgtype": "image",
            "from": { "userid": "user-1" },
            "image": { "url": "https://example.invalid/image" }
        }),
        "req-image",
    );
    assert_eq!(
        decode_server_frame(&frame).unwrap(),
        ServerFrame::UnsupportedMessage {
            request_id: "req-image".to_string(),
            message_type: "image".to_string(),
        }
    );
}

#[test]
fn disconnected_event_requests_reconnect() {
    let frame = json!({
        "cmd": "aibot_event_callback",
        "headers": { "req_id": "req-event" },
        "body": { "event": { "eventtype": "disconnected_event" } }
    });
    assert_eq!(
        decode_server_frame(&frame.to_string()).unwrap(),
        ServerFrame::Event(WeComEvent::Disconnected)
    );
}

#[test]
fn response_and_proactive_encoders_match_native_text_shapes() {
    let response: Value = serde_json::from_str(&build_respond_frame(
        "req-callback",
        "stream-1",
        "answer<|eom|>",
        true,
    ))
    .unwrap();
    assert_eq!(response["cmd"], "aibot_respond_msg");
    assert_eq!(response["headers"]["req_id"], "req-callback");
    assert_eq!(response["body"]["msgtype"], "stream");
    assert_eq!(response["body"]["stream"]["id"], "stream-1");
    assert_eq!(response["body"]["stream"]["finish"], true);
    assert_eq!(response["body"]["stream"]["content"], "answer");

    let proactive: Value = serde_json::from_str(
        &build_proactive_frame("group--group-1", "req-send", "hello").unwrap(),
    )
    .unwrap();
    assert_eq!(proactive["cmd"], "aibot_send_msg");
    assert_eq!(proactive["body"]["chat_type"], 2);
    assert_eq!(proactive["body"]["chatid"], "group-1");
    assert_eq!(proactive["body"]["markdown"]["content"], "hello");
    assert!(build_proactive_frame("user--", "req", "hello").is_err());
}

#[test]
fn utf8_stream_overflow_and_markdown_chunks_are_byte_bounded() {
    let input = "界".repeat(MARKDOWN_MAX_BYTES);
    let (head, overflow) = split_stream_content(&input);
    assert!(head.len() <= MARKDOWN_MAX_BYTES);
    assert!(overflow.is_some());
    assert_eq!(format!("{head}{}", overflow.unwrap()), input);

    let chunks = markdown_chunks(&input);
    assert_eq!(chunks.concat(), input);
    assert!(chunks
        .iter()
        .all(|chunk| chunk.len() <= MARKDOWN_CHUNK_BYTES));
}

#[test]
fn message_id_cache_is_bounded_and_suppresses_replays() {
    let mut cache = MessageIdCache::new(2);
    assert!(cache.record_if_new("one"));
    assert!(!cache.record_if_new("one"));
    assert!(cache.record_if_new("two"));
    assert!(cache.record_if_new("three"));
    assert_eq!(cache.len(), 2);
    assert!(cache.record_if_new("one"));
}
