# `anc_rtp.bin` provenance (issue #648)

No real ST 2110-40 (RFC 8331 ANC-over-RTP) capture exists anywhere in this
repo, and pulling a live one is not possible in this sandboxed environment
(no network access). This fixture is instead built by **reusing the
already-verified real ANC packet content bytes** from the existing,
already-audited `fixtures/st291/anc.bin` fixture (see
`st291/tests/fixture_anc.rs` for the values it asserts) and wrapping them
fresh in RFC-8331-correct RTP + payload-header framing — the parity/checksum
math over `DID`/`SDID`/`Data_Count`/`User_Data_Words`/`Checksum_Word` is
**identical across transports** per `st291/docs/anc_packet_291.md`, so the
content itself carries the same "is this really what a conformant sender
would produce" weight as the ST 2038 fixture, while the RTP-specific framing
around it (placement fields, payload header, word-alignment) is exercised
fresh.

## How it was generated

1. Took the same two ANC packets' content values already asserted by
   `st291/tests/fixture_anc.rs` against `anc.bin`:
   - Packet 0: `DID=0x161 SDID=0x101 Data_Count=0x002
     User_Data_Words=[0x2CF,0x101] Checksum_Word=0x233`
   - Packet 1: `DID=0x241 SDID=0x102 Data_Count=0x003
     User_Data_Words=[0x111,0x222,0x333] Checksum_Word=0x1AB`
2. Re-wrapped them as `st291::RtpAncPacket`s with RFC 8331 §2.1 placement
   fields matching RFC 8331 Figure 1's own worked example (two ANC packets on
   lines 9 and 10 of the SDI raster): packet 0 `C=false Line_Number=9
   Horizontal_Offset=0 S=false StreamNum=0`; packet 1 `C=true Line_Number=10
   Horizontal_Offset=0x10 S=false StreamNum=0`.
3. Built the RFC 8331 §2.1 payload (`st291::AncRtpPayload`) around them —
   `Extended_Sequence_Number=0`, `F=ProgressiveOrUnspecified (0b00)` — and
   serialized it with `st291`'s own spec-correct `Serialize` impl (which
   recomputes `Length`/`ANC_Count` and word-aligns each packet).
4. Wrapped that payload in an RFC 3550 `rtp_packet::RtpPacket` (the
   `rtp-packet` crate's spec-complete codec, issue #646): `marker=true`
   (last ANC RTP packet for the frame), `payload_type=112` (RFC 8331 §4's own
   worked SDP example: `a=rtpmap:112 smpte291/90000`), `sequence_number=1`,
   `timestamp=90_000` (1.0 s at the RFC 8331 §3.1 default 90 kHz clock rate),
   `ssrc=0x53543239` (ASCII `"ST29"`), no CSRC/extension/padding.
5. Serialized the whole packet and verified, before committing:
   - A byte-exact `parse -> serialize -> identical bytes` round trip via both
     `rtp_packet::RtpPacket` (the outer RTP header) and
     `st291::AncRtpPayload` (the inner ANC payload).
   - The decoded fields match every value listed above exactly.

The one-off generator script (`st291/examples/_gen_rtp_fixture.rs`) is not
committed — it was run once and deleted; this file documents the recipe.

## Used by

- `st291/tests/fixture_anc_rtp.rs` (`parses_expected_fields`,
  `byte_exact_round_trip`)
- `st291/examples/parse_anc_rtp.rs`
