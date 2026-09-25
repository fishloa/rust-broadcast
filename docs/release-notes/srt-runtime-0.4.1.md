# srt-runtime 0.4.1 — 2026-09-25

Security patch for the tokio adapter (`io::SrtSocket`, `io::SrtListener`). **Upgrade if you
use the adapter.** The public API is unchanged, so `cargo update -p srt-runtime` picks up the
fix for anything already on `^0.4` (including multimux 0.10).

## Security

| Advisory | Before this release |
|---|---|
| GHSA-28hg-fc5v-m865 | A `HandshakeConfig` with `crypto` set negotiated keys, but the adapter's data path never used them: payloads went out in plaintext, and a peer's encrypted payloads reached the application undecrypted. |
| GHSA-gjm5-23jf-293p | Too-late packet drop was off and DROPREQ was ignored. One packet the sender would never retransmit stalled delivery permanently, while the staging buffer grew at the stream bitrate. |
| GHSA-r6hf-93jv-c3wc | `SrtListener` kept a pending-handshake entry for every source address with no limit, and those entries expired only when the socket went quiet. |

## Behaviour changes

- **Encryption is refused, not faked.** `SrtSocket::connect`, `connect_from` and
  `SrtListener::bind` return an error for a crypto-enabled config before any I/O. An incoming
  handshake that requests encryption is rejected (`REJ_UNSECURE`). A data packet with its
  encryption-key flags set is never delivered. The sans-IO handshake engines still negotiate
  keys; the adapter will support encryption once its data path applies AES-CTR, and that work
  will be verified against libsrt.
- **Live mode: too-late packet drop is on by default.** Once the next buffered packet's play
  time arrives, a gap that can't be recovered in time is skipped (draft-sharabayko-srt-01 §4.6,
  receiver-buffer read rule) and that packet is delivered. Before, the whole stream stalled.
  DROPREQ ranges are skipped immediately.
- **Staging is capped at the negotiated flow window.** A data packet more than
  `max_flow_window_size` ahead of the delivery cursor is dropped instead of being buffered.
- **The listener caps in-progress handshakes at 1024** and expires them on a 100 ms timer that
  runs even while traffic keeps arriving.
- **`tsbpd::TsbpdScheduler`**: with too-late drop enabled, the public engine now applies the
  same gap-skip rule, and `TickOutcome::dropped` reports skipped sequence numbers from both
  `feed_data` and `tick`. With too-late drop disabled it still waits for the gap.

## Testing

Each fix has a regression test that fails against 0.4.0. That was confirmed by running each
test on the old code, and for the encryption refusal by re-disabling it.

MSRV 1.95.0.
