//! Pure WeCom AI Bot WebSocket text-protocol logic.
//!
//! This module has no WIT or socket dependencies. The WASM component shim in
//! `lib.rs` owns transport and timers; host tests exercise the same config,
//! frame parsing, access checks, deduplication, and JSON encoding used in the
//! component.

use std::collections::{HashSet, VecDeque};

use serde::Deserialize;
use serde_json::{json, Value};

pub const CHANNEL: &str = "wecom_ws";
pub const PLUGIN_NAME: &str = "wecom-ws";
pub const WECOM_WS_URL: &str = "wss://openws.work.weixin.qq.com";
pub const MARKDOWN_MAX_BYTES: usize = 20_480;
pub const MARKDOWN_CHUNK_BYTES: usize = 8_000;
pub const MESSAGE_ID_CACHE_SIZE: usize = 4_096;

const PROVIDER_TRAILING_SENTINELS: &[&str] = &["<|eom|>"];

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StreamMode {
    Off,
    #[default]
    Partial,
    #[serde(rename = "multi_message")]
    MultiMessage,
}

/// The host-injected `[channels.wecom_ws.<alias>]` configuration.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WeComWsConfig {
    pub enabled: bool,
    pub bot_id: String,
    pub secret: String,
    pub allowed_users: Vec<String>,
    pub allowed_groups: Vec<String>,
    pub bot_name: Option<String>,
    pub stream_mode: StreamMode,
}

impl Default for WeComWsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_id: String::new(),
            secret: String::new(),
            allowed_users: Vec::new(),
            allowed_groups: Vec::new(),
            bot_name: None,
            stream_mode: StreamMode::Partial,
        }
    }
}

impl WeComWsConfig {
    pub fn from_json(input: &str) -> Result<Self, String> {
        let mut config: Self = serde_json::from_str(input)
            .map_err(|error| format!("wecom-ws config is not valid JSON: {error}"))?;
        config.bot_id = config.bot_id.trim().to_string();
        config.secret = config.secret.trim().to_string();
        config.allowed_users = normalize_allowlist(config.allowed_users);
        config.allowed_groups = normalize_allowlist(config.allowed_groups);
        config.bot_name = config
            .bot_name
            .map(|name| name.trim().trim_start_matches('@').to_string())
            .filter(|name| !name.is_empty());

        if config.stream_mode == StreamMode::MultiMessage {
            return Err(
                "wecom-ws: stream_mode=multi_message is unsupported; use partial or off"
                    .to_string(),
            );
        }
        if config.enabled && !config.has_credentials() {
            return Err("wecom-ws: enabled channels require bot_id and secret".to_string());
        }
        Ok(config)
    }

    pub fn has_credentials(&self) -> bool {
        !self.bot_id.is_empty() && !self.secret.is_empty()
    }

    pub fn is_active(&self) -> bool {
        self.enabled && self.has_credentials()
    }

    pub fn self_mention(&self) -> Option<String> {
        self.bot_name.as_ref().map(|name| format!("@{name}"))
    }
}

fn normalize_allowlist(entries: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::with_capacity(entries.len());
    for entry in entries {
        let entry = entry.trim().to_string();
        if !entry.is_empty() && !normalized.contains(&entry) {
            normalized.push(entry);
        }
    }
    normalized
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboundText {
    pub id: String,
    pub sender: String,
    pub reply_target: String,
    pub content: String,
    pub request_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandAck {
    pub request_id: String,
    pub errcode: i64,
    pub errmsg: String,
}

impl CommandAck {
    pub fn is_success(&self) -> bool {
        self.errcode == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WeComEvent {
    Disconnected,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerFrame {
    CommandAck(CommandAck),
    Text(InboundText),
    UnsupportedMessage {
        request_id: String,
        message_type: String,
    },
    Event(WeComEvent),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessDecision {
    Allowed,
    MissingAllowlist,
    Denied,
}

pub fn access_decision(config: &WeComWsConfig, inbound: &InboundText) -> AccessDecision {
    if config.allowed_users.is_empty() && config.allowed_groups.is_empty() {
        return AccessDecision::MissingAllowlist;
    }
    if allowlist_matches(&config.allowed_users, &inbound.sender) {
        return AccessDecision::Allowed;
    }
    if let Some(group_id) = inbound.reply_target.strip_prefix("group--") {
        if allowlist_matches(&config.allowed_groups, group_id) {
            return AccessDecision::Allowed;
        }
    }
    AccessDecision::Denied
}

fn allowlist_matches(allowlist: &[String], candidate: &str) -> bool {
    !candidate.is_empty()
        && allowlist
            .iter()
            .any(|entry| entry == "*" || entry == candidate)
}

pub fn decode_server_frame(input: &str) -> Result<ServerFrame, String> {
    let frame: Value = serde_json::from_str(input)
        .map_err(|error| format!("invalid WeCom WebSocket JSON: {error}"))?;

    if let Some(errcode) = frame.get("errcode").and_then(Value::as_i64) {
        let request_id = request_id(&frame)
            .ok_or_else(|| "WeCom command response is missing headers.req_id".to_string())?;
        let errmsg = frame
            .get("errmsg")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        return Ok(ServerFrame::CommandAck(CommandAck {
            request_id,
            errcode,
            errmsg,
        }));
    }

    match frame.get("cmd").and_then(Value::as_str).unwrap_or("") {
        "aibot_msg_callback" => decode_message_callback(&frame),
        "aibot_event_callback" => Ok(decode_event_callback(&frame)),
        _ => Ok(ServerFrame::Unknown),
    }
}

fn request_id(frame: &Value) -> Option<String> {
    frame
        .pointer("/headers/req_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn decode_message_callback(frame: &Value) -> Result<ServerFrame, String> {
    let request_id = request_id(frame)
        .ok_or_else(|| "WeCom message callback is missing headers.req_id".to_string())?;
    let body = frame
        .get("body")
        .ok_or_else(|| "WeCom message callback is missing body".to_string())?;
    let message_type = body
        .get("msgtype")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "WeCom message callback is missing msgtype".to_string())?;

    if message_type != "text" {
        return Ok(ServerFrame::UnsupportedMessage {
            request_id,
            message_type: message_type.to_string(),
        });
    }

    let sender = body
        .pointer("/from/userid")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "WeCom text callback is missing from.userid".to_string())?
        .to_string();
    let content = body
        .pointer("/text/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "WeCom text callback has empty text.content".to_string())?;
    let content = match quote_text(body) {
        Some(quote) => {
            format!("[WECOM_QUOTE]\nmsgtype=text\ncontent={quote}\n[/WECOM_QUOTE]\n\n{content}")
        }
        None => content.to_string(),
    };

    let is_group = body
        .get("chattype")
        .and_then(Value::as_str)
        .is_some_and(|chat_type| chat_type.eq_ignore_ascii_case("group"));
    let reply_target = if is_group {
        let chat_id = body
            .get("chatid")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "WeCom group text callback is missing chatid".to_string())?;
        format!("group--{chat_id}")
    } else {
        format!("user--{sender}")
    };
    let id = body
        .get("msgid")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&request_id)
        .to_string();

    Ok(ServerFrame::Text(InboundText {
        id,
        sender,
        reply_target,
        content,
        request_id,
    }))
}

fn quote_text(body: &Value) -> Option<String> {
    let quote = body.get("quote")?;
    if quote.get("msgtype").and_then(Value::as_str) != Some("text") {
        return None;
    }
    quote
        .pointer("/text/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| trim_utf8(value, 4_096))
}

fn decode_event_callback(frame: &Value) -> ServerFrame {
    let event_type = frame
        .pointer("/body/event/eventtype")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    if event_type == "disconnected_event" {
        ServerFrame::Event(WeComEvent::Disconnected)
    } else {
        ServerFrame::Event(WeComEvent::Other(event_type.to_string()))
    }
}

pub fn build_subscribe_frame(config: &WeComWsConfig, request_id: &str) -> String {
    json!({
        "cmd": "aibot_subscribe",
        "headers": { "req_id": request_id },
        "body": {
            "bot_id": config.bot_id,
            "secret": config.secret,
        },
    })
    .to_string()
}

pub fn build_ping_frame(request_id: &str) -> String {
    json!({
        "cmd": "ping",
        "headers": { "req_id": request_id },
    })
    .to_string()
}

pub fn build_respond_frame(
    request_id: &str,
    stream_id: &str,
    content: &str,
    finish: bool,
) -> String {
    json!({
        "cmd": "aibot_respond_msg",
        "headers": { "req_id": request_id },
        "body": {
            "msgtype": "stream",
            "stream": {
                "id": stream_id,
                "finish": finish,
                "content": normalize_stream_content(content),
            },
        },
    })
    .to_string()
}

pub fn build_proactive_frame(
    reply_target: &str,
    request_id: &str,
    content: &str,
) -> Result<String, String> {
    let (chat_type, chat_id) = parse_reply_target(reply_target)?;
    Ok(json!({
        "cmd": "aibot_send_msg",
        "headers": { "req_id": request_id },
        "body": {
            "chatid": chat_id,
            "chat_type": chat_type,
            "msgtype": "markdown",
            "markdown": { "content": content },
        },
    })
    .to_string())
}

fn parse_reply_target(reply_target: &str) -> Result<(u32, &str), String> {
    let (chat_type, chat_id) = if let Some(user_id) = reply_target.strip_prefix("user--") {
        (1, user_id)
    } else if let Some(group_id) = reply_target.strip_prefix("group--") {
        (2, group_id)
    } else {
        return Err(format!(
            "wecom-ws: invalid recipient `{reply_target}`; expected user--<userid> or group--<chatid>"
        ));
    };
    if chat_id.is_empty() {
        return Err("wecom-ws: recipient identifier cannot be empty".to_string());
    }
    Ok((chat_type, chat_id))
}

pub fn split_stream_content(input: &str) -> (String, Option<String>) {
    let input = strip_trailing_provider_sentinels(input);
    if input.len() <= MARKDOWN_MAX_BYTES {
        return (input, None);
    }

    let head = trim_utf8(&input, MARKDOWN_MAX_BYTES);
    let tail = input[head.len()..].to_string();
    (head, (!tail.is_empty()).then_some(tail))
}

pub fn markdown_chunks(input: &str) -> Vec<String> {
    let input = strip_trailing_provider_sentinels(input);
    if input.is_empty() {
        return vec![String::new()];
    }

    let mut chunks = Vec::new();
    let mut remaining = input.as_str();
    while remaining.len() > MARKDOWN_CHUNK_BYTES {
        let hard_split = trim_utf8(remaining, MARKDOWN_CHUNK_BYTES).len();
        let split_at = remaining[..hard_split]
            .rfind('\n')
            .map(|index| index + 1)
            .filter(|index| *index >= MARKDOWN_CHUNK_BYTES / 2)
            .unwrap_or(hard_split);
        chunks.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }
    chunks.push(remaining.to_string());
    chunks
}

fn normalize_stream_content(input: &str) -> String {
    trim_utf8(
        &strip_trailing_provider_sentinels(input),
        MARKDOWN_MAX_BYTES,
    )
}

fn strip_trailing_provider_sentinels(input: &str) -> String {
    let mut trimmed = input.trim_end();
    while let Some(sentinel) = PROVIDER_TRAILING_SENTINELS
        .iter()
        .find(|sentinel| trimmed.ends_with(**sentinel))
    {
        trimmed = trimmed[..trimmed.len() - sentinel.len()].trim_end();
    }
    trimmed.to_string()
}

fn trim_utf8(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    let mut end = max_bytes;
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

/// Bounded replay suppression for callback `msgid` values.
#[derive(Debug)]
pub struct MessageIdCache {
    capacity: usize,
    seen: HashSet<String>,
    order: VecDeque<String>,
}

impl MessageIdCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            seen: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    pub fn record_if_new(&mut self, id: &str) -> bool {
        if id.is_empty() || self.capacity == 0 {
            return true;
        }
        if !self.seen.insert(id.to_string()) {
            return false;
        }
        self.order.push_back(id.to_string());
        while self.order.len() > self.capacity {
            if let Some(expired) = self.order.pop_front() {
                self.seen.remove(&expired);
            }
        }
        true
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}
