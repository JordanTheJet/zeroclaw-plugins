//! Pure Discord Gateway protocol logic — no wasm, no HTTP, no WebSocket deps.
//!
//! Everything here is deterministic serde/string work: config parsing, gateway
//! frame parsing, IDENTIFY/RESUME/HEARTBEAT payload builders, the MESSAGE_CREATE
//! filter pipeline, message chunking, and deriving the bot's own id from its
//! token. The wasm component shim ([`crate`] `lib.rs`) drives the host
//! `ws-client` + `wasi:http` imports and calls into these functions, so this
//! module is fully covered by a host `cargo test` (see `#[cfg(test)]` below)
//! without a live Discord connection.

use serde_json::{json, Value};

/// Discord REST + Gateway API version pinned by this plugin.
pub const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

/// Discord's hard per-message content limit (characters); longer content is
/// chunked before send.
pub const MAX_MESSAGE_CHARS: usize = 2000;

/// Fallback heartbeat interval (ms) if a Hello omits `heartbeat_interval`.
pub const DEFAULT_HEARTBEAT_MS: u64 = 41250;

// ── Gateway intents (API v10 bit positions) ───────────────────────────────────
const INTENT_GUILDS: u64 = 1 << 0;
const INTENT_GUILD_MESSAGES: u64 = 1 << 9;
const INTENT_DIRECT_MESSAGES: u64 = 1 << 12;
const INTENT_MESSAGE_CONTENT: u64 = 1 << 15;

/// The intents every Discord bot needs: guild topology, guild + DM messages, and
/// MESSAGE_CONTENT (privileged, always requested) so it reads text without an
/// @-mention. Mirrors the native channel's `BASELINE_INTENTS` (= 37377).
pub const BASELINE_INTENTS: u64 =
    INTENT_GUILDS | INTENT_GUILD_MESSAGES | INTENT_DIRECT_MESSAGES | INTENT_MESSAGE_CONTENT;

/// Resolved configuration for the Discord channel, parsed from the plugin's
/// `[channels.discord.<alias>]` JSON object. Only the fields the inbound/send
/// paths need are modelled; peer allow-listing and ownership are enforced
/// host-side (they never reach the plugin's config object).
#[derive(Clone, Debug, PartialEq)]
pub struct DiscordConfig {
    pub bot_token: String,
    pub guild_ids: Vec<String>,
    pub channel_ids: Vec<String>,
    pub listen_to_bots: bool,
    pub mention_only: bool,
    pub intents_mask: Option<u64>,
    /// REST + gateway-discovery base, overridable for self-hosted proxies and
    /// offline tests. Defaults to [`DISCORD_API_BASE`].
    pub api_base: String,
    /// Direct Gateway WSS base override. When set, the plugin connects here
    /// instead of discovering the URL via `GET /gateway/bot` — for self-hosted
    /// gateways and offline tests. `resume_gateway_url` still wins on a resume.
    pub gateway_url: Option<String>,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            bot_token: String::new(),
            guild_ids: Vec::new(),
            channel_ids: Vec::new(),
            listen_to_bots: false,
            mention_only: false,
            intents_mask: None,
            api_base: DISCORD_API_BASE.to_string(),
            gateway_url: None,
        }
    }
}

impl DiscordConfig {
    /// Parse the config JSON object the host hands `configure`. Unknown/missing
    /// fields fall back to defaults; a non-object input yields an empty config
    /// (empty `bot_token`, which the shim treats as "not configured").
    pub fn from_json(s: &str) -> Self {
        let v: Value = serde_json::from_str(s).unwrap_or(Value::Null);
        let api_base = v
            .get("api_base")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(DISCORD_API_BASE)
            .trim_end_matches('/')
            .to_string();
        DiscordConfig {
            bot_token: str_field(&v, "bot_token"),
            guild_ids: string_list(&v, "guild_ids"),
            channel_ids: string_list(&v, "channel_ids"),
            listen_to_bots: bool_field(&v, "listen_to_bots"),
            mention_only: bool_field(&v, "mention_only"),
            intents_mask: v.get("intents_mask").and_then(Value::as_u64),
            api_base,
            gateway_url: v
                .get("gateway_url")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from),
        }
    }

    /// The IDENTIFY intent mask: the raw `intents_mask` override verbatim when
    /// set (operator escape hatch, including `Some(0)`), else the baseline.
    pub fn intents(&self) -> u64 {
        self.intents_mask.unwrap_or(BASELINE_INTENTS)
    }
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn bool_field(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A JSON array of IDs, accepting both string (`"123"`) and numeric (`123`)
/// elements — TOML→JSON round-trips snowflakes either way depending on source.
fn string_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    x.as_str()
                        .map(String::from)
                        .or_else(|| x.as_u64().map(|n| n.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── Outbound gateway payloads ─────────────────────────────────────────────────

/// IDENTIFY (op 2): authenticate a fresh session with the token + intents.
/// Uses the modern unprefixed `os`/`browser`/`device` properties; single shard,
/// no presence — matching the native channel byte-for-byte.
pub fn build_identify(token: &str, intents: u64) -> String {
    json!({
        "op": 2,
        "d": {
            "token": token,
            "intents": intents,
            "properties": { "os": "linux", "browser": "zeroclaw", "device": "zeroclaw" }
        }
    })
    .to_string()
}

/// RESUME (op 6): replay a dropped session from the last acked sequence.
pub fn build_resume(token: &str, session_id: &str, seq: i64) -> String {
    json!({ "op": 6, "d": { "token": token, "session_id": session_id, "seq": seq } }).to_string()
}

/// HEARTBEAT (op 1): `d` is the last received sequence, or `null` before any
/// sequenced frame has arrived.
pub fn build_heartbeat(seq: Option<i64>) -> String {
    let d = match seq {
        Some(s) => json!(s),
        None => Value::Null,
    };
    json!({ "op": 1, "d": d }).to_string()
}

/// The REST body for a plain outbound message: `{"content": "..."}`.
pub fn build_send_body(content: &str) -> Value {
    json!({ "content": content })
}

// ── Inbound gateway frames ────────────────────────────────────────────────────

/// A decoded gateway frame: the opcode, the dispatch type (`t`, only on op 0),
/// the sequence (`s`), and the payload (`d`).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub op: u64,
    pub t: Option<String>,
    pub s: Option<i64>,
    pub d: Value,
}

/// Parse one gateway text frame. Returns `None` for non-JSON input (which the
/// shim skips), mirroring the native `serde_json::from_str(..).ok()` guard.
pub fn parse_frame(text: &str) -> Option<Frame> {
    let v: Value = serde_json::from_str(text).ok()?;
    Some(Frame {
        op: v.get("op").and_then(Value::as_u64).unwrap_or(0),
        t: v.get("t").and_then(Value::as_str).map(String::from),
        s: v.get("s").and_then(Value::as_i64),
        d: v.get("d").cloned().unwrap_or(Value::Null),
    })
}

/// A message to hand the host inbound queue, before it is lifted to the WIT type.
#[derive(Debug, Clone, PartialEq)]
pub struct Inbound {
    pub id: String,
    pub sender: String,
    pub reply_target: String,
    pub content: String,
}

/// Conversational message types the bot answers: DEFAULT (0) and REPLY (19).
/// Thread-created, thread-starter, pins, joins, etc. are dropped — matching the
/// native `is_conversational_message_type`.
fn is_conversational_type(t: u64) -> bool {
    t == 0 || t == 19
}

/// Whether `content` @-mentions the bot (`<@id>` or `<@!id>`).
pub fn contains_bot_mention(content: &str, bot_user_id: &str) -> bool {
    !bot_user_id.is_empty()
        && (content.contains(&format!("<@{bot_user_id}>"))
            || content.contains(&format!("<@!{bot_user_id}>")))
}

/// Apply the inbound filter pipeline to a MESSAGE_CREATE `d` payload and return
/// the message to emit, or `None` when it is filtered out. `bot_user_id` is the
/// bot's own id (READY `d.user.id`, or token-derived) for the self-loop guard.
///
/// Mirrors the native gates, in order: conversational-type → non-empty author
/// that isn't the bot → bot/webhook filter (`listen_to_bots`) → guild allowlist
/// (DMs always pass) → channel allowlist (direct match only; thread-parent
/// resolution is host-side and out of scope here) → mention gate (guild-only) →
/// non-empty content. Peer allow-listing and ownership are enforced host-side.
pub fn message_create_to_inbound(
    d: &Value,
    cfg: &DiscordConfig,
    bot_user_id: &str,
) -> Option<Inbound> {
    let msg_type = d.get("type").and_then(Value::as_u64).unwrap_or(0);
    if !is_conversational_type(msg_type) {
        return None;
    }

    let author_id = d
        .get("author")
        .and_then(|a| a.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if author_id.is_empty() || author_id == bot_user_id {
        return None;
    }

    let is_bot = d
        .get("author")
        .and_then(|a| a.get("bot"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if is_bot && !cfg.listen_to_bots {
        return None;
    }

    let guild_id = d.get("guild_id").and_then(Value::as_str);
    let is_dm = guild_id.is_none();
    // Guild allowlist: a DM (no guild_id) always passes; a guild message must be
    // in the allowlist when one is configured.
    if !cfg.guild_ids.is_empty() {
        if let Some(g) = guild_id {
            if !cfg.guild_ids.iter().any(|allowed| allowed == g) {
                return None;
            }
        }
    }

    let channel_id = d.get("channel_id").and_then(Value::as_str).unwrap_or("");
    if !cfg.channel_ids.is_empty()
        && !channel_id.is_empty()
        && !cfg.channel_ids.iter().any(|allowed| allowed == channel_id)
    {
        return None;
    }

    let content = d.get("content").and_then(Value::as_str).unwrap_or("");
    let effective_mention_only = cfg.mention_only && !is_dm;
    if effective_mention_only && !contains_bot_mention(content, bot_user_id) {
        return None;
    }
    if content.trim().is_empty() {
        return None;
    }

    let id = d.get("id").and_then(Value::as_str).unwrap_or("");
    Some(Inbound {
        id: format!("discord_{id}"),
        sender: author_id.to_string(),
        reply_target: if channel_id.is_empty() {
            author_id.to_string()
        } else {
            channel_id.to_string()
        },
        content: content.to_string(),
    })
}

// ── READY payload accessors ───────────────────────────────────────────────────

/// The bot's own user id from a READY `d` payload (`d.user.id`), the authority
/// for the self-loop guard once connected.
pub fn ready_user_id(d: &Value) -> Option<String> {
    d.get("user")
        .and_then(|u| u.get("id"))
        .and_then(Value::as_str)
        .map(String::from)
}

/// The bot's `@username` from a READY `d` payload (`d.user.username`).
pub fn ready_username(d: &Value) -> Option<String> {
    d.get("user")
        .and_then(|u| u.get("username"))
        .and_then(Value::as_str)
        .map(|u| format!("@{u}"))
}

// ── Gateway session state machine ─────────────────────────────────────────────

/// What the component shim must do in response to a gateway frame. Keeps all the
/// socket/timer side effects in the shim while the decision logic stays pure and
/// unit-testable.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameAction {
    /// Session state was updated; nothing else for the shim to do.
    None,
    /// The Hello response (IDENTIFY / RESUME): send it, and treat the handshake
    /// as underway — the shim resets its reconnect backoff and starts heartbeats.
    Handshake(String),
    /// Send this text frame (a server-requested heartbeat).
    Send(String),
    /// Deliver this decoded inbound message to the host queue.
    Emit(Inbound),
    /// Close the socket and reconnect. `keep_session` = RESUME the existing
    /// session vs a fresh IDENTIFY.
    Reconnect { keep_session: bool },
}

/// The resumable Gateway session: identity, the resume coordinates
/// (`session_id`/`resume_url`/`seq`), and whether it is live. The shim owns the
/// socket handle, timers, and backoff; this owns the protocol decisions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    pub bot_user_id: String,
    pub self_handle: Option<String>,
    pub session_id: Option<String>,
    pub resume_url: Option<String>,
    pub seq: Option<i64>,
    pub identified: bool,
    pub hb_interval_ms: u64,
}

impl Session {
    /// A live session can be resumed: it has a session id and a last sequence.
    pub fn can_resume(&self) -> bool {
        self.session_id.is_some() && self.seq.is_some()
    }

    /// Whether Hello has been seen on the current connection, so heartbeats may
    /// flow. `hb_interval_ms == 0` means no Hello yet.
    pub fn hello_seen(&self) -> bool {
        self.hb_interval_ms > 0
    }

    /// Drop the connection-scoped flags (heartbeat interval, live) but keep
    /// resume coordinates, so the next connect RESUMEs.
    pub fn mark_disconnected(&mut self) {
        self.identified = false;
        self.hb_interval_ms = 0;
    }

    /// Discard the session entirely so the next connect does a fresh IDENTIFY.
    pub fn wipe(&mut self) {
        self.session_id = None;
        self.resume_url = None;
        self.seq = None;
        self.identified = false;
        self.hb_interval_ms = 0;
    }

    /// Process one gateway text frame, updating session state and returning the
    /// [`FrameAction`] the shim performs. `pending_resume` is whether the shim
    /// intends to resume on the current connection (drives Hello → RESUME vs
    /// IDENTIFY). A non-JSON frame is ignored (`None`).
    pub fn on_frame(
        &mut self,
        cfg: &DiscordConfig,
        text: &str,
        pending_resume: bool,
    ) -> FrameAction {
        let Some(frame) = parse_frame(text) else {
            return FrameAction::None;
        };
        if let Some(s) = frame.s {
            self.seq = Some(s);
        }
        match frame.op {
            // Hello: adopt the heartbeat interval, then RESUME or IDENTIFY.
            10 => {
                self.hb_interval_ms = frame
                    .d
                    .get("heartbeat_interval")
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_HEARTBEAT_MS);
                let payload = match (pending_resume, self.session_id.as_deref(), self.seq) {
                    (true, Some(sid), Some(seq)) => build_resume(&cfg.bot_token, sid, seq),
                    _ => build_identify(&cfg.bot_token, cfg.intents()),
                };
                FrameAction::Handshake(payload)
            }
            // Server asked for an immediate heartbeat.
            1 => FrameAction::Send(build_heartbeat(self.seq)),
            // Heartbeat ACK.
            11 => FrameAction::None,
            // Reconnect: keep the session and resume.
            7 => FrameAction::Reconnect { keep_session: true },
            // Invalid session: `d` is a bare bool — resumable or not.
            9 => {
                let resumable = frame.d.as_bool().unwrap_or(false);
                if !resumable {
                    self.wipe();
                }
                FrameAction::Reconnect {
                    keep_session: resumable,
                }
            }
            // Dispatch, routed by `t`.
            0 => match frame.t.as_deref() {
                Some("READY") => {
                    self.session_id = frame
                        .d
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(String::from);
                    self.resume_url = frame
                        .d
                        .get("resume_gateway_url")
                        .and_then(Value::as_str)
                        .map(String::from);
                    if let Some(uid) = ready_user_id(&frame.d) {
                        self.bot_user_id = uid;
                    }
                    if let Some(name) = ready_username(&frame.d) {
                        self.self_handle = Some(name);
                    }
                    self.identified = true;
                    FrameAction::None
                }
                Some("RESUMED") => {
                    self.identified = true;
                    FrameAction::None
                }
                Some("MESSAGE_CREATE") => {
                    match message_create_to_inbound(&frame.d, cfg, &self.bot_user_id) {
                        Some(inb) => FrameAction::Emit(inb),
                        None => FrameAction::None,
                    }
                }
                _ => FrameAction::None,
            },
            _ => FrameAction::None,
        }
    }
}

// ── Close-code classification (used when the host surfaces the numeric code) ──

/// Discord close codes that are fatal — the bot is misconfigured and MUST NOT
/// reconnect: 4004 auth failed, 4010 invalid shard, 4011 sharding required,
/// 4012 invalid API version, 4013 invalid intents, 4014 disallowed intents.
pub fn is_fatal_close_code(code: u16) -> bool {
    matches!(code, 4004 | 4010 | 4011 | 4012 | 4013 | 4014)
}

/// Close codes after which the session cannot be resumed and a fresh IDENTIFY is
/// required: 4007 invalid seq, 4009 session timed out.
pub fn requires_new_session(code: u16) -> bool {
    matches!(code, 4007 | 4009)
}

/// Parse a leading Discord close code out of a host `Closed` reason string. The
/// host may prefix the numeric code (`"4014: Disallowed intent(s)."`); when it
/// does not, this returns `None` and the caller treats the close as transient.
pub fn close_code_from_reason(reason: &str) -> Option<u16> {
    let head: String = reason.chars().take_while(|c| c.is_ascii_digit()).collect();
    if head.is_empty() {
        return None;
    }
    head.parse::<u16>().ok()
}

// ── Bot id from token ─────────────────────────────────────────────────────────

/// Derive the bot's user id (a decimal snowflake) from its token: the first
/// `.`-segment is the base64 of the ASCII id. Accepts both standard (`+/`) and
/// url-safe (`-_`) alphabets, with or without padding. Returns `None` if the
/// segment does not decode to a non-empty ASCII-digit string. Lets the shim
/// self-filter and advertise its mention form before the first READY arrives.
pub fn bot_user_id_from_token(token: &str) -> Option<String> {
    let seg = token.split('.').next()?;
    let bytes = base64_decode(seg)?;
    let s = String::from_utf8(bytes).ok()?;
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        Some(s)
    } else {
        None
    }
}

/// Minimal base64 decoder (standard + url-safe alphabets, optional padding). No
/// external crate so the pure core stays dependency-free.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn sextet(b: u8) -> Option<u32> {
        Some(match b {
            b'A'..=b'Z' => u32::from(b - b'A'),
            b'a'..=b'z' => u32::from(b - b'a') + 26,
            b'0'..=b'9' => u32::from(b - b'0') + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for b in input.bytes() {
        if b == b'=' {
            break;
        }
        let v = sextet(b)?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

// ── Message chunking ──────────────────────────────────────────────────────────

/// Split `content` into chunks of at most `max_chars` characters, preferring a
/// newline boundary near the limit so multi-line replies don't split mid-line.
/// Counts by `char`, not bytes, so multibyte text is never split mid-codepoint.
/// Empty input yields a single empty chunk so an intentional blank send still
/// posts once.
pub fn chunk_text(content: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= max_chars {
        return vec![content.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let hard_end = (start + max_chars).min(chars.len());
        // Prefer to break at the last newline in the window (but not so early we
        // emit a tiny chunk); otherwise take the whole window.
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

    #[test]
    fn baseline_intents_matches_native() {
        // guilds(1) | guild_messages(512) | direct_messages(4096) | content(32768)
        assert_eq!(BASELINE_INTENTS, 37377);
    }

    #[test]
    fn config_parses_fields_and_defaults() {
        let cfg = DiscordConfig::from_json(
            r#"{"bot_token":"abc.def.ghi","guild_ids":["1","2"],"channel_ids":[99],
                "listen_to_bots":true,"mention_only":true,"enabled":true}"#,
        );
        assert_eq!(cfg.bot_token, "abc.def.ghi");
        assert_eq!(cfg.guild_ids, vec!["1", "2"]);
        assert_eq!(cfg.channel_ids, vec!["99"], "numeric ids coerce to strings");
        assert!(cfg.listen_to_bots);
        assert!(cfg.mention_only);
        assert_eq!(cfg.intents(), BASELINE_INTENTS);
    }

    #[test]
    fn config_intents_mask_override_wins_including_zero() {
        let cfg = DiscordConfig::from_json(r#"{"bot_token":"t","intents_mask":0}"#);
        assert_eq!(cfg.intents(), 0, "explicit 0 override is honored verbatim");
        let cfg = DiscordConfig::from_json(r#"{"bot_token":"t","intents_mask":33280}"#);
        assert_eq!(cfg.intents(), 33280);
    }

    #[test]
    fn config_empty_on_garbage() {
        assert_eq!(DiscordConfig::from_json("not json").bot_token, "");
        assert_eq!(DiscordConfig::from_json("{}").bot_token, "");
    }

    #[test]
    fn config_api_base_and_gateway_url_overrides() {
        // Defaults: real API base, no gateway override.
        let cfg = DiscordConfig::from_json(r#"{"bot_token":"t"}"#);
        assert_eq!(cfg.api_base, DISCORD_API_BASE);
        assert_eq!(cfg.gateway_url, None);
        // Overrides parse (and api_base loses a trailing slash).
        let cfg = DiscordConfig::from_json(
            r#"{"bot_token":"t","api_base":"http://127.0.0.1:9/api/","gateway_url":"ws://127.0.0.1:8"}"#,
        );
        assert_eq!(cfg.api_base, "http://127.0.0.1:9/api");
        assert_eq!(cfg.gateway_url.as_deref(), Some("ws://127.0.0.1:8"));
        // Empty strings fall back rather than producing a broken base/override.
        let cfg = DiscordConfig::from_json(r#"{"bot_token":"t","api_base":"","gateway_url":""}"#);
        assert_eq!(cfg.api_base, DISCORD_API_BASE);
        assert_eq!(cfg.gateway_url, None);
    }

    #[test]
    fn identify_payload_shape() {
        let v: Value = serde_json::from_str(&build_identify("TKN", 37377)).unwrap();
        assert_eq!(v["op"], 2);
        assert_eq!(v["d"]["token"], "TKN");
        assert_eq!(v["d"]["intents"], 37377);
        assert_eq!(v["d"]["properties"]["os"], "linux");
        assert_eq!(v["d"]["properties"]["browser"], "zeroclaw");
        assert_eq!(v["d"]["properties"]["device"], "zeroclaw");
    }

    #[test]
    fn resume_payload_shape() {
        let v: Value = serde_json::from_str(&build_resume("TKN", "sess-1", 42)).unwrap();
        assert_eq!(v["op"], 6);
        assert_eq!(v["d"]["token"], "TKN");
        assert_eq!(v["d"]["session_id"], "sess-1");
        assert_eq!(v["d"]["seq"], 42);
    }

    #[test]
    fn heartbeat_payload_seq_and_null() {
        let v: Value = serde_json::from_str(&build_heartbeat(Some(7))).unwrap();
        assert_eq!(v["op"], 1);
        assert_eq!(v["d"], 7);
        let v: Value = serde_json::from_str(&build_heartbeat(None)).unwrap();
        assert_eq!(v["op"], 1);
        assert!(v["d"].is_null(), "no sequence yet → d is null");
    }

    #[test]
    fn parse_frame_extracts_op_t_s_d() {
        let f =
            parse_frame(r#"{"op":0,"t":"MESSAGE_CREATE","s":12,"d":{"content":"hi"}}"#).unwrap();
        assert_eq!(f.op, 0);
        assert_eq!(f.t.as_deref(), Some("MESSAGE_CREATE"));
        assert_eq!(f.s, Some(12));
        assert_eq!(f.d["content"], "hi");
        assert!(parse_frame("garbage").is_none());
    }

    fn msg(json: &str) -> Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn message_create_basic_emit() {
        let cfg = DiscordConfig::default();
        let d = msg(r#"{"id":"100","type":0,"channel_id":"chan","guild_id":"g",
                "author":{"id":"user-1","username":"alice"},"content":"hello bot"}"#);
        let inb = message_create_to_inbound(&d, &cfg, "bot-9").unwrap();
        assert_eq!(inb.id, "discord_100");
        assert_eq!(inb.sender, "user-1");
        assert_eq!(inb.reply_target, "chan");
        assert_eq!(inb.content, "hello bot");
    }

    #[test]
    fn message_create_drops_self_and_bots_and_nonconversational() {
        let cfg = DiscordConfig::default();
        // self
        let d = msg(r#"{"id":"1","type":0,"channel_id":"c","author":{"id":"me"},"content":"x"}"#);
        assert!(message_create_to_inbound(&d, &cfg, "me").is_none());
        // other bot, listen_to_bots off
        let d = msg(
            r#"{"id":"1","type":0,"channel_id":"c","author":{"id":"b","bot":true},"content":"x"}"#,
        );
        assert!(message_create_to_inbound(&d, &cfg, "me").is_none());
        // non-conversational type (18 = THREAD_CREATED)
        let d = msg(r#"{"id":"1","type":18,"channel_id":"c","author":{"id":"u"},"content":"x"}"#);
        assert!(message_create_to_inbound(&d, &cfg, "me").is_none());
        // empty content
        let d = msg(r#"{"id":"1","type":0,"channel_id":"c","author":{"id":"u"},"content":"  "}"#);
        assert!(message_create_to_inbound(&d, &cfg, "me").is_none());
    }

    #[test]
    fn message_create_listen_to_bots_admits_bots() {
        let cfg = DiscordConfig {
            listen_to_bots: true,
            ..Default::default()
        };
        let d = msg(
            r#"{"id":"1","type":0,"channel_id":"c","author":{"id":"b","bot":true},"content":"hi"}"#,
        );
        assert!(message_create_to_inbound(&d, &cfg, "me").is_some());
    }

    #[test]
    fn message_create_guild_and_channel_allowlists() {
        let cfg = DiscordConfig {
            guild_ids: vec!["g-ok".into()],
            channel_ids: vec!["c-ok".into()],
            ..Default::default()
        };
        // wrong guild
        let d = msg(
            r#"{"id":"1","type":0,"guild_id":"g-no","channel_id":"c-ok","author":{"id":"u"},"content":"x"}"#,
        );
        assert!(message_create_to_inbound(&d, &cfg, "me").is_none());
        // wrong channel
        let d = msg(
            r#"{"id":"1","type":0,"guild_id":"g-ok","channel_id":"c-no","author":{"id":"u"},"content":"x"}"#,
        );
        assert!(message_create_to_inbound(&d, &cfg, "me").is_none());
        // both ok
        let d = msg(
            r#"{"id":"1","type":0,"guild_id":"g-ok","channel_id":"c-ok","author":{"id":"u"},"content":"x"}"#,
        );
        assert!(message_create_to_inbound(&d, &cfg, "me").is_some());
        // DM (no guild_id) passes the guild allowlist
        let d = msg(r#"{"id":"1","type":0,"channel_id":"c-ok","author":{"id":"u"},"content":"x"}"#);
        assert!(message_create_to_inbound(&d, &cfg, "me").is_some());
    }

    #[test]
    fn message_create_mention_only_gates_guild_not_dm() {
        let cfg = DiscordConfig {
            mention_only: true,
            ..Default::default()
        };
        // guild message without mention → dropped
        let d = msg(
            r#"{"id":"1","type":0,"guild_id":"g","channel_id":"c","author":{"id":"u"},"content":"hello"}"#,
        );
        assert!(message_create_to_inbound(&d, &cfg, "bot").is_none());
        // guild message with mention → admitted
        let d = msg(
            r#"{"id":"1","type":0,"guild_id":"g","channel_id":"c","author":{"id":"u"},"content":"<@bot> hi"}"#,
        );
        assert!(message_create_to_inbound(&d, &cfg, "bot").is_some());
        // DM without mention → admitted (mention gate is guild-only)
        let d = msg(r#"{"id":"1","type":0,"channel_id":"c","author":{"id":"u"},"content":"hi"}"#);
        assert!(message_create_to_inbound(&d, &cfg, "bot").is_some());
    }

    #[test]
    fn mention_matches_both_forms() {
        assert!(contains_bot_mention("hey <@123> there", "123"));
        assert!(contains_bot_mention("hey <@!123> there", "123"));
        assert!(!contains_bot_mention("hey <@999> there", "123"));
        assert!(!contains_bot_mention("no mention", ""));
    }

    #[test]
    fn ready_accessors() {
        let d = msg(r#"{"session_id":"s","user":{"id":"botid","username":"mybot"}}"#);
        assert_eq!(ready_user_id(&d).as_deref(), Some("botid"));
        assert_eq!(ready_username(&d).as_deref(), Some("@mybot"));
    }

    #[test]
    fn close_code_classification() {
        assert!(is_fatal_close_code(4004));
        assert!(is_fatal_close_code(4014));
        assert!(!is_fatal_close_code(4000));
        assert!(requires_new_session(4007));
        assert!(requires_new_session(4009));
        assert!(!requires_new_session(4004));
    }

    #[test]
    fn close_code_parsed_from_reason_prefix() {
        assert_eq!(
            close_code_from_reason("4014: Disallowed intent(s)."),
            Some(4014)
        );
        assert_eq!(close_code_from_reason("4004"), Some(4004));
        // No numeric prefix (host did not surface a code) → None → transient.
        assert_eq!(close_code_from_reason("Authentication failed."), None);
        assert_eq!(close_code_from_reason(""), None);
    }

    #[test]
    fn bot_id_from_token_decodes_first_segment() {
        // "MTIzNDU2" is base64("123456"); the rest of the token is ignored.
        assert_eq!(
            bot_user_id_from_token("MTIzNDU2.Gh1234.abcdefХ").as_deref(),
            Some("123456")
        );
        // Non-digit decode (garbage) → None rather than a bogus id.
        assert_eq!(bot_user_id_from_token("aGVsbG8.x.y"), None); // base64("hello")
        assert_eq!(bot_user_id_from_token(""), None);
    }

    #[test]
    fn chunk_text_under_limit_is_single() {
        assert_eq!(chunk_text("hello", 2000), vec!["hello"]);
        assert_eq!(chunk_text("", 2000), vec![""]);
    }

    #[test]
    fn chunk_text_splits_and_preserves_all_chars() {
        let body = "a".repeat(4500);
        let chunks = chunk_text(&body, 2000);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.chars().count() <= 2000));
        assert_eq!(chunks.concat(), body, "no characters lost across chunks");
    }

    #[test]
    fn chunk_text_prefers_newline_boundary() {
        let mut body = "x".repeat(1500);
        body.push('\n');
        body.push_str(&"y".repeat(1500));
        let chunks = chunk_text(&body, 2000);
        assert_eq!(chunks.len(), 2);
        assert!(
            chunks[0].ends_with('\n'),
            "first chunk breaks at the newline"
        );
        assert_eq!(chunks[1], "y".repeat(1500));
    }

    #[test]
    fn send_body_shape() {
        assert_eq!(build_send_body("hi"), json!({"content":"hi"}));
    }

    // ── Session state machine ────────────────────────────────────────────────

    fn cfg_with_token() -> DiscordConfig {
        DiscordConfig {
            bot_token: "TKN".into(),
            ..Default::default()
        }
    }

    #[test]
    fn hello_fresh_sends_identify_and_adopts_interval() {
        let cfg = cfg_with_token();
        let mut s = Session::default();
        let action = s.on_frame(&cfg, r#"{"op":10,"d":{"heartbeat_interval":30000}}"#, false);
        assert_eq!(s.hb_interval_ms, 30000);
        assert!(s.hello_seen(), "Hello enables heartbeats");
        match action {
            FrameAction::Handshake(payload) => {
                let v: Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["op"], 2, "no session yet → IDENTIFY");
                assert_eq!(v["d"]["intents"], BASELINE_INTENTS);
            }
            other => panic!("expected Handshake(identify), got {other:?}"),
        }
    }

    #[test]
    fn hello_with_pending_resume_sends_resume() {
        let cfg = cfg_with_token();
        let mut s = Session {
            session_id: Some("sess".into()),
            seq: Some(5),
            ..Default::default()
        };
        let action = s.on_frame(&cfg, r#"{"op":10,"d":{"heartbeat_interval":41250}}"#, true);
        match action {
            FrameAction::Handshake(payload) => {
                let v: Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["op"], 6, "resumable + pending_resume → RESUME");
                assert_eq!(v["d"]["session_id"], "sess");
                assert_eq!(v["d"]["seq"], 5);
            }
            other => panic!("expected Handshake(resume), got {other:?}"),
        }
    }

    #[test]
    fn hello_missing_interval_falls_back_to_default() {
        let cfg = cfg_with_token();
        let mut s = Session::default();
        s.on_frame(&cfg, r#"{"op":10,"d":{}}"#, false);
        assert_eq!(s.hb_interval_ms, DEFAULT_HEARTBEAT_MS);
    }

    #[test]
    fn sequence_tracked_from_any_frame() {
        let cfg = cfg_with_token();
        let mut s = Session::default();
        s.on_frame(&cfg, r#"{"op":11,"s":99}"#, false);
        assert_eq!(s.seq, Some(99));
    }

    #[test]
    fn server_heartbeat_request_replies_with_heartbeat() {
        let cfg = cfg_with_token();
        let mut s = Session {
            seq: Some(7),
            ..Default::default()
        };
        match s.on_frame(&cfg, r#"{"op":1,"d":null}"#, false) {
            FrameAction::Send(p) => {
                let v: Value = serde_json::from_str(&p).unwrap();
                assert_eq!(v["op"], 1);
                assert_eq!(v["d"], 7);
            }
            other => panic!("expected Send(heartbeat), got {other:?}"),
        }
    }

    #[test]
    fn heartbeat_ack_is_noop() {
        let cfg = cfg_with_token();
        let mut s = Session::default();
        assert_eq!(s.on_frame(&cfg, r#"{"op":11}"#, false), FrameAction::None);
    }

    #[test]
    fn reconnect_op7_keeps_session() {
        let cfg = cfg_with_token();
        let mut s = Session {
            session_id: Some("sess".into()),
            seq: Some(3),
            ..Default::default()
        };
        assert_eq!(
            s.on_frame(&cfg, r#"{"op":7}"#, false),
            FrameAction::Reconnect { keep_session: true }
        );
        assert!(s.can_resume(), "op 7 must not wipe the session");
    }

    #[test]
    fn invalid_session_resumable_keeps_state() {
        let cfg = cfg_with_token();
        let mut s = Session {
            session_id: Some("sess".into()),
            seq: Some(3),
            ..Default::default()
        };
        assert_eq!(
            s.on_frame(&cfg, r#"{"op":9,"d":true}"#, false),
            FrameAction::Reconnect { keep_session: true }
        );
        assert!(s.can_resume());
    }

    #[test]
    fn invalid_session_nonresumable_wipes_state() {
        let cfg = cfg_with_token();
        let mut s = Session {
            session_id: Some("sess".into()),
            resume_url: Some("wss://resume".into()),
            seq: Some(3),
            identified: true,
            ..Default::default()
        };
        assert_eq!(
            s.on_frame(&cfg, r#"{"op":9,"d":false}"#, false),
            FrameAction::Reconnect {
                keep_session: false
            }
        );
        assert!(!s.can_resume(), "non-resumable invalid session must wipe");
        assert_eq!(s.session_id, None);
        assert_eq!(s.resume_url, None);
        assert!(!s.identified);
    }

    #[test]
    fn ready_captures_session_identity_and_seq() {
        let cfg = cfg_with_token();
        let mut s = Session::default();
        let ready = r#"{"op":0,"t":"READY","s":1,"d":{
            "session_id":"sess-abc","resume_gateway_url":"wss://resume.gg",
            "user":{"id":"bot-77","username":"zc"}}}"#;
        assert_eq!(s.on_frame(&cfg, ready, false), FrameAction::None);
        assert_eq!(s.session_id.as_deref(), Some("sess-abc"));
        assert_eq!(s.resume_url.as_deref(), Some("wss://resume.gg"));
        assert_eq!(s.bot_user_id, "bot-77");
        assert_eq!(s.self_handle.as_deref(), Some("@zc"));
        assert_eq!(s.seq, Some(1));
        assert!(s.identified);
        assert!(s.can_resume());
    }

    #[test]
    fn resumed_marks_identified() {
        let cfg = cfg_with_token();
        let mut s = Session::default();
        assert_eq!(
            s.on_frame(&cfg, r#"{"op":0,"t":"RESUMED","d":{}}"#, false),
            FrameAction::None
        );
        assert!(s.identified);
    }

    #[test]
    fn message_create_emits_inbound_and_respects_self_filter() {
        let cfg = cfg_with_token();
        let mut s = Session {
            bot_user_id: "bot-77".into(),
            ..Default::default()
        };
        // A real user's message → Emit.
        let m = r#"{"op":0,"t":"MESSAGE_CREATE","s":2,"d":{
            "id":"555","type":0,"channel_id":"chan","author":{"id":"u1"},"content":"hi"}}"#;
        match s.on_frame(&cfg, m, false) {
            FrameAction::Emit(inb) => {
                assert_eq!(inb.id, "discord_555");
                assert_eq!(inb.reply_target, "chan");
            }
            other => panic!("expected Emit, got {other:?}"),
        }
        assert_eq!(s.seq, Some(2), "dispatch sequence tracked");
        // The bot's own message → None (self-loop guard).
        let own = r#"{"op":0,"t":"MESSAGE_CREATE","d":{
            "id":"556","type":0,"channel_id":"chan","author":{"id":"bot-77"},"content":"echo"}}"#;
        assert_eq!(s.on_frame(&cfg, own, false), FrameAction::None);
    }

    #[test]
    fn unknown_op_and_garbage_are_noops() {
        let cfg = cfg_with_token();
        let mut s = Session::default();
        assert_eq!(s.on_frame(&cfg, r#"{"op":42}"#, false), FrameAction::None);
        assert_eq!(s.on_frame(&cfg, "not json", false), FrameAction::None);
        assert_eq!(
            s.on_frame(&cfg, r#"{"op":0,"t":"TYPING_START","d":{}}"#, false),
            FrameAction::None
        );
    }

    #[test]
    fn wipe_and_mark_disconnected() {
        let mut s = Session {
            session_id: Some("x".into()),
            resume_url: Some("y".into()),
            seq: Some(1),
            identified: true,
            bot_user_id: "keepme".into(),
            ..Default::default()
        };
        s.mark_disconnected();
        assert!(!s.identified);
        assert!(s.can_resume(), "mark_disconnected keeps resume coordinates");
        s.wipe();
        assert!(!s.can_resume());
        assert_eq!(s.bot_user_id, "keepme", "identity survives a wipe");
    }
}
