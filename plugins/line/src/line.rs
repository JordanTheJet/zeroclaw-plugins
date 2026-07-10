//! Pure LINE Messaging API logic — no wasm, no HTTP deps.
//!
//! Config parsing, `X-Line-Signature` HMAC-SHA256 verification, webhook-event →
//! inbound decoding, and push-message body building. The wasm component shim
//! ([`crate`] `lib.rs`) does the HTTP (`wasi:http` via `waki`) and wires this to
//! the WIT `channel` world, so everything here is covered by a host `cargo test`
//! with no network.

use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// LINE Messaging API base (overridable for a proxy or offline test).
pub const LINE_API_BASE: &str = "https://api.line.me";

/// LINE's per-text-message character limit; longer content is chunked.
pub const MAX_MESSAGE_CHARS: usize = 5000;

/// Resolved config from the plugin's `[channels.line.<alias>]` section.
#[derive(Clone, Debug, PartialEq)]
pub struct LineConfig {
    /// Channel access token — bearer auth for the Messaging API (send).
    pub channel_access_token: String,
    /// Channel secret — verifies the `X-Line-Signature` HMAC over the body.
    pub channel_secret: String,
    pub api_base: String,
}

impl Default for LineConfig {
    fn default() -> Self {
        Self {
            channel_access_token: String::new(),
            channel_secret: String::new(),
            api_base: LINE_API_BASE.to_string(),
        }
    }
}

impl LineConfig {
    pub fn from_json(s: &str) -> Self {
        let v: Value = serde_json::from_str(s).unwrap_or(Value::Null);
        let str_field = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let api_base = v
            .get("api_base_url")
            .or_else(|| v.get("api_base"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(LINE_API_BASE)
            .trim_end_matches('/')
            .to_string();
        LineConfig {
            channel_access_token: str_field("channel_access_token"),
            channel_secret: str_field("channel_secret"),
            api_base,
        }
    }
}

/// An inbound message to hand the host, pre-WIT-lift.
#[derive(Debug, Clone, PartialEq)]
pub struct Inbound {
    pub id: String,
    pub sender: String,
    pub reply_target: String,
    pub content: String,
}

/// Verify the `X-Line-Signature` header: base64(HMAC-SHA256(channel_secret,
/// body)). Constant-time via `Mac::verify_slice`. A bad base64, empty secret, or
/// mismatch all return `false` so the host replies 401 and enqueues nothing.
pub fn verify_signature(channel_secret: &str, body: &[u8], signature_b64: &str) -> bool {
    if channel_secret.is_empty() {
        return false;
    }
    let Ok(mut mac) = HmacSha256::new_from_slice(channel_secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let Ok(provided) = STANDARD.decode(signature_b64) else {
        return false;
    };
    mac.verify_slice(&provided).is_ok()
}

/// Decode a LINE webhook body into inbound text messages. Non-text and
/// non-`message` events (follow, join, the empty verify ping, …) yield nothing,
/// so the host replies 200 and enqueues nothing. The reply target is the
/// group/room when present (so a reply lands in the same conversation), else the
/// sender's userId; the sender is always the userId.
pub fn parse_events(body: &[u8]) -> Vec<Inbound> {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let mut out = Vec::new();
    let Some(events) = v.get("events").and_then(Value::as_array) else {
        return out;
    };
    for ev in events {
        if ev.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let msg = ev.get("message");
        if msg.and_then(|m| m.get("type")).and_then(Value::as_str) != Some("text") {
            continue;
        }
        let text = msg
            .and_then(|m| m.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if text.trim().is_empty() {
            continue;
        }
        let source = ev.get("source");
        let user_id = source
            .and_then(|s| s.get("userId"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let target = source
            .and_then(|s| {
                s.get("groupId")
                    .or_else(|| s.get("roomId"))
                    .or_else(|| s.get("userId"))
            })
            .and_then(Value::as_str)
            .unwrap_or(user_id);
        if target.is_empty() {
            continue;
        }
        let id = msg
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        out.push(Inbound {
            id: format!("line_{id}"),
            sender: if user_id.is_empty() { target } else { user_id }.to_string(),
            reply_target: target.to_string(),
            content: text.to_string(),
        });
    }
    out
}

/// A Push-message body: `POST /v2/bot/message/push` `{to, messages:[{text}]}`.
/// Push (vs the free but ~30 s-expiring reply token) is robust for an async
/// agent whose reply may arrive after the token expires.
pub fn build_push_body(to: &str, text: &str) -> Value {
    json!({ "to": to, "messages": [{ "type": "text", "text": text }] })
}

/// Split into ≤`max_chars` char chunks (by `char`, so multibyte text is never
/// split mid-codepoint), preferring a newline boundary near the limit. Empty
/// input yields one empty chunk.
pub fn chunk_text(content: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= max_chars {
        return vec![content.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let hard_end = (start + max_chars).min(chars.len());
        let mut end = hard_end;
        if hard_end < chars.len() {
            if let Some(nl) = chars[start..hard_end].iter().rposition(|&c| c == '\n') {
                let candidate = start + nl + 1;
                if candidate > start + max_chars / 2 {
                    end = candidate;
                }
            }
        }
        chunks.push(chars[start..end].iter().collect());
        start = end;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        STANDARD.encode(mac.finalize().into_bytes())
    }

    #[test]
    fn config_parses_and_defaults() {
        let c = LineConfig::from_json(
            r#"{"channel_access_token":"tok","channel_secret":"sec","enabled":true}"#,
        );
        assert_eq!(c.channel_access_token, "tok");
        assert_eq!(c.channel_secret, "sec");
        assert_eq!(c.api_base, LINE_API_BASE);
        // api_base override trims a trailing slash.
        let c = LineConfig::from_json(r#"{"api_base_url":"http://127.0.0.1:9/"}"#);
        assert_eq!(c.api_base, "http://127.0.0.1:9");
    }

    #[test]
    fn signature_roundtrip() {
        let secret = "shhh";
        let body = br#"{"events":[]}"#;
        let good = sign(secret, body);
        assert!(verify_signature(secret, body, &good));
        // wrong secret, tampered body, garbage base64, empty secret → all false.
        assert!(!verify_signature("other", body, &good));
        assert!(!verify_signature(secret, br#"{"events":[{}]}"#, &good));
        assert!(!verify_signature(secret, body, "not-base64!!"));
        assert!(!verify_signature("", body, &good));
    }

    #[test]
    fn parse_text_message() {
        let body = br#"{"events":[{"type":"message","replyToken":"rt",
            "source":{"type":"user","userId":"U123"},
            "message":{"id":"m1","type":"text","text":"hello"}}]}"#;
        let out = parse_events(body);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "line_m1");
        assert_eq!(out[0].sender, "U123");
        assert_eq!(out[0].reply_target, "U123");
        assert_eq!(out[0].content, "hello");
    }

    #[test]
    fn parse_group_message_targets_group() {
        let body = br#"{"events":[{"type":"message",
            "source":{"type":"group","groupId":"G9","userId":"U1"},
            "message":{"id":"m2","type":"text","text":"hi all"}}]}"#;
        let out = parse_events(body);
        assert_eq!(out[0].reply_target, "G9", "reply goes to the group");
        assert_eq!(out[0].sender, "U1", "sender is the user");
    }

    #[test]
    fn parse_skips_nontext_and_empty_and_verify_ping() {
        // sticker (non-text)
        let sticker = br#"{"events":[{"type":"message","source":{"userId":"U"},
            "message":{"id":"s","type":"sticker"}}]}"#;
        assert!(parse_events(sticker).is_empty());
        // follow event (non-message)
        let follow = br#"{"events":[{"type":"follow","source":{"userId":"U"}}]}"#;
        assert!(parse_events(follow).is_empty());
        // empty verify ping
        assert!(parse_events(br#"{"events":[]}"#).is_empty());
        // whitespace-only text
        let blank = br#"{"events":[{"type":"message","source":{"userId":"U"},
            "message":{"id":"b","type":"text","text":"   "}}]}"#;
        assert!(parse_events(blank).is_empty());
    }

    #[test]
    fn push_body_shape() {
        assert_eq!(
            build_push_body("U1", "hi"),
            json!({"to":"U1","messages":[{"type":"text","text":"hi"}]})
        );
    }

    #[test]
    fn chunk_preserves_all_chars() {
        let body = "x".repeat(12000);
        let chunks = chunk_text(&body, MAX_MESSAGE_CHARS);
        assert_eq!(chunks.len(), 3);
        assert!(chunks
            .iter()
            .all(|c| c.chars().count() <= MAX_MESSAGE_CHARS));
        assert_eq!(chunks.concat(), body);
        assert_eq!(chunk_text("short", MAX_MESSAGE_CHARS), vec!["short"]);
    }
}
