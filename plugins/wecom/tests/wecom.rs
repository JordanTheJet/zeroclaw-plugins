use serde_json::json;
use wecom::wecom::{build_text_body, check_api_response, webhook_url, WeComConfig};

#[test]
fn config_to_webhook_request() {
    let cfg = WeComConfig::from_json(
        r#"{"enabled":true,"webhook_key":"bot-key","excluded_tools":["shell"]}"#,
    );
    assert!(cfg.is_configured());
    assert_eq!(
        webhook_url(cfg.webhook_key()),
        "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=bot%2Dkey"
    );
    let body = build_text_body("hello");
    assert_eq!(body["msgtype"], "text");
    assert_eq!(body["text"]["content"], "hello");
}

#[test]
fn api_result_controls_send_success() {
    assert!(check_api_response(&json!({"errcode": 0, "errmsg": "ok"})).is_ok());
    assert!(check_api_response(&json!({"errcode": 40014, "errmsg": "invalid key"})).is_err());
}
