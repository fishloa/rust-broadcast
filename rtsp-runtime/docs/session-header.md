# RTSP Session Header

**Source:** RFC 2326 §12.37 (text vendored at `specs/rfc2326_rtsp.txt`, lines 3174-3229);
base syntax §15.1 (lines 4013+); field-value parsing rules inherited from RFC 2068 §2.2/§4.2.

## Grammar (verbatim, RFC 2326 §12.37)

```
Session  = "Session" ":" session-id [ ";" "timeout" "=" delta-seconds ]
```

`session-id` (§3.4): `1*( ALPHA | DIGIT | safe )`, where
`safe = "\$" | "-" | "_" | "." | "+"` (§15.1). Session identifiers are opaque
strings; RFC 2326 §3.4 requires at least eight octets for security but parsers
MUST NOT reject shorter ones received from a peer.

`delta-seconds` (RFC 2068 §3.3.2): `1*DIGIT`.

## Semantics

- The timeout parameter is only allowed in a response header.
- The timeout is measured in seconds, with a default of 60 seconds.
- A client MUST return the session identifier for any request related to that session.
- 454 (Session Not Found) is returned if the session identifier is invalid.

## Interop rules (decided 2026-10-03, owner option (c): own parser)

`rtsp-types` 0.1.3 mis-parses real-world Session headers (probe:
`.delegate/rtsp-types-probe.txt`). Our parser is **lenient on input, canonical on
output**:

| input | parse result | rationale |
|---|---|---|
| `abc;timeout=30` | id `abc`, timeout 30 | grammar |
| `abc; timeout=30` / `abc ; timeout=30` | id `abc`, timeout 30 | LWS between tokens and separators is permitted by RFC 2068 §2.1 implied LWS |
| `abc;TIMEOUT=30`, `abc;timeout = 30` | id `abc`, timeout 30 | parameter names are case-insensitive tokens; implied LWS |
| `abc;timeout=x` | id `abc`, timeout = default (60 s) **and** a recorded warning; header NOT rejected | a bad optional parameter must not discard the mandatory session id |
| `abc;timeout=0` | id `abc`, timeout = default (60 s) **and** a recorded warning | 0 is not a usable timeout (it would make the keepalive deadline "now"); the client additionally floors the keepalive interval at 1 s |
| `"weird"`, `a b;timeout=30`, `abc,def`, `ab"c` | id kept verbatim (`"weird"`, `a b`, `abc,def`, `ab"c`) | interop: real servers send such ids and the client MUST echo the id unchanged; main stored and echoed them all |
| an id containing a control character (CR, LF, ...) | error | echoing it would allow header injection |
| `abc;foo=bar` | id `abc`, unknown parameter preserved | forward compatibility |
| empty / whitespace-only id | error | grammar requires `1*` |

Serializer output is always `<id>[;timeout=<n>]` with no whitespace. The serializer is strict only for ids WE emit: it returns `Error::HeaderSerialize` (never emits) for an empty/untrimmed id, an id with a control character or that would swallow the `;` separator (unbalanced quote), a non-token extension name, or a control character in a value.

## Implementation notes

`src/session_header.rs` (`SessionHeader { id, timeout, extensions }`), shared lexer
`src/rfc2326_lex.rs`. The id is everything before the first `;` outside a quoted-string, LWS-trimmed and non-empty; any such id without a control character is accepted verbatim. A malformed
`timeout` (including more than 19 digits) keeps the id and sets the 60 s default, reported by
`SessionHeader::parse_with_warnings`. Round-trip invariants as for Transport.
