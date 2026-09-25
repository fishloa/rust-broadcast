# webrtc-runtime 0.2.0 — 2026-09-25

Security release for the `media` feature (the ICE + DTLS-SRTP transport). **Upgrade if you use
`media::MediaTransport`.** This is a breaking release, because the fix needs one new required
config field.

## Security

**GHSA-48qq-7p78-2jvj** — before this release, the DTLS peer was never authenticated.
`MediaTransport` built its DTLS configuration with certificate verification off and no
replacement check. It accepted DTLS from any source address, and every completed handshake
replaced the session's SRTP keys. In the passive (WHIP ingest) role, an off-path host could
complete its own handshake against the media port and substitute the session's keys.

## Breaking change

`media::MediaTransportConfig` has a new required field:

```rust
pub remote_fingerprint: String, // the remote SDP's a=fingerprint, e.g. "sha-256 AB:CD:…"
```

Take it from the remote peer's SDP with the new helper:

```rust
let remote_fingerprint = webrtc_runtime::media::parse_remote_fingerprint(&remote_sdp)
    .ok_or("offer has no a=fingerprint")?;
```

`MediaTransport::new` rejects a value that isn't `sha-256` followed by 32 colon-separated hex
bytes. SHA-256 is the hash WebRTC endpoints are required to support (RFC 8827 §6.5).

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

## Testing

- **Integration tests** (`tests/dtls_fingerprint.rs`): a wrong fingerprint never completes the
  handshake and nothing decrypts; malformed fingerprints are rejected; SDP fingerprint parsing
  prefers the media-level attribute over the session level.
- **Unit tests:** a matching fingerprint completes and media flows both ways; a ClientHello
  from a third address after pair selection yields no events and leaves the keys intact.
- **Mutation checks:** forcing the fingerprint check to pass, or removing the address pin,
  makes the corresponding test fail.

## multimux

multimux's WHIP input and WHEP output were updated to pass the offer's fingerprint and reject
offers without one. Those changes reach multimux users in multimux's next release.

MSRV 1.95.0.
