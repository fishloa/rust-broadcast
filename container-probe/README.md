# container-probe

[![Crates.io](https://img.shields.io/crates/v/container-probe.svg)](https://crates.io/crates/container-probe)
[![docs.rs](https://img.shields.io/docsrs/container-probe)](https://docs.rs/container-probe)

**Media container-format detection over a byte prefix: MPEG-2 TS, ISOBMFF, Matroska/WebM, MXF, MPEG-PS, FLV, WAV, Ogg, ASF and the ADTS/MP3/Annex B elementary streams, scored by an evidence-confidence model.**

## What it does / what it does not

Give it the first bytes of a file or network read and it tells you what the
bytes are, how sure it is, or that it needs to read more. Every prober always
runs and scores its evidence; the highest score wins and a near-tie is reported
as `Ambiguous` rather than silently picked, so the verdict never depends on
declaration order. Scoring is structural, not just magic bytes: a TS lane must
sync at 50% coverage, and an elementary stream needs a frame-length chain.
It is the detector `transmux`, `media-doctor` and `multimux` use to choose a
demuxer.

It is a one-shot function over a caller-owned slice: no demuxing, no codec
identification ("this is TS", not "this TS carries H.264"), no file or network
I/O, no format conversion or repair, and no streaming state (a streaming caller
reads more and re-probes, guided by `Insufficient`).

## Standards supported

Detection needs only each format's framing, so the table lists what each prober
checks. Editions are the ones cited in the prober's module docs; where the
specification is not in the vendored `private/specs-md` set the edition is
marked unverified.

| Format | Specification (as cited in code) | Edition | Probe scope | Status | Notes |
|---|---|---|---|---|---|
| MPEG-2 TS | ISO/IEC 13818-1 §2.4; 192-byte M2TS/BDAV (Blu-ray Disc Association); ETSI EN 300 421/300 744 RS-coded 204-byte; 208-byte TP_extra_header + 204 variant | H.222.0 08/2023 (vendored) | Sync-byte lattice over strides 188/192/204/208 with phase search; 50% coverage rule | Full detection | 204-stride covered only by a marked synthetic fixture; 208-stride has no fixture. |
| ISOBMFF | ISO/IEC 14496-12 §4.2 | 2015 edition as cited (unverified, not vendored) | Top-level box-chain walk; `ftyp` major brand; fragmented vs progressive layout | Full detection | Reports `IsobmffLayout`. |
| Matroska / WebM | RFC 8794 (EBML), RFC 9559 (Matroska) | RFCs as published (unverified against a vendored copy) | EBML magic + `DocType` | Full detection | Unknown `DocType` reports Matroska with `DocType::Other`. |
| MXF | SMPTE ST 377-1 §7.1 Partition Pack | module cites ST 377-1.1:2004; the vendored copy is ST 377-1:2019 (Partition Pack unchanged for this purpose, unverified) | Partition-pack key + BER length; decodes `PartitionKind` | Full detection | |
| MPEG-PS | ISO/IEC 13818-1 §2.5.3, Table 2-39 | H.222.0 08/2023 (vendored) | Pack start code + SCR/mux-rate marker bits | Full detection | `HEURISTIC` when only the start code matches. |
| FLV | Adobe Flash Video File Format Specification, Annex E | v10.1 (vendored) | `"FLV"` + header fields; audio/video flags | Full detection | |
| WAV | Microsoft RIFF/WAVE | edition unverified | `"RIFF".."WAVE"` | Full detection | |
| Ogg | Xiph Ogg page header | edition unverified | `"OggS"` | Full detection | |
| ASF | Microsoft ASF Specification §3.1.2.1 | edition unverified | Header Object GUID | Full detection | |
| ADTS AAC | ISO/IEC 13818-7 §6.2 | edition unverified | Frame-length chain | Full detection | |
| MP3 | ISO/IEC 11172-3 §2.4.2.3 | edition unverified | Frame-header chain | Full detection | 18 239 MP3 syncwords occur inside one TS fixture; chaining, not counting, rejects them. |
| Annex B | ITU-T H.264 §7.4 / Annex B | H.264 vendored (edition per `private/specs-md/INDEX.md`) | Start-code NAL chain, 1-byte header | H.264 only | **HEVC is not detected** (2-byte NAL header; no real fixture exists, and the workspace does not implement without one). |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | Forwards `broadcast-common/std`; off gives pure `#![no_std]` + `alloc` |

The only runtime dependency is `broadcast-common`. A probe that finds no
candidate allocates nothing; a match allocates the internal candidate `Vec`
once (pinned by `tests/alloc_bound.rs`).

```toml
[dependencies]
container-probe = "0.2"
```

## Quick start

```rust
use container_probe::{probe, Format, Probe};

// Eight bytes of a TS packet are too few to conclude from: read more.
let bytes = [0x47, 0x40, 0x11, 0x10, 0x00, 0x42, 0xf0, 0x25];
match probe(&bytes) {
    Probe::Identified { format, .. } => println!("detected {}", format.name()),
    Probe::Insufficient { need_at_least, .. } => println!("need at least {need_at_least} bytes"),
    Probe::Unknown => println!("nothing matched; stop"),
    Probe::Ambiguous { candidates, .. } => println!("tied: {candidates:?}"),
    _ => {} // `#[non_exhaustive]` requires a wildcard arm.
}

// A real capture (committed fixture; path is relative to the crate root).
let data = std::fs::read("../fixtures/ts/h264_aac.ts").unwrap();
match probe(&data) {
    Probe::Identified { format, .. } => assert_eq!(format, Format::MpegTs),
    other => panic!("expected an identified TS, got {other:?}"),
}
```

### Reading until the probe can answer

`Insufficient { need_at_least }` means "read more"; `Unknown` means "stop".
`need_at_least` always exceeds the bytes *examined* (`min(len, budget)`), so a
caller that grows the buffer but leaves `probe`'s fixed `DEFAULT_BUDGET` can
stall once the budget caps the read. Feed the buffer length back as the budget:

```rust,no_run
use container_probe::{probe_with_budget, Probe, DEFAULT_BUDGET};
# fn read_at_least(_n: usize) -> Vec<u8> { Vec::new() }
let mut buf = read_at_least(DEFAULT_BUDGET);
let verdict = loop {
    match probe_with_budget(&buf, buf.len()) {
        Probe::Insufficient { need_at_least, .. } if need_at_least > buf.len() => {
            let grown = read_at_least(need_at_least);
            // EOF with nothing conclusive: stop; no further read will change it.
            if grown.len() <= buf.len() {
                break Probe::Unknown;
            }
            buf = grown;
        }
        other => break other,
    }
};
# let _ = verdict;
```

The `grown.len() <= buf.len()` arm is the caller's own termination guarantee at
EOF; the crate cannot know whether more bytes exist. `need_at_least` grows
geometrically when no structural need can be named, so the loop converges in
O(log n) reads.

## What is implemented

| Item | Purpose |
|---|---|
| `probe(&[u8]) -> Probe`, `probe_with_budget(&[u8], budget)`, `DEFAULT_BUDGET` (64 KiB) | The entry points |
| `Probe` (`Identified`, `Ambiguous`, `Insufficient`, `Unknown`; `best_confidence`, `name`) | The verdict; `#[non_exhaustive]` |
| `Format` (`MpegTs`, `Isobmff`, `MpegPs`, `Matroska`, `WebM`, `Flv`, `Mxf`, `Wav`, `Ogg`, `Asf`, `AdtsAac`, `Mp3`, `AnnexB`; `is_elementary_stream`) | What was found |
| `Confidence` tiers `CERTAIN` 240, `STRONG` 192, `STRUCTURAL` 160, `LATTICE_STRONG` 128, `LATTICE_WEAK` 96, `HEURISTIC` 64 (`as_u8`) | Evidence strength |
| `Candidate { format, confidence, detail }` | One entry of an `Ambiguous` result |
| `Detail` (`Ts { stride, phase }`, `Isobmff { major_brand, boxes_walked, layout }`, `Ebml { doc_type }`, `Flv { has_audio, has_video, data_offset }`, `Mxf { partition_kind }`, `MpegPs { structurally_valid }`, `None`) | Prober-specific facts so a demuxer need not re-derive them |
| `IsobmffLayout`, `DocType`, `PartitionKind` | Typed discriminators (no raw bytes exposed) |
| `segment::ts_structure`, `segment::media_fragment`, `segment::progressive_movie` | Is a fetched segment body structurally media: TS at any packet phase (half the 188-byte lanes synced), a complete `moof` then `mdat`, a `moov` with no `moof` |

## How detection works

Every prober is a pure function over the slice. The highest score wins; the top
two within `TIE_THRESHOLD` (16) yield `Ambiguous`, listing every candidate. A
container matched at `LATTICE_STRONG` or above zeroes every elementary-stream
candidate, because ES frames inside a container payload are expected data.

**Run length alone is not evidence.** Two real cases are pinned by tests: a
CENC-encrypted MP4 (`fixtures/mp4/cenc.mp4`) whose high-entropy payload aligned
three `0x47` bytes on one of 792 lattice lanes by chance (a TS lane must now
cover at least 50% of its positions: ~100% for real TS, ~2.5% for noise); and a
TS file containing 18 239 MP3 syncwords, 141 ADTS syncwords and 273 Annex B
start codes (each ES prober follows the frame-length field to where the next
syncword must be and counts the chain: 40+ for a real stream, 0 or 1 for a
container). A literal magic that a too-short prefix already contradicts (`XY`
for `OggS`) is "stop" for that prober, not "read more".

## Known gaps

- Annex B detection is H.264 only (see the table).
- 204-byte TS is covered only by a synthetic fixture
  (`fixtures/container-probe/PROVENANCE.md`); 208-byte has none.
- `.ts` is an ambiguous extension (TypeScript uses it): only the probe result
  is authoritative.

## Design notes

`#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`. All public enums are
`#[non_exhaustive]` with `name()` and `Display` (label convention, guarded by
`tests/label_coverage.rs`). The crate needs no spec transcription of its own:
every constant is reused from a sibling crate that round-trips real fixtures,
pinned by a dev-dependency drift guard (`mpeg-ts`, `mpeg-ps`, `st377-1`). There
is no wire format, so the workspace round-trip invariant does not apply; the
real-fixture identification tests are its equivalent gate.

## Related crates

Depends on `broadcast-common` only (dev-dependencies `mpeg-ts`, `mpeg-ps`,
`st377-1`). Used by `transmux` (CLI container detection, replacing its
fixed-offset check, issue #960), `media-doctor` and `multimux`.

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
