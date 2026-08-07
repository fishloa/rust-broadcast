# WebRTC-HTTP Egress Protocol (WHEP)

## Status

IETF Internet-Draft: draft-ietf-wish-whep (WISH working group)
Current revision: 04 (June 2026)
Status: Standards track (not yet RFC)

Source: https://datatracker.ietf.org/doc/draft-ietf-wish-whep/

---

## Overview

WebRTC-HTTP Egress Protocol (WHEP) defines a simple HTTP-based signaling protocol that allows WebRTC-based viewers to consume live or on-demand streaming content from origin servers, CDNs, or WebRTC Transmission Networks (WTNs).

WHEP is the egress (playback) counterpart to WHIP (WebRTC-HTTP Ingestion Protocol). The protocol standardizes viewer interaction using standard HTTP methods:
- **POST** to create a session and send SDP offer
- **PATCH** to update ICE candidates or send SDP answer
- **DELETE** to terminate the session

The protocol aims to minimize latency while remaining compatible with web browsers and HTTP infrastructure.

---

## HTTP Endpoints and Methods

### Primary Endpoint Operations

#### POST - Session Creation

Client initiates playback by sending HTTP POST with SDP offer to the WHEP endpoint URL.

```
POST /whep HTTP/1.1
Host: example.com
Content-Type: application/sdp
Authorization: Bearer <token>

<SDP offer body>
```

Response codes and behaviors:
- **201 Created** — Server accepts the offer (Pattern 1)
- **406 Not Acceptable** — Server provides counter-offer (Pattern 2)
- **409 Conflict** — Live publishing not available (include Retry-After header)
- **415 Unsupported Media Type** — Invalid Content-Type (expected application/sdp)
- **422 Unprocessable Content** — Invalid or unsupported SDP
- **503 Service Unavailable** — Server overload (include Retry-After header)

#### PATCH - Session Updates

Client sends ICE candidates or SDP answer via HTTP PATCH to the session URL returned in POST response (Location header).

```
PATCH /whep/session/<id> HTTP/1.1
Host: example.com
Content-Type: application/trickle-ice-sdpfrag
If-Match: "<etag>"
Authorization: Bearer <token>

<ICE candidates or SDP answer>
```

Response codes:
- **204 No Content** — Successful update (no body returned)
- **200 OK** — ICE restart completed (new ETag and ICE info returned)
- **400 Bad Request** — Malformed request
- **428 Precondition Required** — Missing ETag header
- **412 Precondition Failed** — ETag mismatch (session state changed)
- **422 Unprocessable Content** — Invalid ICE candidates or SDP

#### DELETE - Session Teardown

Client terminates the session by sending HTTP DELETE to the session URL.

```
DELETE /whep/session/<id> HTTP/1.1
Host: example.com
Authorization: Bearer <token>
```

Response codes:
- **200 OK** — Session successfully terminated
- **204 No Content** — Session already terminated or not found

Server frees associated resources. Server may also terminate sessions unilaterally by revoking ICE consent per RFC 7675.

#### HEAD - Endpoint Discovery

Client may use HTTP HEAD to identify WHEP endpoints without consuming content.

```
HEAD /whep HTTP/1.1
Host: example.com
```

WHEP endpoints respond with:
- **200 OK** and Content-Type: application/sdp header
- Empty body (no SDP payload)

---

## SDP Offer/Answer Exchange Flow

Two negotiation patterns are supported:

### Pattern 1: Server Accepts Offer (Common Case)

1. Client sends SDP offer via POST
2. Server responds with **201 Created** containing:
   - SDP answer in body
   - `Location` header with session URL (e.g., `/whep/session/<uuid>`)
   - `ETag` header with session identifier
   - Optional `Link` headers (STUN/TURN servers, extensions)
3. Playback begins (ICE gathering/connection in parallel)

### Pattern 2: Server Counter-Offer

1. Client sends SDP offer via POST
2. Server responds with **406 Not Acceptable** containing:
   - Alternative SDP offer in body
   - `Location` header with session URL for counter-offer response
3. Client sends SDP answer via PATCH to session URL
4. Server responds with **204 No Content**
5. Playback begins

Both patterns must be supported by WHEP players.

---

## ICE Candidate Trickle Exchange

Incremental ICE candidate exchange is performed via HTTP PATCH with `application/trickle-ice-sdpfrag` content type.

### Candidate Buffering

Clients may buffer ICE candidates until receiving the initial **201 Created** response before transmitting them. This reduces HTTP requests when candidates arrive before SDP answer.

### Candidate Transmission

Candidates are transmitted in SDP fragment format (RFC 8829 §4.1.16):

```
PATCH /whep/session/<id> HTTP/1.1
Content-Type: application/trickle-ice-sdpfrag
If-Match: "<etag>"

a=candidate:...
a=candidate:...
```

Multiple candidates may be included in a single PATCH request.

### Server Candidate Handling

Server must gather all received candidates before responding to initial POST. ICE candidate trickling must not delay the initial SDP answer delivery.

---

## Session State Management with ETags

### ETag Header

Server returns `ETag` header in **201 Created** response. ETag value identifies the specific ICE session state and prevents race conditions during concurrent updates.

```
201 Created
ETag: "abc123def456"
Location: /whep/session/12345
```

### If-Match Precondition Header

Client includes `If-Match` header in all PATCH requests with the ETag value received from the initial POST response.

```
PATCH /whep/session/12345 HTTP/1.1
If-Match: "abc123def456"
```

Precondition failure responses:
- **428 Precondition Required** — If-Match header missing (client must retry with ETag)
- **412 Precondition Failed** — ETag value does not match server state (session state changed; client should re-POST)

### ICE Restart with Wildcard Match

For ICE restarts, client sends `If-Match: "*"` to explicitly request session restart:

```
PATCH /whep/session/12345 HTTP/1.1
If-Match: "*"
Content-Type: application/trickle-ice-sdpfrag

<new ICE credentials and candidates>
```

Server responds with **200 OK** containing:
- New ETag in response header
- Updated ICE information (new ufrag/pwd)

---

## Layer and Simulcast Selection

WHEP does not define explicit layer/simulcast selection in the core protocol. Selection is typically encoded in SDP offer via:

- **Codec constraints** — Client specifies supported codecs to enable server source selection
- **Media format parameters** — Profile-level-id and other codec-specific constraints
- **SIMULCAST a= lines** — Client indicates supported simulcast layers (if applicable)

Server adapts stream to client capabilities signaled in SDP offer.

---

## Session Teardown (DELETE)

### Client-Initiated Teardown

Client sends HTTP DELETE to terminate playback:

```
DELETE /whep/session/<id> HTTP/1.1
Host: example.com
Authorization: Bearer <token>
```

Server responds with **200 OK** or **204 No Content** and frees resources:
- Stops streaming media
- Releases connection slots
- Cleans up ICE state
- Revokes ICE consent per RFC 7675

### Server-Initiated Teardown

Server may terminate sessions unilaterally:
- Revoke ICE consent
- Stop media transmission
- Clean up resources

Server does NOT send HTTP response for unilateral termination. Media stops when ICE connectivity is lost.

---

## Authentication

### Bearer Token Authentication

Authentication uses HTTP Authorization header with Bearer scheme (RFC 6750):

```
Authorization: Bearer <access_token>
```

WHEP players MUST implement Bearer token authentication and include the Authorization header in all HTTP requests (POST, PATCH, DELETE, HEAD).

Bearer tokens are typically:
- Opaque strings (server-specific format)
- Time-limited (with explicit expiry)
- Issued by authentication/authorization service
- Validated by WHEP endpoint on each request

### Token Validation

Server validates Bearer token on each request:
- Verify token signature/validity
- Check token expiry
- Verify subject has access to requested stream/session
- Return **401 Unauthorized** for invalid/expired tokens

### Other Authentication Schemes

The core specification defines Bearer token support. Other schemes (Basic, Digest, mTLS) may be defined as extensions.

---

## ETag and If-Match for Session State

See "Session State Management with ETags" section above.

---

## Server-Sent Events (SSE)

### Extension Signaling

Server may advertise Server-Sent Events support via Link header in **201 Created** response:

```
Link: <https://example.com/whep/session/<id>/events>; rel="urn:ietf:params:whep:ext:server-sent-events"
```

### Event Stream

Client opens persistent connection to SSE URL:

```
GET /whep/session/<id>/events HTTP/1.1
Host: example.com
Accept: text/event-stream
Authorization: Bearer <token>
```

Server may transmit events indicating:
- Metadata changes (title, bitrate, language options)
- Stream status changes
- Layer/simulcast availability
- Connection quality information
- Custom application events

Events use standard SSE format with `event:` and `data:` fields.

---

## HTTP Status Codes and Error Handling

### Success Responses

| Status | Meaning | Context |
|--------|---------|---------|
| 200 OK | Request succeeded; body may contain response data | ICE restart completion; session deletion; HEAD endpoint discovery |
| 201 Created | Session created; body contains SDP answer | POST response (Pattern 1) |
| 204 No Content | Request succeeded; no body to return | PATCH ICE candidate; PATCH SDP answer (Pattern 2) |

### Client Error Responses

| Status | Meaning | Context | Retry |
|--------|---------|---------|-------|
| 400 Bad Request | Malformed request (e.g., invalid Content-Type without sdp) | POST/PATCH/DELETE with syntax errors | Yes (after correction) |
| 401 Unauthorized | Missing or invalid credentials | Missing Authorization header; invalid/expired token | Yes (with new token) |
| 403 Forbidden | Authenticated but lacks permission to stream | Valid token but no access to stream | No (permission required) |
| 406 Not Acceptable | Server counter-offer provided instead | POST with incompatible SDP offer | Yes (respond to counter-offer) |
| 409 Conflict | Live publishing not available; stream unavailable | POST when stream not actively publishing | Yes (with Retry-After) |
| 412 Precondition Failed | ETag mismatch; session state changed | PATCH with outdated If-Match header | Yes (re-POST for new session) |
| 415 Unsupported Media Type | Invalid Content-Type | POST/PATCH without application/sdp or application/trickle-ice-sdpfrag | Yes (after correction) |
| 422 Unprocessable Content | Invalid or unsupported SDP/ICE | POST/PATCH with syntactically correct but semantically invalid SDP | No (incompatible offer) |
| 428 Precondition Required | Missing ETag header | PATCH without If-Match header | Yes (with ETag from POST response) |

### Server Error Responses

| Status | Meaning | Context | Retry |
|--------|---------|---------|-------|
| 500 Internal Server Error | Unexpected server error | Internal server fault | Yes (exponential backoff) |
| 503 Service Unavailable | Server overload or maintenance | Too many concurrent sessions; server restarting | Yes (with Retry-After) |

### Retry-After Header

Server may include `Retry-After` header (HTTP-date or delay-seconds) with **409** and **503** responses to indicate when client should retry.

```
Retry-After: 120
```

### Error Details

Servers MAY include additional error details in response body using JSON Problem Details format (RFC 9457):

```json
{
  "type": "urn:ietf:params:whep:errors:incompatible-sdp",
  "title": "Incompatible SDP Offer",
  "detail": "Requested codec H.265 not supported by this server"
}
```

### Graceful Degradation

WHEP players MUST gracefully handle all applicable HTTP status codes. Unknown status codes MUST fall back to generic semantics of their 1xx/2xx/3xx/4xx/5xx class.

---

## Link Header Extensions

### Extension Advertisement

Server advertises supported extensions in **201 Created** response using Link headers with `rel` attribute containing extension URN:

```
Link: <urn:ietf:params:whep:ext:core-api>; rel="urn:ietf:params:whep:ext:core-api"
Link: <https://example.com/docs/custom-ext>; rel="urn:ietf:params:whep:ext:custom-feature"
```

Extension URN format: `urn:ietf:params:whep:ext:<extension-name>`

### Standard Extensions (Examples)

#### STUN/TURN Server Discovery

Server may advertise STUN/TURN URLs via Link headers:

```
Link: <stun:stun.example.com>; rel="ice-server"
Link: <turn:turn.example.com?transport=udp>; rel="ice-server"; credential="password"; username="user"
```

Clients use these servers for ICE connectivity if configured to do so.

#### Server-Sent Events Extension

Server advertises SSE support for metadata/events:

```
Link: <https://example.com/whep/session/<id>/events>; rel="urn:ietf:params:whep:ext:server-sent-events"
```

#### Custom Extensions

Vendors may define custom extensions with vendor-specific URNs. Clients MUST NOT fail if unknown extensions are advertised; they simply ignore them.

---

## Codec and Media Constraints

### SDP Offer Codec Coverage

Clients SHOULD include comprehensive codec support in SDP offer to enable:
- Dynamic source switching (server selecting from available streams)
- Adaptive bitrate scenarios (server transcoding to supported codecs)
- Multi-codec stream delivery

### Mandatory SDP Constraints

#### Bundle Policy

- Clients MUST include `a=group:BUNDLE` to indicate support for bundled media
- All m-lines MUST share identical `a=msid` values (single MediaStream)
- RTP and RTCP MUST be multiplexed (RFC 8853: `a=rtcp-mux`)

#### Partial Media Acceptance

m-lines not supported by server are rejected by setting port to 0:

```
m=video 0 RTP/SAVP 120
```

#### Media Flow Control

Clients and servers MUST support:
- `a=rtcp-fb` for RTCP feedback (PLI, NACK)
- `a=rtcp-rsize` for reduced-size RTCP
- `a=maxptime` for packetization limits

---

## Endpoint Discovery and Capability Negotiation

### HEAD Request for Endpoint Verification

Client may issue HTTP HEAD to verify WHEP endpoint availability:

```
HEAD /whep HTTP/1.1
Host: example.com
```

WHEP endpoints respond with:

```
200 OK
Content-Type: application/sdp
```

No body is returned. This allows clients to detect WHEP capability before sending POST.

### Capability Signaling via SDP Offer

Clients signal capabilities in SDP offer:
- Supported codecs (VP8, VP9, H.264, H.265, AV1)
- Profile-level-id constraints
- Maximum bitrate (via `a=b:` bandwidth lines)
- Media types (audio, video, data)
- Extensions (RTX, FEC, ULPFEC)

Server adapts stream based on client offer; codec selection is implicit (not explicitly negotiated).

---

## Wire Protocol Examples

### Complete POST/201 Exchange (Pattern 1)

```http
POST /whep HTTP/1.1
Host: example.com
Content-Type: application/sdp
Content-Length: 512
Authorization: Bearer eyJhbGc...

v=0
o=- 123456789 2 IN IP4 198.51.100.1
s=Live Stream
t=0 0
a=group:BUNDLE 0
a=extmap-allow-mixed
a=msid-semantic: WMS stream
m=video 9 RTP/SAVP 96 97 98
a=rtcp:9 IN IP4 0.0.0.0
a=rtcp-mux
a=setup:actpass
a=mid:0
a=sendrecv
a=rtcp-fb:* transport-cc
a=rtcp-fb:* ccm fir
a=rtpmap:96 VP9/90000
a=rtpmap:97 VP8/90000
a=rtpmap:98 H264/90000
a=ice-ufrag:ABCDEfGHIJKLMN
a=ice-pwd:0123456789abcdefghijklmnopqr
a=fingerprint:sha-256 AA:BB:CC:DD:...
```

Server Response:

```http
201 Created
Location: /whep/session/550e8400-e29b-41d4-a716-446655440000
ETag: "550e8400e29b41d4a716446655440000"
Content-Type: application/sdp
Link: <stun:stun.example.com>; rel="ice-server"
Link: <https://example.com/whep/session/550e8400-e29b-41d4-a716-446655440000/events>; rel="urn:ietf:params:whep:ext:server-sent-events"
Content-Length: 512

v=0
o=- 987654321 2 IN IP4 198.51.100.2
s=Live Stream
t=0 0
a=group:BUNDLE 0
a=extmap-allow-mixed
a=msid-semantic: WMS stream
m=video 5000 RTP/SAVP 96
a=rtcp:5001 IN IP4 198.51.100.2
a=rtcp-mux
a=setup:passive
a=mid:0
a=sendonly
a=rtcp-fb:* transport-cc
a=rtcp-fb:* ccm fir
a=rtpmap:96 VP9/90000
a=ice-ufrag:DEFGHIJKLMNOPQRSTUVWXYZab
a=ice-pwd:zyxwvutsrqponmlkjihgfedcba98765
a=fingerprint:sha-256 11:22:33:44:...
```

### ICE Candidate Exchange

```http
PATCH /whep/session/550e8400-e29b-41d4-a716-446655440000 HTTP/1.1
Content-Type: application/trickle-ice-sdpfrag
If-Match: "550e8400e29b41d4a716446655440000"
Content-Length: 256

a=candidate:1 1 UDP 2113937151 203.0.113.1 54400 typ host
a=candidate:2 1 UDP 2113937151 2001:db8::1 54401 typ host
a=end-of-candidates
```

Server Response:

```http
204 No Content
```

### Session Deletion

```http
DELETE /whep/session/550e8400-e29b-41d4-a716-446655440000 HTTP/1.1
Authorization: Bearer eyJhbGc...
```

Server Response:

```http
200 OK
```

---

## Timing and Resource Management

### Connection Timeout

Client MUST establish WebRTC connection within a server-defined timeout (typically 10-30 seconds). Server may close session if ICE fails to connect within this window.

### Keep-Alive Handling

Once WebRTC connection is established:
- Media packets maintain connection alive
- RTCP keeps connection alive
- HTTP connection (for PATCH/DELETE) is separate

### Resource Limits

Server may enforce:
- Maximum concurrent sessions per user/IP
- Maximum session duration
- Bandwidth throttling
- Subscription limits

---

## Extensibility and Future Protocols

### Extension Mechanism

The Link header `rel="urn:ietf:params:whep:ext:*"` allows future extensions without breaking existing implementations:

1. New features advertised as extensions via Link headers
2. Clients detect support via rel attribute
3. Clients optionally use extension (graceful degradation if not supported)
4. No breaking changes to core protocol

### Future Protocol Evolution

Possible extensions (not yet standardized):
- Explicit layer/bitrate selection
- Metadata streaming (via SSE)
- Codec preference signaling
- Quality metrics reporting
- Custom authentication schemes
- Recording/archival extensions

---

## References

- **WHIP Specification** — WebRTC-HTTP Ingestion Protocol (complementary egress protocol)
- **RFC 3550** — RTP: A Transport Protocol for Real-Time Applications
- **RFC 3551** — RTP Profile for Audio and Video Conferences with Minimal Control
- **RFC 5245** — Interactive Connectivity Establishment (ICE)
- **RFC 5763** — Framework for Establishing a Secure Real-time Transport Protocol (SRTP) Sessions with DTLS
- **RFC 5764** — DTLS Extension to Establish Keys for SRTP
- **RFC 6750** — The OAuth 2.0 Bearer Token Usage
- **RFC 7675** — Session Traversal Utilities for NAT (STUN) Usage for Consent Freshness
- **RFC 8174** — Ambiguity of Uppercase vs Lowercase in RFC 2119 Key Words
- **RFC 8285** — A General Mechanism for Negotiating the Use of the RTP Header Extension
- **RFC 8829** — JavaScript Session Establishment Protocol (JSEP)
- **RFC 8853** — Using Transport Layer Security (TLS) in RTCP
- **RFC 9457** — Problem Details for HTTP APIs
