# webrtc-runtime 0.2.0 — 2026-09-25

Security release for the `media` feature (the ICE + DTLS-SRTP transport). **Upgrade if you use
`media::MediaTransport`.** This is a breaking release, because the fixes need two new required
config fields.

## Security

| Advisory | Before this release |
|---|---|
| GHSA-48qq-7p78-2jvj | The DTLS peer was never authenticated. `MediaTransport` built its DTLS configuration with certificate verification off and no replacement check. It accepted DTLS from any source address, and every completed handshake replaced the session's SRTP keys. In the passive (WHIP ingest) role, an off-path host could complete its own handshake against the media port and substitute the session's keys. |
| GHSA-89f2-5m24-r6m7 | `add_remote_candidate`'s cap on remote ICE candidates (RFC 8445 §6.1.2.5) could be bypassed entirely: an authenticated STUN Binding Request from a source address the transport didn't already recognize made the ICE agent create its own peer-reflexive remote candidate, uncounted by the cap. Since the remote peer already knows the negotiated ICE ufrag/password, it could grow the remote-candidate (and pair) count without bound by sending from many source ports (RFC 8445 §19.5.1). |

## Breaking change

`media::MediaTransportConfig` has two new required fields:

```rust
pub remote_fingerprint: String,     // the remote SDP's a=fingerprint, e.g. "sha-256 AB:CD:…"
pub max_remote_candidates: usize,   // cap on admitted remote ICE candidates (RFC 8445 §6.1.2.5)
```

Take the fingerprint from the remote peer's SDP with the new helper:

```rust
let remote_fingerprint = webrtc_runtime::media::parse_remote_fingerprint(&remote_sdp)
    .ok_or("offer has no a=fingerprint")?;
```

`MediaTransport::new` rejects a fingerprint that isn't `sha-256` followed by 32
colon-separated hex bytes. SHA-256 is the hash WebRTC endpoints are required to support
(RFC 8827 §6.5). For `max_remote_candidates`, pass the new `MAX_REMOTE_CANDIDATES` constant
(100, the spec's own recommended default) unless a stricter cap is wanted.

## Behaviour changes

- **Certificate check.** The peer's leaf certificate must hash to `remote_fingerprint`
  (RFC 8122). This is checked twice: in the DTLS verify callback, and again before any SRTP key
  is derived. The digests are compared without an early exit.
- **Client certificate required.** The passive (DTLS server) role now requires a client
  certificate.
- **Address pinning.** DTLS datagrams are accepted only from the remote address of the
  ICE-selected pair, and dropped before a pair is selected.
- **No key replacement.** Once a session's SRTP keys are installed, a handshake completed by a
  different address returns an error and leaves the keys untouched.
- **Remote candidate cap enforced end-to-end.** `add_remote_candidate` rejects candidates past
  `max_remote_candidates` with `Error::Media` (never silently dropped), and a new STUN Binding
  Request from a source address the transport doesn't already recognize is now checked against
  the same cap before the ICE agent is allowed to create a peer-reflexive remote candidate for
  it; an address already admitted keeps working.
- **`media::MediaEvent::RtcpUnsupported(rtcp_packet::Error)`.** An inbound SRTCP packet that
  decrypts and authenticates but isn't a decodable RFC 3550 §6 compound `rtcp-packet` (e.g. RFC
  4585 PLI/NACK/REMB feedback, which is most of what a browser receiver sends) now surfaces as
  this event instead of `handle_datagram` returning `Err` for a genuine, authenticated peer
  packet. It still counts toward the RFC 3711 key-lifetime read counter.

## Testing

- **Integration tests** (`tests/dtls_fingerprint.rs`): a wrong fingerprint never completes the
  handshake and nothing decrypts; malformed fingerprints are rejected; SDP fingerprint parsing
  prefers the media-level attribute over the session level.
- **Unit tests:** a matching fingerprint completes and media flows both ways; a ClientHello
  from a third address after pair selection yields no events and leaves the keys intact; a STUN
  Binding Request from an unrecognized source address is rejected once the cap is reached.
- **Mutation checks:** forcing the fingerprint check to pass, removing the address pin, or
  skipping the STUN-source cap check, each makes the corresponding test fail.

## multimux

multimux's WHIP input and WHEP output were updated to pass the offer's fingerprint, supply
`max_remote_candidates`, and reject offers without a fingerprint. Those changes reach multimux
users in multimux's next release.

MSRV 1.95.0.
