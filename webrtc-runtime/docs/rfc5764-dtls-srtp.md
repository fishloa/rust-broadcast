# RFC 5764 — DTLS Extension to Establish Keys for SRTP (DTLS-SRTP)

Source: `https://www.rfc-editor.org/rfc/rfc5764.txt`, May 2010. Section
numbers below are RFC 5764's own.

Implemented by [`webrtc_runtime::media::transport::MediaTransport`]
(`webrtc-runtime/src/media/transport.rs`) via the external `rtc-dtls`
crate for the handshake itself, with this crate directly performing the
`use_srtp` extension's profile choice and the keying-material export/split
(§4.1.2, §4.2) — those two are the parts checked line-by-line below.

## 1. Session model (§3)

- One DTLS-SRTP session = one DTLS association on one UDP source/dest
  **port pair**, protecting either one SRTP context (unidirectional media)
  or two (bidirectional).
- Which side is DTLS client vs. server is decided **out of band** (e.g.
  SDP `a=setup`, RFC 8842) — DTLS-SRTP itself does not pick.
- RTP and RTCP conventionally use separate ports, needing separate
  DTLS-SRTP sessions per direction unless:
  - **Symmetric RTP** (RFC 4961, RECOMMENDED): one bidirectional session
    covers both directions of a port.
  - **RTP/RTCP mux** (RFC 5761, see `rfc5761-rtcp-mux.md`): one
    DTLS-SRTP session covers both RTP and RTCP on the shared port. The
    DTLS layer does not need to distinguish RTP from RTCP itself — that
    demux happens in the SRTP layer per RFC 5761.
- Client-originated and server-originated packets use **distinct SRTP
  keys** (see §4.2 below) — this is why the exporter yields four values,
  not two.
- SSRCs from the same device over the same DTLS-SRTP channel MUST be
  distinct (avoids RFC 3711 §9.1's two-time-pad problem); this does not
  matter across independently-keyed streams even if their SSRCs collide.

## 2. `use_srtp` extension (§4.1)

Negotiated in the DTLS extended ClientHello/ServerHello:

```c
uint8 SRTPProtectionProfile[2];
struct {
   SRTPProtectionProfiles SRTPProtectionProfiles;
   opaque srtp_mki<0..255>;
} UseSRTPData;
SRTPProtectionProfile SRTPProtectionProfiles<2..2^16-1>;
```

Client lists acceptable profiles in descending preference order + an
optional MKI it intends to use (empty = no MKI). Server, if accepting,
echoes back **exactly one** chosen profile (MUST be one the client
offered) + either a matching `srtp_mki` or an empty one (meaning "won't
use the MKI"). No shared profile ⇒ server SHOULD omit the extension (fall
back to plain DTLS) or send an alert.

### 2.1 Defined protection profiles (§4.1.2)

| Profile | Value | cipher | key len | salt len | max lifetime | auth | auth key len | auth tag len |
|---|---|---|---|---|---|---|---|---|
| `SRTP_AES128_CM_HMAC_SHA1_80` | `{0x00,0x01}` | AES_128_CM | 128 | 112 | 2^31 | HMAC-SHA1 | 160 | 80 |
| `SRTP_AES128_CM_HMAC_SHA1_32` | `{0x00,0x02}` | AES_128_CM | 128 | 112 | 2^31 | HMAC-SHA1 | 160 | 32 (RTCP: 80) |
| `SRTP_NULL_HMAC_SHA1_80` | `{0x00,0x05}` | NULL | 0 | 0 | 2^31 | HMAC-SHA1 | 160 | 80 |
| `SRTP_NULL_HMAC_SHA1_32` | `{0x00,0x06}` | NULL | 0 | 0 | 2^31 | HMAC-SHA1 | 160 | 32 (RTCP: 80) |

All four are RFC 3711 crypto (see `rfc3711-srtp.md`). Implicit options for
every one of these profiles:

- TLS PRF generates the exporter material (DTLS 1.2's cipher-suite PRF, or
  the TLS PRF for DTLS 1.0 — this spec works with either).
- **Key Derivation Rate (KDR) = 0** — keys are never re-derived by SRTP
  sequence number (matches RFC 3711 §5.3's own default).
- RFC 3711 §4.3's AES-CM PRF key-derivation procedure is used.
- Every other SRTP parameter (replay window size, FEC order, ...) takes
  RFC 3711's default. Signalling non-default values needs a separate spec.

`this crate` only ever offers `SRTP_AES128_CM_HMAC_SHA1_80` — the "every
WebRTC implementation must support this one" profile (see
`OFFERED_SRTP_PROFILE` in `transport.rs`); the other three are valid per
RFC 5764 but not offered by this cut.

### 2.2 `srtp_mki` (§4.1.3)

Optional. If used, the client MUST choose an MKI distinct from its own
last-used value; the server MUST either echo a matching value or return
empty (declining MKI use); a client-observed mismatched nonzero MKI in the
server's response is an abort (`invalid_parameter` alert). One DTLS-SRTP
session has **at most one active MKI** at a time (the exception being a
rehandshake in flight). **Not used by this crate** — `MediaTransportConfig`
has no MKI field and `ConfigBuilder`/`SrtpContext::new` in `transport.rs`
pass `None, None` for the MKI-related parameters.

## 3. Key derivation / export (§4.2)

The TLS/DTLS exporter (RFC 5705) is invoked with:

- **label**: the literal string `"EXTRACTOR-dtls_srtp"` (the `EXTRACTOR`
  prefix is a historical artifact, not a live meaning).
- **context**: empty (the per-association context value, RFC 5705's own
  term, is unused here).
- **length**: `2 * (master_key_len + master_salt_len)` bytes, where the
  lengths come from the negotiated `SRTPProtectionProfile` (table above).

The exported bytes are split into exactly four values, **in this order**:

```
client_write_SRTP_master_key  [master_key_len]
server_write_SRTP_master_key  [master_key_len]
client_write_SRTP_master_salt [master_salt_len]
server_write_SRTP_master_salt [master_salt_len]
```

i.e. both keys first, then both salts — **not** interleaved
key/salt-per-side. Each `(master_key, master_salt)` pair for a direction is
then run through RFC 3711 §4.3's key derivation independently to produce
that direction's SRTP session keys and (when RTP+RTCP share a port) its
SRTCP session keys too.

- `client_write_*` → used by the client to protect what it sends; the
  **server** decrypts/verifies inbound traffic with these (never encrypts
  with them).
- `server_write_*` → the mirror: used by the server to protect what it
  sends; the **client** decrypts/verifies inbound traffic with these.

When one DTLS-SRTP session protects RTCP only (RTP on a separate,
non-multiplexed session), the SRTP-labelled keys derived from it are
simply discarded, and vice versa (§4.2, Figure 2) — the exporter always
produces both SRTP- and SRTCP-shaped key material regardless of which the
session actually needs.

### 3.1 Cross-check against `transport.rs::on_dtls_handshake_complete`

The code:

```rust
let material = state.export_keying_material(
    SRTP_KEYING_MATERIAL_LABEL, &[], 2 * (key_len + salt_len),
)?;
let client_key  = &material[0..key_len];
let server_key  = &material[key_len..2*key_len];
let client_salt = &material[2*key_len..2*key_len + salt_len];
let server_salt = &material[2*key_len + salt_len..2*key_len + 2*salt_len];
```

`SRTP_KEYING_MATERIAL_LABEL = "EXTRACTOR-dtls_srtp"` and the slicing order
(`client_key, server_key, client_salt, server_salt`) is **byte-for-byte
what §4.2 specifies** — no discrepancy. The read-side selection —

```rust
let (read_key, read_salt) = if state.is_client() {
    (server_key, server_salt)   // we're the client: read what the server wrote
} else {
    (client_key, client_salt)   // we're the server: read what the client wrote
};
```

— also matches §4.2's "the server MUST only use [client_write_*] keys to
decrypt inbound traffic" / "the client MUST only use [server_write_*] keys
to decrypt inbound traffic" rule exactly.

One gap worth flagging as a **scope limitation, not a spec violation**:
`MediaTransport` only ever builds a single `SrtpContext` for **decryption**
(`srtp_read`) — there is no corresponding write-side context or
`encrypt_rtp`/`encrypt_rtcp` call anywhere in `transport.rs`. This crate,
as it stands, can receive and decrypt inbound SRTP/SRTCP but has no path to
originate protected outbound RTP/RTCP itself. Whether that is intentional
(caller does its own encoding elsewhere) or a real gap for a media-server
use case that must originate RTCP (SR/RR, etc.) is worth confirming with
whoever owns `media/` next — RFC 5764 itself gives no signal either way,
since it only defines key establishment, not a requirement that both
directions be exercised by any one component.

## 4. Key scope / usage limits (§4.3, §4.4)

- §4.3: implementations SHOULD retain multiple key sets across a rekey
  (packet reordering can deliver old-keyed packets after a new handshake
  completes).
- §4.4: `maximum_lifetime` (2^31 for every profile in the table above) is
  the max packets protectable under one key, RTP and RTCP counted
  **separately** (independent keys). At the limit, a new DTLS session
  SHOULD establish replacement keys; the old keys MUST NOT be reused for
  either direction after that.

Neither is implemented in this crate (no packet counting, no key-set
retention across rehandshake) — `MediaTransport` builds exactly one
`srtp_read` context per completed handshake and never replaces or retires
it. Not necessarily wrong for a short WHIP/WHEP session, but a caller
running a very long-lived session would hit RFC 5764's own recommended
rekey point with no code path here to act on it.

## 5. Demultiplexing one UDP flow (§5.1.2, Reception)

First byte of an inbound datagram on the shared RTP/RTCP/DTLS/STUN port:

```
+----------------+
| 127 < B < 192 -+--> forward to RTP (or RTCP, if rtcp-mux)
|                |
|  19 < B < 64  -+--> forward to DTLS
|                |
|       B < 2   -+--> forward to STUN
+----------------+
```

i.e. STUN = `{0,1}`, DTLS = `[20,63]`, RTP/RTCP = `[128,191]`. Anything
else has no defined meaning on this flow.

### 5.1 Cross-check against `transport.rs`'s demux constants

```rust
const DEMUX_STUN_MAX: u8 = 1;     // 0 or 1  -> matches "B < 2"
const DEMUX_DTLS_MIN: u8 = 20;
const DEMUX_DTLS_MAX: u8 = 63;    // matches "19 < B < 64"
const DEMUX_RTP_MIN: u8 = 128;
const DEMUX_RTP_MAX: u8 = 191;    // matches "127 < B < 192"
```

**Exact match, no discrepancy.** `handle_datagram`'s
`if first <= DEMUX_STUN_MAX { .. } else if (DTLS_MIN..=DTLS_MAX) { .. }
else if (RTP_MIN..=RTP_MAX) { .. }` (with a silent-ignore fallthrough for
anything else) reproduces Figure 3 precisely, including the "other values
have no defined meaning, so don't act on them" behaviour implied by the
figure only covering those three bands.

§5.1.2 also describes an SSRC→association mapping table for the (out of
scope here) SIP-forking case of multiple simultaneous DTLS-SRTP
associations sharing one local port; `MediaTransport` handles exactly one
peer association per instance, so that multiplexing concern does not
apply to it.

## 6. Rehandshake / rekey (§5.2)

Not implemented: `MediaTransport` has no path to re-enter the DTLS
handshake after `DtlsHandshakeComplete`, so §5.2's "SHOULD keep both key
sets around for MSL" guidance has nothing to attach to in this cut. Purely
a scope gap (there's currently one handshake per `MediaTransport` for its
whole lifetime), not a misimplementation of anything that does exist.
