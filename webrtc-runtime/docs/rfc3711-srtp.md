# RFC 3711 — The Secure Real-time Transport Protocol (SRTP/SRTCP)

Source: `https://www.rfc-editor.org/rfc/rfc3711.txt`, March 2004. Section
numbers below are RFC 3711's own.

This is the protocol `rtc-srtp` (an external crate this workspace consumes,
`Cargo.toml` `rtc-srtp = { version = "0.20", optional = true }`) implements
underneath [`webrtc_runtime::media::transport::MediaTransport`]'s
`srtp_read: Option<SrtpContext>` field — this crate never implements SRTP
crypto itself, it only builds the context from DTLS-exported keying material
(RFC 5764 §4.2, see `rfc5764-dtls-srtp.md`) and calls `decrypt_rtp`/
`decrypt_rtcp`. This transcription exists to check that call site's inputs
against the spec, not to re-derive the crypto (that's `rtc-srtp`'s job).

## 1. SRTP packet format (§3.1)

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+<+
|V=2|P|X|  CC   |M|     PT      |       sequence number         | |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
|                           timestamp                           | |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
|           synchronization source (SSRC) identifier            | |
+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+ |
|            contributing source (CSRC) identifiers             | |
|                               ....                            | |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
|                   RTP extension (OPTIONAL)                    | |
+>+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
| |                          payload  ...                         | |
| |                               +-------------------------------+ |
| |                               | RTP padding   | RTP pad count | |
+>+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+<+
| ~                     SRTP MKI (OPTIONAL)                       ~ |
| +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
| :                 authentication tag (RECOMMENDED)              : |
| +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
|                                                                   |
+- Encrypted Portion*                      Authenticated Portion ---+
```

The header (through CSRC + RTP extension) is RFC 3550's own RTP fixed header
— SRTP adds nothing there. What SRTP adds, appended after the RTP payload
(+ RTP padding, if any):

| Field | Length | Required? | Notes |
|---|---|---|---|
| MKI (Master Key Identifier) | configurable, whole octets | OPTIONAL | Identifies which master key derived the session key(s) for this packet. **Does not** identify the crypto context (that's `<SSRC, dest addr, dest port>`, §3.2.3). |
| Authentication tag | configurable, whole octets | RECOMMENDED | Covers RTP header + Encrypted Portion. If both encryption and authentication apply, **encryption before authentication on send, the reverse on receive** (§3.1). |

"Encrypted Portion" = the encryption of the RTP payload (+ RTP padding if
present) of the equivalent RTP packet. None of §4's pre-defined transforms
pad, so encrypted length == plaintext length for those.

## 2. Cryptographic context (§3.2)

One context per `<SSRC, destination network address, destination transport
port>` triple (§3.2.3) — a packet with no matching context MUST be
discarded. Two key kinds: **master key** (random, from key management —
here, the DTLS-SRTP exporter, §4.2) and **session key** (derived from the
master key, used directly by a transform).

Transform-independent context state (§3.2.1):

- 32-bit **ROC** (rollover counter): senders/receivers maintain this
  themselves — it counts how many times the 16-bit RTP sequence number has
  wrapped through 65535. **Not carried on the wire for SRTP** (contrast
  SRTCP, which carries its index explicitly — §3.4).
- Packet **index** `i = 2^16 * ROC + SEQ` — a 48-bit quantity.
- Receiver-only 16-bit `s_l` (highest received sequence number so far).
- Encryption/authentication algorithm identifiers.
- Receiver-only Replay List (§3.3.2).
- MKI indicator (0/1) + MKI length if set.
- Master key(s) + a per-master-key sent-packet counter.
- `n_e`, `n_a` — session key lengths for encryption/authentication.

Optional per-master-key state: **master salt** (RECOMMENDED, may be
public), **key_derivation_rate** (a power of 2 in `{1,2,4,...,2^24}`,
unspecified ⇒ treated as 0), MKI value, `<From,To>` validity range.

**SRTCP shares the SRTP crypto context by default**, except:

- no ROC/`s_l` (SRTCP's index is explicit in the packet, §3.4);
- a **separate** Replay List;
- a **separate** sent-packet counter for the master key, even when the
  master key itself is shared with SRTP.

Master key(s) MAY be shared between SRTP and SRTCP; **session keys MUST
NOT** be. §4.3.2 below is the mechanism that keeps the two session-key sets
distinct despite the shared master key/salt.

## 3. Packet index (§3.3.1) — SRTP only

- Sender: ROC starts at 0; incremented by 1 (mod 2^32) every time SEQ wraps
  mod 2^16. `i = 2^16*ROC + SEQ`.
- Receiver: estimates `i = 2^16*v + SEQ` where `v` is chosen from
  `{ROC-1, ROC, ROC+1}` (mod 2^32) so that `i` is closest, mod 2^48, to
  `2^16*ROC + s_l`. Update rule after authenticating: `v=ROC-1` ⇒ no
  change; `v=ROC` ⇒ `s_l = max(s_l, SEQ)`; `v=ROC+1` ⇒ `s_l = SEQ, ROC = v`.
- ROC is **never reset** on re-key.
- **Absolute cap**: 2^48 packets under one key (ROC is 32 bits × SEQ 16
  bits) — the sender MUST NOT send more once hit; re-keying (§8.1) MUST
  happen before this limit.

## 4. Replay protection (§3.3.2)

- Only meaningful with authentication.
- Sliding-window Replay List of received+authenticated indices.
  **`SRTP-WINDOW-SIZE` is receiver-side, implementation-defined, and MUST
  be >= 64** (MAY be larger). This is a **stated minimum, not a formula** —
  the RFC does not tell an implementer how to pick a larger value.
- Accept rule: index ahead of the window, OR inside the window but not yet
  seen. Everything else (behind the window, or already-seen inside it) is
  rejected as a replay.

## 5. Secure RTCP (§3.4)

SRTCP = RTCP compound packet (RFC 3550, MUST start with SR or RR) + three
mandatory added fields + one optional field, appended after any other
profile-specific extensions:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+<+
|V=2|P|    RC   |   PT=SR or RR   |             length          | |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
|                         SSRC of sender                        | |
+>+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+ |
| ~                          sender info                          ~ |
| +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
| ~                         report block(s)                       ~ |
| +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
| |                    [other RTCP packets...]                    | |
+>+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+=+ |
| |E|                         SRTCP index                         | |
| +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+<+
| ~                     SRTCP MKI (OPTIONAL)                      ~ |
| +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
| :                     authentication tag                        : |
| +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+ |
|                                                                   |
+-- Encrypted Portion                    Authenticated Portion -----+
```

| Field | Width | Required? | Notes |
|---|---|---|---|
| E-flag | 1 bit | REQUIRED | `1` = this SRTCP packet is encrypted, `0` = not. RFC 3550 §9.1 permits splitting a compound RTCP packet into an encrypted half and a cleartext half. |
| SRTCP index | 31 bits | REQUIRED | **Explicit** counter (unlike SRTP's implicit ROC||SEQ). MUST start at 0 before the first SRTCP packet, MUST increment by 1 mod 2^31 after each packet sent. **Never reset on re-key.** |
| Authentication tag | configurable | REQUIRED | Message authentication is mandatory for SRTCP (it carries BYE and other control that must not be spoofed). |
| MKI | configurable | OPTIONAL | Same semantics as SRTP's MKI. |

Encrypted Portion = encryption of the RTCP payload of the equivalent
compound packet, **from the 9th octet to the end** (i.e. everything after
the first packet's fixed SR/RR header + SSRC, which stays in the clear so
demuxers/monitors can always read the SSRC). Authenticated Portion = the
*entire* equivalent RTCP packet + E-flag + SRTCP index, computed **after**
encryption is applied to the payload.

The RTCP encryption prefix of RFC 3550 §6.1 (a random 32-bit quantity) is
explicitly **not used** by SRTCP — it existed for RFC 3550's own, different,
encryption method.

SRTCP-specific processing deltas from SRTP (§3.4 bullets):

- No index estimation needed — it's on the wire.
- Encryption/auth transform defaults to whatever protects the associated
  SRTP stream, but **MAY differ** (e.g. SRTCP NULL-encrypted while SRTP
  isn't) — the E-flag and this per-packet choice is how that's expressed.
  NULL cipher applies to any RTCP packet whose E-flag is 0.
  Decryption only runs if E==1.
- Separate Replay List, keyed by the SRTCP index instead of the SRTP i.
- Sender-only: increment the SRTCP index (mod 2^31) and check the
  packet-count limit (§9.2) after each send.
- `avg_rtcp_size` (RFC 3550 §6.3) accounting MUST include the SRTCP-added
  fields (index/E-bit/tag/MKI) both at init and per-update, so SRTCP
  doesn't silently blow its RTCP bandwidth share; the net effect if this
  is honoured is simply longer intervals between RTCP packets, not more
  bandwidth used.

## 6. Pre-defined transforms (§4)

### 6.1 Encryption (§4.1)

Two non-NULL transforms defined: **AES-CM** (Segmented Integer Counter
Mode, §4.1.1) and **AES-f8** (§4.1.2). Keystream generation maps
`(SRTP packet index, secret key)` → pseudorandom keystream; ciphertext =
plaintext XOR keystream (identical operation both directions).

**AES-CM** (§4.1.1) — the default (see §5.1):

```
IV = (k_s * 2^16) XOR (SSRC * 2^64) XOR (i * 2^16)
```
all three terms zero-padded to 128 bits before the XOR. Keystream segment =
`E(k_e, IV) || E(k_e, IV+1 mod 2^128) || E(k_e, IV+2 mod 2^128) || ...`.
For SRTCP: SSRC = the first header's SSRC in the compound packet, `i` = the
31-bit SRTCP index, `k_e`/`k_s` = the *SRTCP* session encryption key/salt.

Constraint: no more than 2^16 AES blocks may be generated per fixed IV
(bounds the max packet size the counter-mode construction can safely
cover — 2^16 blocks × 128 bits = 2^23 bits, i.e. enough for any RTP packet
short of an IPv6 jumbogram). For a given key, the `(ROC||SEQ, SSRC)` pair
used to build IV **MUST be distinct** — reuse is a "two-time pad" failure
(§9.1).

**AES-f8** (§4.1.2) — optional (§5, Table 1). Variant of OFB. Key mask
`m = k_s || 0x555...5` (salt padded out to `n_e` bits with the `0101...`
pattern). `IV' = E(k_e XOR m, IV)`, `S(-1)=0`, then
`S(j) = E(k_e, IV' XOR j XOR S(j-1))` for `j = 0..L-1`. Sender SHOULD NOT
generate more than 2^32 blocks per key (soft bound, not an absolute
security threshold the way AES-CM's 2^16 is).

f8 SRTP IV (§4.1.2.2, "implicit header authentication"):
```
IV = 0x00 || M || PT || SEQ || TS || SSRC || ROC
```
(`M`, `PT`, `SEQ`, `TS`, `SSRC` from the RTP header; `ROC` from the context.)

f8 SRTCP IV (§4.1.2.3):
```
IV = 0..0 || E || SRTCP index || V || P || RC || PT || length || SSRC
```
(`V,P,RC,PT,length,SSRC` from the first header of the RTCP compound packet;
`E`/index are the appended SRTCP fields.)

**NULL cipher** (§4.1.3) — mandatory-to-implement; ciphertext = plaintext
verbatim.

### 6.2 Authentication (§4.2) — HMAC-SHA1 (§4.2.1), the sole pre-defined
transform:

- `M` (the data authenticated) = Authenticated Portion || ROC for SRTP;
  Authenticated Portion alone for SRTCP (the SRTCP index already includes
  the analogous role ROC plays for SRTP, so it isn't appended again).
- `SRTP_PREFIX_LENGTH = 0` for HMAC-SHA1.
- Tag = `HMAC(k_a, M)` truncated to the left-most `n_tag` bits.

### 6.3 Key derivation (§4.3)

```
r = index DIV key_derivation_rate         (DIV = integer division, a DIV 0 := 0)
key_id = <label> || r
x = key_id XOR master_salt                (right-aligned)
session_key/salt = PRF_n(k_master, x)
```

`index` = the 48-bit `ROC||SEQ` for SRTP. `<label>` is an 8-bit constant,
unique per derived-key kind; §4.3.1 reserves `0x00`–`0x05` for the six
kinds below, `0x06`–`0xff` for future use:

| Label | Key | `n` |
|---|---|---|
| `0x00` | SRTP session encryption key `k_e` | `n_e` |
| `0x01` | SRTP session authentication key `k_a` | `n_a` |
| `0x02` | SRTP session salting key `k_s` | `n_s` |
| `0x03` | SRTCP session encryption key | (SRTCP `n_e`) |
| `0x04` | SRTCP session authentication key | (SRTCP `n_a`) |
| `0x05` | SRTCP session salting key | (SRTCP `n_s`) |

Labels `0x03`–`0x05` are §4.3.2's **SRTCP key derivation**: same formula,
but `index` is replaced by the 32-bit quantity `0 || SRTCP index` (the
E-bit position is fixed to 0), and the three SRTCP-specific labels are used
instead of `0x00`–`0x02` — this is exactly the "shared master key, distinct
session keys" mechanism §3.2.1 promises.

At least one key-derivation invocation is REQUIRED before the first
packet. If `key_derivation_rate = 0` (the default, §5.3), derivation
happens **exactly once** — no periodic re-derivation.

**AES-CM PRF** (§4.3.3, the default, §5.3): `PRF_n(k_master, x)` = AES-CM
(§4.1.1) keyed by `k_master`, `IV = x * 2^16`, output truncated to the
first (left-most) `n` bits. `m = 128` (input block size); can produce
outputs up to `n = 2^23` bits.

## 7. Default / mandatory-to-implement transforms (§5, Table 1)

| Function | Mandatory-to-implement | Optional | Default |
|---|---|---|---|
| Encryption | AES-CM, NULL | AES-f8 | AES-CM |
| Message integrity | HMAC-SHA1 | — | HMAC-SHA1 |
| Key derivation (PRF) | AES-CM | — | AES-CM |

Default sizes (§5.1–5.3), all in bits:

| Parameter | Default |
|---|---|
| `n_e` (session encryption key) | 128 |
| `n_s` (session salt) | 112 |
| `n_a` (session auth key, HMAC-SHA1) | 160 |
| `n_tag` (auth tag, HMAC-SHA1) | 80 |
| `SRTP_PREFIX_LENGTH` (HMAC-SHA1) | 0 |
| Master key length (AES-CM PRF) | 128 |
| Master salt length | 112 |
| `key_derivation_rate` | 0 |

§5.2 also states: for SRTCP, HMAC-SHA1 **MUST NOT** be used with `n_tag` or
`n_a` smaller than these defaults; for SRTP, smaller values are merely NOT
RECOMMENDED.

## 8. What this crate actually touches

`MediaTransport` (`webrtc-runtime/src/media/transport.rs`) never implements
any of the above itself — it:

1. Picks the *offered* protection profile in the DTLS handshake
   (`OFFERED_SRTP_PROFILE = SRTP_AES128_CM_HMAC_SHA1_80`, i.e. §5's
   defaults: AES-CM / HMAC-SHA1 / 80-bit tag — see `rfc5764-dtls-srtp.md`).
2. Exports keying material and hands `(read_key, read_salt)` +
   `ProtectionProfile` to `rtc_srtp::context::Context::new(..)`, which is
   where all of §3–§4 above actually executes.
3. Calls `ctx.decrypt_rtp`/`ctx.decrypt_rtcp` per inbound datagram and
   trusts the result.

Everything about ROC maintenance, the replay window, ordering of decrypt-
vs-verify, ROC/`s_l` update, key derivation, and IV formation lives inside
`rtc-srtp` (an external crate, not vendored in this repo) — this
transcription is the reference for auditing *that* crate's behaviour or for
writing this crate's own SRTP stack later, not a description of code that
exists in `webrtc-runtime/src/media` today.

**No discrepancy found** between `MediaTransport`'s use of the exported
keying material and RFC 3711 — the crate does not implement enough of SRTP
itself (by design — it delegates to `rtc-srtp`) for there to be a
divergence to report at this layer. See `rfc5764-dtls-srtp.md` for the one
piece this crate *does* implement directly (the key-export byte layout),
which was checked byte-for-byte against RFC 5764 §4.2 and matches.
