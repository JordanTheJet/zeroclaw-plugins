//! Pure Matrix Client-Server API logic: configuration, URLs, payloads, and
//! `/sync` event mapping. Network I/O stays in the WASM component shim.

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

pub const CHANNEL: &str = "matrix";
const CLIENT_V3: &str = "_matrix/client/v3";

fn default_reply_in_thread() -> bool {
    true
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct MatrixConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub homeserver: String,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub allowed_rooms: Vec<String>,
    #[serde(default)]
    pub mention_only: bool,
    #[serde(default = "default_reply_in_thread")]
    pub reply_in_thread: bool,
}

impl MatrixConfig {
    pub fn from_json(input: &str) -> Self {
        serde_json::from_str(input).unwrap_or_default()
    }

    pub fn homeserver(&self) -> &str {
        self.homeserver.trim_end_matches('/')
    }

    pub fn access_token(&self) -> &str {
        self.access_token.as_deref().unwrap_or("").trim()
    }

    pub fn user_id(&self) -> Option<&str> {
        self.user_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn is_configured(&self) -> bool {
        !self.homeserver().is_empty() && !self.access_token().is_empty()
    }

    fn room_allowed(&self, room_id: &str) -> bool {
        self.allowed_rooms.is_empty()
            || self
                .allowed_rooms
                .iter()
                .any(|allowed| allowed.trim() == room_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inbound {
    pub id: String,
    pub sender: String,
    pub reply_target: String,
    pub content: String,
    pub timestamp: u64,
    pub thread_ts: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncBatch {
    pub next_batch: String,
    pub messages: Vec<Inbound>,
}

fn encode(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

fn api_url(homeserver: &str, path: &str) -> String {
    format!(
        "{}/{}/{}",
        homeserver.trim_end_matches('/'),
        CLIENT_V3,
        path.trim_start_matches('/')
    )
}

pub fn whoami_url(homeserver: &str) -> String {
    api_url(homeserver, "account/whoami")
}

pub fn sync_url(homeserver: &str, since: Option<&str>) -> String {
    let base = api_url(homeserver, "sync?timeout=0");
    match since.filter(|value| !value.is_empty()) {
        Some(cursor) => format!("{base}&since={}", encode(cursor)),
        None => base,
    }
}

pub fn room_alias_url(homeserver: &str, alias: &str) -> String {
    api_url(homeserver, &format!("directory/room/{}", encode(alias)))
}

pub fn send_url(homeserver: &str, room_id: &str, transaction_id: u64) -> String {
    api_url(
        homeserver,
        &format!(
            "rooms/{}/send/m.room.message/{transaction_id}",
            encode(room_id)
        ),
    )
}

pub fn parse_whoami(value: &Value) -> Option<String> {
    value
        .get("user_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

pub fn parse_room_id(value: &Value) -> Option<String> {
    value
        .get("room_id")
        .and_then(Value::as_str)
        .filter(|id| id.starts_with('!'))
        .map(str::to_string)
}

pub fn build_send_body(content: &str, thread_ts: Option<&str>, reply_in_thread: bool) -> Value {
    let mut body = json!({
        "msgtype": "m.text",
        "body": content,
    });
    if reply_in_thread {
        if let Some(anchor) = thread_ts.filter(|value| !value.is_empty()) {
            body["m.relates_to"] = json!({
                "rel_type": "m.thread",
                "event_id": anchor,
                "is_falling_back": true,
                "m.in_reply_to": { "event_id": anchor },
            });
        }
    }
    body
}

fn room_is_direct(room: &Value) -> bool {
    room.pointer("/summary/m.joined_member_count")
        .and_then(Value::as_u64)
        .is_some_and(|count| count <= 2)
}

fn direct_room_ids(value: &Value) -> HashSet<String> {
    let mut rooms = HashSet::new();
    let Some(events) = value
        .pointer("/account_data/events")
        .and_then(Value::as_array)
    else {
        return rooms;
    };
    for event in events {
        if event.get("type").and_then(Value::as_str) != Some("m.direct") {
            continue;
        }
        let Some(content) = event.get("content").and_then(Value::as_object) else {
            continue;
        };
        for room_ids in content.values().filter_map(Value::as_array) {
            rooms.extend(
                room_ids
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string),
            );
        }
    }
    rooms
}

fn mentions_user(content: &Value, body: &str, user_id: &str) -> bool {
    body.contains(user_id)
        || content
            .pointer("/m.mentions/user_ids")
            .and_then(Value::as_array)
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(user_id)))
}

fn thread_anchor(content: &Value) -> Option<String> {
    let relation = content.get("m.relates_to")?;
    if relation.get("rel_type").and_then(Value::as_str) == Some("m.replace") {
        return None;
    }
    (relation.get("rel_type").and_then(Value::as_str) == Some("m.thread"))
        .then(|| relation.get("event_id").and_then(Value::as_str))
        .flatten()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn parse_event(
    room_id: &str,
    event: &Value,
    cfg: &MatrixConfig,
    self_user_id: &str,
    is_direct: bool,
) -> Option<Inbound> {
    if event.get("type").and_then(Value::as_str) != Some("m.room.message") {
        return None;
    }
    let sender = event.get("sender").and_then(Value::as_str)?;
    if !self_user_id.is_empty() && sender == self_user_id {
        return None;
    }
    let content = event.get("content")?;
    if content.get("msgtype").and_then(Value::as_str) != Some("m.text") {
        return None;
    }
    if content
        .get("m.relates_to")
        .and_then(|relation| relation.get("rel_type"))
        .and_then(Value::as_str)
        == Some("m.replace")
    {
        return None;
    }
    let body = content.get("body").and_then(Value::as_str)?.trim();
    if body.is_empty() {
        return None;
    }
    if cfg.mention_only
        && !is_direct
        && (self_user_id.is_empty() || !mentions_user(content, body, self_user_id))
    {
        return None;
    }
    let event_id = event
        .get("event_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    let thread_ts =
        thread_anchor(content).or_else(|| cfg.reply_in_thread.then(|| event_id.to_string()));
    Some(Inbound {
        id: event_id.to_string(),
        sender: sender.to_string(),
        reply_target: room_id.to_string(),
        content: body.to_string(),
        timestamp: event
            .get("origin_server_ts")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        thread_ts,
    })
}

pub fn parse_sync(value: &Value, cfg: &MatrixConfig, self_user_id: &str) -> SyncBatch {
    let next_batch = value
        .get("next_batch")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut messages = Vec::new();
    let direct_rooms = direct_room_ids(value);
    let Some(rooms) = value.pointer("/rooms/join").and_then(Value::as_object) else {
        return SyncBatch {
            next_batch,
            messages,
        };
    };
    for (room_id, room) in rooms {
        if !cfg.room_allowed(room_id) {
            continue;
        }
        let Some(events) = room.pointer("/timeline/events").and_then(Value::as_array) else {
            continue;
        };
        messages.extend(events.iter().filter_map(|event| {
            parse_event(
                room_id,
                event,
                cfg,
                self_user_id,
                direct_rooms.contains(room_id) || room_is_direct(room),
            )
        }));
    }
    SyncBatch {
        next_batch,
        messages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> MatrixConfig {
        MatrixConfig {
            enabled: true,
            homeserver: "https://matrix.example/base/".to_string(),
            access_token: Some(" token ".to_string()),
            user_id: Some("@bot:example".to_string()),
            allowed_rooms: Vec::new(),
            mention_only: false,
            reply_in_thread: true,
        }
    }

    #[test]
    fn config_and_urls_use_native_keys() {
        let parsed = MatrixConfig::from_json(
            r#"{"enabled":true,"homeserver":"https://m.example/","access_token":"abc","user_id":"@bot:m","allowed_rooms":["!ok:m"],"mention_only":true}"#,
        );
        assert!(parsed.is_configured());
        assert_eq!(parsed.user_id(), Some("@bot:m"));
        assert_eq!(
            whoami_url(parsed.homeserver()),
            "https://m.example/_matrix/client/v3/account/whoami"
        );
        assert_eq!(
            sync_url("https://m.example", Some("s/1")),
            "https://m.example/_matrix/client/v3/sync?timeout=0&since=s%2F1"
        );
        assert!(send_url("https://m.example", "!room:m", 7).contains("rooms/%21room%3Am/send"));
    }

    #[test]
    fn send_body_threads_only_when_enabled() {
        let threaded = build_send_body("hello", Some("$root"), true);
        assert_eq!(threaded["m.relates_to"]["event_id"], "$root");
        assert_eq!(
            threaded["m.relates_to"]["m.in_reply_to"]["event_id"],
            "$root"
        );
        let plain = build_send_body("hello", Some("$root"), false);
        assert!(plain.get("m.relates_to").is_none());
    }

    #[test]
    fn sync_maps_text_and_filters_self_non_text_and_disallowed_rooms() {
        let mut config = cfg();
        config.allowed_rooms = vec!["!ok:m".to_string()];
        let value = json!({
            "next_batch": "s2",
            "rooms": { "join": {
                "!ok:m": { "summary": { "m.joined_member_count": 3 }, "timeline": { "events": [
                    {"type":"m.room.message","event_id":"$1","sender":"@alice:m","origin_server_ts":12,"content":{"msgtype":"m.text","body":"hello"}},
                    {"type":"m.room.message","event_id":"$2","sender":"@bot:example","content":{"msgtype":"m.text","body":"self"}},
                    {"type":"m.room.message","event_id":"$3","sender":"@alice:m","content":{"msgtype":"m.image","body":"pic"}}
                ]}},
                "!blocked:m": { "timeline": { "events": [
                    {"type":"m.room.message","event_id":"$4","sender":"@alice:m","content":{"msgtype":"m.text","body":"blocked"}}
                ]}}
            }}
        });
        let batch = parse_sync(&value, &config, "@bot:example");
        assert_eq!(batch.next_batch, "s2");
        assert_eq!(batch.messages.len(), 1);
        assert_eq!(batch.messages[0].id, "$1");
        assert_eq!(batch.messages[0].reply_target, "!ok:m");
        assert_eq!(batch.messages[0].thread_ts.as_deref(), Some("$1"));
        assert_eq!(batch.messages[0].timestamp, 12);
    }

    #[test]
    fn mention_gate_allows_dm_and_explicit_matrix_mention() {
        let mut config = cfg();
        config.mention_only = true;
        let value = json!({
            "next_batch": "s3",
            "rooms": { "join": {
                "!group:m": { "summary": { "m.joined_member_count": 4 }, "timeline": { "events": [
                    {"type":"m.room.message","event_id":"$skip","sender":"@a:m","content":{"msgtype":"m.text","body":"hello"}},
                    {"type":"m.room.message","event_id":"$mention","sender":"@a:m","content":{"msgtype":"m.text","body":"hello","m.mentions":{"user_ids":["@bot:example"]}}}
                ]}},
                "!dm:m": { "summary": { "m.joined_member_count": 2 }, "timeline": { "events": [
                    {"type":"m.room.message","event_id":"$dm","sender":"@b:m","content":{"msgtype":"m.text","body":"hello"}}
                ]}}
            }}
        });
        let mut ids: Vec<_> = parse_sync(&value, &config, "@bot:example")
            .messages
            .into_iter()
            .map(|message| message.id)
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["$dm", "$mention"]);
    }

    #[test]
    fn mention_gate_uses_m_direct_without_a_room_summary() {
        let mut config = cfg();
        config.mention_only = true;
        let value = json!({
            "next_batch": "s3",
            "account_data": { "events": [{
                "type": "m.direct",
                "content": { "@friend:m": ["!dm:m"] }
            }]},
            "rooms": { "join": { "!dm:m": { "timeline": { "events": [{
                "type":"m.room.message",
                "event_id":"$dm",
                "sender":"@friend:m",
                "content":{"msgtype":"m.text","body":"hello"}
            }]}}}}
        });
        let messages = parse_sync(&value, &config, "@bot:example").messages;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, "$dm");
    }

    #[test]
    fn edits_are_not_reingested_and_existing_threads_are_preserved() {
        let value = json!({
            "next_batch": "s4",
            "rooms": { "join": { "!r:m": { "timeline": { "events": [
                {"type":"m.room.message","event_id":"$edit","sender":"@a:m","content":{"msgtype":"m.text","body":"edited","m.relates_to":{"rel_type":"m.replace","event_id":"$old"}}},
                {"type":"m.room.message","event_id":"$reply","sender":"@a:m","content":{"msgtype":"m.text","body":"reply","m.relates_to":{"rel_type":"m.thread","event_id":"$root"}}}
            ]}}}}
        });
        let messages = parse_sync(&value, &cfg(), "@bot:example").messages;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].thread_ts.as_deref(), Some("$root"));
    }
}
