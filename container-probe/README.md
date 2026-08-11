# container-probe

[![Crates.io](https://img.shields.io/crates/v/container-probe.svg)](https://crates.io/crates/container-probe)
[![docs.rs](https://img.shields.io/docsrs/container-probe)](https://docs.rs/container-probe)

Robust media container-format detection over a byte prefix — the MPEG-2 TS,
ISOBMFF, Matroska/WebM, MXF, MPEG-PS, FLV, WAV, Ogg, ASF, ADTS AAC, MP3 and
Annex B probers, scored and compared by an evidence-confidence model with
cross-prober suppression.

`#![no_std]` + `alloc`; runtime dependency is `broadcast-common` only.

## What it detects

Every registered prober runs over the same bytes and returns a scored
candidate; the highest score wins.

| Format | Detected by | Confidence tier |
|---|---|---|
| MPEG-2 TS | sync lattice over 188/192/204/208-byte strides | `LATTICE_STRONG` (144) |
| ISOBMFF (`.mp4`/`.m4s`) | box-chain walk (ISO/IEC 14496-12 §4.2) | `STRUCTURAL` (160) |
| Matroska (`.mkv`) | EBML magic + `DocType == "matroska"` | `CERTAIN` (240) |
| WebM (`.webm`) | EBML magic + `DocType == "webm"` | `CERTAIN` (240) |
| MXF (`.mxf`) | partition-pack key + valid BER length | `CERTAIN` (240) |
| MPEG-PS (`.ps`/`.mpg`) | pack start code + marker bits | `STRUCTURAL` (160) |
| FLV (`.flv`) | `"FLV"` signature + header fields | `STRONG` (192) |
| WAV (`.wav`) | `"RIFF".."WAVE"` | `STRONG` (192) |
| Ogg (`.ogg`) | `"OggS"` | `STRONG` (192) |
| ASF (`.asf`/`.wmv`) | 16-byte header GUID | `STRONG` (192) |
| ADTS AAC (`.aac`/`.adts`) | frame-length chaining | `LATTICE_STRONG` (144) |
| MP3 (`.mp3`) | frame-length chaining | `LATTICE_STRONG` (144) |
| Annex B H.264/H.265 (`.h264`) | start-code NAL chaining | `LATTICE_STRONG` (144) |

## Usage

```rust
use container_probe::{probe, Format, Probe};

let bytes = [0x47, 0x40, 0x11, 0x10, 0x00, 0x42, 0xf0, 0x25]; // a TS packet head
match probe(&bytes) {
    Probe::Identified { format, .. } => println!("detected {}", format.name()),
    Probe::Insufficient { need_at_least } => println!("need at least {need_at_least} bytes"),
    Probe::Unknown => println!("nothing matched; stop"),
    Probe::Ambiguous { candidates } => println!("tied: {:?}", candidates),
}
```

To probe a real file:

```rust
use std::fs;
let data = fs::read("fixtures/ts/h264_aac.ts").unwrap();
println!("{:?}", container_probe::probe(&data));
```

## How detection works

Every prober is a pure function over the byte slice; all of them always run.
Each returns a `Confidence` tier and a `Detail`, and the highest score wins. If
the top two are within `TIE_THRESHOLD` (16) the result is `Ambiguous` listing
every candidate, never an arbitrary pick. `Insufficient { need_at_least }`
means "read more bytes"; `Unknown` means "stop, more bytes will not help".

**Run length alone is not evidence.** The test suite guards two real cases
where naive magic-byte counting fails:

- **A CENC-encrypted MP4** (`fixtures/mp4/cenc.mp4`): its high-entropy payload
  aligned three consecutive `0x47` bytes on one of 792 TS lattice lanes purely
  by chance. A run-length-only TS prober called that a match; a candidate lane
  must now *cover* at least 50% of its positions with sync bytes — a real TS
  syncs at ~100%, noise at ~2.5%.
- **A TS file with 18,239 MP3 syncwords** (`fixtures/ts/h264_aac.ts`, plus 141
  ADTS syncwords and 273 Annex B start codes): raw syncword counting would
  identify every container as an elementary stream. Each ES prober instead
  follows each frame's own length field to where the next syncword must be and
  counts how many **chain**; a real stream chains 40+, a container chains 0-1.
  And because ES frames genuinely appear inside container payloads, a container
  match at `LATTICE_STRONG` or above zeroes every elementary-stream candidate.

## Known gaps

- **204-byte-stride TS** (DVB with Reed–Solomon parity) is covered only by a
  marked synthetic fixture — `fixtures/container-probe/PROVENANCE.md` explains
  why: no real DVB Reed–Solomon capture exists in this repository.
- **208-byte stride** has **no fixture at all**.
- **`.ts` is an ambiguous extension** — TypeScript declaration files also use
  it — so a caller must not infer format from a `.ts` extension; only the probe
  result is authoritative.

## Minimum Supported Rust Version

1.95.0

## License

MIT OR Apache-2.0
