# gmail_push — ZeroClaw channel plugin

A WASM (`wasm32-wasip2`) channel plugin mirroring the built-in **Gmail Pub/Sub
push** channel. `provides = "gmail_push"`, so it reads the existing
`[channels.gmail_push.<alias>]` config as the single source of truth and honors
native-wins.

## How it works

Gmail delivers change notifications via Google Cloud **Pub/Sub push** (an
HTTP `POST`), so this is a **webhook** channel, not a poller. The host serves
`POST /plugin/gmail_push`. On each notification the plugin:

1. **Authenticates** the request. When `webhook_secret` is set, the push must
   carry `Authorization: Bearer <webhook_secret>` (matching the native gateway);
   otherwise the request is rejected with `401`. When the secret is empty, no
   auth is required.
2. **Decodes** the Pub/Sub envelope. The base64 `message.data` decodes to
   `{emailAddress, historyId}` — the push carries **only a `historyId`, never the
   message body**.
3. **Fetches** the actual messages: it calls the Gmail **History** API since the
   last-seen `historyId`, then **`messages.get`** for each new message, using the
   configured `oauth_token`. The From/Subject/plain-text-body are returned as
   inbound messages (`content` = `"Subject: …\n\n<body>"`).

The **first** notification only records the `historyId` and returns nothing (it
establishes the cursor), matching the native channel. The `historyId` cursor is
kept in `thread_local` state across notifications on the plugin instance.

Replies are sent via the Gmail `messages.send` API (RFC 2822 message, URL-safe
base64), with CRLF-sanitized headers.

## Config (`[channels.gmail_push.<alias>]`)

- `oauth_token` — Gmail API OAuth bearer (required to fetch + send).
- `topic` — Pub/Sub topic for `users.watch` registration.
- `label_filter` — Gmail labels to watch (default `["INBOX"]`).
- `webhook_secret` — optional shared secret for the inbound `Authorization`
  header.
- `webhook_url` — informational (the push subscription's endpoint).

## Watch registration & renewal (important limitation)

On `configure` the plugin makes a **best-effort** `users.watch` registration
(when `oauth_token` + `topic` are set) so Pub/Sub starts delivering. Unlike the
native channel — which renews the 7-day watch on a background loop — a webhook
plugin has **no long-running task**, so **watch renewal is not performed**. Keep
the subscription alive by either restarting the host within the 7-day window or
managing `users.watch` out of band. This is the one behavioral gap versus the
native channel.

## Security / scope

- No sender allowlist is applied in the plugin; the host gates senders via
  `peer_groups`.
- **Text only.** Attachments are ignored; the plain-text body (or HTML-stripped
  `text/html`, or the snippet) is used. Deferred: attachments, threading beyond
  `thread_ts`, richer MIME.
- The History + `messages.get` orchestration runs inside `parse_webhook` over
  `wasi:http`; on a transient fetch failure the plugin returns `Err`, so Pub/Sub
  redelivers (the `historyId` cursor is only advanced on success).

## Build

```bash
cargo test --lib
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release
```
