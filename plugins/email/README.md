# email

Text-only IMAP/SMTP channel plugin for ZeroClaw. The component drives both
protocols over the host-mediated zeroclaw:plugin/socket WIT import; the host
owns TCP, DNS, and TLS while the plugin owns framing and application state.

The manifest mirrors the built-in email channel with provides = "email" and
reads the host-resolved [channels.email.<alias>] section as its only config
source. sender_match = "email" leaves peer authorization with the host and
uses email-aware canonicalization for address and display-name forms.

This implementation remains registry = false because the socket_client host
capability is not on ZeroClaw upstream master. It can be built and reviewed,
but a stock upstream host cannot instantiate it yet.

## Configuration

The schema matches the native EmailConfig field names and defaults:

~~~toml
[channels.email.work]
enabled = true
imap_host = "imap.example.com"
imap_port = 993
imap_folder = "INBOX"
smtp_host = "smtp.example.com"
smtp_port = 465
smtp_tls = true
username = "bot@example.com"
password = "imap-password"
smtp_username = "bot@example.com"       # optional; falls back to username
smtp_password = "smtp-password"         # optional; falls back to password
from_address = "bot@example.com"
poll_interval_secs = 60
default_subject = "Re: Message"
observer_mode = false
~~~

The host decrypts secrets before calling configure. The plugin keeps that
parsed config as the canonical channel configuration and stores only live
protocol/session state beside it.

## Supported behavior

- IMAP uses host-managed implicit TLS, LOGIN password authentication, SELECT,
  UID SEARCH, and one-at-a-time UID FETCH commands.
- Active mode drains UNSEEN mail at startup with RFC822, then polls from
  UIDNEXT using BODY.PEEK[].
- Observer mode records UIDNEXT at SELECT time, ignores older mail, and only
  uses BODY.PEEK[], so it does not change message flags.
- Arbitrary TCP chunks are reassembled into IMAP lines and bounded literals.
  Message literals are capped at 4 MiB.
- Plain-text MIME bodies are emitted with sender, subject, Message-ID, and
  parsed Date. HTML-only bodies are reduced to text. MIME attachments are not
  surfaced.
- SMTP uses AUTH PLAIN or AUTH LOGIN and advances one command per complete
  server reply. It does not assume PIPELINING.
- Outbound messages are RFC 5322 text/plain MIME with encoded UTF-8 subjects,
  base64 bodies, dot stuffing, Date, Message-ID, and validated reply headers.
- SMTP connections and queued messages are bounded. A disconnect after DATA
  is not retried because delivery may already have succeeded.

The WIT send call validates and queues an SMTP transaction. SMTP replies arrive
through later nonblocking poll_message calls, so a later rejection is emitted
to the host log and cannot be returned to the original send caller.

## Explicit limits

- OAuth2/XOAUTH2 is rejected during configure.
- STARTTLS is not implemented. smtp_tls = true means host-managed implicit TLS;
  smtp_tls = false means plaintext. Servers that require STARTTLS, commonly on
  port 587, are unsupported.
- IMAP IDLE is not implemented. poll_interval_secs controls UID polling;
  idle_timeout_secs is accepted for native config compatibility but unused.
- Outbound attachments are rejected and inbound attachments are ignored.
  max_attachment_bytes therefore has no effect in this text-only slice.
- html_body is accepted for native config compatibility, but outbound mail is
  always text/plain.
- Address envelopes are conservative ASCII addr-spec values. Display names,
  address groups, internationalized envelopes, SMTPUTF8, DSN, and multiple
  recipients are unsupported.
- Delivery confirmation beyond the SMTP final 250 response is unsupported.

No live IMAP or SMTP credentials are stored in this repository, and the
automated suite uses protocol transcripts rather than a live provider.

## Validation

Run from the zeroclaw-plugins repository root:

~~~bash
cargo fmt --manifest-path plugins/email/Cargo.toml -- --check
cargo test --manifest-path plugins/email/Cargo.toml
cargo clippy --manifest-path plugins/email/Cargo.toml --all-targets -- -D warnings
cargo build --manifest-path plugins/email/Cargo.toml --target wasm32-wasip2 --release
cargo clippy --manifest-path plugins/email/Cargo.toml --target wasm32-wasip2 -- -D warnings
~~~
