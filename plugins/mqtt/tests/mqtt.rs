use mqtt::mqtt::{
    decode_packet, encode_connect, encode_pingreq, encode_publish, encode_subscribe, MqttConfig,
    MqttSession, Packet, PacketDecoder, ReconnectBackoff, SessionState,
};

fn config(qos: u8) -> MqttConfig {
    MqttConfig::from_json(&format!(
        r#"{{
            "enabled": true,
            "broker_url": "mqtt://broker.example:1883",
            "client_id": "zeroclaw",
            "topics": ["Sensors/#"],
            "qos": {qos},
            "keep_alive_secs": 30
        }}"#
    ))
    .expect("test config is valid")
}

fn online_session(qos: u8) -> (MqttConfig, MqttSession) {
    let config = config(qos);
    let mut session = MqttSession::default();
    session.begin(&config).expect("CONNECT encodes");
    session
        .feed(&[0x20, 0x02, 0x00, 0x00])
        .expect("CONNACK buffers");
    let subscribe = session
        .process_next(&config)
        .expect("CONNACK parses")
        .expect("CONNACK is available")
        .outbound
        .pop()
        .expect("SUBSCRIBE is emitted");
    let packet_id = u16::from_be_bytes([subscribe[2], subscribe[3]]);
    session
        .feed(&[0x90, 0x03, (packet_id >> 8) as u8, packet_id as u8, qos])
        .expect("SUBACK buffers");
    session
        .process_next(&config)
        .expect("SUBACK parses")
        .expect("SUBACK is available");
    assert_eq!(session.state(), SessionState::Online);
    (config, session)
}

#[test]
fn config_uses_native_fields_and_resolves_broker_endpoint() {
    let config = MqttConfig::from_json(
        r#"{
            "enabled": true,
            "broker_url": "mqtts://[2001:db8::1]",
            "client_id": "agent-1",
            "topics": ["sensors/+", "alerts/#"],
            "username": "user",
            "password": "secret",
            "use_tls": true
        }"#,
    )
    .expect("native config parses");

    assert_eq!(config.qos, 1);
    assert_eq!(config.keep_alive_secs, 30);
    let endpoint = config.endpoint().expect("endpoint resolves");
    assert_eq!(endpoint.host, "2001:db8::1");
    assert_eq!(endpoint.port, 8883);
    assert!(endpoint.tls);
}

#[test]
fn config_rejects_tls_qos_credentials_and_topic_mismatches() {
    let tls_error = MqttConfig::from_json(
        r#"{"enabled":true,"broker_url":"mqtt://host","client_id":"id","topics":["a"],"use_tls":true}"#,
    )
    .expect_err("scheme mismatch must fail");
    assert!(tls_error.to_string().contains("use_tls"));

    let qos_error = MqttConfig::from_json(
        r#"{"enabled":true,"broker_url":"mqtt://host","client_id":"id","topics":["a"],"qos":3}"#,
    )
    .expect_err("invalid QoS must fail");
    assert!(qos_error.to_string().contains("qos"));

    let password_error = MqttConfig::from_json(
        r#"{"enabled":true,"broker_url":"mqtt://host","client_id":"id","topics":["a"],"password":"secret"}"#,
    )
    .expect_err("password without username must fail");
    assert!(password_error.to_string().contains("username"));

    let topic_error = MqttConfig::from_json(
        r#"{"enabled":true,"broker_url":"mqtt://host","client_id":"id","topics":["bad/#/tail"]}"#,
    )
    .expect_err("invalid topic filter must fail");
    assert!(topic_error.to_string().contains("topic filter"));
}

#[test]
fn connect_packet_matches_mqtt_311_binary_layout() {
    let config = MqttConfig::from_json(
        r#"{
            "enabled": true,
            "broker_url": "mqtt://host:1883",
            "client_id": "zc",
            "topics": ["a"],
            "qos": 0,
            "username": "u",
            "password": "p",
            "keep_alive_secs": 30
        }"#,
    )
    .expect("config parses");

    assert_eq!(
        encode_connect(&config).expect("CONNECT encodes"),
        vec![
            0x10, 0x14, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0xc2, 0x00, 0x1e, 0x00, 0x02,
            b'z', b'c', 0x00, 0x01, b'u', 0x00, 0x01, b'p',
        ]
    );
}

#[test]
fn subscribe_packet_contains_identifier_filters_and_qos() {
    let topics = vec!["sensors/#".to_string(), "alerts/+".to_string()];
    let packet = encode_subscribe(0x1234, &topics, 1).expect("SUBSCRIBE encodes");

    assert_eq!(packet[0], 0x82);
    assert_eq!(&packet[2..4], &[0x12, 0x34]);
    assert_eq!(
        &packet[4..15],
        &[0, 9, b's', b'e', b'n', b's', b'o', b'r', b's', b'/', b'#']
    );
    assert_eq!(packet[15], 1);
    assert_eq!(
        &packet[16..],
        &[0, 8, b'a', b'l', b'e', b'r', b't', b's', b'/', b'+', 1]
    );
}

#[test]
fn decoder_reassembles_fragmented_multibyte_publish_frame() {
    let payload = vec![b'x'; 180];
    let frame =
        encode_publish("Sensors/Temp", &payload, 1, true, Some(7)).expect("PUBLISH encodes");
    assert!(frame[1] & 0x80 != 0, "remaining length uses two bytes");

    let mut decoder = PacketDecoder::default();
    decoder.feed(&frame[..2]).expect("first fragment buffers");
    assert!(decoder.next_packet().expect("fragment is valid").is_none());
    decoder
        .feed(&frame[2..17])
        .expect("second fragment buffers");
    assert!(decoder.next_packet().expect("fragment is valid").is_none());
    decoder.feed(&frame[17..]).expect("final fragment buffers");

    let Packet::Publish(publish) = decoder
        .next_packet()
        .expect("frame parses")
        .expect("frame is complete")
    else {
        panic!("expected PUBLISH");
    };
    assert_eq!(publish.topic, "Sensors/Temp");
    assert_eq!(publish.payload, payload);
    assert_eq!(publish.packet_id, Some(7));
    assert_eq!(publish.qos, 1);
    assert!(publish.retain);
}

#[test]
fn decoder_handles_coalesced_packets_and_rejects_bad_remaining_length() {
    let mut decoder = PacketDecoder::default();
    decoder
        .feed(&[0xd0, 0x00, 0x40, 0x02, 0x00, 0x09])
        .expect("coalesced frames buffer");
    assert_eq!(
        decoder.next_packet().expect("PINGRESP parses"),
        Some(Packet::PingResp)
    );
    assert_eq!(
        decoder.next_packet().expect("PUBACK parses"),
        Some(Packet::PubAck(9))
    );
    assert!(decoder.next_packet().expect("buffer is empty").is_none());

    let error = decode_packet(&[0xd0, 0x80, 0x00]).expect_err("non-minimal length must fail");
    assert!(error.to_string().contains("minimally encoded"));

    let error = decode_packet(&[0x30, 0x81, 0x80, 0x40])
        .expect_err("remaining length above the guest limit must fail");
    assert!(error.to_string().contains("plugin limit"));
}

#[test]
fn session_connects_subscribes_and_maps_case_sensitive_qos1_publish() {
    let (config, mut session) = online_session(1);
    let publish = encode_publish("Sensors/Temp", b"21.5", 1, false, Some(42))
        .expect("inbound PUBLISH encodes");
    session.feed(&publish).expect("PUBLISH buffers");
    let output = session
        .process_next(&config)
        .expect("PUBLISH parses")
        .expect("PUBLISH is available");

    assert_eq!(output.outbound, vec![vec![0x40, 0x02, 0x00, 0x2a]]);
    assert_eq!(output.inbound.len(), 1);
    assert_eq!(output.inbound[0].topic, "Sensors/Temp");
    assert_eq!(output.inbound[0].payload, b"21.5");
}

#[test]
fn qos2_publish_is_delivered_only_after_pubrel() {
    let (config, mut session) = online_session(2);
    let publish = encode_publish("Sensors/Exact", b"payload", 2, false, Some(17))
        .expect("QoS 2 PUBLISH encodes");
    session.feed(&publish).expect("PUBLISH buffers");
    let received = session
        .process_next(&config)
        .expect("PUBLISH parses")
        .expect("PUBLISH is available");
    assert!(received.inbound.is_empty());
    assert_eq!(received.outbound, vec![vec![0x50, 0x02, 0x00, 0x11]]);

    session
        .feed(&[0x62, 0x02, 0x00, 0x11])
        .expect("PUBREL buffers");
    let released = session
        .process_next(&config)
        .expect("PUBREL parses")
        .expect("PUBREL is available");
    assert_eq!(released.outbound, vec![vec![0x70, 0x02, 0x00, 0x11]]);
    assert_eq!(released.inbound[0].payload, b"payload");
}

#[test]
fn outbound_publish_and_ping_packets_use_session_qos() {
    let (config, mut session) = online_session(1);
    let publish = session
        .publish(&config, "Commands/Pump", b"start")
        .expect("online session publishes");
    let Packet::Publish(decoded) = decode_packet(&publish).expect("outbound PUBLISH decodes")
    else {
        panic!("expected PUBLISH");
    };
    assert_eq!(decoded.topic, "Commands/Pump");
    assert_eq!(decoded.payload, b"start");
    assert_eq!(decoded.qos, 1);
    assert!(decoded.packet_id.is_some());
    assert_eq!(encode_pingreq(), vec![0xc0, 0x00]);
}

#[test]
fn disconnect_clears_partial_frames_and_allows_clean_reconnect() {
    let config = config(0);
    let mut session = MqttSession::default();
    session.begin(&config).expect("first CONNECT encodes");
    session
        .feed(&[0x20, 0x02, 0x00])
        .expect("partial CONNACK buffers");
    session.disconnect();
    assert_eq!(session.state(), SessionState::Disconnected);

    session.begin(&config).expect("second CONNECT encodes");
    session
        .feed(&[0x20, 0x02, 0x00, 0x00])
        .expect("fresh CONNACK buffers");
    assert!(session
        .process_next(&config)
        .expect("fresh CONNACK parses")
        .is_some());
}

#[test]
fn reconnect_backoff_is_capped_and_resets_after_success() {
    let mut backoff = ReconnectBackoff::default();
    assert!(backoff.ready(0));
    assert_eq!(backoff.record_failure(1_000), 250);
    assert_eq!(backoff.retry_at_ms(), 1_250);
    assert!(!backoff.ready(1_249));
    assert_eq!(backoff.record_failure(2_000), 500);

    let mut delay = 0;
    for attempt in 0..20 {
        delay = backoff.record_failure(3_000 + attempt);
    }
    assert_eq!(delay, 30_000);
    backoff.reset();
    assert!(backoff.ready(0));
    assert_eq!(backoff.retry_at_ms(), 0);
}

#[test]
fn manifest_uses_sensitive_exact_topic_matching() {
    let manifest = include_str!("../manifest.toml");
    assert!(manifest.contains("sender_match = \"exact\""));
    assert!(!manifest.contains("sender_match = \"case-insensitive\""));
}
