//! Pure MQTT 3.1.1 protocol and session logic.
//!
//! This module performs no I/O. The WASM component owns the host socket calls,
//! while configuration validation, packet framing, decoding, QoS handshakes,
//! and reconnect timing remain host-testable here.

use std::collections::{BTreeMap, BTreeSet};
use std::str;

use serde::Deserialize;
use thiserror::Error;

pub const CHANNEL: &str = "mqtt";
pub const MAX_PACKET_SIZE: usize = 1024 * 1024;

const MAX_REMAINING_LENGTH: usize = 268_435_455;
const MAX_INFLIGHT: usize = 64;
const INITIAL_RECONNECT_DELAY_MS: u64 = 250;
const MAX_RECONNECT_DELAY_MS: u64 = 30_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MqttError {
    #[error("invalid MQTT configuration: {0}")]
    Config(String),
    #[error("malformed MQTT packet: {0}")]
    Malformed(String),
    #[error("MQTT packet size {0} exceeds the {MAX_PACKET_SIZE}-byte plugin limit")]
    PacketTooLarge(usize),
    #[error("MQTT broker rejected CONNECT with return code {0}")]
    ConnectionRefused(u8),
    #[error("MQTT session is not online")]
    NotOnline,
    #[error("MQTT in-flight window is full")]
    InflightFull,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct MqttConfig {
    #[serde(default)]
    pub enabled: bool,
    pub broker_url: String,
    pub client_id: String,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default = "default_qos")]
    pub qos: u8,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub use_tls: bool,
    #[serde(default = "default_keep_alive_secs")]
    pub keep_alive_secs: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerEndpoint {
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

fn default_qos() -> u8 {
    1
}

fn default_keep_alive_secs() -> u64 {
    30
}

impl MqttConfig {
    pub fn from_json(input: &str) -> Result<Self, MqttError> {
        let config: Self = serde_json::from_str(input)
            .map_err(|error| MqttError::Config(format!("invalid JSON: {error}")))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), MqttError> {
        if self.qos > 2 {
            return Err(MqttError::Config(format!(
                "qos must be 0, 1, or 2, got {}",
                self.qos
            )));
        }
        if self.client_id.is_empty() {
            return Err(MqttError::Config("client_id must not be empty".to_string()));
        }
        validate_mqtt_string(&self.client_id, "client_id", false)?;
        if self.topics.is_empty() {
            return Err(MqttError::Config(
                "at least one topic must be configured".to_string(),
            ));
        }
        for topic in &self.topics {
            validate_topic_filter(topic)?;
        }
        if self.password.is_some() && self.username.is_none() {
            return Err(MqttError::Config(
                "password requires username to be configured".to_string(),
            ));
        }
        if let Some(username) = &self.username {
            validate_mqtt_string(username, "username", true)?;
        }
        if let Some(password) = &self.password {
            validate_binary_length(password.as_bytes(), "password")?;
        }
        if self.keep_alive_secs > u16::MAX.into() {
            return Err(MqttError::Config(format!(
                "keep_alive_secs must be at most {}",
                u16::MAX
            )));
        }

        let endpoint = parse_broker_url(&self.broker_url)?;
        if endpoint.tls != self.use_tls {
            let expected = if endpoint.tls { "true" } else { "false" };
            return Err(MqttError::Config(format!(
                "use_tls must be {expected} for the broker_url scheme"
            )));
        }
        Ok(())
    }

    pub fn endpoint(&self) -> Result<BrokerEndpoint, MqttError> {
        parse_broker_url(&self.broker_url)
    }
}

fn parse_broker_url(url: &str) -> Result<BrokerEndpoint, MqttError> {
    let (authority, tls, default_port) = if let Some(rest) = url.strip_prefix("mqtt://") {
        (rest, false, 1883)
    } else if let Some(rest) = url.strip_prefix("mqtts://") {
        (rest, true, 8883)
    } else {
        return Err(MqttError::Config(
            "broker_url must start with mqtt:// or mqtts://".to_string(),
        ));
    };

    if authority.is_empty()
        || authority.contains(['/', '?', '#', '@'])
        || authority.chars().any(char::is_whitespace)
    {
        return Err(MqttError::Config(
            "broker_url must contain only a host and optional port".to_string(),
        ));
    }

    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let close = bracketed.find(']').ok_or_else(|| {
            MqttError::Config("broker_url contains an unterminated IPv6 address".to_string())
        })?;
        let host = &bracketed[..close];
        let suffix = &bracketed[close + 1..];
        let port = if suffix.is_empty() {
            default_port
        } else {
            parse_port(suffix.strip_prefix(':').ok_or_else(|| {
                MqttError::Config("invalid broker_url after IPv6 address".to_string())
            })?)?
        };
        (host, port)
    } else {
        if authority.matches(':').count() > 1 {
            return Err(MqttError::Config(
                "IPv6 broker addresses must be enclosed in brackets".to_string(),
            ));
        }
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, parse_port(port)?),
            None => (authority, default_port),
        }
    };

    if host.is_empty() {
        return Err(MqttError::Config(
            "broker_url host must not be empty".to_string(),
        ));
    }

    Ok(BrokerEndpoint {
        host: host.to_string(),
        port,
        tls,
    })
}

fn parse_port(input: &str) -> Result<u16, MqttError> {
    let port = input
        .parse::<u16>()
        .map_err(|_| MqttError::Config("broker_url contains an invalid port".to_string()))?;
    if port == 0 {
        return Err(MqttError::Config(
            "broker_url port must not be zero".to_string(),
        ));
    }
    Ok(port)
}

fn validate_mqtt_string(value: &str, field: &str, allow_empty: bool) -> Result<(), MqttError> {
    if !allow_empty && value.is_empty() {
        return Err(MqttError::Config(format!("{field} must not be empty")));
    }
    validate_binary_length(value.as_bytes(), field)?;
    if value.chars().any(|ch| {
        ch == '\0'
            || ('\u{0001}'..='\u{001f}').contains(&ch)
            || ('\u{007f}'..='\u{009f}').contains(&ch)
            || ('\u{fdd0}'..='\u{fdef}').contains(&ch)
            || (ch as u32) & 0xffff >= 0xfffe
    }) {
        return Err(MqttError::Config(format!(
            "{field} contains a character MQTT 3.1.1 does not permit"
        )));
    }
    Ok(())
}

fn validate_binary_length(value: &[u8], field: &str) -> Result<(), MqttError> {
    if value.len() > usize::from(u16::MAX) {
        return Err(MqttError::Config(format!(
            "{field} exceeds the MQTT 65,535-byte limit"
        )));
    }
    Ok(())
}

fn validate_topic_filter(topic: &str) -> Result<(), MqttError> {
    validate_mqtt_string(topic, "topic filter", false)?;
    for (index, byte) in topic.bytes().enumerate() {
        match byte {
            b'#' => {
                let valid_prefix = index == 0 || topic.as_bytes()[index - 1] == b'/';
                if !valid_prefix || index + 1 != topic.len() {
                    return Err(MqttError::Config(format!(
                        "invalid MQTT topic filter `{topic}`"
                    )));
                }
            }
            b'+' => {
                let valid_prefix = index == 0 || topic.as_bytes()[index - 1] == b'/';
                let valid_suffix = index + 1 == topic.len() || topic.as_bytes()[index + 1] == b'/';
                if !valid_prefix || !valid_suffix {
                    return Err(MqttError::Config(format!(
                        "invalid MQTT topic filter `{topic}`"
                    )));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_topic_name(topic: &str) -> Result<(), MqttError> {
    validate_mqtt_string(topic, "topic", false).map_err(config_to_malformed)?;
    if topic.contains(['#', '+']) {
        return Err(MqttError::Malformed(
            "PUBLISH topic must not contain wildcards".to_string(),
        ));
    }
    Ok(())
}

fn config_to_malformed(error: MqttError) -> MqttError {
    MqttError::Malformed(error.to_string())
}

pub fn encode_connect(config: &MqttConfig) -> Result<Vec<u8>, MqttError> {
    config.validate()?;

    let mut body = Vec::new();
    push_utf8(&mut body, "MQTT", "protocol name")?;
    body.push(4);
    let mut connect_flags = 0b0000_0010;
    if config.username.is_some() {
        connect_flags |= 0b1000_0000;
    }
    if config.password.is_some() {
        connect_flags |= 0b0100_0000;
    }
    body.push(connect_flags);
    body.extend_from_slice(&(config.keep_alive_secs as u16).to_be_bytes());
    push_utf8(&mut body, &config.client_id, "client_id")?;
    if let Some(username) = &config.username {
        push_utf8(&mut body, username, "username")?;
    }
    if let Some(password) = &config.password {
        push_binary(&mut body, password.as_bytes(), "password")?;
    }
    frame_packet(0x10, body)
}

pub fn encode_subscribe(packet_id: u16, topics: &[String], qos: u8) -> Result<Vec<u8>, MqttError> {
    validate_packet_id(packet_id)?;
    if qos > 2 {
        return Err(MqttError::Malformed(format!(
            "SUBSCRIBE qos must be 0, 1, or 2, got {qos}"
        )));
    }
    if topics.is_empty() {
        return Err(MqttError::Malformed(
            "SUBSCRIBE requires at least one topic filter".to_string(),
        ));
    }

    let mut body = Vec::new();
    body.extend_from_slice(&packet_id.to_be_bytes());
    for topic in topics {
        validate_topic_filter(topic).map_err(config_to_malformed)?;
        push_utf8(&mut body, topic, "topic filter").map_err(config_to_malformed)?;
        body.push(qos);
    }
    frame_packet(0x82, body)
}

pub fn encode_publish(
    topic: &str,
    payload: &[u8],
    qos: u8,
    retain: bool,
    packet_id: Option<u16>,
) -> Result<Vec<u8>, MqttError> {
    validate_topic_name(topic)?;
    if qos > 2 {
        return Err(MqttError::Malformed(format!(
            "PUBLISH qos must be 0, 1, or 2, got {qos}"
        )));
    }
    match (qos, packet_id) {
        (0, None) => {}
        (0, Some(_)) => {
            return Err(MqttError::Malformed(
                "QoS 0 PUBLISH must not contain a packet identifier".to_string(),
            ));
        }
        (_, Some(id)) => validate_packet_id(id)?,
        (_, None) => {
            return Err(MqttError::Malformed(
                "QoS 1 or 2 PUBLISH requires a packet identifier".to_string(),
            ));
        }
    }

    let mut body = Vec::new();
    push_utf8(&mut body, topic, "topic").map_err(config_to_malformed)?;
    if let Some(id) = packet_id {
        body.extend_from_slice(&id.to_be_bytes());
    }
    body.extend_from_slice(payload);

    let mut header = 0x30 | (qos << 1);
    if retain {
        header |= 0x01;
    }
    frame_packet(header, body)
}

pub fn encode_pingreq() -> Vec<u8> {
    vec![0xc0, 0x00]
}

fn encode_puback(packet_id: u16) -> Result<Vec<u8>, MqttError> {
    encode_packet_id_frame(0x40, packet_id)
}

fn encode_pubrec(packet_id: u16) -> Result<Vec<u8>, MqttError> {
    encode_packet_id_frame(0x50, packet_id)
}

fn encode_pubrel(packet_id: u16) -> Result<Vec<u8>, MqttError> {
    encode_packet_id_frame(0x62, packet_id)
}

fn encode_pubcomp(packet_id: u16) -> Result<Vec<u8>, MqttError> {
    encode_packet_id_frame(0x70, packet_id)
}

fn encode_packet_id_frame(header: u8, packet_id: u16) -> Result<Vec<u8>, MqttError> {
    validate_packet_id(packet_id)?;
    Ok(vec![header, 0x02, (packet_id >> 8) as u8, packet_id as u8])
}

fn validate_packet_id(packet_id: u16) -> Result<(), MqttError> {
    if packet_id == 0 {
        return Err(MqttError::Malformed(
            "packet identifier must not be zero".to_string(),
        ));
    }
    Ok(())
}

fn push_utf8(output: &mut Vec<u8>, value: &str, field: &str) -> Result<(), MqttError> {
    validate_mqtt_string(value, field, true)?;
    push_binary(output, value.as_bytes(), field)
}

fn push_binary(output: &mut Vec<u8>, value: &[u8], field: &str) -> Result<(), MqttError> {
    validate_binary_length(value, field)?;
    output.extend_from_slice(&(value.len() as u16).to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

fn frame_packet(header: u8, body: Vec<u8>) -> Result<Vec<u8>, MqttError> {
    if body.len() > MAX_PACKET_SIZE {
        return Err(MqttError::PacketTooLarge(body.len()));
    }
    let mut packet = Vec::with_capacity(body.len() + 5);
    packet.push(header);
    encode_remaining_length(body.len(), &mut packet)?;
    packet.extend_from_slice(&body);
    Ok(packet)
}

fn encode_remaining_length(mut length: usize, output: &mut Vec<u8>) -> Result<(), MqttError> {
    if length > MAX_REMAINING_LENGTH {
        return Err(MqttError::PacketTooLarge(length));
    }
    loop {
        let mut byte = (length % 128) as u8;
        length /= 128;
        if length > 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if length == 0 {
            return Ok(());
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishPacket {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub packet_id: Option<u16>,
    pub duplicate: bool,
    pub retain: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    ConnAck {
        session_present: bool,
        return_code: u8,
    },
    Publish(PublishPacket),
    PubAck(u16),
    PubRec(u16),
    PubRel(u16),
    PubComp(u16),
    SubAck {
        packet_id: u16,
        return_codes: Vec<u8>,
    },
    PingResp,
    Other(u8),
}

#[derive(Debug, Default)]
pub struct PacketDecoder {
    buffer: Vec<u8>,
}

impl PacketDecoder {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), MqttError> {
        let new_size = self.buffer.len().saturating_add(bytes.len());
        if new_size > MAX_PACKET_SIZE + 5 {
            return Err(MqttError::PacketTooLarge(new_size));
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    pub fn next_packet(&mut self) -> Result<Option<Packet>, MqttError> {
        let Some((header_length, remaining_length)) = frame_lengths(&self.buffer)? else {
            return Ok(None);
        };
        let frame_length = header_length + remaining_length;
        if self.buffer.len() < frame_length {
            return Ok(None);
        }
        let frame: Vec<u8> = self.buffer.drain(..frame_length).collect();
        decode_packet(&frame).map(Some)
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

pub fn decode_packet(frame: &[u8]) -> Result<Packet, MqttError> {
    let Some((header_length, remaining_length)) = frame_lengths(frame)? else {
        return Err(MqttError::Malformed("incomplete packet".to_string()));
    };
    if frame.len() != header_length + remaining_length {
        return Err(MqttError::Malformed(
            "packet length does not match remaining length".to_string(),
        ));
    }

    let header = frame[0];
    let packet_type = header >> 4;
    if packet_type == 0 || packet_type == 15 {
        return Err(MqttError::Malformed(format!(
            "reserved packet type {packet_type}"
        )));
    }
    let flags = header & 0x0f;
    let body = &frame[header_length..];

    match packet_type {
        2 => decode_connack(flags, body),
        3 => decode_publish(flags, body),
        4 => decode_ack(flags, body, 0, Packet::PubAck),
        5 => decode_ack(flags, body, 0, Packet::PubRec),
        6 => decode_ack(flags, body, 2, Packet::PubRel),
        7 => decode_ack(flags, body, 0, Packet::PubComp),
        9 => decode_suback(flags, body),
        13 => {
            expect_flags(flags, 0, "PINGRESP")?;
            if !body.is_empty() {
                return Err(MqttError::Malformed(
                    "PINGRESP remaining length must be zero".to_string(),
                ));
            }
            Ok(Packet::PingResp)
        }
        _ => Ok(Packet::Other(packet_type)),
    }
}

fn frame_lengths(input: &[u8]) -> Result<Option<(usize, usize)>, MqttError> {
    if input.len() < 2 {
        return Ok(None);
    }

    let mut value = 0usize;
    let mut multiplier = 1usize;
    for index in 1..=4 {
        let Some(byte) = input.get(index).copied() else {
            return Ok(None);
        };
        value = value.saturating_add(usize::from(byte & 0x7f) * multiplier);
        if byte & 0x80 == 0 {
            if index > 1 && byte == 0 {
                return Err(MqttError::Malformed(
                    "remaining length is not minimally encoded".to_string(),
                ));
            }
            if value > MAX_PACKET_SIZE {
                return Err(MqttError::PacketTooLarge(value));
            }
            return Ok(Some((index + 1, value)));
        }
        if index == 4 {
            return Err(MqttError::Malformed(
                "remaining length uses more than four bytes".to_string(),
            ));
        }
        multiplier *= 128;
    }
    unreachable!("remaining-length loop always returns")
}

fn decode_connack(flags: u8, body: &[u8]) -> Result<Packet, MqttError> {
    expect_flags(flags, 0, "CONNACK")?;
    if body.len() != 2 || body[0] > 1 || body[1] > 5 || (body[1] != 0 && body[0] != 0) {
        return Err(MqttError::Malformed("invalid CONNACK".to_string()));
    }
    Ok(Packet::ConnAck {
        session_present: body[0] == 1,
        return_code: body[1],
    })
}

fn decode_publish(flags: u8, body: &[u8]) -> Result<Packet, MqttError> {
    let duplicate = flags & 0x08 != 0;
    let qos = (flags >> 1) & 0x03;
    let retain = flags & 0x01 != 0;
    if qos == 3 || (qos == 0 && duplicate) {
        return Err(MqttError::Malformed(
            "PUBLISH contains invalid QoS/DUP flags".to_string(),
        ));
    }

    let mut cursor = 0;
    let topic = read_utf8(body, &mut cursor, "PUBLISH topic")?;
    validate_topic_name(&topic)?;
    let packet_id = if qos > 0 {
        Some(read_packet_id(body, &mut cursor)?)
    } else {
        None
    };
    Ok(Packet::Publish(PublishPacket {
        topic,
        payload: body[cursor..].to_vec(),
        qos,
        packet_id,
        duplicate,
        retain,
    }))
}

fn decode_ack<F>(
    flags: u8,
    body: &[u8],
    expected_flags: u8,
    constructor: F,
) -> Result<Packet, MqttError>
where
    F: FnOnce(u16) -> Packet,
{
    expect_flags(flags, expected_flags, "acknowledgement")?;
    if body.len() != 2 {
        return Err(MqttError::Malformed(
            "acknowledgement remaining length must be two".to_string(),
        ));
    }
    let mut cursor = 0;
    Ok(constructor(read_packet_id(body, &mut cursor)?))
}

fn decode_suback(flags: u8, body: &[u8]) -> Result<Packet, MqttError> {
    expect_flags(flags, 0, "SUBACK")?;
    if body.len() < 3 {
        return Err(MqttError::Malformed(
            "SUBACK requires a packet identifier and return code".to_string(),
        ));
    }
    let mut cursor = 0;
    let packet_id = read_packet_id(body, &mut cursor)?;
    let return_codes = body[cursor..].to_vec();
    if return_codes
        .iter()
        .any(|code| !matches!(code, 0 | 1 | 2 | 0x80))
    {
        return Err(MqttError::Malformed(
            "SUBACK contains an invalid return code".to_string(),
        ));
    }
    Ok(Packet::SubAck {
        packet_id,
        return_codes,
    })
}

fn expect_flags(actual: u8, expected: u8, packet: &str) -> Result<(), MqttError> {
    if actual != expected {
        return Err(MqttError::Malformed(format!(
            "{packet} fixed-header flags must be {expected:#x}, got {actual:#x}"
        )));
    }
    Ok(())
}

fn read_utf8(body: &[u8], cursor: &mut usize, field: &str) -> Result<String, MqttError> {
    let bytes = read_binary(body, cursor, field)?;
    let value = str::from_utf8(bytes)
        .map_err(|_| MqttError::Malformed(format!("{field} is not valid UTF-8")))?;
    validate_mqtt_string(value, field, true).map_err(config_to_malformed)?;
    Ok(value.to_string())
}

fn read_binary<'a>(body: &'a [u8], cursor: &mut usize, field: &str) -> Result<&'a [u8], MqttError> {
    let length_bytes = body
        .get(*cursor..*cursor + 2)
        .ok_or_else(|| MqttError::Malformed(format!("{field} length is truncated")))?;
    *cursor += 2;
    let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
    let value = body
        .get(*cursor..*cursor + length)
        .ok_or_else(|| MqttError::Malformed(format!("{field} is truncated")))?;
    *cursor += length;
    Ok(value)
}

fn read_packet_id(body: &[u8], cursor: &mut usize) -> Result<u16, MqttError> {
    let bytes = body
        .get(*cursor..*cursor + 2)
        .ok_or_else(|| MqttError::Malformed("packet identifier is truncated".to_string()))?;
    *cursor += 2;
    let packet_id = u16::from_be_bytes([bytes[0], bytes[1]]);
    validate_packet_id(packet_id)?;
    Ok(packet_id)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionState {
    #[default]
    Disconnected,
    AwaitingConnAck,
    AwaitingSubAck {
        packet_id: u16,
    },
    Online,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SessionOutput {
    pub outbound: Vec<Vec<u8>>,
    pub inbound: Vec<PublishPacket>,
    pub ping_response: bool,
}

#[derive(Debug, Default)]
pub struct MqttSession {
    decoder: PacketDecoder,
    state: SessionState,
    next_packet_id: u16,
    incoming_qos2: BTreeMap<u16, PublishPacket>,
    outgoing_qos1: BTreeSet<u16>,
    outgoing_qos2: BTreeSet<u16>,
}

impl MqttSession {
    pub fn begin(&mut self, config: &MqttConfig) -> Result<Vec<u8>, MqttError> {
        self.disconnect();
        self.next_packet_id = 1;
        let connect = encode_connect(config)?;
        self.state = SessionState::AwaitingConnAck;
        Ok(connect)
    }

    pub fn disconnect(&mut self) {
        self.decoder.clear();
        self.state = SessionState::Disconnected;
        self.incoming_qos2.clear();
        self.outgoing_qos1.clear();
        self.outgoing_qos2.clear();
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn is_online(&self) -> bool {
        self.state == SessionState::Online
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), MqttError> {
        self.decoder.feed(bytes)
    }

    pub fn process_next(
        &mut self,
        config: &MqttConfig,
    ) -> Result<Option<SessionOutput>, MqttError> {
        let Some(packet) = self.decoder.next_packet()? else {
            return Ok(None);
        };
        self.process_packet(packet, config).map(Some)
    }

    pub fn publish(
        &mut self,
        config: &MqttConfig,
        topic: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, MqttError> {
        if !self.is_online() {
            return Err(MqttError::NotOnline);
        }

        match config.qos {
            0 => encode_publish(topic, payload, 0, false, None),
            1 | 2 => {
                if self.outgoing_qos1.len() + self.outgoing_qos2.len() >= MAX_INFLIGHT {
                    return Err(MqttError::InflightFull);
                }
                let packet_id = self.allocate_packet_id()?;
                let packet = encode_publish(topic, payload, config.qos, false, Some(packet_id))?;
                if config.qos == 1 {
                    self.outgoing_qos1.insert(packet_id);
                } else {
                    self.outgoing_qos2.insert(packet_id);
                }
                Ok(packet)
            }
            qos => Err(MqttError::Config(format!(
                "qos must be 0, 1, or 2, got {qos}"
            ))),
        }
    }

    fn process_packet(
        &mut self,
        packet: Packet,
        config: &MqttConfig,
    ) -> Result<SessionOutput, MqttError> {
        let mut output = SessionOutput::default();
        match packet {
            Packet::ConnAck {
                session_present,
                return_code,
            } => {
                if self.state != SessionState::AwaitingConnAck {
                    return Err(MqttError::Malformed(
                        "unexpected CONNACK for current session state".to_string(),
                    ));
                }
                if return_code != 0 {
                    return Err(MqttError::ConnectionRefused(return_code));
                }
                if session_present {
                    return Err(MqttError::Malformed(
                        "broker resumed a session after clean-session CONNECT".to_string(),
                    ));
                }
                let packet_id = self.allocate_packet_id()?;
                output
                    .outbound
                    .push(encode_subscribe(packet_id, &config.topics, config.qos)?);
                self.state = SessionState::AwaitingSubAck { packet_id };
            }
            Packet::SubAck {
                packet_id,
                return_codes,
            } => {
                let SessionState::AwaitingSubAck {
                    packet_id: expected,
                } = self.state
                else {
                    return Err(MqttError::Malformed(
                        "unexpected SUBACK for current session state".to_string(),
                    ));
                };
                if packet_id != expected {
                    return Err(MqttError::Malformed(format!(
                        "SUBACK packet identifier {packet_id} does not match {expected}"
                    )));
                }
                if return_codes.len() != config.topics.len()
                    || return_codes
                        .iter()
                        .any(|code| *code == 0x80 || *code > config.qos)
                {
                    return Err(MqttError::Malformed(
                        "broker rejected or changed an MQTT subscription unexpectedly".to_string(),
                    ));
                }
                self.state = SessionState::Online;
            }
            Packet::Publish(publish) => {
                if matches!(
                    self.state,
                    SessionState::Disconnected | SessionState::AwaitingConnAck
                ) {
                    return Err(MqttError::Malformed(
                        "PUBLISH arrived before the MQTT connection was acknowledged".to_string(),
                    ));
                }
                match publish.qos {
                    0 => output.inbound.push(publish),
                    1 => {
                        let packet_id = publish.packet_id.ok_or_else(|| {
                            MqttError::Malformed(
                                "QoS 1 PUBLISH is missing a packet identifier".to_string(),
                            )
                        })?;
                        output.outbound.push(encode_puback(packet_id)?);
                        output.inbound.push(publish);
                    }
                    2 => {
                        let packet_id = publish.packet_id.ok_or_else(|| {
                            MqttError::Malformed(
                                "QoS 2 PUBLISH is missing a packet identifier".to_string(),
                            )
                        })?;
                        if !self.incoming_qos2.contains_key(&packet_id) {
                            if self.incoming_qos2.len() >= MAX_INFLIGHT {
                                return Err(MqttError::InflightFull);
                            }
                            self.incoming_qos2.insert(packet_id, publish);
                        }
                        output.outbound.push(encode_pubrec(packet_id)?);
                    }
                    _ => unreachable!("decoder rejects QoS 3"),
                }
            }
            Packet::PubAck(packet_id) => {
                self.outgoing_qos1.remove(&packet_id);
            }
            Packet::PubRec(packet_id) => {
                if self.outgoing_qos2.contains(&packet_id) {
                    output.outbound.push(encode_pubrel(packet_id)?);
                }
            }
            Packet::PubRel(packet_id) => {
                if let Some(publish) = self.incoming_qos2.remove(&packet_id) {
                    output.inbound.push(publish);
                }
                output.outbound.push(encode_pubcomp(packet_id)?);
            }
            Packet::PubComp(packet_id) => {
                self.outgoing_qos2.remove(&packet_id);
            }
            Packet::PingResp => output.ping_response = true,
            Packet::Other(_) => {}
        }
        Ok(output)
    }

    fn allocate_packet_id(&mut self) -> Result<u16, MqttError> {
        for _ in 0..u16::MAX {
            let candidate = if self.next_packet_id == 0 {
                1
            } else {
                self.next_packet_id
            };
            self.next_packet_id = candidate.wrapping_add(1);
            let used_by_subscribe = matches!(
                self.state,
                SessionState::AwaitingSubAck { packet_id } if packet_id == candidate
            );
            if !used_by_subscribe
                && !self.outgoing_qos1.contains(&candidate)
                && !self.outgoing_qos2.contains(&candidate)
            {
                return Ok(candidate);
            }
        }
        Err(MqttError::InflightFull)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconnectBackoff {
    failures: u8,
    retry_at_ms: u64,
}

impl ReconnectBackoff {
    pub fn ready(&self, now_ms: u64) -> bool {
        now_ms >= self.retry_at_ms
    }

    pub fn record_failure(&mut self, now_ms: u64) -> u64 {
        let shift = self.failures.min(7);
        let delay = INITIAL_RECONNECT_DELAY_MS
            .saturating_mul(1u64 << shift)
            .min(MAX_RECONNECT_DELAY_MS);
        self.failures = self.failures.saturating_add(1);
        self.retry_at_ms = now_ms.saturating_add(delay);
        delay
    }

    pub fn reset(&mut self) {
        self.failures = 0;
        self.retry_at_ms = 0;
    }

    pub fn retry_at_ms(&self) -> u64 {
        self.retry_at_ms
    }
}
