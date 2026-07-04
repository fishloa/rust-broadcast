# MPEG-H 3D Audio — MPEG-2 TS carriage (#579)

Sources: **ETSI TS 101 154** (vendored, `dvb_a001r18_draft_ts_101_154_v02.07.01_av_coding.pdf`
+ the two newer `etsi_ts_101_154_*.pdf` revisions), §6.8 "MPEG-H Audio" (encoding/decoding
requirements) and §4.1.8.31 "MPEG-H_3dAudio_descriptor" (PMT signalling); **ATSC A/342-3**
(vendored, `atsc_a342-3_2025_mpegh_system.pdf`) §5.2 "Bit Stream Encapsulation". Both cite
**ISO/IEC 23008-3** (MPEG-H 3D Audio) and **ISO/IEC 13818-1** (MPEG-2 Systems) clauses that are
themselves paid and not vendored — where that's the boundary of what this crate can ground in
text, this doc says so explicitly and records what was instead **empirically verified** against
the real fixture, `private/fixtures/ts/mpegh-cicp01-baseline.ts` (Fraunhofer-IIS
`mpegh-test-content`, CICP_01 baseline single-stream 32 kbps; CC BY-NC-ND, private submodule).

## PMT signalling

- **`stream_type` `0x2D`** — ISO/IEC 13818-1 Table 2-34, "ISO/IEC 23008-3 Audio with MHAS
  transport syntax — single stream". TS 101 154 §4.1.6 ("Packetized Elementary Stream (PES)")
  text: *"The value of the stream_id field for MHAS formatted MPEG-H Audio packetized elementary
  streams ... MPEG-H Audio packetized elementary streams shall be 0x2D or 0x2E ... The
  stream_type value 0x2D shall be used ... for main stream ... 0x2E ... for auxiliary ...
  multi-stream delivery"*. Only `0x2D` (single/main stream) is implemented; `0x2E` (auxiliary
  multi-stream component, §6.8.7) is out of scope.
- **`MPEG-H_3dAudio_descriptor`** (§4.1.8.31) — an `ISO/IEC 13818-1` `extension_descriptor`
  (tag `0x3F`), included at most once in the ES_info loop of an MPEG-H `stream_type 0x2D`/`0x2E`
  ES. TS 101 154's own text: *"The profile and level value shall be signalled in the
  mpegh3daProfileLevelIndication field in the MPEG-H_3dAudio_descriptor() as specified in
  ISO/IEC 13818-1, clause 2.6.106"* — the full descriptor syntax is deferred wholesale to that
  (paid, unvendored) clause.
  - **Empirically observed** (the real fixture's ES_info): `3F 04 08 10 7F C1` = tag `0x3F`,
    `descriptor_length 4`, then 4 body bytes `08 10 7F C1`.
  - Byte `08` is therefore the `MPEGH_3dAudio_descriptor`'s registered
    `extension_descriptor_tag` — not printed in either vendored spec (ISO/IEC 13818-1 §2.6.106
    registers it); taken as ground truth from this real, DVB-conformant broadcast test stream
    produced by Fraunhofer, the format's own originator.
  - Byte `10` is `mpegh3daProfileLevelIndication` = `0x10` (BL Profile Level 1) — confirmed
    against the independently-recovered `mpegh3daConfig()`'s own leading byte (see below): both
    read `0x10`.
  - Bytes `7F C1` have no grounding in either vendored spec (further fields of §2.6.106); this
    crate does not reproduce them when muxing (see below).

## MHAS elementary stream framing

TS 101 154 §6.8.3: *"The MPEG-H Audio elementary streams shall be encapsulated in the MPEG-H
Audio Stream Format (MHAS) according to ISO/IEC 23008-3, clause 14"* — Clause 14 itself (the
`MHASPacketInfo()` byte/bit layout, `MHASPacketType` enumeration) is paid and not vendored.

What TS 101 154 §6.8.4.1 ("Definition of RAP with MPEG-H Audio") *does* specify — and this crate
relies on — is the **packet order at a random access point**:

> a RAP into an MPEG-H Audio Stream consists of the following MHAS packets, in the following
> order: `PACTYP_MPEGH3DACFG`; `PACTYP_AUDIOSCENEINFO` if present (directly following the config
> packet); `PACTYP_BUFFERINFO`; `PACTYP_MPEGH3DAFRAME`.

### Empirical verification against the real fixture

`transmux/src/mpegh.rs`'s MHAS walker (a three-tier "escaped value" bit-reader for
`MHASPacketType`/`Label`/`Length`) was derived and checked against the fixture's audio PID
(PID `0x20`, 29 PES access units) by an out-of-band Python decode:

- All 29 access units parse **end-to-end with zero truncation/overrun** under the scheme:
  `escapedValue(3,8,8)` → type, `escapedValue(2,8,32)` → label, `escapedValue(11,24,24)` → length
  (bit widths chosen so every combination — escaped or not — sums to a whole number of bytes,
  matching ATSC A/342-3 §4.2.3: *"any MHAS packet payload always is byte-aligned"*).
- The two random-access access units (PES index 0 and 24) decode to **exactly** the packet
  sequence `SYNC, MPEGH3DACFG, AUDIOSCENEINFO, BUFFERINFO, MARKER, MPEGH3DAFRAME` — the
  `MPEGH3DACFG → AUDIOSCENEINFO → BUFFERINFO → MPEGH3DAFRAME` sub-sequence matches §6.8.4.1
  exactly (`MARKER`/`SYNC` are permitted-but-unlisted extras per §6.8.3's "other MHAS packets may
  be present" clause).
- The other 27 access units are single `MPEGH3DAFRAME` packets; the final one carries a leading
  `AUDIOTRUNCATION` packet (permitted, §6.8.3).
- The recovered `mpegh3daConfig()` (the `MPEGH3DACFG` packet's 60-byte payload) is byte-identical
  at both RAPs, and its **leading byte is `0x10`** — agreeing with the PMT descriptor's
  `mpegh3daProfileLevelIndication` byte (also `0x10`), an independent cross-check that the packet
  boundaries this scheme finds are correct.
- A single stray byte `0xA5` (the conventional MHAS `PACTYP_SYNC` payload) sits before the first
  config packet in each RAP access unit, consistent with type `6` = `PACTYP_SYNC`.

This is the same evidentiary standard already used elsewhere in this issue's own investigation
(deriving `stream_type 0x2D` and the descriptor bytes from the same fixture) — real,
conformant broadcast bytes from the format's originator, cross-checked two independent ways —
not a guess from unverifiable memory of the paid ISO/IEC 23008-3 text.

### What this crate does **not** do

- **No MPEG-H audio bitstream decode.** `mpegh3daConfig()` is carried opaquely (identical
  posture to the existing ISOBMFF `mhaC` path in `mpegh.rs`); this crate cannot recover
  `referenceChannelLayout`, real `channel_count`, or `sample_rate` from it (those are CICP
  fields *inside* the opaque blob, ISO/IEC 23008-3 §5, paid) — `ts_demux.rs` sets them to
  documented `0`/"unspecified" placeholders rather than guess plausible-looking values. Sample
  timing is unaffected: durations are computed from the 90 kHz TS clock (PTS deltas), not an
  audio sample count.
- **No reconstruction of the two ungrounded descriptor bytes** (`7F C1` above) when muxing
  IR → TS; the emitted `MPEG-H_3dAudio_descriptor` carries only `extension_descriptor_tag` +
  `mpegh3daProfileLevelIndication`, correctly length-prefixed (a decoder skips exactly
  `descriptor_length` bytes regardless of which optional fields are present).
- **`0x2E` (auxiliary/multi-stream) not recognised** — only the single/main-stream `0x2D`
  carriage is implemented.
