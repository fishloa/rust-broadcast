# RFC 5769 — STUN Test Vectors (transcribed)

Source: https://www.rfc-editor.org/rfc/rfc5769.txt — see `PROVENANCE.md` for
licence/retrieval details. Bytes below are copied from Appendix A ("Source
Code for Test Vectors"), the RFC's own C byte-array restatement of §2's
messages — using the C arrays rather than hand-copying the prose hex tables
avoids a transposition error going through an intermediate transcription
step.

## 2.1 / Appendix A `req[]` — Sample Request

Parameters: Software = `"STUN test client"`, Username = `"evtj:h6vY"`,
Password = `"VOkJxbRl1RmTxUk/WvJxBt"` (short-term credential — the
MESSAGE-INTEGRITY key is the raw password bytes, RFC 5389 §15.4).

```
00 01 00 58   21 12 a4 42
b7 e7 a7 01   bc 34 d6 86   fa 87 df ae
80 22 00 10   53 54 55 4e   20 74 65 73   74 20 63 6c   69 65 6e 74
00 24 00 04   6e 00 01 ff
80 29 00 08   93 2f f9 b1   51 26 3b 36
00 06 00 09   65 76 74 6a   3a 68 36 76   59 20 20 20
00 08 00 14   9a ea a7 0c   bf d8 cb 56   78 1e f2 b5   b2 d3 f2 49   c1 b5 71 a2
80 28 00 04   e5 7a 3b cf
```

Total length: 0x58 (88) + 20-byte header = 108 bytes.

## 2.2 / Appendix A `respv4[]` — Sample IPv4 Response

Parameters: Password = `"VOkJxbRl1RmTxUk/WvJxBt"`, Software =
`"test vector"`. Mapped address: `192.0.2.1:32853`.

```
01 01 00 3c   21 12 a4 42
b7 e7 a7 01   bc 34 d6 86   fa 87 df ae
80 22 00 0b   74 65 73 74   20 76 65 63   74 6f 72 20
00 20 00 08   00 01 a1 47   e1 12 a6 43
00 08 00 14   2b 91 f5 99   fd 9e 90 c3   8c 74 89 f9   2a f9 ba 53   f0 6b e7 d7
80 28 00 04   c0 7d 4c 96
```

Total length: 0x3c (60) + 20-byte header = 80 bytes.

## 2.3 / Appendix A `respv6[]` — Sample IPv6 Response

Parameters: Password = `"VOkJxbRl1RmTxUk/WvJxBt"`, Software =
`"test vector"`. Mapped address:
`[2001:db8:1234:5678:11:2233:4455:6677]:32853`.

```
01 01 00 48   21 12 a4 42
b7 e7 a7 01   bc 34 d6 86   fa 87 df ae
80 22 00 0b   74 65 73 74   20 76 65 63   74 6f 72 20
00 20 00 14   00 02 a1 47   01 13 a9 fa   a5 d3 f1 79   bc 25 f4 b5   be d2 b9 d9
00 08 00 14   a3 82 95 4e   4b e6 7b f1   17 84 c9 7c   82 92 c2 75   bf e3 ed 41
80 28 00 04   c8 fb 0b 4c
```

Total length: 0x48 (72) + 20-byte header = 92 bytes.

## 2.4 / Appendix A `reqltc[]` — Sample Request with Long-Term Authentication

Parameters: Username = `"マトリックス"` (Japanese
"matorikkusu", unaffected by SASLprep), Password before SASLprep =
`"The­MªtrⅨ"`, after SASLprep = `"TheMatrIX"`, Nonce =
`"f//499k954d6OL34oL9FSTvy64sA"`, Realm = `"example.org"`. The
MESSAGE-INTEGRITY key for long-term credentials is
`MD5(username ':' realm ':' password)` with the **post-SASLprep** password
(RFC 5389 §15.4) — i.e. `MD5("<username>:example.org:TheMatrIX")`.

```
00 01 00 60   21 12 a4 42
78 ad 34 33   c6 ad 72 c0   29 da 41 2e
00 06 00 12   e3 83 9e e3   83 88 e3 83   aa e3 83 83   e3 82 af e3   82 b9 00 00
00 15 00 1c   66 2f 2f 34   39 39 6b 39   35 34 64 36   4f 4c 33 34   6f 4c 39 46
              53 54 76 79   36 34 73 41
00 14 00 0b   65 78 61 6d   70 6c 65 2e   6f 72 67 00
00 08 00 14   f6 70 24 65   6d d6 4a 3e   02 b8 e0 71   2e 85 c9 a2   8c a8 96 66
```

Total length: 0x60 (96) + 20-byte header = 116 bytes. This message has no
FINGERPRINT attribute (per the RFC: long-term auth "is not used by ICE",
and this sample only exercises USERNAME/NONCE/REALM/MESSAGE-INTEGRITY).

These four byte arrays are what `stun_rfc5769_vectors.rs` parses with
`rtc_stun::message::Message::unmarshal_binary` — the same type
`webrtc-runtime`'s own `src/media/gather.rs` uses for its STUN Binding
transaction — then checks MESSAGE-INTEGRITY (`rtc_stun::integrity::MessageIntegrity`),
FINGERPRINT (`rtc_stun::fingerprint::FingerprintAttr`), and XOR-MAPPED-ADDRESS
(`rtc_stun::xoraddr::XorMappedAddress`) against the parameters above.
