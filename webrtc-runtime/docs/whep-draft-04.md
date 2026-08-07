# WHEP — WebRTC-HTTP Egress Protocol (draft-ietf-wish-whep-04)

Unidirectional media egress: media server → WHEP player via HTTP signalling + WebRTC media transport. Mirror of WHIP but reversed direction.

## Protocol flow

```
Player                          Endpoint                    Server
  |                                |                          |
  |--POST (SDP offer)------------>|                          |
  |   Content-Type: application/sdp                          |
  |   Authorization: Bearer <tok>  |                          |
  |                                |                          |
  |<--201 Created-----------------+  (accepted)              |
  |   Location: <session-url>      |                          |
  |   ETag: "<ice-session>"        |                          |
  |   body: SDP answer             |                          |
  |                                |                          |
  | — OR —                         |                          |
  |                                |                          |
  |<--406 Not Acceptable---------+  (counter-offer)          |
  |   Location: <session-url>      |                          |
  |   Content-Type: application/sdp; valid-until="<date>"    |
  |   body: SDP offer from server  |                          |
  |                                |                          |
  |--PATCH (SDP answer)---------->|                          |
  |   Content-Type: application/sdp                          |
  |   body: SDP answer             |                          |
  |                                |                          |
  |<--204 No Content------------- |                          |
  |                                |                          |
  |         [ICE/DTLS/SRTP established]                      |
  |<========== RTP (recvonly) =================================
  |                                |                          |
  |--DELETE <session-url>-------->|                          |
  |<--200 OK--------------------- |                          |
```

## Key differences from WHIP

| Aspect | WHIP (ingest) | WHEP (egress) |
|--------|---------------|---------------|
| Media direction | client → server | server → client |
| Client SDP direction | `sendonly` | `recvonly` |
| Server SDP direction | `recvonly` | `sendonly` |
| Counter-offer | not supported | 406 Not Acceptable |
| Server trickle ICE | via PATCH response | NOT possible after SDP answer sent |
| Partial m-line reject | not supported (full reject) | supported (port=0) |
| HEAD discovery | not specified | SHOULD respond 200 + `Content-Type: application/sdp` |
| No-publisher state | N/A | 409 Conflict + Retry-After |

## §3 — SDP offer/answer (HTTP POST)

### Request

| Field | Value |
|-------|-------|
| Method | `POST` |
| URL | WHEP endpoint (pre-configured) |
| Content-Type | `application/sdp` |
| Authorization | `Bearer <token>` (optional) |
| Body | SDP offer from player |

### SDP offer constraints

- Direction: `recvonly` (SHOULD; MAY `sendrecv`; MUST NOT `inactive`/`sendonly`)
- Bundle: `a=group:BUNDLE` with max-bundle policy
- Mux: `rtcp-mux-only`
- Single `MediaStream`

### Response path A — 201 Created (accepted)

| Header | Value |
|--------|-------|
| Content-Type | `application/sdp` |
| Location | WHEP session URL |
| ETag | ICE session entity-tag |

Body: SDP answer. Direction: `sendonly`.

### Response path B — 406 Not Acceptable (counter-offer)

| Header | Value |
|--------|-------|
| Content-Type | `application/sdp; valid-until="<HTTP-date>"` |
| Location | WHEP session URL |

Body: SDP offer FROM SERVER. Direction: `sendonly` (SHOULD; MAY `sendrecv`).
Default validity: 30 seconds.

Player MUST respond within validity window via PATCH:

| Field | Value |
|-------|-------|
| Method | `PATCH` |
| URL | WHEP session URL |
| Content-Type | `application/sdp` |
| Body | SDP answer from player. Direction: `recvonly` |

Response: `204 No Content`.

### Partial media acceptance

Either party may reject individual m= lines by setting port to 0 in SDP answer. At least one audio or video m-line must remain accepted.

### Error responses

| Status | Meaning |
|--------|---------|
| 400/422 | Malformed/incompatible SDP |
| 409 | No active publisher; `Retry-After` header |
| 415 | Wrong Content-Type |
| 307 | Redirect (preferred) |
| 503 | Overloaded; `Retry-After` |

## §4 — Trickle ICE (HTTP PATCH)

Same as WHIP: PATCH to session URL with `application/trickle-ice-sdpfrag`, `If-Match` header.

**Critical constraint**: server CANNOT send additional ICE candidates after the SDP answer. Server must gather ALL candidates before responding to POST.

### ICE restart

Same as WHIP: `If-Match: "*"`, new ufrag/pwd in body.
Response: `200 OK`, new ETag, server's new credentials + candidates.

Both sides MUST replace the previous remote candidate set entirely.

### PATCH error responses

Same status codes as WHIP (422, 428, 412).

## §5 — Session termination (HTTP DELETE)

`DELETE` to WHEP session URL → `200 OK`.

## §6 — Extensions

Same mechanism as WHIP:
```
Link: <url>; rel="urn:ietf:params:whep:ext:<name>"
```
Note: URN namespace is `whep`, not `whip`.

## §7 — Authentication

Same as WHIP: Bearer token mandatory support, additional schemes allowed.

## §8 — CORS

WHEP endpoints MUST support OPTIONS for CORS.
SHOULD include `Accept-Post: application/sdp` in OPTIONS response.

## §9 — HEAD discovery

WHEP endpoints SHOULD respond to `HEAD` with:
- `200 OK`
- `Content-Type: application/sdp`

Enables automatic endpoint type detection.

## §10 — Security

Same as WHIP: HTTPS mandatory, random session URLs, rate limiting.

## Wire-format content types

| Content-Type | Used in |
|-------------|---------|
| `application/sdp` | POST request/response, PATCH (counter-offer answer) |
| `application/sdp; valid-until="<date>"` | 406 counter-offer response |
| `application/trickle-ice-sdpfrag` | PATCH trickle/restart request/response |
| `application/problem+json` | Error bodies (optional) |
