//! Pure WeCom source-only channel migration logic.
//!
//! This module is deliberately host-testable and performs no I/O. The wasm shim
//! owns the WIT boundary; transport work stays host-gated until the native
//! WeCom channel is fully ported into a publishable plugin.

use serde::Deserialize;
use serde_json::Value;

pub const CHANNEL: &str = "wecom";
pub const PLUGIN_NAME: &str = "wecom";
pub const HOST_GATE: &str = "WeCom HTTP/webhook parity is source-only until the host webhook-ingress path is released for plugins.";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceOnlyConfig {
    pub enabled: bool,
    pub self_handle: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    self_handle: Option<String>,
    #[serde(default)]
    bot_username: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

fn default_enabled() -> bool {
    true
}

impl SourceOnlyConfig {
    pub fn from_json(input: &str) -> Self {
        let raw = serde_json::from_str::<RawConfig>(input).unwrap_or_else(|_| RawConfig {
            enabled: true,
            self_handle: None,
            bot_username: None,
            username: None,
            handle: None,
            user_id: None,
            account_id: None,
        });
        Self {
            enabled: raw.enabled,
            self_handle: first_non_empty(&[
                raw.self_handle,
                raw.bot_username,
                raw.username,
                raw.handle,
                raw.user_id,
                raw.account_id,
            ]),
        }
    }
}

pub fn first_non_empty(values: &[Option<String>]) -> Option<String> {
    values
        .iter()
        .filter_map(|v| v.as_deref())
        .map(str::trim)
        .find(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

pub fn has_config(input: &str) -> bool {
    match serde_json::from_str::<Value>(input) {
        Ok(Value::Object(map)) => map.iter().any(|(key, value)| {
            key != "enabled" && !value.is_null() && value.as_str().map(str::is_empty) != Some(true)
        }),
        _ => false,
    }
}

pub fn send_unavailable() -> String {
    format!("{PLUGIN_NAME} ({CHANNEL}): source-only channel migration is host-gated. {HOST_GATE}")
}
