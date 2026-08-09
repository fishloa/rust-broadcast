# RFC 8489 — Session Traversal Utilities for NAT (STUN)

Source: `https://www.rfc-editor.org/rfc/rfc8489.txt`, February 2020.
Obsoletes RFC 5389. Section numbers below are RFC 8489's own.

Implemented by the external `rtc-stun` crate underneath
[`webrtc_runtime::media::gather::StunGather`] (server-reflexive candidate
gathering — a one-shot Binding transaction) and, separately, underneath
`rtc-ice`'s connectivity checks inside
[`webrtc_runtime::media::transport::MediaTransport`]'s `IceAgent`. This
crate's own code (`gather.rs`) only builds an unauthenticated Binding
request and reads `XOR-MAPPED-ADDRESS` back out of the response — the rest
of the STUN machinery below (message-integrity, long-term credentials,
retransmission timing) lives in `rtc-stun`.

**Note on scope**: PRIORITY, USE-CANDIDATE, ICE-CONTROLLED, and
ICE-CONTROLLING are *not* defined in this RFC — despite being STUN
attributes, they are ICE's own extension to STUN, formally defined in
RFC 8445 §7.1/§16.1. They're transcribed in `rfc8445-ice.md`, not here;
listing them under "STUN" in a file-organization sense would misattribute
the citation.

## 1. Message structure (§5)

20-byte fixed header + zero or more TLV attributes, all big-endian:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|0 0|     STUN Message Type     |         Message Length        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                         Magic Cookie                          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
|                     Transaction ID (96 bits)                  |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

| Field | Width | Notes |
|---|---|---|
| top 2 bits | 2 | MUST be `00` — lets STUN be told apart from other protocols multiplexed on the port (see the RFC 5764 demux rule in `rfc5764-dtls-srtp.md`, which relies on this: byte 0 < 2 ⇒ STUN). |
| STUN Message Type | 14 | Method (12 bits, M11..M0) + class (2 bits, C1 C0) — see below. |
| Message Length | 16 | Size of the message **after** the 20-byte header, in bytes, **not including** the header. Attributes are always padded to a 4-byte multiple, so the low 2 bits of this field are always 0. |
| Magic Cookie | 32 | Fixed `0x2112A442`, network byte order. |
| Transaction ID | 96 | MUST be uniformly random over `0..2^96-1` and cryptographically random. Request/response pairs share one; an indication picks its own. Resent requests reuse the same ID; a genuinely new request needs a new one unless it's a byte-identical retransmission from the same transport address. |

### 1.1 Message Type field layout (§5, Figure 3)

```
                    0                 1
                    2  3  4 5 6 7 8 9 0 1 2 3 4 5
                   +--+--+-+-+-+-+-+-+-+-+-+-+-+-+
                   |M |M |M|M|M|C|M|M|M|C|M|M|M|M|
                   |11|10|9|8|7|1|6|5|4|0|3|2|1|0|
                   +--+--+-+-+-+-+-+-+-+-+-+-+-+-+
```

`C1 C0`: `00` = request, `01` = indication, `10` = success response,
`11` = error response. `M11..M0` = 12-bit method. This RFC defines exactly
one method, **Binding** (`0b000000000001` = `0x001`). Worked encodings:
Binding request = `0x0001`; Binding success response = `0x0101`.

## 2. Attribute structure (§14)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|         Type                  |            Length             |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                         Value (variable)                ....
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

Type (16 bits) + Length (16 bits, the *unpadded* value size in bytes) +
Value, zero-padded (1-3 zero bytes) up to a 4-byte boundary; the padding
bits are ignored on receipt. Attribute type-space split:

- `0x0000`–`0x7FFF` — **comprehension-required**: an agent that doesn't
  understand the attribute cannot process the message.
- `0x8000`–`0xFFFF` — **comprehension-optional**: safe to ignore if
  unrecognized.

An attribute type MAY repeat; unless stated otherwise only the first
occurrence is normative, duplicates MAY be ignored.

## 3. Retransmission timing over UDP (§6.2.1)

- Client retransmits a request starting at interval **RTO**, doubling
  each retransmit, computed per RFC 6298 with two exceptions: initial RTO
  SHOULD be >= 500 ms (RECOMMENDED value: exactly 500 ms on fixed-line
  links — ICE's own §14 formula, see `rfc8445-ice.md`, is one of the named
  exceptions to plain RFC 6298 computation); RTO SHOULD NOT round to whole
  seconds (maintain 1 ms accuracy).
- Karn's algorithm RECOMMENDED: don't fold RTT samples from a transaction
  that needed a retransmit into the RTO estimate.
- Cache RTO per-server (by IP) across transactions; discard as stale after
  10 minutes of no traffic to that server.
- Retransmit until a response arrives or **Rc** requests have been sent
  (SHOULD be configurable, default **7**). After the last request, if
  **Rm** × RTO elapses with no response (SHOULD be configurable, default
  **16**), the transaction is considered failed. Worked example at
  RTO=500ms: requests at 0, 500, 1500, 3500, 7500, 15500, 31500 ms (7
  sends, doubling each time); failure declared at 39500 ms if nothing
  came back (31500 + 16×500 = 39500).
- A hard ICMP error also fails the transaction immediately.

`gather.rs`'s `StunGather` does not itself implement retransmission — that
schedule lives in `rtc-stun`'s `Client`
(`Client::handle_timeout`/`Protocol::handle_timeout`, driven by
`MediaTransport::handle_timeout`). Nothing to compare in this crate's own
code beyond confirming it drives the timer at all, which it does
(`gather.rs::handle_timeout` calls `Protocol::handle_timeout` every time
`MediaTransport::handle_timeout` fires).

## 4. Attributes this crate touches

### 4.1 XOR-MAPPED-ADDRESS (§14.2, type `0x0020`)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|0 0 0 0 0 0 0 0|    Family     |         X-Port                |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                X-Address (Variable)
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

- Family: `0x01` = IPv4 (32-bit address), `0x02` = IPv6 (128-bit).
- `X-Port` = mapped port XOR the top 16 bits of the magic cookie.
- `X-Address` = mapped IP XOR the magic cookie (IPv4) or magic cookie ||
  transaction ID (IPv6), all in network byte order.
- Exists (vs. plain MAPPED-ADDRESS, §14.1) specifically because some NATs
  rewrite 32-bit values that look like their own public IP found in a
  payload; XOR'ing defeats that "helpful" ALG rewriting.

`gather.rs::handle_datagram` reads this attribute (`XorMappedAddress`)
from the `StunEvent::Message` success case — the one attribute this
crate's own code decodes directly. No discrepancy: the crate delegates
the XOR math itself to `rtc_stun::xoraddr::XorMappedAddress::get_from`,
consuming only the resolved `SocketAddr`.

### 4.2 MESSAGE-INTEGRITY (§14.5, type `0x0008`) and MESSAGE-INTEGRITY-SHA256 (§14.6, type `0x001C`)

- MESSAGE-INTEGRITY: HMAC-SHA1 over the message up to (not including) this
  attribute, with the header's Length field temporarily adjusted to end at
  this attribute; tag is fixed 20 bytes.
- MESSAGE-INTEGRITY-SHA256: same idea with HMAC-SHA256, output 16-32
  bytes (multiple of 4), full 32 bytes unless the STUN Usage explicitly
  permits truncation.
- Key comes from whichever credential mechanism is in play (§9.1.1 short-
  term: `key = OpaqueString(password)`; §9.2.2 long-term: a realm-bound
  MD5/SHA-256 digest — not used by ICE, so not detailed here).
- Ordering rule: FINGERPRINT and MESSAGE-INTEGRITY-SHA256 (when both
  present) come **after** MESSAGE-INTEGRITY, so computing MESSAGE-
  INTEGRITY never covers them.

**Not used anywhere in this crate.** `gather.rs`'s Binding request carries
no USERNAME/MESSAGE-INTEGRITY at all — consistent with RFC 8445 §5.1.1.2's
own statement that "Binding requests to a STUN server are not
authenticated" for server-reflexive gathering. The short-term credential
mechanism *is* exercised for ICE connectivity checks (RFC 8445 §7.2.2, see
`rfc8445-ice.md`), but that machinery lives entirely inside `rtc-ice`, not
in this crate's own `media/` code — nothing here to check `gather.rs`
against on that front.

### 4.3 FINGERPRINT (§14.7, type `0x8028`)

- Value = CRC-32 (ITU V.42 polynomial) of the message up to (excluding)
  this attribute, XOR'ed with `0x5354554e`. MUST be the last attribute if
  present (after MESSAGE-INTEGRITY/-SHA256).
- Its purpose per §7 is exactly the demux role RFC 5764 §5.1.2 leans on:
  telling STUN packets apart from other protocols sharing a port.

Not touched by this crate's own code (`gather.rs` builds only a bare
Binding request with a transaction ID, no FINGERPRINT); whether `rtc-stun`
adds one under the hood for `ClientBuilder`-built requests wasn't checked
(out of scope — that's the external crate's internals, not this
repository's).

### 4.4 USERNAME (§14.3, type `0x0006`)

UTF-8, OpaqueString-processed (RFC 8265), < 509 bytes; a compliant
implementation must also be able to *parse* up to 763 octets for
RFC 5389 back-compat. Not used by `gather.rs`'s unauthenticated Binding
request; used inside `rtc-ice`'s connectivity checks per RFC 8445 §7.2.2
(see `rfc8445-ice.md`), outside this crate's own code.

## 5. Attribute type registry (relevant subset, §18.3.1/§18.3.2)

| Type | Attribute |
|---|---|
| `0x0001` | MAPPED-ADDRESS |
| `0x0006` | USERNAME |
| `0x0008` | MESSAGE-INTEGRITY |
| `0x0009` | ERROR-CODE |
| `0x000A` | UNKNOWN-ATTRIBUTES |
| `0x0014` | REALM |
| `0x0015` | NONCE |
| `0x001C` | MESSAGE-INTEGRITY-SHA256 |
| `0x001D` | PASSWORD-ALGORITHM |
| `0x001E` | USERHASH |
| `0x0020` | XOR-MAPPED-ADDRESS |
| `0x8002` | PASSWORD-ALGORITHMS |
| `0x8003` | ALTERNATE-DOMAIN |
| `0x8022` | SOFTWARE |
| `0x8023` | ALTERNATE-SERVER |
| `0x8028` | FINGERPRINT |

(PRIORITY `0x0024`, USE-CANDIDATE `0x0025`, ICE-CONTROLLED `0x8029`,
ICE-CONTROLLING `0x802A` are ICE's, RFC 8445 §16.1 — see
`rfc8445-ice.md`, not this table.)

## 6. What this crate does NOT implement (all delegated to `rtc-stun`/`rtc-ice`)

- Long-term credential mechanism (§9.2) — irrelevant to ICE's short-term
  use anyway.
- TCP/TLS-over-TCP/DTLS-over-UDP framing (§6.2.2/§6.2.3) — this crate is
  UDP-only (`transport_protocol: TransportProtocol::UDP` hardcoded
  throughout `transport.rs`/`gather.rs`).
- ALTERNATE-SERVER (§10) — RFC 8445 §7.3.1 explicitly forbids an ICE agent
  from using it anyway, and `gather.rs` doesn't touch it.
- Server-side Binding processing (§6.3, §7) — this crate is a STUN
  *client* only (both for server-reflexive gathering and, via `rtc-ice`,
  for ICE checks); it never answers a Binding request itself as a server
  in `media/`'s own code (ICE requires answering checks too, but again
  that's `rtc-ice`'s internals).

No discrepancy found in what this crate's own code (`gather.rs`) actually
does with STUN — it is a thin, spec-consistent slice (unauthenticated
Binding request out, `XOR-MAPPED-ADDRESS` in) of a much larger protocol
whose remaining surface is implemented by `rtc-stun`/`rtc-ice`.
