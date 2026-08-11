# `container-probe` — design

**Date:** 2026-08-11
**Status:** approved
**Relation:** sub-project 1 of two. Sub-project 2 is
[linear channel playout (#748)](2026-08-11-linear-playout-design.md), whose
`FileReader` consumes this crate. Build this first.

Robust, fast container-format detection over a byte prefix. A new workspace
crate: `no_std` + `alloc`, depending only on `broadcast-common`, independently
versioned from 0.1.0.

## Why this exists

`transmux::cli::detect_container` is the current implementation. It is ~20 lines
of first-match-wins magic-byte comparison behind the `cli` feature (which pulls
`clap`), returning a CLI-specific error type. It is wrong in ways that matter to
a file-based playout source:

- **MPEG-2 TS**: checks the sync byte at offset 0 and 188 only. Every M2TS/BDAV
  file (192-byte stride, 4-byte timestamp prefix) fails. Every DVB file with
  Reed-Solomon parity (204) fails. Every capture that does not begin exactly on
  a packet boundary fails. Two syncs is weak evidence — `0x47` is `'G'`.
- **First-match-wins**: a file whose first three bytes happen to be `FLV`
  returns FLV with no further checking, and no way for a caller to learn a
  stronger candidate existed.
- **ISOBMFF**: a fourcc at offset 4, with no box-chain validation and no brand
  inspection.
- **No elementary streams**: raw ADTS AAC, MP3, and AnnexB H.264/H.265 — all
  ordinary playout assets — are unrecognized.
- **No MXF**, despite `st377-1` being in this workspace with the partition-pack
  key constants already in it.

Container identification is primary broadcast logic, used by `multimux`'s file
ingest, `media-doctor`, `ts-fix`, and `transmux`'s own CLI. It deserves a crate.

## Verified sources — no new transcription needed

This workspace's rule is no implementation without a truthful source. This crate
needs no new spec transcription, because every constant and structural rule it
depends on already exists in a workspace crate that round-trips **real
fixtures**:

| Format | In-tree source |
|---|---|
| MPEG-2 TS | `mpeg-ts` — `TS_SYNC_BYTE`, `TS_PACKET_SIZE` |
| ISOBMFF | `transmux/src/box_types.rs` — box header layout, size-0/size-1 handling |
| Matroska/WebM | `transmux/docs/webm/` — EBML transcription |
| MXF | `st377-1/src/partition.rs`, `src/klv.rs` — partition-pack UL prefix, KLV BER |
| MPEG-PS | `mpeg-ps` — `pack_start_code` (`0x000001BA`) + marker-bit validation |
| FLV | `private/specs/adobe_flv_f4v_v10_1.pdf` |

Reusing values already validated against real data — rather than re-deriving
them — is what removes the fabrication risk. A drift-guard test pins them (see
Testing).

## Global constraints

- MSRV **1.95.0**; build and test with `--locked`.
- `#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`. Must build for
  `thumbv7em-none-eabi`.
- Runtime dependencies: `broadcast-common` only.
- Public enums get `name()` + `broadcast_common::impl_spec_display!` (the #204
  convention), and `#[non_exhaustive]`.
- No magic numbers outside `#[cfg(test)]` — every literal is a named constant.
- Module docs cite the spec (or the in-tree crate) each layout comes from.
- Clears `docs/CRATE-ACCEPTANCE.md` in full, with one documented deviation (see
  Testing).

## API

One-shot over a byte slice. The caller already holds bytes — a file prefix, a
network read — so the probe owns no buffer and no state.

```rust
/// The identified container/stream format.
#[non_exhaustive]
pub enum Format {
    MpegTs, Isobmff, MpegPs, Matroska, WebM, Flv, Mxf,
    Wav, Ogg, Asf,
    AdtsAac, Mp3, AnnexB,
}

/// Evidence strength behind a match, in named tiers (see "Confidence model").
pub struct Confidence(u8);

/// What a prober learned on the way to its conclusion — the difference between
/// "it is TS" and "it is TS, 192-byte stride, first sync at offset 7".
#[non_exhaustive]
pub enum Detail {
    Ts { stride: u16, phase: u16 },
    Isobmff { major_brand: Option<[u8; 4]>, boxes_walked: u8 },
    Ebml { doc_type: DocType },
    None,
}

/// One scored candidate.
pub struct Candidate {
    pub format: Format,
    pub confidence: Confidence,
    pub detail: Detail,
}

/// What the probe concluded.
#[non_exhaustive]
pub enum Probe {
    /// A single best match.
    Identified { format: Format, confidence: Confidence, detail: Detail },
    /// Two or more candidates within `TIE_THRESHOLD`, ordered by score.
    Ambiguous { candidates: Vec<Candidate> },
    /// Nothing matched, but more bytes could change that.
    Insufficient { need_at_least: usize },
    /// Nothing matched and more bytes will not help.
    Unknown,
}

/// Probe with the default budget (`DEFAULT_BUDGET`).
pub fn probe(data: &[u8]) -> Probe;

/// Probe reading at most `budget` bytes.
pub fn probe_with_budget(data: &[u8], budget: usize) -> Probe;
```

`Insufficient` and `Unknown` are deliberately distinct: the first tells a
streaming caller to read more, the second tells it to stop.

## Confidence model

Every prober returns `Option<Match { confidence, detail }>`. **All probers
always run** over the same buffer; the highest score wins.

| Tier constant | Value | Evidence | Example |
|---|---|---|---|
| `CERTAIN` | 240 | Unambiguous magic at offset 0 **plus** a structural check confirming it | EBML magic + valid DocType; MXF partition key + well-formed BER length |
| `STRONG` | 192 | Unambiguous magic at a defined offset, no further validation available | FLV signature + version; `RIFF`…`WAVE`; `OggS`; ASF header GUID |
| `STRUCTURAL` | 160 | A validated structure chain, not merely a signature | ISOBMFF: >=2 top-level boxes whose sizes chain exactly to the buffer end or a clean truncation; MPEG-PS pack header with valid marker bits |
| `LATTICE_STRONG` | 144 | A repeating sync lattice with many confirmations | TS: >=8 consecutive syncs at a consistent stride |
| `LATTICE_WEAK` | 96 | A repeating lattice with few confirmations | TS: 3-7 syncs; ADTS: >=4 frames chaining by their own length fields |
| `HEURISTIC` | 64 | A signature with meaningful false-positive probability | Bare MPEG-PS pack start code; bare AnnexB start code |

**Ties.** If the top two scores are within `TIE_THRESHOLD` (16), the result is
`Ambiguous` carrying both, ordered by score. A caller that wants a decision
takes the first; a caller that wants correctness refuses. Silently picking one
is precisely what today's implementation does, and is the failure being fixed.

**Why all probers always run.** Ordering bias is a real bug class here — a stray
`0x47` lattice inside an `mdat`, an ADTS syncword inside a TS payload. Scoring
every prober makes the outcome independent of declaration order and makes a
wrong answer debuggable: the losing candidates and their scores are inspectable
rather than never computed.

**Cross-prober suppression.** One explicit exception to pure scoring: a
container match at `LATTICE_STRONG` or above zeroes every elementary-stream
candidate. ADTS frames inside a TS payload are expected, not evidence the file
is raw AAC. The rule is one-directional and named, rather than an implicit
ordering that a future edit could silently reverse.

## The probers

One module per format. Each is a pure function
`fn probe(data: &[u8]) -> Option<Match>` — no state, no allocation in the scan.

### MPEG-2 TS — the one needing real work

A lattice search rather than fixed-offset checks. For each candidate stride and
each phase offset in `0..stride` (bounded by the budget), count consecutive
`TS_SYNC_BYTE` at `phase + n*stride`. The `(stride, phase)` pair with the most
confirmations wins.

- Strides: **188** (ISO/IEC 13818-1), **192** (M2TS/BDAV — a 4-byte timestamp
  prefix per packet), **204** (DVB with Reed-Solomon parity), **208**.
- `>=8` confirmations scores `LATTICE_STRONG`; 3-7 scores `LATTICE_WEAK`; below
  3 is no match.
- Reports `Detail::Ts { stride, phase }`, so a demuxer does not re-derive them.

The phase scan is what handles a capture beginning mid-packet — the case that
fails outright today. A cheap precondition (no `0x47` anywhere in the first
stride window) skips the phase loop entirely for non-TS input.

### ISOBMFF

Walk the top-level box chain: read `u32` size + 4-byte type; handle size 0
(extends to end of file) and size 1 (a 64-bit `largesize` follows). Validate
each size is `>= 8` and does not overflow the buffer, then step to the next box.

- Must start with a recognized leading fourcc: `ftyp`, `styp`, `moov`, `moof`,
  `skip`, `free`, `mdat`.
- `>=2` boxes chaining cleanly scores `STRUCTURAL`.
- When `ftyp` is present, its major brand goes into
  `Detail::Isobmff { major_brand, boxes_walked }`.

Header layout follows `transmux/src/box_types.rs`.

### Matroska / WebM

EBML magic `1A 45 DF A3`, then read the EBML header's element chain to locate
`DocType` and read its string. `"webm"` and `"matroska"` map to the two
`Format`s; both report `Detail::Ebml { doc_type }`. Magic **plus** a valid
DocType is `CERTAIN`; magic alone with an unreadable header is `STRONG`.

### MXF

The partition-pack KLV key (16-byte UL, prefix `06 0E 2B 34 02 05 01`) plus a
well-formed BER length. Constants from `st377-1`. `CERTAIN` when both hold.

### MPEG-PS

Pack start code `00 00 01 BA` at offset 0, then validate the pack header's
marker bits and, where present, a following start code. Marker-bit validation
lifts this to `STRUCTURAL`; the bare 4-byte start code alone is `HEURISTIC`.

### FLV

`"FLV"` + version byte + reserved-bit check + header-size field. `STRONG`.

### RIFF/WAV, Ogg, ASF

`"RIFF"`…`"WAVE"`, `"OggS"`, and ASF's 16-byte header GUID. All `STRONG` on
magic.

### Elementary streams

- **ADTS AAC** — syncword `0xFFF` plus a frame chain: each frame's own length
  field must land on the next syncword. `>=4` chained frames scores
  `LATTICE_WEAK`.
- **MP3** — frame sync plus header sanity (valid bitrate/sample-rate indices,
  not the reserved values) plus a length chain, same shape as ADTS.
- **AnnexB** — start code `00 00 01` or `00 00 00 01` plus NAL header sanity
  (forbidden_zero_bit clear, a plausible nal_unit_type).

All three are suppressed by a strong container match, per the suppression rule.

## Performance

"Fast" here means bounded and predictable.

- **Budget.** `DEFAULT_BUDGET` is 64 KiB — comfortably above the worst case
  (a 208-stride TS lattice needing 8 confirmations plus a full phase search).
  `probe_with_budget` caps it lower. No prober reads past the budget; one that
  could conclude with more bytes reports how many, surfacing as
  `Insufficient { need_at_least }`.
- **Single pass, no allocation in the scan.** Probers borrow the input and
  return `Match` by value. `Detail` holds only `Copy` scalars and fixed arrays
  (a `[u8; 4]` brand, never a `String`). The crate's only allocation is
  `Probe::Ambiguous`'s candidate `Vec`, which holds a handful of entries and
  occurs only on a genuine tie.
- **Early exit within a prober.** A prober stops once more evidence cannot raise
  its tier: the TS lattice at 8 confirmations, the ISOBMFF walk once enough
  boxes chain, ADTS at 4 frames.
- **Cheapest discriminator first.** Probers run fixed-offset-magic before
  lattice scans. Every prober still runs, but the expensive scans exit on a
  cheap precondition, so a file with strong magic resolves after a few byte
  comparisons.
- **Worst case** — a buffer matching nothing — is `budget × prober_count` byte
  reads, no allocation, and fully caller-controlled.

## Testing

**Real fixtures are the bar.** Every format in scope gets a real file, not
hand-made bytes. The workspace already has a corpus: `dvb-si/tests/fixtures/*.ts`,
`transmux`'s ISOBMFF/WebM/FLV fixtures, `st377-1`'s MXF fixture. Each is
asserted to probe to its correct `Format` with the expected `Detail`. Fixtures
live in this crate's own `tests/fixtures/`, or are referenced from the existing
corpus where licensing permits — permissive only, provenance documented, no
copyleft.

**Coverage gaps are documented, never silent.** Any in-scope format lacking a
real fixture is recorded in the crate's `FIXTURES.md` with what is missing and
why, and its tests are marked as covering hand-built bytes only. A prober tested
solely against bytes we authored proves only that it agrees with our own reading
of the format.

**Bite tests — the cases today's implementation fails:**

- An M2TS (192-stride) file. Today returns `Unknown`; must return TS with
  `stride: 192`.
- A TS capture starting mid-packet. Must return TS with the correct non-zero
  phase.
- A DVB 204-stride file.
- An MP4 whose leading bytes coincidentally match a weaker prober. Must still
  return ISOBMFF, with the losing candidate's score inspectable.
- A TS file carrying ADTS audio. Must return TS, not AAC — the suppression rule.

**Mutation proofs, recorded in each test's doc comment.** Disabling the phase
search must break the mid-packet test. Disabling the stride search must break
M2TS. Removing ES suppression must break the TS-with-ADTS test. A test that does
not bite when its rule is removed is not testing the rule.

**Drift guard.** A test with dev-dependencies on `mpeg-ts`, `st377-1`, and
`mpeg-ps` asserts this crate's constants equal theirs (`TS_SYNC_BYTE`,
`TS_PACKET_SIZE`, the MXF partition-key prefix, `pack_start_code`). Dev-only, so
the runtime graph stays `broadcast-common` only; no publish cycle, since none of
those crates depends on this one.

Where a cited crate keeps a needed constant private (`st377-1`'s
`PARTITION_KEY_PREFIX` is not `pub`), the guard asserts against the nearest
public equivalent and the spec-cited literal is carried here with its source
named in the constant's own doc comment — never a bare literal.

**Fuzz target.** `probe()` over arbitrary bytes must never panic, never read out
of bounds, and always terminate within budget — the workspace rule that a new
parser meets the same bar as a new crate.

**Negative cases.** Empty input, one byte, all-zeros, all-`0x47` (a pathological
TS-shaped buffer with no real structure), and random bytes. Each must return
`Insufficient` or `Unknown` — never a confident wrong answer.

**Crate acceptance.** Clears `docs/CRATE-ACCEPTANCE.md` in full: >=2 examples,
`#[non_exhaustive]`, the #204 label convention, the 6-gate suite, full
RELEASE-DOCS.

**One documented deviation:** the round-trip invariant does not apply. A probe
reads and concludes; it has no wire format to serialize. Its equivalent hard
gate is the real-fixture corpus — the same substitution `broadcast-loudness`
makes with the EBU Tech 3341 compliance vectors.

**Gate suite**, run against the real tree and not accepted on a delegate's
report:

```
cargo build   --workspace --all-features --locked
cargo test    --workspace --all-features --locked
cargo build   --workspace --no-default-features --locked
cargo clippy  --workspace --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```

## Consumers

- **`multimux`'s `FileReader`** (sub-project 2) — the reason this exists now.
- **`transmux::cli::detect_container`** — becomes a thin wrapper mapping
  `Probe` to its `CliResult<Container>`, deleting the duplicate logic. Keeps the
  CLI's public API unchanged.
- **`media-doctor`**, **`ts-fix`** — both currently assume or require a declared
  container; both can adopt this later. Not in this sub-project's scope.

## Explicit non-goals

- **No demuxing.** The probe identifies a format and reports what it learned
  getting there. Parsing the content is the demuxer's job.
- **No codec identification.** "This is TS" is the answer; "this TS carries
  H.264 and AAC" requires reading the PMT, which is `mpeg-ts`/`dvb-si` work.
- **No incremental/streaming API.** One-shot over a slice, with `Insufficient`
  telling a streaming caller to read more. A stateful wrapper would duplicate
  buffering the caller already does.
- **No file IO.** `no_std`; the caller supplies bytes.
- **No format conversion or repair.** Identification only.
