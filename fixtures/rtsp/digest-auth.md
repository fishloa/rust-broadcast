# Digest Authentication Fixture

**Provenance:** RFC 2326 §14 (Security Considerations), §12.5/§12.44, and
Appendix D.1.2. Digest parameters (realm, nonce, qop, nc, cnonce, response)
are representative of what a typical IP camera (e.g. ONVIF-compliant) returns.
All messages use CRLF line endings (\r\n).

This fixture documents the 401 challenge / authenticated-retry flow for a
DESCRIBE request.

---

## Message 1 — Initial DESCRIBE (no credentials) (C→S)

```
DESCRIBE rtsp://camera.example.com/live RTSP/1.0\r\n
CSeq: 1\r\n
User-Agent: ExampleClient/1.0\r\n
Accept: application/sdp\r\n
\r\n
```

## Message 2 — 401 Unauthorized challenge (S→C)

```
RTSP/1.0 401 Unauthorized\r\n
CSeq: 1\r\n
WWW-Authenticate: Digest realm="IP Camera",nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093",qop="auth",algorithm=MD5\r\n
\r\n
```

Challenge fields:
- `realm="IP Camera"` — protection space identifier.
- `nonce` — server-generated opaque string; changes on each 401.
- `qop="auth"` — quality of protection: authentication only (not auth-int).
- `algorithm=MD5` — hash algorithm.

---

## Message 3 — Authenticated DESCRIBE retry (C→S)

The client computes Digest per RFC 2617 using:
- `method = "DESCRIBE"`
- `uri = "rtsp://camera.example.com/live"` (the RTSP request URI)
- `username = "admin"`, `password = "12345"`
- `realm`, `nonce` from the challenge
- `cnonce = "0a4f113b"` (client-generated random)
- `nc = 00000001` (nonce use count)
- `qop = auth`

```
DESCRIBE rtsp://camera.example.com/live RTSP/1.0\r\n
CSeq: 2\r\n
User-Agent: ExampleClient/1.0\r\n
Accept: application/sdp\r\n
Authorization: Digest username="admin",realm="IP Camera",nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093",uri="rtsp://camera.example.com/live",qop=auth,nc=00000001,cnonce="0a4f113b",response="6629fae49393a05397450978507c4ef1",algorithm=MD5\r\n
\r\n
```

## Message 4 — 200 OK with SDP (S→C)

```
RTSP/1.0 200 OK\r\n
CSeq: 2\r\n
Content-Type: application/sdp\r\n
Content-Length: 143\r\n
\r\n
v=0\r\n
o=- 1234567890 1234567890 IN IP4 192.0.2.10\r\n
s=IP Camera Live Stream\r\n
t=0 0\r\n
a=control:rtsp://camera.example.com/live\r\n
m=video 0 RTP/AVP 96\r\n
a=rtpmap:96 H264/90000\r\n
a=control:trackID=0\r\n
```

---

## Notes

- CSeq 1 is used for the unauthenticated attempt; CSeq 2 for the retry
  (the retry is a new request).
- The `response` value `"6629fae49393a05397450978507c4ef1"` is illustrative;
  a real implementation computes it per RFC 2617 §3.2.2.
- The `uri` in the Authorization header MUST be the RTSP request URI, not an
  HTTP URL (RFC 2326 §14).
- On stale nonce (`stale=true` in a subsequent 401), the client regenerates
  cnonce and recomputes response using the new nonce, without prompting for
  credentials.
- All subsequent SETUP, PLAY, TEARDOWN requests must also carry `Authorization`
  with updated nc/cnonce/response.
