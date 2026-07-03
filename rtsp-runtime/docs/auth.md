# RTSP Authentication

**Source:** RFC 2326 §14 (Security Considerations), §12.5 (Authorization),
§12.44 (WWW-Authenticate), Appendix D.1.2/D.2.2

---

## Overview

RTSP reuses HTTP authentication verbatim (RFC 2326 §14, §16):

> "RTSP and HTTP share common authentication schemes, and thus should follow
> the same prescriptions with regards to authentication." (RFC 2326 §16)

Both **Basic** (RFC 2617 / RFC 7617) and **Digest** (RFC 2617 / RFC 7616)
authentication are defined. Servers SHOULD implement both; Digest is preferred
for environments requiring tighter security (RFC 2326 §16).

The headers used are identical to HTTP:

- **`WWW-Authenticate`** — server challenge (RFC 2326 §12.44, §16; see HTTP
  RFC 2068 §14.46).
- **`Authorization`** — client credential (RFC 2326 §12.5; see HTTP RFC 2068
  §14.8).

The only RTSP-specific rule is that the **request URI used in the Digest
computation is the RTSP request URI** (e.g.
`rtsp://camera.example.com/stream`), not an HTTP URL.

---

## Client Authentication Flow

### Step 1 — Initial request (unauthenticated)

The client sends the request without an `Authorization` header:

```
DESCRIBE rtsp://camera.example.com/stream RTSP/1.0
CSeq: 1
User-Agent: ExampleClient/1.0
```

### Step 2 — Server challenge (401)

The server responds with `401 Unauthorized` and a `WWW-Authenticate` header
containing the challenge parameters:

```
RTSP/1.0 401 Unauthorized
CSeq: 1
WWW-Authenticate: Digest realm="RTSP Server",
    nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093",
    qop="auth",
    algorithm=MD5
```

### Step 3 — Authenticated retry

The client computes the `Authorization` header using the challenge and resends
the **same request** with the credential attached. The CSeq is incremented
because this is a new request:

```
DESCRIBE rtsp://camera.example.com/stream RTSP/1.0
CSeq: 2
User-Agent: ExampleClient/1.0
Authorization: Digest username="admin",
    realm="RTSP Server",
    nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093",
    uri="rtsp://camera.example.com/stream",
    qop=auth,
    nc=00000001,
    cnonce="0a4f113b",
    response="6629fae49393a05397450978507c4ef1",
    algorithm=MD5
```

### Step 4 — Success

The server responds with `200 OK` and the requested content.

---

## Subsequent Requests

After authentication succeeds, the client **MUST re-sign every subsequent
request** using the same `Authorization` header (updating `nc`, `cnonce`, and
`response` for each request in Digest mode).

The `realm` and `nonce` from the challenge remain valid for the duration of the
session unless the server signals otherwise.

---

## Nonce Staleness (`stale=true`)

If the server returns `401` with `stale=true` in the `WWW-Authenticate`
challenge, the credentials (username/password) are still valid but the nonce
has expired. The client MUST:

1. Generate a fresh `cnonce`.
2. Recompute the `Authorization` using the new `nonce` from the challenge.
3. Resend the request without prompting the user for credentials.

If `stale=false` (or absent) and a `401` is received after authentication was
already attempted, the credentials are wrong and the user must be prompted.

---

## Implementation with the `http-auth` crate

The `http-auth` crate provides a `PasswordClient` that handles both Basic and
Digest scheme negotiation. Typical usage for `rtsp-runtime`:

```rust
use http_auth::PasswordClient;

// On receiving 401: parse the WWW-Authenticate header.
let mut pw_client = PasswordClient::try_from(www_authenticate_value)
    .expect("parse WWW-Authenticate");

// On each (re-)request: compute the Authorization value.
// The `uri` passed here MUST be the RTSP request URI.
let authorization = pw_client.respond(&http_auth::PasswordParams {
    username: "admin",
    password: "secret",
    uri:      "rtsp://camera.example.com/stream",
    method:   "DESCRIBE",
    body:     Some(&[]),
}).expect("compute Authorization");

// Attach as the Authorization header on the outgoing RTSP request.
```

Key points:
- Call `respond()` for **every** request (not just the first retry) so that
  `nc` (nonce count) increments correctly.
- The `method` field must match the RTSP method name exactly (e.g. `"SETUP"`,
  `"PLAY"`, `"DESCRIBE"`).
- The `uri` field must be the RTSP request URI, not an HTTP URL.
- When `stale=true`: call `PasswordClient::try_from` again with the new
  `WWW-Authenticate` value to pick up the fresh nonce, then `respond()` as
  normal.

---

## Minimal Implementation Requirements

Per RFC 2326 Appendix D.1.2 (client) and D.2.2 (server):

**Client MUST:**
- Recognise the `401` status code.
- Parse and include the `WWW-Authenticate` header.
- Implement Basic Authentication and Digest Authentication.

**Server MUST:**
- Generate the `401` status code when authentication is required.
- Parse and include the `WWW-Authenticate` header.
- Implement Basic Authentication and Digest Authentication.
