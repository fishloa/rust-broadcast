# WHIP — WebRTC-HTTP Ingestion Protocol (RFC 9725)

Unidirectional media ingestion: encoder (WHIP client) → media server via HTTP signalling + WebRTC media transport.

## Protocol flow

```
Client                          Endpoint                    Server
  |                                |                          |
  |--POST (SDP offer)------------>|                          |
  |   Content-Type: application/sdp                          |
  |   Authorization: Bearer <tok>  |                          |
  |                                |                          |
  |<--201 Created-----------------+                          |
  |   Location: <session-url>      |                          |
  |   ETag: "<ice-session>"        |                          |
  |   Content-Type: application/sdp                          |
  |   Link: <stun:…>; rel="ice-server"                      |
  |   body: SDP answer             |                          |
  |                                |                          |
  |--PATCH (trickle ICE)--------->|                          |
  |   If-Match: "<ice-session>"    |                          |
  |   Content-Type: application/trickle-ice-sdpfrag          |
  |   body: SDP fragment           |                          |
  |                                |                          |
  |<--204 No Content------------- |                          |
  |                                |                          |
  |         [ICE/DTLS/SRTP established]                      |
  |=========== RTP (sendonly) ================================>
  |                                |                          |
  |--DELETE <session-url>-------->|                          |
  |<--200 OK--------------------- |                          |
```

## §3 — SDP offer/answer (HTTP POST)

### Request

| Field | Value |
|-------|-------|
| Method | `POST` |
| URL | WHIP endpoint (pre-configured) |
| Content-Type | `application/sdp` |
| Authorization | `Bearer <token>` (optional) |
| Body | SDP offer per JSEP (RFC 9429 §5.2.1) |

### SDP offer constraints

- Direction: `sendonly` (SHOULD; MAY `sendrecv`; MUST NOT `inactive`/`recvonly`)
- Bundle: `a=group:BUNDLE` with max-bundle policy (RFC 9143)
- Mux: `rtcp-mux-only` (RFC 8858)
- Single `MediaStream`, matching `msid` across all m= sections
- Max one `MediaStreamTrack` per media type
- Setup: `setup:actpass` or `setup:active`
- Trickle: `ice-options:trickle ice2` when supported

### Response — 201 Created

| Header | Value |
|--------|-------|
| Content-Type | `application/sdp` |
| Location | WHIP session URL (absolute HTTPS URI) |
| ETag | Strong entity-tag, quoted (identifies ICE session) |
| Link | Optional STUN/TURN servers (see §4.4) |

Body: SDP answer per JSEP (RFC 9429 §5.3.1).

- Direction: `recvonly`
- MAY include full ICE candidate list
- `a=ice-lite` if ICE-lite

### Error responses

| Status | Meaning |
|--------|---------|
| 400 | Malformed SDP |
| 422 | Incompatible constraints (multiple streams, multiple tracks per type, wrong setup) |
| 307 | Redirect (preferred over 301/302 — preserves POST) |
| 503 | Overloaded; `Retry-After` header |

Error bodies MAY use `application/problem+json` (RFC 9457).

## §4 — Trickle ICE (HTTP PATCH)

### Request — candidate addition

| Field | Value |
|-------|-------|
| Method | `PATCH` |
| URL | WHIP session URL (from `Location`) |
| Content-Type | `application/trickle-ice-sdpfrag` |
| If-Match | `"<ETag>"` — REQUIRED |
| Body | SDP fragment per RFC 8840 §4.4 |

Client MUST buffer candidates until 201 received (needs session URL + ETag).
Client SHOULD send single aggregated PATCH.

### Response — 204 No Content

Empty body. No ETag update.

### ICE restart

| Field | Value |
|-------|-------|
| If-Match | `"*"` (unconditional) |
| Body | New `ice-ufrag`, `ice-pwd`, full candidate set |

Response: `200 OK`, new `ETag`, `Content-Type: application/trickle-ice-sdpfrag`, body with server's new credentials + candidates.

Client MUST discard previous credentials on restart initiation.
Client MUST ignore responses to older requests after sending a restart.

### PATCH error responses

| Status | Meaning |
|--------|---------|
| 422 | Unsupported operation (trickle when only restart supported, or vice versa) |
| 428 | Missing `If-Match` |
| 412 | ETag mismatch |

## §4.4 — STUN/TURN configuration (Link header)

```
Link: <stun:stun.example.com>; rel="ice-server"
Link: <turn:turn.example.com?transport=udp>; rel="ice-server";
      username="user"; credential="pass"
Link: <turns:turn.example.com?transport=tcp>; rel="ice-server";
      username="user"; credential="pass"
```

## §5 — Session termination (HTTP DELETE)

`DELETE` to WHIP session URL → `200 OK`.
Frees server resources, terminates ICE/DTLS.

## §6 — Extensions

Advertised via `Link` header in 201 response:
```
Link: <url>; rel="urn:ietf:params:whip:ext:<name>"
```

Client MUST ignore unknown `rel` values.

## §7 — Authentication

- Bearer token in `Authorization` header (RFC 6750) — mandatory support
- Token distribution out of scope (JWTs, shared secrets, etc.)
- MAY embed credentials in endpoint URL instead
- Additional HTTP auth schemes per RFC 9110 §11.6 allowed

## §8 — Security

- HTTPS mandatory
- Session URLs: cryptographically random (RFC 4086)
- Rate-limit POST/PATCH/DELETE
- Consent freshness: RFC 7675 (30s timeout for full ICE)

## Wire-format content types

| Content-Type | Used in |
|-------------|---------|
| `application/sdp` | POST request/response bodies |
| `application/trickle-ice-sdpfrag` | PATCH request/response bodies |
| `application/problem+json` | Error response bodies (optional) |

## SDP renegotiation

NOT supported. Only ICE information (candidates, ufrag, pwd) updatable via PATCH.
m= sections frozen after initial exchange.
