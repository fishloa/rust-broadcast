# RTSP Methods and Status Codes

**Source:** RFC 2326 §10 (Method Definitions), §11 (Status Code Definitions)

---

## Methods

(RFC 2326 §10)

Method names are tokens and MUST NOT start with `$` (0x24).

If a server does not support a particular method, it MUST return `501 Not
Implemented` and a client SHOULD NOT try the method again for this server.
(RFC 2326 §10, Table 2 notes.)

| Method          | Direction    | Object | Req? | State effect | Purpose |
|-----------------|-------------|--------|------|--------------|---------|
| `OPTIONS`       | C→S, S→C    | P, S   | required (S→C optional) | None | Query supported methods. May be issued at any time without affecting state. |
| `DESCRIBE`      | C→S         | P, S   | recommended | None | Retrieve presentation/media description (e.g. SDP). Constitutes the media initialization phase. |
| `ANNOUNCE`      | C→S, S→C    | P, S   | optional | None | C→S: post a presentation description to the server. S→C: update the session description in real-time. |
| `SETUP`         | C→S         | S      | required | Init→Ready; Playing→Playing (changed transport); Recording→Recording | Specify transport parameters for a stream. Generates a session identifier. |
| `PLAY`          | C→S         | P, S   | required | Ready→Playing; Playing→Playing | Start or resume delivery. Client MUST NOT issue PLAY until outstanding SETUP has been acknowledged. |
| `PAUSE`         | C→S         | P, S   | recommended | Playing→Ready; Recording→Ready | Halt stream delivery temporarily. Server resources are kept. |
| `TEARDOWN`      | C→S         | P, S   | required | Any→Init | Stop stream delivery and free resources. Session identifier becomes invalid. |
| `GET_PARAMETER` | C→S, S→C    | P, S   | optional | None | Retrieve the value of a presentation/stream parameter. With no entity body, used as a liveness ping. |
| `SET_PARAMETER` | C→S, S→C    | P, S   | optional | None | Set the value of a presentation/stream parameter. Transport parameters MUST be set with SETUP, not this method. |
| `REDIRECT`      | S→C         | P, S   | optional | Any→Init | Inform client to connect to another server. Effective immediately unless a `Range` header specifies a future time. |
| `RECORD`        | C→S         | P, S   | optional | Ready→Recording; Recording→Recording | Initiate recording of media data. |

**State-affecting methods summary:** SETUP, PLAY, PAUSE, TEARDOWN, RECORD, REDIRECT.
**State-neutral methods:** OPTIONS, DESCRIBE, ANNOUNCE, GET_PARAMETER, SET_PARAMETER.

---

## Status Codes

(RFC 2326 §11; HTTP status codes from RFC 2068 are reused where applicable.)

### 2xx Success

| Code | Reason | Notes |
|------|--------|-------|
| 200 | OK | Standard success. |
| 201 | Created | Used by RECORD when the server stores the data under a different URI. Response SHOULD contain `Location` header. |
| 250 | Low on Storage Space | Returned after RECORD when the server may not be able to fulfill the request completely due to insufficient storage. Response SHOULD contain `Range` to indicate how long recording can continue. |

### 3xx Redirection

3xx responses cause client and server state to become **Init** (RFC 2326 §A.1,
§A.2). Used for load balancing or routing to a topologically closer server.
Receiving a `REDIRECT` request (S→C) is equivalent to a 3xx response.

| Code | Reason | Notes |
|------|--------|-------|
| 301 | Moved Permanently | See HTTP. |
| 302 | Moved Temporarily | See HTTP. |

### 4xx Client Error

| Code | Reason | Notes — engine handling |
|------|--------|------------------------|
| 400 | Bad Request | Malformed syntax. |
| 401 | **Unauthorized** | Authentication required. Response MUST include `WWW-Authenticate`. Engine: retry request with `Authorization` header (see `auth.md`). |
| 403 | Forbidden | Server understood but refuses. |
| 404 | Not Found | Resource not found. |
| 405 | Method Not Allowed | Method not permitted for this resource. Response MUST include `Allow` header. Also returned if RECORD is attempted when Transport `mode` only specified PLAY. |
| 406 | Not Acceptable | Cannot generate content matching `Accept` headers. |
| 408 | Request Timeout | |
| 451 | Parameter Not Understood | Recipient does not support one or more request parameters. |
| 452 | Conference Not Found | Conference indicated by `Conference` header is unknown. |
| 453 | Not Enough Bandwidth | Request refused due to insufficient bandwidth (e.g. RSVP failure). |
| 454 | **Session Not Found** | `Session` header is missing, invalid, or has timed out. Engine: session is gone; must re-SETUP. |
| 455 | **Method Not Valid In This State** | Client or server cannot process this request in its current state. Response SHOULD contain `Allow`. Engine: this is the canonical error for state-machine violations; return it for any message received in a state where it is not listed. |
| 456 | Header Field Not Valid for Resource | Required header cannot be acted upon (e.g. `Range` on a non-seekable stream). |
| 457 | Invalid Range | Range value is out of bounds (e.g. beyond the end of the presentation). |
| 458 | Parameter Is Read-Only | SET_PARAMETER target can be read but not modified. |
| 459 | **Aggregate Operation Not Allowed** | Method may not be applied to the aggregate (presentation) URL; use a stream URL instead. Engine: returned when SETUP includes a Session id referring to an existing session that cannot be bundled. |
| 460 | Only Aggregate Operation Allowed | Method may not be applied to a stream URL; use the presentation URL. |
| 461 | **Unsupported Transport** | `Transport` field did not contain a supported transport specification. Engine: return this when no offered transport can be satisfied. |
| 462 | Destination Unreachable | Data transmission channel could not be established; client address unreachable. |

### 5xx Server Error

| Code | Reason | Notes |
|------|--------|-------|
| 500 | Internal Server Error | |
| 501 | Not Implemented | Method is not supported by this server. Client SHOULD not retry. |
| 503 | Service Unavailable | |
| 551 | Option Not Supported | An option in `Require` or `Proxy-Require` was not supported. Response SHOULD include `Unsupported` header naming the unsupported option. |

---

## Key Engine Behaviours

- **3xx received** → state transitions to Init; client must re-SETUP.
- **4xx received** → no state change; client may retry if appropriate.
- **401 received** → retry with `Authorization` (see `auth.md`).
- **454 received** → session is gone; must start from Init.
- **455 to be returned** → whenever the server receives a method not listed for
  the current state in the server state table (Appendix A.2).
- **461 to be returned** → when SETUP `Transport` contains no transport the server
  can satisfy.
