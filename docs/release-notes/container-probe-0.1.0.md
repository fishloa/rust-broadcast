# container-probe 0.1.0

_Released 2026-08-12._

First release. `container-probe` identifies the container format of a byte
prefix — a file's leading bytes, a network read — and reports what it learned
getting there.

`no_std` + `alloc`. Its only runtime dependency is `broadcast-common`.

## Why it exists

`transmux::cli::detect_container` was the workspace's format detection: about
twenty lines of first-match-wins magic-byte comparison, behind the `cli`
feature (so it pulled `clap`), returning a CLI-specific error type. It was
wrong in ways that matter:

- **Every M2TS file failed** — it checked the sync byte at offset 0 and 188
  only, so the 192-byte BDAV stride was invisible.
- **Every DVB 204-byte-stride file failed**, for the same reason.
- **Every capture not starting on a packet boundary failed** — no phase search.
- **First match won** — a file whose first three bytes were `FLV` returned FLV
  with no further checking, and no way for a caller to learn a stronger
  candidate existed.
- **No elementary streams and no MXF**, despite `st377-1` sitting in the same
  workspace with the partition keys.

## What it does

Thirteen formats: MPEG-2 TS, ISOBMFF, Matroska, WebM, MPEG-PS, FLV, MXF, WAV,
Ogg, ASF, and the ADTS AAC / MP3 / Annex B elementary streams.

**Every prober always runs and scores its evidence** — `CERTAIN`, `STRONG`,
`STRUCTURAL`, `LATTICE_STRONG`, `LATTICE_WEAK`, `HEURISTIC`. The highest score
wins, and two candidates within `TIE_THRESHOLD` return `Ambiguous` rather than
an arbitrary pick, so a verdict never depends on prober declaration order.

`Detail` carries what each prober measured — TS stride and phase, ISOBMFF major
brand and `IsobmffLayout`, EBML `DocType` — so a consumer picking a demuxer
does not re-derive any of it.

`Insufficient { need_at_least }` and `Unknown` are deliberately distinct:
the first tells a streaming caller to read more, the second tells it to stop.

## Scoring is structural, not magic-byte

Two real cases forced this, and both are pinned by regression tests:

- **A lane needs 50% sync coverage, not merely three bytes in a row.**
  `fixtures/mp4/cenc.mp4` is a CENC-encrypted MP4; across the 792 candidate
  lanes, three consecutive `0x47` bytes aligned by chance and it was
  confidently identified as MPEG-TS. A real transport stream syncs at ~100% of
  its lane positions; random noise at ~2.5%.
- **An elementary stream needs a frame-length chain.** `fixtures/ts/h264_aac.ts`
  contains **18,239 MP3 syncwords** — 34× the count in the actual MP3 fixture.
  Counting syncwords identifies every container as an elementary stream.
  Following each frame header's own length field to the next expected syncword
  separates them cleanly: real streams chain 44–48 frames, containers chain 0
  or 1.

`IsobmffLayout` claims `Progressive` only after a complete walk of the supplied
buffer. Every fragmented file opens with a `ftyp` + `moov` init segment and
reaches its first `moof` later, so a truncated prefix of a fragmented file is
indistinguishable in shape from a progressive one.

## Verification

- **Corpus sweep** over every media file in the repository: 127 scanned, 121
  identified, 6 allowlisted as genuinely too small, **0 missed, 0 false
  positives, 0 ambiguous**. The sweep fails on a wrong format, an `Ambiguous`,
  or any unexplained non-identification.
- **Mutation proofs** with verbatim observed failures for every discriminator:
  TS phase search, TS stride set, the coverage threshold, the ISOBMFF box
  chain, EBML `DocType`, MXF BER length, MPEG-PS marker bits, ADTS/MP3 chain
  thresholds, the Annex B forbidden-zero-bit check, the ID3v2 skip, and
  elementary-stream suppression.
- **Constant drift guards** in both directions — that upstream `mpeg-ts`,
  `mpeg-ps` and `st377-1` still declare what this crate was written against,
  and that this crate's own copies have not drifted from them.
- **Fuzz target** (`fuzz/fuzz_targets/container_probe.rs`) — several probers
  walk attacker-influenced length fields.

## Known gaps, stated rather than hidden

- **204-byte-stride TS** is covered only by a clearly-marked synthetic fixture.
  No real 204-stride capture exists in this repository; all 36 real `.ts`
  captures are 188-stride, and genuine 204 (DVB Reed–Solomon) comes only from
  modulator hardware.
- **208-byte stride** has no fixture at all.
- **`.ts` is an ambiguous extension** — TypeScript declaration files use it
  too, so callers must not infer format from extension.

Published from tag `container-probe-v0.1.0`.
