# RFC 3711 Appendix B — Test Vectors (transcribed)

Source: https://www.rfc-editor.org/rfc/rfc3711.txt — see `PROVENANCE.md` for
licence/retrieval details. All values are hexadecimal, transcribed verbatim
from the fetched plain-text RFC (no OCR involved — the RFC is plain ASCII
text). Byte spacing added only for readability; the underlying hex digits are
unchanged from the source.

## B.2 — AES-CM Test Vectors

```
Keystream segment length: 1044512 octets (65282 AES blocks)
Session Key:      2B7E151628AED2A6ABF7158809CF4F3C
Rollover Counter: 00000000
Sequence Number:  0000
SSRC:             00000000
Session Salt:     F0F1F2F3F4F5F6F7F8F9FAFBFCFD0000 (already shifted)
Offset:           F0F1F2F3F4F5F6F7F8F9FAFBFCFD0000

Counter                            Keystream

F0F1F2F3F4F5F6F7F8F9FAFBFCFD0000   E03EAD0935C95E80E166B16DD92B4EB4
F0F1F2F3F4F5F6F7F8F9FAFBFCFD0001   D23513162B02D0F72A43A2FE4A5F97AB
F0F1F2F3F4F5F6F7F8F9FAFBFCFD0002   41E95B3BB0A2E8DD477901E4FCA894C0
...                                ...
F0F1F2F3F4F5F6F7F8F9FAFBFCFDFEFF   EC8CDF7398607CB0F2D21675EA9EA1E4
F0F1F2F3F4F5F6F7F8F9FAFBFCFDFF00   362B7C3C6773516318A077D7FC5073AE
F0F1F2F3F4F5F6F7F8F9FAFBFCFDFF01   6A2CC3787889374FBEB4C81B17BA6C44
```

Note (RFC's own): "this test case is contrived so that the latter part of
the keystream segment coincides with the test case in Section F.5.1 of
[CTR]." — i.e. `Session Key` here is literally the NIST SP 800-38A /
FIPS-197 AES-128 example key `2B7E151628AED2A6ABF7158809CF4F3C`.

Since AES-CM's keystream is AES-ECB-encrypting successive 16-byte counter
blocks with the session key (RFC 3711 §4.1.1), each `Counter -> Keystream`
row above is directly `AES-128-ECB-encrypt(Session Key, Counter) ==
Keystream` — no session/master-key derivation involved. This is the
vector `srtp_rfc3711_vectors.rs` uses for `aes_cm_keystream_matches_rfc`.

## B.3 — Key Derivation Test Vectors

Master key and master salt (SRTP-side, AES-128 default KDF, key derivation
rate `kdr = 0`):

```
master key:  E1F97A0D3E018BE0D64FA32C06DE4139
master salt: 0EC675AD498AFEEBB6960B3AABE6
```

Cipher key (label `0x00`, `LABEL_SRTP_ENCRYPTION`):

```
index DIV kdr:                 000000000000
label:                       00
master salt:   0EC675AD498AFEEBB6960B3AABE6
-----------------------------------------------
xor:           0EC675AD498AFEEBB6960B3AABE6     (x, PRF input)

x*2^16:        0EC675AD498AFEEBB6960B3AABE60000 (AES-CM input)

cipher key:    C61E7A93744F39EE10734AFE3FF7A087 (AES-CM output)
```

Cipher salt (label `0x02`, `LABEL_SRTP_SALT`):

```
index DIV kdr:                 000000000000
label:                       02
master salt:   0EC675AD498AFEEBB6960B3AABE6

----------------------------------------------
xor:           0EC675AD498AFEE9B6960B3AABE6     (x, PRF input)

x*2^16:        0EC675AD498AFEE9B6960B3AABE60000 (AES-CM input)

               30CBBC08863D8C85D49DB34A9AE17AC6 (AES-CM ouptut)

cipher salt:   30CBBC08863D8C85D49DB34A9AE1
```

(Note: the RFC's own worked example above truncates the 16-byte AES-CM
output to the 14-byte cipher salt — `30CBBC08863D8C85D49DB34A9AE1` is the
first 14 bytes of `30CBBC08863D8C85D49DB34A9AE17AC6`.)

Auth key (label `0x01`, `LABEL_SRTP_AUTHENTICATION_TAG`, 20 bytes for the
HMAC-SHA1-80 auth key length):

```
index DIV kdr:                   000000000000
label:                         01
master salt:     0EC675AD498AFEEBB6960B3AABE6
-----------------------------------------------
xor:             0EC675AD498AFEEAB6960B3AABE6     (x, PRF input)

x*2^16:          0EC675AD498AFEEAB6960B3AABE60000 (AES-CM input)

auth key                           AES input blocks
CEBE321F6FF7716B6FD4AB49AF256A15   0EC675AD498AFEEAB6960B3AABE60000
6D38BAA48F0A0ACF3C34E2359E6CDBCE   0EC675AD498AFEEAB6960B3AABE60001
E049646C43D9327AD175578EF7227098   0EC675AD498AFEEAB6960B3AABE60002
6371C10C9A369AC2F94A8C5FBCDDDC25   0EC675AD498AFEEAB6960B3AABE60003
6D6E919A48B610EF17C2041E47403576   0EC675AD498AFEEAB6960B3AABE60004
6B68642C59BBFC2F34DB60DBDFB2       0EC675AD498AFEEAB6960B3AABE60005
```

The RFC's own prose calls this a "94-octet session authentication key" —
and the table above genuinely is 94 bytes (five full 16-byte AES-CM output
blocks = 80 bytes, plus a sixth row truncated to 14 bytes = 94 total). This
is *not* the auth key length any real profile uses; it is the RFC choosing
a longer illustrative length to demonstrate that the KDF can extend past
one AES block. The AES-128-CM-HMAC-SHA1-80 profile this project's
`rtc_srtp` dependency implements only consumes the leading 20 bytes of it
(`ProtectionProfile::Aes128CmHmacSha1_80.auth_key_len() == 20`, matching
HMAC-SHA1's 20-byte key/output size) — the first 16 bytes of the first
AES-CM output block (`CEBE321F6FF7716B6FD4AB49AF256A15`) concatenated with
the first 4 bytes of the second block (`6D38BAA4`):

```
auth key (20 bytes, HMAC-SHA1-80 profile):
  CEBE321F6FF7716B6FD4AB49AF256A15 6D38BAA4
```

This is the vector `srtp_rfc3711_vectors.rs` uses for
`key_derivation_matches_rfc_appendix_b3` (cipher key, cipher salt, and this
20-byte auth key) and for `srtp_context_reproduces_appendix_b3_ciphertext`
(building an `rtc_srtp::context::Context` keyed with the master
key/salt above and cross-checking its `encrypt_rtp`/`decrypt_rtp` output
against an independent AES-CTR + HMAC-SHA1 computation using these same
derived values).

### SRTCP labels (not given a numeric example in the RFC)

The RFC defines the label byte for every one of the six derived values in
§4.3.2 (encryption `0x00`, MSG-auth `0x01`, salt `0x02` for SRTP; `0x03`,
`0x04`, `0x05` respectively for SRTCP), but **Appendix B.3 only walks
through the three SRTP-side labels above** — it gives no published numeric
cipher-key/salt/auth-key output for the SRTCP labels. Tests that need an
SRTCP session key derive it themselves (same formula, label `0x03`/`0x04`/
`0x05`, independently implemented via the `aes` crate) and can only assert
*internal* consistency (two independently-written implementations of the
published formula agree) — not "matches an RFC-published number", because
no such number exists for the SRTCP labels. Documented here so this
distinction isn't lost: see `srtp_rfc3711_vectors.rs`'s SRTCP test comments.
