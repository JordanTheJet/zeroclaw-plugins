# MQTT channel plugin

This plugin mirrors `[channels.mqtt.<alias>]` through `provides = "mqtt"`.
It implements MQTT 3.1.1 over ZeroClaw's host-mediated raw socket transport:

1. resolve `mqtt://` or `mqtts://` from the injected channel configuration;
2. open host-owned TCP/TLS and send a clean-session `CONNECT`;
3. subscribe to every configured topic after `CONNACK`;
4. reassemble arbitrary TCP chunks into bounded MQTT packets;
5. map each inbound publish topic to both `sender` and `reply_target`;
6. publish outbound text to the topic in `SendMessage.recipient`;
7. drive QoS acknowledgements, keepalive pings, and capped reconnect backoff.

MQTT topic names are case-sensitive. The manifest therefore uses
`sender_match = "exact"`, matching the native MQTT topic matcher and ensuring
that peer-group entries authorize the exact topic delivered by the broker.

The implementation is real but remains `registry = false` until ZeroClaw's
`socket_client` host capability lands on upstream master. It can run only
against a host branch that imports `zeroclaw:plugin/socket` and grants the
manifest's `socket_client` permission.

## Configuration

The host-injected alias section is the only configuration source:

```toml
[channels.mqtt.default]
enabled = true
broker_url = "mqtts://broker.example.com:8883"
client_id = "zeroclaw-agent"
topics = ["sensors/#", "alerts/+/critical"]
qos = 1
username = "<broker user>"
password = "<encrypted broker password>"
use_tls = true
keep_alive_secs = 30
```

`use_tls` must agree with the URL scheme. Ports default to 1883 for `mqtt://`
and 8883 for `mqtts://`. QoS 0, 1, and 2 are supported for subscriptions,
inbound acknowledgements, and outbound publishes. TLS certificate validation
and SNI belong to the host socket implementation.

## Limits

- MQTT 5 properties, WebSockets, Last Will, retained outbound publishes, and
  persistent sessions are not implemented.
- Clean-session reconnects do not durably retransmit outbound QoS 1/2 messages
  that were in flight when the socket failed.
- WIT channel content is text, so non-UTF-8 MQTT payloads are converted with
  UTF-8 replacement characters.
- Packets larger than 1 MiB are rejected to bound guest memory use.
- Unit tests use deterministic binary fixtures. No live broker or live TLS
  connection is exercised in this repository because the required stock-host
  capability is not yet available on upstream master.

## Validation

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --target wasm32-wasip2 --release
cargo clippy --target wasm32-wasip2 -- -D warnings
```
