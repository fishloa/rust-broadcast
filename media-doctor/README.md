# media-doctor — DVB / MPEG-TS diagnostics

A lint-style analysis harness for MPEG-2 Transport Streams (and HLS playlists).
Each [`Diagnostic`] checks one rule against a TS byte buffer and pushes
[`Finding`]s into a [`Report`]; a small CLI runs the full set over a file.
`no_std` + `alloc` (the CLI is `std`).

## Diagnostics

| Check | Detects |
|---|---|
| `SyncByteCheck` | missing `0x47` sync bytes |
| `PatPmtVersionCheck` | PAT/PMT `version_number` changes across the stream |
| `CcAnomalyCheck` | continuity-counter discontinuities (honouring legal duplicates + `discontinuity_indicator`) |
| `PcrCheck` | PCR jitter / discontinuity on the PCR PID (TR 101 290-style), honouring signalled discontinuities |
| `PtsCheck` | non-monotonic **decode** timestamps (DTS, else PTS — legal B-frame PTS reorder is not flagged) + forbidden `PTS_DTS_flags == 0b01`, on real PES PIDs only |
| `Scte35Check` | SCTE-35 splice consistency — unbalanced `splice_insert` out/in pairs, duplicate open "out"s |
| `CodecSignallingCheck` | codec signalling vs bitstream — PMT `stream_type` vs actual ES codec; `esds` ASC vs ADTS (reuses `transmux` SPS decoders) |
| `check_container_codec` | `avcC`/`hvcC` profile/level/chroma/bit-depth vs in-band SPS; sample-entry dims vs SPS-decoded dims |
| `FpsCadenceCheck` | VUI frame rate vs track timescale cadence |
| `ParamSetsCheck` | missing SPS/PPS/VPS before the first IDR/IRAP |
| `InterlaceCheck` | interlaced coding (`frame_mbs_only_flag == 0`) reported as content fact |
| `check_playlist` | HLS playlist validation (RFC 8216): missing `#EXTM3U`, missing `#EXT-X-TARGETDURATION`, `#EXTINF` exceeding target, malformed `#EXT-X-DATERANGE` |

Diagnostics are validated against **real captures** (e.g. a clean H.264+AAC
stream and a multi-programme DVB capture yield zero false positives) plus
crafted fault fixtures.

## CLI

```console
$ media-doctor check --input stream.ts
Findings: 0 error(s), 0 warning(s), 0 info(s)
```

`--json` emits the report as JSON (requires the `serde` feature, on by default).

## Library

```rust
use media_doctor::{Diagnostic, PtsCheck, Report};

let mut report = Report::new();
PtsCheck.run(&ts_bytes, &mut report);
for f in report.findings() {
    println!("[{:?}] {} @ {:?}", f.severity, f.rule_id, f.location);
}
```

HLS playlists are validated with the free function `check_playlist(&str, &mut Report)`.

## License

MIT OR Apache-2.0.
