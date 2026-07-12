# amqp - ZeroClaw channel plugin

AMQP 0-9-1 channel for RabbitMQ-compatible brokers. The guest implements the
application protocol; the ZeroClaw host owns TCP, DNS, and optional TLS through
the `zeroclaw:plugin/socket` WIT import.

The manifest declares `provides = "amqp"`. Each instance receives its canonical
`[channels.amqp.<alias>]` object from the host; there is no plugin-specific
configuration copy.

## Configuration

```toml
[channels.amqp.events]
enabled = true
amqp_url = "amqp://agent:replace-me@broker.internal:5672/%2F"
exchange = "amq.topic"
routing_keys = ["build.completed", "release.#"]
queue = "zeroclaw-events" # omit for a broker-named exclusive auto-delete queue
sender_label = "release-bus"
content_template = "Release {project.name} {version}"
thread_id_field = "project.name"
durable_ack = true
dispatch = "agent_loop"
```

`amqp_url` supplies the host/port, virtual host, and PLAIN credentials. Missing
credentials default to `guest` / `guest`; missing ports default to 5672 or 5671.
Virtual hosts and credentials use URI percent encoding.

The configured exchange must already exist. The plugin declares the configured
queue (or an anonymous exclusive auto-delete queue), binds every routing key,
sets a bounded prefetch for durable acknowledgements, and starts one consumer.
A named queue uses the native channel's non-exclusive, non-auto-delete,
non-durable declaration flags.

`sender_label` is emitted verbatim as the inbound sender and is authorized by
the host with case-sensitive exact matching. It is an external source identity,
not the plugin's self handle. JSON bodies can be mapped with `content_template`;
dotted placeholders and `thread_id_field` are supported. Non-JSON bodies are
delivered as lossy UTF-8 text.

With `durable_ack = true`, a delivery is acknowledged on the poll call after it
was returned across the channel WIT boundary. The host does not poll again until
the previous message has completed its authorization and queue handoff, so a
crash before handoff leaves the broker delivery unacked for redelivery. With
`false`, `basic.consume` uses `no-ack` (at-most-once).

## Publishing

`send` publishes UTF-8 message content to the configured exchange, using the
WIT `recipient` as the AMQP routing key. Messages are marked persistent and use
`text/plain; charset=utf-8`; `subject` maps to the AMQP `type` property and
`in_reply_to` (or `thread_ts`) maps to `correlation-id`. Large bodies are split
at the negotiated `frame-max`.

The call succeeds when bytes are accepted by the host socket queue. Publisher
confirms are not enabled, so it does not prove broker acceptance.

## Connection behavior

- AMQP protocol header and 0-9-1 `connection.start` / `tune` / `open` handshake
- SASL PLAIN authentication and `en_US` locale
- channel 1 open, queue declare, queue bind, QoS, and basic consume
- arbitrary TCP chunk reassembly and multi-frame delivery-body assembly
- individual durable acknowledgements
- negotiated heartbeats (30-second client ceiling) and receive timeout
- bounded nonblocking receive drains and exponential reconnect (1 to 60 seconds)
- 16 MiB hard limits for individual frames and message bodies

## Explicit limits

- `registry = false` remains required because `socket_client` is not available
  on upstream ZeroClaw master.
- Only AMQP 0-9-1 and SASL PLAIN are implemented. AMQP 1.0, challenge-response
  SASL mechanisms, and URI query parameters are unsupported.
- `dispatch = "sop"` and `"sop_and_agent_loop"` are rejected because the
  channel WIT cannot call the host SOP engine.
- The socket ABI validates TLS with host WebPKI roots. The native-required
  `ca_cert` field is accepted for configuration compatibility but its file is
  not readable by the guest; private/custom CA roots are unsupported.
- `client_cert` / `client_key` are rejected because the socket ABI cannot pass a
  client identity, so Fedora Messaging-style mutual TLS is not supported.
- The plugin does not create or redeclare exchanges and cannot configure queue
  arguments, quorum queues, dead-lettering, priorities, or consumer arguments.
- Publisher confirms, transactions, mandatory returns, alternate exchanges,
  and broker failover URL lists are unsupported.
- WIT binary attachments are rejected. AMQP header tables and inbound basic
  properties are not surfaced as channel-message fields.

## Build and test

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --target wasm32-wasip2 --release
cargo clippy --target wasm32-wasip2 -- -D warnings
```

Host tests use encoded broker frames and do not require a live broker. Live
RabbitMQ interoperability and custom network policy remain deployment tests.
