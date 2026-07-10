//! A ZeroClaw WIT **channel** plugin source scaffold: Twitch.
//!
//! This is the Phase 4 migration landing point for the built-in `twitch`
//! channel. It compiles as a channel plugin and mirrors the existing channel
//! config via `provides = "twitch"`, but remains `registry = false` until
//! the native transport/API behavior is fully ported and validated.

pub mod twitch;

#[cfg(target_family = "wasm")]
mod component {
    wit_bindgen::generate!({
        path: "../../wit/v0",
        world: "channel-plugin",
        features: ["plugins-wit-v0"],
    });

    use std::cell::RefCell;

    use crate::twitch::{send_unavailable, SourceOnlyConfig, CHANNEL, PLUGIN_NAME};

    use exports::zeroclaw::plugin::channel::{
        ApprovalRequest, ApprovalResponse, ChannelCapabilities, Guest as Channel, InboundMessage,
        SendMessage,
    };
    use exports::zeroclaw::plugin::plugin_info::Guest as PluginInfo;
    use zeroclaw::plugin::inbound::{self, HostInboundMessage};

    const PLUGIN_VERSION: &str = "0.1.0";

    thread_local! {
        static CONFIG: RefCell<SourceOnlyConfig> = RefCell::new(SourceOnlyConfig::default());
    }

    fn from_host(msg: HostInboundMessage) -> InboundMessage {
        InboundMessage {
            id: msg.id,
            sender: msg.sender,
            reply_target: msg.reply_target,
            content: msg.content,
            channel: if msg.channel.is_empty() {
                CHANNEL.to_string()
            } else {
                msg.channel
            },
            channel_alias: msg.channel_alias,
            timestamp: msg.timestamp,
            thread_ts: msg.thread_ts,
            interruption_scope_id: msg.interruption_scope_id,
            attachments: Vec::new(),
            subject: msg.subject,
        }
    }

    struct SourceOnlyChannel;

    impl PluginInfo for SourceOnlyChannel {
        fn plugin_name() -> String {
            PLUGIN_NAME.to_string()
        }

        fn plugin_version() -> String {
            PLUGIN_VERSION.to_string()
        }
    }

    impl Channel for SourceOnlyChannel {
        fn name() -> String {
            PLUGIN_NAME.to_string()
        }

        fn configure(config: String) -> Result<(), String> {
            CONFIG.with(|c| *c.borrow_mut() = SourceOnlyConfig::from_json(&config));
            Ok(())
        }

        fn send(_message: SendMessage) -> Result<(), String> {
            Err(send_unavailable())
        }

        fn poll_message() -> Option<InboundMessage> {
            inbound::inbound_poll().map(from_host)
        }

        fn get_channel_capabilities() -> ChannelCapabilities {
            ChannelCapabilities::HEALTH_CHECK
                | ChannelCapabilities::SELF_HANDLE
                | ChannelCapabilities::SELF_ADDRESSED_MENTION
        }

        fn health_check() -> bool {
            false
        }

        fn self_handle() -> Option<String> {
            CONFIG.with(|c| c.borrow().self_handle.clone())
        }

        fn self_addressed_mention() -> Option<String> {
            CONFIG.with(|c| c.borrow().self_handle.clone())
        }

        fn drop_self_message(_msg: InboundMessage) -> bool {
            false
        }
        fn start_typing(_recipient: String) -> Result<(), String> {
            Ok(())
        }
        fn stop_typing(_recipient: String) -> Result<(), String> {
            Ok(())
        }
        fn supports_draft_updates() -> bool {
            false
        }
        fn send_draft(_message: SendMessage) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn update_draft(_r: String, _m: String, _t: String) -> Result<(), String> {
            Ok(())
        }
        fn update_draft_progress(_r: String, _m: String, _t: String) -> Result<(), String> {
            Ok(())
        }
        fn finalize_draft(_r: String, _m: String, _t: String) -> Result<(), String> {
            Ok(())
        }
        fn cancel_draft(_r: String, _m: String) -> Result<(), String> {
            Ok(())
        }
        fn supports_multi_message_streaming() -> bool {
            false
        }
        fn multi_message_delay_ms() -> u64 {
            800
        }
        fn add_reaction(_c: String, _m: String, _e: String) -> Result<(), String> {
            Ok(())
        }
        fn remove_reaction(_c: String, _m: String, _e: String) -> Result<(), String> {
            Ok(())
        }
        fn pin_message(_c: String, _m: String) -> Result<(), String> {
            Ok(())
        }
        fn unpin_message(_c: String, _m: String) -> Result<(), String> {
            Ok(())
        }
        fn redact_message(_c: String, _m: String, _reason: Option<String>) -> Result<(), String> {
            Ok(())
        }
        fn request_approval(
            _recipient: String,
            _request: ApprovalRequest,
        ) -> Result<Option<ApprovalResponse>, String> {
            Ok(None)
        }
        fn request_choice(
            _question: String,
            _choices: Vec<String>,
            _timeout_secs: u64,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn supports_free_form_ask() -> bool {
            false
        }
        fn webhook_path() -> Option<String> {
            None
        }
        fn parse_webhook(
            _headers: Vec<(String, String)>,
            _body: Vec<u8>,
        ) -> Result<Vec<InboundMessage>, String> {
            Err(format!(
                "{PLUGIN_NAME}: webhook ingress is not implemented in this source-only scaffold"
            ))
        }
    }

    export!(SourceOnlyChannel);
}
