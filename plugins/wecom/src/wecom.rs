//! Pure WeCom Bot Webhook logic. The WASM shim owns the HTTP request.

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Value};

pub const CHANNEL: &str = "wecom";
const WEBHOOK_BASE: &str = "https://qyapi.weixin.qq.com/cgi-bin/webhook/send";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct WeComConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub webhook_key: String,
}

impl WeComConfig {
    pub fn from_json(input: &str) -> Self {
        serde_json::from_str(input).unwrap_or_default()
    }

    pub fn webhook_key(&self) -> &str {
        self.webhook_key.trim()
    }

    pub fn is_configured(&self) -> bool {
        !self.webhook_key().is_empty()
    }
}

pub fn webhook_url(key: &str) -> String {
    format!(
        "{WEBHOOK_BASE}?key={}",
        utf8_percent_encode(key.trim(), NON_ALPHANUMERIC)
    )
}

pub fn build_text_body(content: &str) -> Value {
    json!({
        "msgtype": "text",
        "text": {
            "content": content,
        }
    })
}

pub fn check_api_response(value: &Value) -> Result<(), String> {
    let errcode = value.get("errcode").and_then(Value::as_i64).unwrap_or(-1);
    if errcode == 0 {
        return Ok(());
    }
    let message = value
        .get("errmsg")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    Err(format!("wecom API error (errcode={errcode}): {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_uses_native_webhook_key() {
        let cfg = WeComConfig::from_json(
            r#"{"enabled":true,"webhook_key":" key-123 ","excluded_tools":["shell"]}"#,
        );
        assert!(cfg.is_configured());
        assert_eq!(cfg.webhook_key(), "key-123");
    }

    #[test]
    fn webhook_url_encodes_key_as_query_value() {
        assert_eq!(
            webhook_url("abc/123+z"),
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=abc%2F123%2Bz"
        );
    }

    #[test]
    fn text_body_matches_wecom_contract() {
        let body = build_text_body("hello");
        assert_eq!(body["msgtype"], "text");
        assert_eq!(body["text"]["content"], "hello");
    }

    #[test]
    fn response_requires_zero_errcode() {
        assert!(check_api_response(&json!({"errcode": 0, "errmsg": "ok"})).is_ok());
        assert_eq!(
            check_api_response(&json!({"errcode": 93000, "errmsg": "invalid webhook"}))
                .unwrap_err(),
            "wecom API error (errcode=93000): invalid webhook"
        );
        assert!(check_api_response(&json!({})).is_err());
    }
}
