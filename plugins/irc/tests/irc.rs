use irc::irc::{drain_lines, format_privmsg, IrcConfig, IrcSession, SessionAction};

fn config() -> IrcConfig {
    IrcConfig::from_json(
        r##"{
            "enabled": true,
            "server": "irc.example.net",
            "port": 6697,
            "nickname": "zeroclaw",
            "channels": ["#bots"],
            "mention_only": false
        }"##,
    )
    .unwrap()
}

#[test]
fn configuration_drives_registration_commands() {
    let config = config();
    let session = IrcSession::new(&config);
    assert_eq!(
        session.registration_commands(&config),
        ["NICK zeroclaw", "USER zeroclaw 0 * :ZeroClaw"]
    );
}

#[test]
fn fragmented_inbound_message_maps_to_reply_target() {
    let config = config();
    let mut session = IrcSession::new(&config);
    session
        .handle_line(&config, ":server 001 zeroclaw :welcome")
        .unwrap();
    let mut buffer = Vec::new();
    assert!(drain_lines(&mut buffer, b":alice!u@h PRIVMSG #bots :hel")
        .unwrap()
        .is_empty());
    let lines = drain_lines(&mut buffer, b"lo\r\n").unwrap();
    let actions = session.handle_line(&config, &lines[0]).unwrap();
    let SessionAction::Message(message) = &actions[0] else {
        panic!("expected inbound message")
    };
    assert_eq!(message.sender, "alice");
    assert_eq!(message.reply_target, "#bots");
}

#[test]
fn outbound_text_is_split_into_safe_privmsg_lines() {
    assert_eq!(
        format_privmsg("#bots", "first\nsecond").unwrap(),
        ["PRIVMSG #bots :first", "PRIVMSG #bots :second"]
    );
    assert!(format_privmsg("#bots\r\nQUIT", "injected").is_err());
}
