# Test Fixtures

## Provenance

All fixtures in this directory are derived from RFC 2326 (Real Time Streaming
Protocol, RTSP 1.0) example exchanges and section-specified formats. They are
not captures from real network traffic; they are spec-authored illustrative
messages verbatim or closely based on the RFC examples, with CSeq numbers and
session identifiers made consistent across a complete exchange.

## Files

### `session_tcp_interleaved.md`

A documented, ordered full exchange for a TCP-interleaved session:

```
OPTIONS → DESCRIBE (with SDP body) → SETUP (Transport: RTP/AVP/TCP;interleaved=0-1)
       → PLAY → TEARDOWN
```

Based on RFC 2326 §10.12 (interleaved binary data) and §14.2 (container file
streaming). CSeq increments 1–5; Session: 12345678 is set in the SETUP response
and echoed throughout.

### `digest_auth.md`

A documented `DESCRIBE → 401 Unauthorized (WWW-Authenticate: Digest …) →
DESCRIBE (Authorization: Digest …) → 200 OK` exchange.

Based on RFC 2326 §12.44, §12.5, §14 (Security), and Appendix D.1.2.
Uses realistic Digest parameters (realm, nonce, qop="auth", algorithm=MD5)
as returned by typical IP cameras.

### `sdp_sample.sdp`

A two-stream (H.264 video + PCMU audio) SDP body in the format returned
by a DESCRIBE response. Parseable by `sdp-types`.

Based on RFC 2326 §14.2 and Appendix C (Use of SDP for RTSP Session
Descriptions). Contains `a=control:` attributes for aggregate and per-stream
control URLs, `a=rtpmap:` for dynamic payload types, and `a=fmtp:` for H.264
parameters.

## Section References

- §10.12 — Embedded (Interleaved) Binary Data
- §12.5  — Authorization header
- §12.39 — Transport header
- §12.44 — WWW-Authenticate header
- §14    — Security Considerations (authentication)
- §14.2  — Streaming of a Container File (example exchange)
- Appendix A — State machines
- Appendix C — Use of SDP for RTSP Session Descriptions
- Appendix D.1.2 — Authentication-enabled client requirements
