# Inventory for the `broadcast-bitstream` extraction

Status: measurement only. This document changes no source file. It measures the
code that the approved design (`docs/broadcast-bitstream` extraction, lane bs0)
would move out of `transmux` into a new `broadcast-bitstream` crate.

Every number below comes from a command run in this worktree. The commands are
given once per kind in §0. File and line references are the evidence for each
verdict; `path:line` means the line was read.

## 0. How the counts were taken

Line counts and the test split:

```bash
# total lines per file
wc -l transmux/src/<file>.rs
# first `mod tests` / `mod <name>_tests` line; non-test = line-1, test = total-line+1
grep -n 'mod tests' transmux/src/<file>.rs
```

Where the test module is a separate file (`h262/tests.rs`,
`mpeg2_display/tests.rs`) the split is `wc -l` of the module file and of
`tests.rs`.

Cross-file reference counts:

```bash
# whole-word occurrences per file, grouped by first path component
python3 - <<'EOF'
import os, re, collections
# walk every *.rs outside .git/target/.crush, re.findall(r'\b'+name+r'\b')
EOF
# external imports
grep -rn 'use transmux::' --include='*.rs' . | grep -v '^./transmux/src'
grep -rn 'transmux::<module>' --include='*.rs' . | grep -v '^./transmux/src'
```

Public-field counts:

```bash
# count `    pub name:` lines between `pub struct X {` and its closing brace,
# ignoring everything from the test-module line onward
python3 - <<'EOF'   # field parser, see §4
EOF
```

Caveats, stated once and relied on below:

- `.width`, `.height`, `.data`, `.display`, `.channels`, `.rate`, `.profile`,
  `.frame_length`, `.samples_per_frame` are used by unrelated types across the
  workspace. Raw `\.<field>` counts are therefore upper bounds. The exact
  numbers in §4 are field *declarations*, and struct-literal construction
  counts, not raw `.field` counts.
- `BitReader`, `escape`, `unescape`, `nal_unit_type` also exist in crates that
  do not depend on `transmux`. The per-crate tables name the crates that
  actually import `transmux`; the false-positive crates are called out where
  they matter.

## 1. Candidate modules

`non-test` / `test` are lines in the file (or its `tests.rs`). `moves` means
the whole non-test body moves to `broadcast-bitstream`; `splits` means only the
bitstream half moves; `stays` means the file is not bitstream code. A split's
moving/staying line counts are given in the reason and counted from the
function boundaries named there.

| file | non-test | test | target feature | verdict |
|---|---:|---:|---|---|
| `transmux/src/bitreader.rs` | 270 | 203 | (always) | moves whole — exp-Golomb + RBSP reader, `crate::error` only (`bitreader.rs:11`) |
| `transmux/src/rbsp.rs` | 63 | 20 | (always) | moves whole — emulation-prevention bytes, no `crate::` imports |
| `transmux/src/nal.rs` | 824 | 831 | (always) | moves whole — NAL type, keyframe/IRAP, SEI walk (`nal.rs:7,42-44`) |
| `transmux/src/annexb.rs` | 276 | 281 | (always) | moves whole — Annex B and length-prefix conversion (`annexb.rs:19`) |
| `transmux/src/au.rs` | 462 | 557 | (always) | moves whole — access-unit boundaries (`au.rs:38-40`) |
| `transmux/src/sps.rs` | 2224 | 2152 | `h264`, `hevc` | moves whole — SPS/VUI/PTL decode and `rfc6381_*` (`sps.rs:23-28`) |
| `transmux/src/slice_header.rs` | 468 | 244 | `h264` | moves whole — slice/PPS prefix (`slice_header.rs:13-17`) |
| `transmux/src/interlace.rs` | 504 | 232 | `h264`, `hevc` | splits — 395 move (types + `avc_facts`/`hevc_facts`, lines 1-293 and 403-504), 109 stay (`impl CodecConfig`, `video_scan`, `video_picture_facts`, lines 294-402) |
| `transmux/src/hdr10plus.rs` | 892 | 70 | `h264`, `hevc` | moves whole — ST 2094-40 SEI (`hdr10plus.rs:31-33`) |
| `transmux/src/hdr.rs` | 141 | 235 | `h264`, `hevc` + `isobmff` | splits — 112 move (type + SEI absorb/emit, lines 1-112), 29 stay (`sample_entry_boxes`/`from_sample_entry_boxes`, lines 113-141) |
| `transmux/src/aac_asc.rs` | 1237 | 658 | `aac` | moves whole — AudioSpecificConfig + ADTS (`aac_asc.rs:23`) |
| `transmux/src/ac3_syncframe.rs` | 1360 | 1246 | `ac3` + `isobmff` | moves whole — AC-3/E-AC-3 BSI (`ac3_syncframe.rs:19-23`) |
| `transmux/src/dts_core.rs` | 379 | 420 | `dts` + `isobmff` | moves whole — DTS core header (`dts_core.rs:9-13`) |
| `transmux/src/mpeg_legacy.rs` | 372 | 109 | `mpeg2`, `mpeg-audio` | splits by feature — `Mpeg2SeqHeader` (`mpeg_legacy.rs:34-110`) to `mpeg2`; `MpegAudioLayer`/`MpegAudioFrameHeader` (`mpeg_legacy.rs:112-372`) to `mpeg-audio` |
| `transmux/src/user_data.rs` | 184 | 0 | `mpeg2` (also used by AVC/HEVC display, `display_track.rs:449-486`) | moves whole — AFD/bar data (`user_data.rs:5`) |
| `transmux/src/h262.rs` | 456 | 350 | `mpeg2` | moves whole except `to_display_geometry` — 426 move (lines 1-207 and 238-456), 30 stay (lines 208-237, `to_display_geometry`, builds `DisplayGeometry`) |
| `transmux/src/mpeg2_display.rs` | 235 | 536 | — | stays — per-access-unit tracker over `DisplayInfo`/`DisplayChangeSet`, also calls `ts_demux::mpeg2_is_sync` (`mpeg2_display.rs:28-35,130`) |
| `transmux/src/audio_profile.rs` | 151 | 194 | — | stays, not bitstream code — operator declarations applied to `TrackSpec` (`audio_profile.rs:13`) |
| `transmux/src/nalu_ext.rs` | 69 | 24 | `isobmff` | moves whole as glue — trait impls over `isobmff::{AvcSps, HevcNalUnit}` (`nalu_ext.rs:7-9`); its own doc says "stay in transmux" (`nalu_ext.rs:4-5`) |
| `transmux/src/codec_config_ext.rs` | 93 | 61 | `isobmff` | moves whole as glue — trait impls over `isobmff` config records (`codec_config_ext.rs:8-10`); doc says "stay in transmux" (`codec_config_ext.rs:4-6`) |
| `transmux/src/smooth_codec.rs` | 341 | 423 | — | stays, not bitstream code — Smooth `QualityLevel` → `TrackSpec` and init synthesis (`smooth_codec.rs:35-49`) |
| `transmux/src/mhas.rs` | 174 | 174 | none assigned | discovered, not in the design's table: MHAS packet framing, no `crate::` imports (`mhas.rs:1-12`). Moves cleanly but needs a decision on a `mpeg-h` feature |
| `transmux/src/units.rs` | 9 | 0 | (always) | shared constant only — `BITS_PER_KILOBIT` used by `ac3_syncframe.rs:21` and `mpeg_legacy.rs:26`; move or duplicate |
| `transmux/src/ir/display.rs` | 1360 | 79 | (always) | splits — 843 non-test lines are coded-fact types that move (§3); the rest (`DisplayGeometry`, `DisplayInfo`, `DisplayChangeSet`, `ActiveFormat`, `AfdPersistence`, `UserDataCarriage`) stays |

Totals for the table: 12,544 non-test and 9,099 test lines are in scope. Of
those, 10,949 non-test and 6,483 test lines move (or move behind a feature).
The 6,483 is the own-test line count from §6, not the whole test modules: the
remaining 2,616 test lines are IR-bound tests, shared test helpers, and the
test modules of the staying files.

`mpeg2_display.rs`, `audio_profile.rs`, and `smooth_codec.rs` turn out **not**
to be bitstream code. `nalu_ext.rs` and `codec_config_ext.rs` are glue, not
parsers. `hdr.rs` is half SEI parser, half `isobmff` box helper.

## 2. Imports to cut

Grouped by the imported item a moving module uses that does not move with it.
A moving-to-moving import (`sps` ↔ `aac`, `nal` ↔ `hdr`, `ac3` → `dts`) needs
no conversion but constrains the PR order (§10) and is listed at the end.

### `crate::error::{Error, Result}` — 17 use lines in moving code

`bitreader.rs:11`, `sps.rs:24,763,842,1520`, `annexb.rs:19`, `au.rs:39`
(imported as `AuResult`), `slice_header.rs:15`, `hdr10plus.rs:32`, `hdr.rs:13`,
`aac_asc.rs:23`, `ac3_syncframe.rs:20`, `dts_core.rs:10`,
`mpeg_legacy.rs:25`, `nalu_ext.rs:7`, `codec_config_ext.rs:9`, `h262.rs:33`.
Replace with the new crate's own `Error`/`Result`; `bit_error`
(`bitreader.rs:69-95`) is the one converter that stays and maps
`broadcast_core::bits::BitError` into it.

### `crate::units::BITS_PER_KILOBIT` — 2 use lines

`ac3_syncframe.rs:21`, `mpeg_legacy.rs:26`. The constant is private to
`transmux` (`units.rs:5`). Replace with a constant in the new crate (or the
existing `broadcast-common` unit constant if one is chosen).

### `crate::ir::*` — 5 imports plus 4 doc links in moving code

| item | use | replacement |
|---|---|---|
| `AspectRatioIdc`, `ColourDescription`, `EXTENDED_SAR_IDC`, `VideoFormat`, `VideoSignalType`, `PictureRect` | `sps.rs:25,415` | new-crate types if the `ir` coded-fact types move (§3); otherwise a conversion at the `transmux` boundary |
| `CodecConfig` | `interlace.rs:29` | stays: only the `impl CodecConfig` half uses it; the moving `avc_facts`/`hevc_facts` do not |
| `ActiveFormatDescription`, `BarData` | `user_data.rs:5` | new-crate types if they move; otherwise `transmux` converts at the return |
| `AspectRatioIdc`, `ColourDescription`, `ColourPrimaries`, `DisplayGeometry`, `MatrixCoefficients`, `PictureRect`, `TransferCharacteristics`, `VideoFormat`, `VideoSignalType` | `h262.rs:34-37`, `h262.rs:393,400` | same split: enum facts move, `DisplayGeometry` stays and is built by the staying `to_display_geometry` |
| `DisplayGeometry` | `sps.rs:414` (doc link) | doc link retargets |
| `CodecConfig` | `mpeg_legacy.rs:5-6` (doc links) | doc link retargets |

### `crate::pipeline::{CodecConfig, Sample}` — 4 references, of which 1 is non-test code

`interlace.rs:29` (`CodecConfig`, the staying wrapper), `ac3_syncframe.rs:1463`
(test only, `CodecConfig`), `mpeg_legacy.rs:5-6` (doc links). `interlace.rs:9,16`
also name `StreamingTsDemux`/`Sample` in doc comments. Replace by keeping the
IR-facing wrappers in `transmux` and never passing `CodecConfig` into a moving
parser.

Test-only IR imports are `sps.rs:3426` (`crate::ir`), `sps.rs:4040` (`crate::pipeline::FrameRateBasis`), `interlace.rs:540` (`crate::ir::CodecConfig`), and `ac3_syncframe.rs:1463` (`crate::pipeline::CodecConfig`). They stay with the IR tests (§6).

### `crate::test_bits::BitWriter` — 6 test-only uses

`sps.rs:2769`, `slice_header.rs:473`, `interlace.rs:541`,
`ac3_syncframe.rs:1685`, `dts_core.rs:382`, `aac_asc.rs:1240`. `test_bits.rs` is
98 lines behind `#[cfg(test)]` (`lib.rs:238-239`). The new crate needs its own
dev-only writer or a shared test dependency; otherwise these six test modules
stay in `transmux`.

### `crate::collect::media_from_ts` — 1 test-only use

`ac3_syncframe.rs:1465` inside an AC-3 test helper. `collect` is
`#[cfg(test)]`-gated (`lib.rs:220-224`). The test either stays in `transmux` or
is rewritten against a fixture file directly.

### `crate::iter_annexb_nals` (root re-export) — 1 use

`sps.rs:277`, inside the SPS display-facts path. Replace with the new crate's
`annexb::iter_annexb_nals`.

### Staying modules that import a moving module

These are not "imports to cut" from a moving file, but they pin the blast
radius and must change with the move:

| moving item | staying users |
|---|---|
| `bitreader::BitReader` | `display_track.rs:51`, `live_join.rs:60` (non-test); `sample_aes.rs:1192,1198`, `ts_demux.rs:3158` (test/helper) |
| `rbsp::{escape,unescape}` | `sample_aes.rs`, `ts_demux.rs` (test/helper) |
| `h262::{...}`, `user_data::{...}` | `mpeg2_display.rs:28,35,232`; `display_track.rs:129,449-486`; `caption_detect.rs:223` |
| `ac3_syncframe::{Ec3SplitFrame,split_ac3_syncframes}` | `dts_core.rs:340,354` — doc links only |

### `broadcast_common`, not `broadcast_core`

The design says the new crate depends only on `broadcast-core`. Five moving
modules import `broadcast-common`:

| file:line | item |
|---|---|
| `nal.rs:45` | `wire::{be_u16, be_u32}` |
| `ac3_syncframe.rs:23` | `wire::u16_at` |
| `mpeg_legacy.rs:27` | `wire::be_u32` |
| `user_data.rs:6` | `wire::{be_u16, be_u32}` |
| `sps.rs:2033,2083,2123` | `hex::hex_encode_upper` |

`broadcast-core` has no hex encoder and no `wire` module (`broadcast-core/src`:
`bits`, `cenc`, `error`, `len`, `lib`, `reserved`, `timescale`, `traits`). See
§11.

### Moving-to-moving imports that force PR order

- `aac_asc.rs:561` → `sps::rfc6381_mp4a`.
- `nal.rs:168,803,818` → `sps::decode_*`; `hdr.rs:48,53,85` and
  `hdr10plus.rs:629-691` → `nal::*`.
- `dts_core.rs:340,354` → `ac3_syncframe` (doc only).
- `h262.rs:38` → `mpeg_legacy::SEQUENCE_HEADER_CODE`; `h262.rs:395,401` →
  `user_data::{AFD_ACTIVE_FORMAT_FLAG_BIT,AFD_CODE_MASK}`.
- `codec_config_ext.rs:10,30,62` and `nalu_ext.rs:9` → `sps::*`.

## 3. Types: moves or stays

The rule applied: a type moves if a codec specification defines it (a
parameter set and what it states, a coded picture fact, a coded metadata
record); it stays if it is a presentation or container concept built from those.
Counts are whole-word references **outside the defining file**, per crate.

### 3.1 Types defined in the candidate modules

| type | defined | references outside its file | verdict |
|---|---|---|---|
| `BitReader` | `bitreader.rs:113` | 0 real external (other crates' `BitReader` is theirs) | moves, including its owning `Vec` wrapper |
| `NalCodec` | `nal.rs:113` | `media-plane` 19, `multimux` 7, `fuzz` 5, `media-doctor` 4, `broadcast-client` 1, `transmux` 333 (mostly tests) | moves; design confirms it needs no feature |
| `AnnexBNalIter` | `annexb.rs:106` | `transmux` 4 | moves |
| `AccessUnitSplitter` | `au.rs:211` | `transmux` 24 | moves |
| `AvcSlicePps`, `AvcSliceHeader`, `AvcPicture`, `AvcFieldInfo`, `AvcParamSets` | `slice_header.rs:56,148,310,325,365` | `transmux` 6/8/42/17/29; `fuzz` 2 (`AvcParamSets`); `workspace-guards` 2/1 | move — coded slice/PPS facts |
| `VideoScan`, `PicStruct`, `SourceScanType`, `PictureFacts` | `interlace.rs:82,119,234,270` | `transmux` 31/93/28/11 | move — coded interlace facts; `impl CodecConfig` (line 294) stays |
| `AvcVuiTiming`, `AvcPicTimingLayout`, `AvcReorderDepth`, `HevcVuiTiming`, `HevcInterlaceSps`, `HevcVpsTiming`, `VvcSpsInfo` | `sps.rs:214,345,358,1053,1072,1090,1844` | whole-workspace occurrences 6/9/10/12/14/11/5, all in `transmux` | move — timing, PTL and cropping facts |
| `OverlapProcessOption`, `WindowRegion`, `PeakLuminanceTable`, `DistributionPoint`, `BezierToneMapping`, `ColorVolumeTransform`, `AdditionalWindow`, `Hdr10PlusMetadata` | `hdr10plus.rs:116-259` | `transmux` 15/10/15/8/10/16/9/57; `multimux` 2 (`Hdr10PlusMetadata`) | move — ST 2094-40 coded metadata |
| `HdrStaticMetadata` | `hdr.rs:31` | `transmux` 26 | moves; its fields keep `isobmff::hdr_boxes` types (`hdr.rs:33-35`), so it needs the `isobmff` feature |
| `AudioObjectType`, `SamplingFrequencyIndex`, `ChannelConfiguration`, `AudioSpecificConfig`, `HeAacSignaling`, `AdtsHeader`, `HeAacProfile` | `aac_asc.rs:101,228,340,443,458,867,1053` | `transmux` 97/83/69/156/6/12/38; `isobmff` 1 (`ChannelConfiguration`, unrelated field mention); `bindings` 4, `smooth-runtime` 4, `demo` 2, `test-harness` 2, `multimux` 1 (`AudioSpecificConfig`); `test-harness` 1 (`HeAacProfile`) | move — audio decoder config and ADTS header |
| `Ac3SyncframeInfo`, `Ec3SyncframeInfo`, `Ec3SplitFrame` | `ac3_syncframe.rs:241,407,798` | `transmux` 24/81/5; `workspace-guards` 1 (`Ac3SyncframeInfo`, non_exhaustive table) | move — BSI facts |
| `DtsCoreFrameInfo`, `DtsCoreFrame` | `dts_core.rs:170,343` | `transmux` 30/3; `workspace-guards` 1 (`DtsCoreFrameInfo`) | move — core frame header |
| `Mpeg2SeqHeader` | `mpeg_legacy.rs:57` | `transmux` 18 | move to `mpeg2` |
| `MpegAudioLayer`, `MpegAudioFrameHeader` | `mpeg_legacy.rs:112,165` | `transmux` 39/22 | move to `mpeg-audio` |
| `AfdSignal`, `PictureUserData` | `user_data.rs:51,70` | `transmux` 19/1 | move; they carry `ActiveFormatDescription`/`BarData` (§3.2) |
| `SequenceHeaderGeometry`, `SequenceDisplayExtension` | `h262.rs:151,165` | `transmux` 5/7 | move the facts; `to_display_geometry` (line 208) stays as a `transmux` conversion |
| `Mpeg2Outcome`, `Mpeg2DisplayTracker` | `mpeg2_display.rs:43,61` | `transmux` 24/26; `workspace-guards` 2 (`Mpeg2Outcome` label skip) | **stay** — access-unit tracker over IR `DisplayInfo`/`DisplayChangeSet`, depends on `ts_demux` |
| `AudioTrackSelector`, `AudioProfile`, `AudioProfileOverride`, `AppliedOverrides` | `audio_profile.rs:23,62,94,104` | `multimux` 36/11/14, `media-plane` 25/3/4, `transmux` 19/16/10/3 | **stay** — operator configuration applied to `TrackSpec`, not a coded fact. `AudioProfile::he_aac` (`audio_profile.rs:79`) returns the moving `HeAacProfile`, so its import changes |
| `AvcSpsCodecInfo`, `HevcNalUnitCodecInfo` | `nalu_ext.rs:16,40` | `transmux` tests, `media-doctor` 2 imports | move behind `isobmff` (glue) |
| `HevcRecordCodecInfo`, `VvcRecordCodecInfo`, `EsdsBoxCodecInfo` | `codec_config_ext.rs:21,50,82` | `transmux` tests, `demo` 1 import | move behind `isobmff` (glue) |

### 3.2 Types in `transmux/src/ir/display.rs`

Flagged as **purely coded facts**, therefore moving under the design's rule:

| type | line | references outside `ir/display.rs` | why it is a codec fact |
|---|---|---:|---|
| `AspectRatioIdc` | 73 | `transmux` 148, `media-plane` 5, `hls-runtime` 2 | VUI `aspect_ratio_idc` + `sar_width`/`sar_height` |
| `SampleAspectRatio` | 232 | `transmux` 18 | the ratio the SPS/MPEG-2 header states |
| `PictureRect` | 288 | `transmux` 52 | cropping rectangle / conformance window |
| `VideoFormat` | 339 | `transmux` 44, `hls-runtime` 7 | `video_format` table |
| `ColourPrimaries` | 411 | `transmux` 20 | VUI colour code |
| `TransferCharacteristics` | 513 | `transmux` 42, `broadcast-matroska` 3, `broadcast-hls` 1 | VUI colour code |
| `MatrixCoefficients` | 649 | `transmux` 28, `broadcast-matroska` 3 | VUI colour code |
| `ColourDescription` | 780 | `transmux` 35, `hls-runtime` 6 | the three codes together |
| `VideoSignalType` | 810 | `transmux` 37, `hls-runtime` 7 | `video_signal_type()` as signalled |
| `ActiveFormatDescription` | 1029 | `transmux` 97, `broadcast-client` 2 | AFD code table |
| `BarData` | 1115 | `transmux` 34 | 14-bit bar positions |

Counted total for these declarations: 843 non-test lines (line spans
73-231, 232-287, 288-312, 339-410, 411-512, 513-648, 649-779, 780-809,
810-845, 1029-1114, 1115-1124).

**Staying** in `ir/display.rs` (presentation/container concepts), with the
constructor that will take the moved type:

| type | line | constructor / bridge |
|---|---|---|
| `DisplayGeometry` | 846 | `from_avc_sps` (906) and `from_hevc_sps` (912) today take `&[u8]`; they will take the moved SPS fact type. `from_codec_config` (921) stays IR-only. MPEG-2 has no equivalent constructor; `SequenceHeaderGeometry::to_display_geometry` (`h262.rs:208`) is the only one |
| `ActiveFormat` | 1145 | `ActiveFormat::new(afd: Option<ActiveFormatDescription>, bar_data: Option<BarData>, carriage)` (1153) takes the moved AFD/bar types |
| `DisplayInfo` | 1201 | `from_geometry` (1228), no moved argument |
| `DisplayChangeSet` | 1303 | no constructor; its fields compare staying/moved types |
| `AfdPersistence` | 1171 | no moved argument |
| `UserDataCarriage` | 1125 | names the wrapper, not a codec code; stays |

`EXTENDED_SAR_IDC` (53) and the `INFERRED_*` constants (313-330) stay: they are
the presentation-side inference, not a parsed field.

### 3.3 What this does to public paths

The design says `transmux` re-exports nothing that moved. Today the crate root
re-exports the moving items (`lib.rs:278-287,320-357,423-440`): `NalCodec`,
`AudioSpecificConfig`, `AdtsHeader`, `parse_adts_header`, `build_adts_header`,
`Ac3SyncframeInfo`, `Ec3SyncframeInfo`, `SequenceHeaderGeometry`,
`SequenceDisplayExtension`, `PicStruct`, `PictureFacts`, `VideoScan`,
`SourceScanType`, `HdrStaticMetadata`, `Hdr10PlusMetadata`/`WindowRegion`/…, `Mpeg2SeqHeader`,
`MpegAudioFrameHeader`, `MpegAudioLayer`, `AvcSpsInfo`, `HevcSpsInfo`,
`decode_*`, `rfc6381_*`, plus the `ir` display enums listed in §3.2. Removing
those re-exports is a breaking change for every crate in §7.

## 4. Public fields

Count of `pub` field declarations on public structs in non-test code, per
moving module. The design bans them: each becomes a private field plus an
accessor.

| module | `pub` fields | structs carrying them |
|---|---:|---|
| `sps.rs` | 59 | `HevcSpsInfo` 14, `AvcSpsInfo` 13, `AvcSliceSps` 9, `VvcSpsInfo` 6, `AvcVuiTiming` 3, `HevcVuiTiming` 5, `HevcVpsTiming` 2, `HevcInterlaceSps` 2, `AvcPicTimingLayout` 1, `AvcReorderDepth` 4 |
| `slice_header.rs` | 23 | `AvcSliceHeader` 14, `AvcSlicePps` 5, `AvcFieldInfo` 4 |
| `interlace.rs` | 5 | `PictureFacts` 5 |
| `hdr10plus.rs` | 34 | `WindowRegion` 11, `Hdr10PlusMetadata` 7, `ColorVolumeTransform` 6, `PeakLuminanceTable` 3, `BezierToneMapping` 3, `DistributionPoint` 2, `AdditionalWindow` 2 |
| `hdr.rs` | 2 | `HdrStaticMetadata` 2 |
| `aac_asc.rs` | 16 | `AdtsHeader` 7, `AudioSpecificConfig` 6, `HeAacSignaling` 3 |
| `ac3_syncframe.rs` | 21 | `Ec3SyncframeInfo` 12, `Ac3SyncframeInfo` 7, `Ec3SplitFrame` 2 |
| `dts_core.rs` | 12 | `DtsCoreFrameInfo` 10, `DtsCoreFrame` 2 |
| `user_data.rs` | 2 | `PictureUserData` 2 |
| `mpeg_legacy.rs` | 8 | `MpegAudioFrameHeader` 6, `Mpeg2SeqHeader` 2 |
| `h262.rs` | 8 | `SequenceHeaderGeometry` 4, `SequenceDisplayExtension` 4 |
| `bitreader.rs`, `rbsp.rs`, `nal.rs`, `annexb.rs`, `au.rs`, `mpeg2_display.rs`, `nalu_ext.rs`, `codec_config_ext.rs` | 0 | — |
| **total** | **190** | |

If the `ir/display.rs` coded-fact structs also move (§3.2), add 14:
`PictureRect` 4, `BarData` 4, `ColourDescription` 3, `VideoSignalType` 3
(`SampleAspectRatio` is already private), making **204**.

Ten structs with the most fields, and the construction sites outside their
module (a struct literal cannot survive private fields):

| struct | pub fields | struct literals outside the module (exact) | raw `.<field>` count outside (upper bound) |
|---|---:|---:|---:|
| `HevcSpsInfo` | 14 | 1 (`codec_config_ext.rs:30`) | 619 |
| `AvcSliceHeader` | 14 | 0 | 55 |
| `AvcSpsInfo` | 13 | 0 | 628 |
| `Ec3SyncframeInfo` | 12 | 0 | 234 |
| `WindowRegion` | 11 | 2 (`transmux/tests/it/hdr10plus.rs:212,274`) | 26 |
| `DtsCoreFrameInfo` | 10 | 0 | 167 |
| `AvcSliceSps` | 9 | 0 | 30 |
| `AdtsHeader` | 7 | 0 | 138 |
| `Ac3SyncframeInfo` | 7 | 0 | 168 |
| `Hdr10PlusMetadata` | 7 | 7 (`transmux/tests/it/hdr10plus.rs:259,329,343,362,415,472,615`) | 48 |

The next three, at 6 fields each, are `VvcSpsInfo`, `ColorVolumeTransform`
(1 test literal) and `MpegAudioFrameHeader` (0). The constructor cost is
concentrated in the HDR10+ tests, which build 7 `Hdr10PlusMetadata`, 2
`WindowRegion` and 1 `ColorVolumeTransform` literal.

The raw-`.<field>` column is an upper bound because the field names collide
across types; it shows where accessor conversion will be widest, not a
mechanical edit count. The exact column is the struct-literal count: 10 sites in this table (11 in
the whole moving set, with `ColorVolumeTransform`) must become constructor calls
or move into the new crate's tests.

## 5. The bit reader

`transmux/src/bitreader.rs` (270 non-test lines) against
`broadcast-core/src/bits.rs` (811 lines). Every function of the first:

| `bitreader.rs` function | `broadcast-core::bits` offers | gap at signature level |
|---|---|---|
| `read_bits_at(data: &[u8], bit_pos: &mut usize, n: usize) -> Option<u64>` (28) | `BitReader::starting_at(data, bit_pos) -> Result<Self, BitError>` (140) + `read_bits(n: u32) -> Result<u64, BitError>` (177) + `bits_read()` (154) | no one-call cursor helper; composable, or add one |
| `read_bits_checked(data, bit_pos, n, what) -> Result<u64>` (40) | the same composition | no typed-error wrapper; stays in the new crate |
| `bytes_needed(bit_pos: usize, n: usize) -> usize` (62) | none | add to the new crate (or core) |
| `bit_error(what: &'static str, e: BitError) -> Error` (69) | partial: `impl From<BitError> for BufferTooShort` (`bits.rs:86`) | `TooManyBits`/`ValueTooWide` mapping is specific to the new error; stays |
| `write_bits_checked(w, value, n, what) -> Result<()>` (97) | `BitWriter::write_bits(value: u64, n: u32) -> Result<(), BitError>` (296) + `len::fit_bits` | no combined write; stays |
| `BitReader::from_rbsp(&[u8], what) -> Result<Self>` (121) | `BitReader<'a>` borrows (`bits.rs:123`) | no owning reader; the new crate keeps the owning wrapper and constructs a core reader per read |
| `BitReader::with_unescape(&[u8], what)` (137) | none | RBSP removal stays in the new crate (`rbsp.rs:19`) |
| `BitReader::read_bits(&mut self, n: usize, what) -> Result<u64>` (166) | `read_bits(n: u32) -> Result<u64, BitError>` | error type and `usize`→`u32` conversion |
| `BitReader::read_flag(&mut self, what) -> Result<bool>` (202) | `read_bool() -> Result<bool, BitError>` (207) | error type only |
| `BitReader::align_to_byte(&mut self, what) -> Result<()>` (208) | `align_to_byte(&mut self)` (231), infallible | core's cannot fail and has no `what`; the wrapper keeps the typed result |
| `BitReader::read_ue(&mut self, what) -> Result<u64>` (219) | **none** | must be added |
| `BitReader::read_se(&mut self, what) -> Result<i64>` (260) | **none** | must be added |

What would have to be added to `broadcast-core`:

1. `BitReader::read_ue(&mut self) -> Result<u64, BitError>`, with the
   32-leading-zero bound `transmux` enforces (`bitreader.rs:243-252`) expressed
   as a `BitError` variant (or `TooManyBits` reuse).
2. `BitReader::read_se(&mut self) -> Result<i64, BitError>` built on it
   (`bitreader.rs:264-269`).
3. Optional, if the owning/cursor layer is to live in core rather than the new
   crate: `read_bits_at`/`bytes_needed` equivalents. They are not required for
   correctness; the new crate can compose `starting_at` + `read_bits` as
   `bitreader.rs:166-192` already does.

Both additions are additive and go through the reviewed `core_api_pin` edit
(`workspace-guards/tests/guards/core_api_pin.txt`). No second reader is needed:
the design's one-reader rule is satisfiable.

## 6. Tests

### 6.1 Unit tests per moving module

`own` = the test body uses only the module's own types (moves). `IR` = the body
constructs `transmux` IR/pipeline types (`CodecConfig`, `TrackSpec`,
`DisplayGeometry`, …) and therefore stays in `transmux` or is rewritten. Counts
are `#[test]` functions; line totals are the function spans.

| module | own tests | IR tests | own test lines | IR test lines | three own examples | three IR examples |
|---|---:|---:|---:|---:|---|---|
| `bitreader.rs` | 13 | 0 | 199 | 0 | `read_bits_checked_distinguishes_oversize_from_short_buffer`, `a_64_bit_field_reads_and_a_65_bit_request_is_refused`, `exp_golomb_across_byte_boundaries_and_the_32_zero_bound` | — |
| `rbsp.rs` | 1 | 0 | 17 | 0 | `unescape_only_drops_a_three_byte_after_two_zero_bytes` | — |
| `sps.rs` | 41 | 7 | 1492 | 642 | `reorder_depth_is_read_from_real_x264_sps`, `rfc6381_avc1_profiles`, `avc_sps_vui_timing_25fps` | `avc_field_pair_parity_comes_from_the_first_slice`, `every_colour_code_round_trips_and_has_a_label`, `a_vui_truncated_inside_the_aspect_field_never_yields_a_half_read_value` |
| `nal.rs` | 41 | 0 | 821 | 0 | `real_parameter_sets_are_valid`, `oversized_parameter_set_is_invalid`, `parameter_set_length_boundaries_are_inclusive_and_reject_a_bare_header` | — |
| `annexb.rs` | 14 | 0 | 276 | 0 | `splits_mixed_start_codes_and_strips_trailing_zeros`, `annexb_to_length_prefixed_bijection`, `length_prefixed_rejects_overrun` | — |
| `au.rs` | 20 | 0 | 509 | 0 | `splits_audless_stream_on_first_slice_and_config`, `multi_slice_picture_stays_one_au`, `streaming_matches_whole_buffer_at_every_split_point` | — |
| `slice_header.rs` | 6 | 0 | 136 | 0 | `parses_every_prefix_field_bit_exact`, `pps_slice_group_type_6_walked`, `classify_field_frame_and_unknown` | — |
| `interlace.rs` | 3 | 1 | 66 | 163 | `pic_struct_codes_match_tables_d1_and_d2`, `hevc_facts_come_from_the_config_sps_and_prefix_sei`, `hevc_only_prefix_sei_nals_are_read_as_sei` | `source_scan_type_codes_map_per_d33` |
| `hdr10plus.rs` | 2 | 0 | 64 | 0 | `has_t35_header_rejects_each_wrong_byte`, `hevc_sei_nal_payloads_keeps_only_prefixed_t35_messages` | — |
| `hdr.rs` | 11 | 0 | 225 | 0 | `mdcv_round_trips_and_decodes`, `clli_round_trips_and_decodes`, `sei_nal_with_both_messages_is_absorbed` | — |
| `aac_asc.rs` | 30 | 0 | 646 | 0 | `parse_real_fixture`, `serialize_byte_identical`, `adts_round_trip` | — |
| `ac3_syncframe.rs` | 32 | 1 | 1199 | 42 | `ec3_channels_with_lfe_counts_chan_loc_dependents`, `lookup_tables_match_the_specification`, `dac3_round_trip` | `half_rate_frame_uses_sr_code2` |
| `dts_core.rs` | 23 | 0 | 410 | 0 | `round_trip_unit`, `from_es_parses_core_header`, `from_es_scans_for_sync_not_at_offset_zero` | — |
| `mpeg_legacy.rs` | 10 | 0 | 106 | 0 | `seq_header_320x240`, `mp2_frame_header_44100_mono`, `mp3_frame_header_44100` | — |
| `user_data.rs` | 0 in-file | 0 | 0 | 0 | (tests are `transmux/tests/it/user_data_t35.rs`) | — |
| `h262.rs` (`h262/tests.rs`) | 9 | 4 | 189 | 95 | `aspect_codes_derive_the_table_6_3_ratios`, `sequence_extension_extends_the_coded_size`, `first_display_extension_after_a_sequence_extension_wins` | `no_sequence_header_is_an_error`, `codec_config_without_sequence_header_yields_none`, `codec_config_sequence_header_gives_geometry` |
| `nalu_ext.rs` | 1 | 0 | 17 | 0 | `hevc_nal_unit_decode_sps_distinguishes_short_from_sps_typed` | — |
| `codec_config_ext.rs` | 4 | 0 | 49 | 0 | (four VVC/HEVC record tests) | — |

Totals: 261 own tests / 6,421 lines move, plus 4 of the 5 tests in
`ir/display.rs` (62 lines: `idc_round_trips_for_every_wire_value`,
`variants_land_on_their_table_e1_rows`, `labels_follow_the_spec_tokens`,
`sample_aspect_ratio_rejects_a_zero_term`), for 265 tests / 6,483 lines.
13 IR tests / 942 lines stay, plus the staying `ir/display.rs` test
`codecs_without_an_sps_have_no_geometry` (12 lines).

`mpeg2_display.rs` has 13 tests / 536 lines and 9 own + 4 IR; it stays in
`transmux` entirely (e.g. `first_picture_raises_the_initial_event`,
`mid_gop_afd_change_through_the_demuxer`).

### 6.2 Integration tests, benches, fuzz, demo

Files under `transmux/tests/`, `transmux/benches/`, `fuzz/`, `demo/` that
reference a moving module by its `transmux::<module>` path:

| module | files |
|---|---|
| `sps` | `transmux/tests/it/{codec_config,dash,hevc_sps,paff_pairing,slice_header_pps,smooth,sps_profile_matrix}.rs`, `fuzz/fuzz_targets/transmux_interlace_join.rs`, `demo/src/mp4.rs` |
| `nal` | `transmux/tests/it/{hevc_damaged_parameter_sets,nal_keyframe,opengop_anchor,ps_demux_substreams,timed_metadata_sei_caption_fixture}.rs` |
| `annexb` | `transmux/tests/it/{cenc_decrypt,cenc_fragmented_fixture,codec_config,display_afd,display_fmp4_inband,dovi,hdr10,hdr10plus,hevc_damaged_parameter_sets,nal_keyframe,opengop_anchor,pmt_version_diff,rtp,sample_aes_h264_oracle,ts_to_cmaf}.rs`, `transmux/benches/annexb_scan.rs` |
| `au` | `transmux/tests/it/ps_demux_substreams.rs` |
| `slice_header` | `transmux/tests/it/{classify_nals,paff_pairing,slice_header_pps}.rs`, `fuzz/fuzz_targets/transmux_slice_header.rs` |
| `interlace` | `transmux/tests/it/{ir_labels,interlace_facts}.rs` |
| `hdr10plus` | `transmux/tests/it/{hdr10plus,ir_labels}.rs` |
| `hdr` | `transmux/tests/it/hdr10.rs` |
| `aac_asc` | `transmux/tests/it/{dash,smooth,ts_to_cmaf,flv,codecs_new,heaac_signalling,audio_metadata,ts_demux,mkv_mux,mpegh,rtp,segmenter,alloc_counts_sweep,rtcp_av_sync,streaming_tshls_config_change}.rs` |
| `ac3_syncframe` | `transmux/tests/it/{ac3_syncframe_ranges,audio_pes_straddle,dolby,eac3_bsi_walk,eac3_crc2_real_capture,eac3_real_capture,eac3_real_dependent_substreams,ir_timing,review_findings}.rs` |
| `dts_core` | `transmux/tests/it/dts_ts.rs` |
| `mpeg_legacy` | `transmux/tests/it/{mpeg_legacy,dash_audit22}.rs` |
| `user_data` | `transmux/tests/it/{user_data_t35,display_afd,display_ps}.rs` |
| `h262` | `transmux/tests/it/{display_afd,display_ps,display_fmp4_inband}.rs` |

Root-imported moving items (`use transmux::{NalCodec, …}`) add:
`transmux/tests/it/{idr_fallback,segment_cut_policy,sei_caption_real_capture,live_join_boundaries,live_join_fixtures,frame_rate,hevc_vps_timing,ts_hevc,ts_hevc_carriage,ts_hls,ts_mux,segmenter,segmenter_progress_bounds}.rs`
and `multimux/tests/it/{ts_hls_idr,ts_hls_idr_fallback}.rs`,
`multimux/tests/it/hdr10plus_route.rs`, `multimux/tests/it/hdr10_route.rs`.

Fuzz targets that exercise the moving code:
`fuzz/fuzz_targets/transmux_slice_header.rs` (`AvcParamSets`),
`fuzz/fuzz_targets/transmux_interlace_join.rs` (`decode_avc_slice_sps`,
`decode_hevc_interlace_sps`, `NalCodec`). Other fuzz targets touch only the
staying IR.

The 98-line `transmux/src/test_bits.rs` is used by six moving test modules
(§2). Its destination (new-crate test support vs a shared dev-dependency) must
be decided before the first test module moves.

## 7. Consumers

Uses of a moving public item *outside* `transmux/src/`, grouped by item. Counts
are whole-word occurrences in `.rs` files (excluding the private submodule and
build output), so a `use` line and each use it enables both count. The
`transmux` column means `transmux/tests`, `transmux/examples` and
`transmux/benches`, not the crate's own source.

| item | other crates | `transmux` tests/benches | total |
|---|---|---:|---:|
| `NalCodec` | `media-plane` 19, `multimux` 7, `fuzz` 5, `media-doctor` 4, `broadcast-client` 1 | 37 | 73 |
| `nal_unit_type` | `media-doctor` 13, `test-harness` 4, `multimux` 3, `broadcast-client` 1 (`isobmff`/`container-probe` hits are their own fields) | 41 | 62 real |
| `access_unit_is_idr` | `multimux` 7, `media-plane` 1 | 15 | 23 |
| `access_unit_is_keyframe` | — | 9 | 9 |
| `avc_access_unit_parameter_sets` | `multimux` 3, `broadcast-client` 1 | 0 | 4 |
| `avc_sps_dimensions`, `avc_access_unit_dimensions` | `broadcast-client` 1 each | 0 | 2 |
| `caption_cc_data` | — | 15 | 15 |
| `iter_length_prefixed_nals` | `multimux` 2, `broadcast-client` 1, `test-harness` 1, `media-doctor` 1 | 38 | 43 |
| `iter_annexb_nals` | `media-doctor` 10 | 18 | 28 |
| `annexb_to_length_prefixed` | `media-plane` 1 | 10 | 11 |
| `iter_length_prefixed_nals_with` | `media-doctor` 3 | 0 | 3 |
| `normalise_nal_length_size`, `length_prefixed_to_annexb`, `NAL_LENGTH_SIZE` | — | 4/4/1 | 9 |
| `AudioSpecificConfig` | `bindings` 4, `smooth-runtime` 4, `demo` 2, `test-harness` 2, `multimux` 1 | 44 | 57 |
| `AudioObjectType` | — | 5 | 5 |
| `SamplingFrequencyIndex` | — | 4 | 4 |
| `ChannelConfiguration` | — (the `isobmff`/`dvb-si` hits name their own fields) | 12 | 12 |
| `HeAacProfile` | `test-harness` 1 | 16 | 17 |
| `parse_adts_header` | `media-doctor` 3 | 6 | 9 |
| `build_adts_header` | `media-doctor` 1 | 0 | 1 |
| `Ac3SyncframeInfo` | `workspace-guards` 1 | 4 | 5 |
| `Ec3SyncframeInfo` | — | 16 | 16 |
| `split_ac3_syncframes`, `split_eac3_syncframes`, `split_ac3_syncframe_ranges`, `split_eac3_syncframe_ranges` | — | 1/14/15/4 | 34 |
| `DtsCoreFrameInfo` | `workspace-guards` 1 | 0 | 1 |
| `Hdr10PlusMetadata` | `multimux` 2 | 39 | 41 |
| `WindowRegion`, `PeakLuminanceTable`, `DistributionPoint`, `BezierToneMapping`, `ColorVolumeTransform`, `AdditionalWindow`, `OverlapProcessOption` | — | 4/7/4/6/3/5/9 | 38 |
| `HdrStaticMetadata`, `HEVC_PREFIX_SEI_NUT` | `multimux` 2 (`HEVC_PREFIX_SEI_NUT`) | 4/3 | 9 |
| `PicStruct`, `VideoScan`, `SourceScanType`, `PictureFacts` | — | 17/13/4/3 | 37 |
| `AvcParamSets` | `fuzz` 2 | 12 | 14 |
| `AvcPicture` | `workspace-guards` 2 | 17 | 19 |
| `AvcSliceHeader`, `AvcSlicePps`, `decode_avc_slice_pps`, `parse_avc_slice_header` | — | 1/0/9/5 | 15 |
| `decode_avc_sps` | `media-doctor` 6 | 8 | 14 |
| `decode_hevc_sps` | `media-doctor` 3 | 15 | 18 |
| `decode_hevc_vui_timing` | — | 15 | 15 |
| `rfc6381_avc1` | `bindings` 3, `demo` 2 | 3 | 8 |
| `rfc6381_hvc1`, `rfc6381_mp4a` | — | 4/0 | 4 |
| `Mpeg2SeqHeader`, `MpegAudioLayer`, `MpegAudioFrameHeader` | — | 3/7/5 | 15 |
| `AfdSignal`, `afd_from_t35_payload`, `bar_data_from_t35_payload`, `bar_data_from_ga94_payload` | — | 7/13/10/10 | 40 |
| `SequenceHeaderGeometry`, `SequenceDisplayExtension` | — | 5/7 | 12 |
| `AvcSpsCodecInfo`, `HevcNalUnitCodecInfo` | `media-doctor` 4 imports | 6 | 10 |
| `HevcRecordCodecInfo`, `VvcRecordCodecInfo` | `demo` 2 | 3 | 5 |
| `avc_config_from_sps_pps` | `multimux` 2, `broadcast-client` 1 | 0 | 3 (plus `smooth_codec.rs:48,119`, staying) |

Total lexical references outside `transmux/src`: **723**, of which 574 are in
`transmux/tests`/`examples`/`benches`, 6 are `workspace-guards` table entries,
and 143 are in other crates (`media-doctor` 50, `multimux` 31, `media-plane`
21, `fuzz` 11, `test-harness` 8, `broadcast-client` 7, `bindings` 7, `demo` 4,
`smooth-runtime` 4). This is the size of the import edit, not the number of
semantic changes: one `use` line can cover several sites.

The 723 figure excludes `isobmff` 56, `container-probe` 6, `dvb-si` 1,
`broadcast-smooth` 1 and `test-fixtures` 1, whose hits are same-named types or
doc mentions rather than imports of `transmux`.

### 7.1 The duplicates the design says die

| file | what it parses | moving function that replaces it |
|---|---|---|
| `container-probe/src/adts.rs:117-139` `adts_frame_len` | 6-byte ADTS sync + 13-bit `frame_length` | `aac_asc::parse_adts_header` + `AdtsHeader::frame_length` (moved `aac`). **Not equivalent**: `parse_adts_header` needs 7 bytes, validates `layer == 00`, and rejects `frame_length < header_len` (`aac_asc.rs:915-972`), while `adts_frame_len` checks 6 bytes and neither gate. The probe's scoring may change on short/hostile input |
| `container-probe/src/adts.rs:141-153` `longest_adts_chain` | chain walk + scoring | stays: it is the probe's evidence, not the header parse |
| `container-probe/src/adts.rs:155-180` `synthetic_ts_carrying_adts` | test fixture builder | stays |
| `container-probe/src/mp3.rs:173-208` `mp3_frame_len` | MPEG-1 Layer III header + frame length | `mpeg_legacy::MpegAudioFrameHeader::parse` (moved `mpeg-audio`). **Not equivalent**: the probe rejects MPEG-2/2.5 and Layers I/II, but the moved type exposes no MPEG-version accessor (`mpeg_legacy.rs:165-183`), so the probe cannot keep its gate through it |
| `container-probe/src/mp3.rs:161-171` `longest_mp3_chain` | chain walk + scoring | stays |
| `container-probe/src/mp3.rs:211-229` `id3_skip` | ID3v2 tag skip | stays (no bitstream equivalent) |
| `media-doctor/src/diagnostics/codec_common.rs:229-238` `has_adts_sync` | already calls `transmux::parse_adts_header` | no duplicate: import changes only |
| `media-doctor/src/diagnostics/codec_signalling.rs:8,317` | already calls `transmux::parse_adts_header` / `build_adts_header` | no duplicate: import changes only |

The design's claim that `media-doctor`'s `codec_common`/`codec_signalling`
"duplicate" the parser is contradicted by the code: both already delegate to
`transmux`. Only `container-probe` carries a second header parse.

## 8. Guard entries

There are 19 `workspace-guards/tests/guards/*_data.rs` tables. Two kinds of
entry matter: a line that pins a path or name in a moving file, and the
per-crate row a new workspace member needs.

### 8.1 Entries pinned to a moving file

| guard | data file | entries | what |
|---|---|---:|---|
| `magic_numbers` | `magic_numbers_data.rs:733-738` | 1 | `src/annexb.rs` allow entry for `NAL_LENGTH_SIZE_MINUS_ONE` |
| `truncation` | `truncation_data.rs:204-251` | 7 | `src/annexb.rs` 1 (`length_prefix`), `src/h262.rs` 6 (`u8::try_from` builders) |
| `label` | `label_data.rs:1804` | 1 | skips `AvcPicture` as a data-carrying ADT; `AvcPicture` moves, so the new crate's label table needs the same skip (or a `name()` if the design prefers) |
| `non_exhaustive` | `non_exhaustive_data.rs:196-202` | 0 paths, 1 comment | states `DtsCoreFrameInfo`/`Ac3SyncframeInfo`/`HevcSpsInfo` "stay in transmux" — directly contradicted by the design (§11) |

No other table pins a moving path. The entries in `bounded_input_data.rs:55`,
`clock33_data.rs:60`, `handroll_data.rs:39`, `lint_suppressions_data.rs:50`,
`secret_debug_data.rs` (`src/klv.rs`, `src/payload.rs`), `no_dom_data.rs:105`,
`one_box_home_data.rs:148`, `dep_policy_data.rs:21` are crate-level or name
files that stay.

### 8.2 Rows a new crate needs

Nine guard tables enforce `table_covers_every_workspace_member` and therefore
need a `broadcast-bitstream` row (or an `EXEMPT` with a reason):
`bounded_input`, `dep_policy`, `fixtures_one_home`, `label`, `lint_suppressions`,
`magic_numbers`, `non_exhaustive`, `one_box_home`, `truncation`.

Eight more use `table_names_are_workspace_members`, so an existing row must
name a real member but a new member is optional unless it has the property:
`clock33`, `deadlines`, `deleted_names`, `no_dom`, `panic_idioms`,
`serialize_mask`, `spawn_sites`, `tls_provider` (plus `one_ebml_home`).

The `broadcast-core` additions in §5 require a reviewed edit of
`workspace-guards/tests/guards/core_api_pin.txt` for `read_ue`/`read_se`.

## 9. Glue

| function | location | dependencies | design verdict |
|---|---|---|---|
| `avc_config_from_sps_pps` | `rtp_sdp.rs:167-215` | `isobmff::avc_config::{AVCConfigurationBox,AVCDecoderConfigurationRecord}`, `sps::{decode_avc_sps,is_high_profile}`, `annexb::NAL_LENGTH_SIZE_MINUS_ONE` | **holds**: moves to `broadcast-bitstream` behind `isobmff`. Signature takes `Vec<isobmff::AvcSps>`/`Vec<isobmff::AvcPps>`, not "parameter-set bytes" as the design says |
| HEVC counterpart of `avc_config_from_sps_pps` | **does not exist** | — | **contradicted**: the design names "its HEVC counterpart"; `grep -rn 'hevc_config_from\|hvc_config_from' transmux/src` returns nothing. This is new code, not a move |
| `avc_config_from_sprop` | `rtp_sdp.rs:128-147` | base64 (`crate::rtp::base64_decode`), `nal_unit_type`, the moved `avc_config_from_sps_pps` | **holds**: stays in `transmux` |
| `avc_config_from_fmtp` | `rtp_sdp.rs:280-285` | `fmtp_param` + `avc_config_from_sprop` | **holds**: stays |
| `rfc6381_avc1` | `sps.rs:2030-2051` | none | moves |
| `rfc6381_hvc1` | `sps.rs:2053-2097` | `HevcSpsInfo`, `broadcast_common::hex::hex_encode_upper` | moves, but brings the `broadcast-common` dependency (§11) |
| `rfc6381_vvc1` | `sps.rs:2099-2129` | `broadcast_common::hex::hex_encode_upper` | moves; VVC is not named in the design's feature table |
| `rfc6381_mp4a` | `sps.rs:2131-2136` | none | moves; it sits in `sps.rs`, so the `aac` feature depends on the `h264`/`hevc` file (§10) |
| `rfc6381_codec_string` | `dash.rs:570-572` | IR `CodecConfig`, `DashPackager::codec_string` (`dash.rs:616-680`), the moved helpers | **holds**: stays; it dispatches on the IR type and calls the moved per-codec helpers |
| `rfc6381_track_codec_string` | `dash.rs:605-614` | IR `TrackSpec`, Dolby Vision descriptor | **holds**: stays |

`avc_config_from_sps_pps` is the only glue whose move is both stated and
supported by the code. The HEVC half must be written. `rfc6381_codec_string`'s
verdict holds because the code already separates the IR dispatch from the
per-codec helpers.

## 10. Proposed PR sequence

The design's order is kept, with `sps` split out because it alone is larger
than every other PR. Sizes are non-test/test lines from §1; "call sites" is the
per-module external reference count from §7 (a `use` line can cover several, so
treat it as the size of the import edit).

| PR | contents | files | non-test | test | call sites | depends on |
|---|---|---:|---:|---:|---:|---|
| 1 | bit reader + RBSP + NAL/Annex B + AU splitter: `bitreader.rs`, `rbsp.rs`, `nal.rs`, `annexb.rs`, `au.rs` | 5 | 1,895 | 1,822 | 289 | — |
| 2a | `sps.rs` + the 11 coded-fact `ir/display.rs` types | 2 | 3,067 | 1,554 | 88 + all `ir` sites | PR1 |
| 2b | `slice_header.rs` + the moving half of `interlace.rs` | 2 | 863 | 202 | 96 | PR2a (slice header imports `sps`) |
| 2c | `hdr10plus.rs` + the moving half of `hdr.rs` | 2 | 1,004 | 289 | 84 | PR1 |
| 2d | `nalu_ext.rs` + `codec_config_ext.rs` (the `isobmff` feature) | 2 | 162 | 66 | 21 | PR2a |
| 3 | `aac_asc.rs` | 1 | 1,237 | 646 | 95 | PR1, PR2a (`rfc6381_mp4a` lives in `sps.rs`) |
| 4 | `ac3_syncframe.rs` + `dts_core.rs` | 2 | 1,739 | 1,609 | 61 | PR1, the `isobmff` feature from PR2d |
| 5 | `mpeg_legacy.rs` (split), `user_data.rs`, `h262.rs`, and the `container-probe` change | 3 + 2 edited | 982 | 295 | 55 + 2 probe files | PR1, PR2a (IR display types), PR3 (ADTS for `container-probe`) |

Totals: 10,949 non-test and 6,483 test lines move; 723 external references
change; 18 moving files plus 2 `container-probe` files edited and 1
`media-doctor` import pair edited.

Ordering that forces a different sequence:

- `aac` cannot precede `sps` unless `rfc6381_mp4a` is first relocated out of
  `sps.rs` (`aac_asc.rs:561`).
- `ac3`/`dts` need the `isobmff` feature that PR2d establishes; if PR2d is
  deferred, the feature must be introduced in PR1 instead.
- `container-probe`'s ADTS replacement needs PR3; its MPEG-audio replacement
  needs PR5's `mpeg-audio` module. So the `container-probe` edit belongs at the
  end of PR5, not before it.
- PR2a is still 4,621 lines including tests. If one PR must be smaller, split
  it again: 2a-i `sps.rs` (3,716 lines) and 2a-ii the `ir/display.rs` types
  (843 non-test + 62 test), with 2a-ii first only if `sps` is to return the
  moved types; otherwise 2a-ii follows 2a-i.

The design's PR "H.264 and HEVC" is 5,096 non-test lines if taken literally
(`sps` + `slice_header` + `interlace` + `hdr10plus` + `hdr` + the two glue
files + the `ir` types). That is 46.5% of all moving non-test code; the split
above is the recommendation.

## 11. Contradictions and risks

Each item is a code fact the design excerpt does not cover, or covers wrongly.

1. **The new crate cannot depend only on `broadcast-core`.** Five moving
   modules import `broadcast-common`: `nal.rs:45`, `ac3_syncframe.rs:23`,
   `mpeg_legacy.rs:27`, `user_data.rs:6`, and `sps.rs:2033,2083,2123`. The core
   crate has no `wire` or `hex` module (`broadcast-core/src`). Either those
   helpers move to `broadcast-core` (API-pin additions) or the design's
   dependency rule is relaxed to `broadcast-core` + `broadcast-common`.
2. **`isobmff` is needed by more than the two config builders.** The design
   gives an `isobmff` feature only for `avc_config_from_sps_pps`/its HEVC
   counterpart. In the code, `ac3_syncframe.rs:312,777` build
   `isobmff::ac3::{Ac3SpecificBox,Ec3SpecificBox}`; `dts_core.rs:304` builds
   `isobmff::dts::DtsSpecificBox`; `hdr.rs:19,98,113-137` uses
   `isobmff::hdr_boxes` and `HEVCDecoderConfigurationRecord`; `nalu_ext.rs:9`
   and `codec_config_ext.rs:12-14` implement traits over `isobmff` types.
   `no_std` `AC-3`/`DTS`/`HDR` parsing would drag `isobmff` in even though the
   design presents those as plain parsers.
3. **Two glue files say, in their own docs, that they stay.** `nalu_ext.rs:4-5`
   ("the SPS bitstream parsers, which stay in transmux") and
   `codec_config_ext.rs:4-6` ("which stay in transmux"). The design lists both
   as likely seams. The code supports the design's move (they are trait impls
   over `isobmff` types), but the doc comments must be rewritten.
4. **The `non_exhaustive` guard currently asserts the opposite.**
   `non_exhaustive_data.rs:196-202` records that `DtsCoreFrameInfo`,
   `Ac3SyncframeInfo` and `HevcSpsInfo` "stay in transmux". The design moves
   all three; the comment and the table's rationale must change with the first
   PR that moves one of them.
5. **The private-field rule breaks 11 struct literals outside the modules.**
   The design bans public fields, but `Hdr10PlusMetadata` has 7 literals,
   `WindowRegion` 2, and `HevcSpsInfo` 1 (`codec_config_ext.rs:30`) and
   `ColorVolumeTransform` 1 (`transmux/tests/it/hdr10plus.rs:196`). Every one needs a
   constructor, and every `.field` read becomes an accessor call (the raw
   counts are in §4). `codec_config_ext.rs:30` is the hardest: it
   constructs `crate::sps::HevcSpsInfo` field by field from an `hvcC` record, so
   the new crate must expose a constructor that fields an SPS from a record.
6. **There is no `Sps` type for the design's named constructor.**
   `DisplayGeometry::from_avc_sps` (`ir/display.rs:906`) and `from_hevc_sps`
   (912) take `&[u8]` and call `sps::decode_*_display_facts`. The design says
   `DisplayGeometry::from_avc_sps` "takes the moved `Sps`"; no such type
   exists. The closest is `pub(crate) DisplayFacts` (`sps.rs:417-441`), which
   itself holds IR types (`PictureRect`, `AspectRatioIdc`, `VideoSignalType`).
   A new public fact type is required either way.
7. **`interlace.rs` has an inherent impl on an IR type.**
   `impl CodecConfig` starts at `interlace.rs:294` and defines
   `CodecConfig::video_scan`; it cannot move. The moving `avc_facts`/
   `hevc_facts` (414, 465) are free functions and can. The split is at that
   line, not by feature.
8. **`h262.rs` has the MPEG-2 counterpart of the same problem.**
   `SequenceHeaderGeometry::to_display_geometry` (`h262.rs:208-236`) is an
   inherent method on a moving type that builds `DisplayGeometry`. The type
   moves, the method stays; `transmux` needs a free conversion, which the
   design does not name.
9. **The no-raw-byte rule is violated by existing public API.**
   `annexb_to_length_prefixed` (`annexb.rs:150`), `length_prefixed_to_annexb`
   (`annexb.rs:266`), `normalise_nal_length_size` (`annexb.rs:247`),
   `build_adts_header` (`aac_asc.rs:883`), `AudioSpecificConfig::to_adts_header`
   (`aac_asc.rs:985`), `Hdr10PlusMetadata::to_hevc_prefix_sei_nal`
   (`hdr10plus.rs:689`) and `hdr::to_hevc_declarative_sei_nal` (`hdr.rs:69`) all
   return raw `Vec<u8>`/`[u8; 7]`. Moving them under the design's own API rules
   requires typed wrappers or a documented exception per function.
10. **`container-probe` cannot delete `adts.rs`/`mp3.rs`.** Both files also hold
    the chain walk and scoring (`longest_adts_chain`, `longest_mp3_chain`,
    `probe`, `need_at_least`) and a synthetic fixture builder
    (`adts.rs:155`). Only `adts_frame_len` (`adts.rs:117`) and `mp3_frame_len`
    (`mp3.rs:173`) are duplicate header parses. The design's "deletes `adts.rs`
    and `mp3.rs`" overstates the change.
11. **The MP3 probe gate has no accessor to preserve it.**
    `container-probe/src/mp3.rs:181-184` rejects anything but MPEG-1 Layer III;
    the moved `MpegAudioFrameHeader` exposes `layer`, `sample_rate`, `channels`,
    `bitrate_bps`, `frame_length`, `samples_per_frame` (`mpeg_legacy.rs:165-183`)
    and no MPEG-version field. Replacing `mp3_frame_len` with the moved parser
    widens what the probe accepts unless a version accessor is added.
12. **ADTS strictness differs between the two parsers.**
    `parse_adts_header` needs 7 bytes, checks `layer == 00`, and rejects a
    `frame_length` smaller than the header (`aac_asc.rs:915-972`).
    `container-probe/src/adts.rs:117-139` needs 6 bytes and checks neither.
    Reuse changes probe verdicts on truncated input.
13. **`media-doctor` has no duplicate to remove.** `codec_common.rs:229-238`
    and `codec_signalling.rs:8,317` already call `transmux`. The design's
    duplicate claim is false; the work is an import edit.
14. **`rfc6381_mp4a` lives in `sps.rs`.** `aac_asc.rs:561` calls
    `crate::sps::rfc6381_mp4a`. Moving AAC before SPS requires relocating that
    one function first, otherwise the `aac` feature depends on the `h264`/`hevc`
    module.
15. **VVC is inside the files the design calls `h264`/`hevc`.** `NalCodec::Vvc`
    (`nal.rs:119`), `decode_vvc_sps`/`VvcSpsInfo`/`rfc6381_vvc1`
    (`sps.rs:1844-1929`) are not covered by a `vvc` feature in the design's
    table. Either `vvc` is accepted under `hevc`, or the table needs a third
    feature.
16. **`mhas.rs` is missing from the design's table.** It is 174 non-test lines
    of MHAS packet framing with no `crate::` imports (`mhas.rs:1-40`), a clean
    move, but no feature is assigned.
17. **`user_data.rs` is listed only under `mpeg2`, but AVC/HEVC use it.**
    `display_track.rs:449-486` calls `user_data::afd_from_t35_payload` and
    `bar_data_from_t35_payload` for the H.264/H.265 path, and
    `caption_detect.rs:223` uses `GA94_USER_IDENTIFIER`. The module must be
    available whenever the display/caption paths are, not only with `mpeg2`.
18. **The HEVC `avcC` counterpart does not exist.** The design says
    `avc_config_from_sps_pps` "and its HEVC counterpart" move; only the AVC
    function exists (`rtp_sdp.rs:167`). The HEVC one is new code.
19. **`avc_config_from_sps_pps` takes typed newtypes, not bytes.**
    `rtp_sdp.rs:167` takes `Vec<isobmff::AvcSps>` and `Vec<isobmff::AvcPps>`.
    The design's "from parameter-set bytes" describes a different signature.
20. **The `annexb` helpers are `pub(crate)`.** `START_CODE_PREFIX_LEN`
    (`annexb.rs:22`) and `find_start_code_prefix` (`annexb.rs:71`) are used by
    `h262.rs:31`, `mpeg_legacy.rs:24` and `au.rs:38`. A new crate cannot import them as
    `pub(crate)`; they must become public or be duplicated.
21. **The owning reader and the core reader have different shapes.**
    `transmux`'s `BitReader` owns `Vec<u8>` (`bitreader.rs:113-116`) so it can
    hold unescaped RBSP; `broadcast_core::bits::BitReader<'a>` borrows
    (`bits.rs:123`). The design's "built on `broadcast_core::bits::BitReader`"
    is achievable only by keeping an owning wrapper that constructs a core
    reader per call, as `bitreader.rs:166-192` already does.
22. **`sps.rs`'s public structs are not `#[non_exhaustive]`.** The source says
    so explicitly (`sps.rs:413-414`: "neither is `#[non_exhaustive]`"). The
    design requires every public struct/enum to be. Adding the attribute is
    breaking for any downstream struct literal, which is why item 5's 11 sites
    are not the only cost.
23. **`test_bits.rs` is a test-only cross-module dependency.** 98 lines
    (`lib.rs:238-239`) used by six moving test modules. It has no home in the
    design.
24. **The re-export deletion is the widest change.** `lib.rs:278-440`
    re-exports the moving items; §7 counts 723 references outside
    `transmux/src`. A patch release cannot absorb this. The design records it
    under `[Unreleased]`; the version plan must say which crates take a major
    (or epoch) bump.

### What was checked and found consistent

- `AudioSpecificConfig` and `Hdr10PlusMetadata` really are the two moving
  types with a symmetric serializer: `aac_asc.rs:741,819` and
  `hdr10plus.rs:698,813`.
- `rfc6381_codec_string` really does dispatch on the IR `CodecConfig` and call
  the per-codec helpers (`dash.rs:616-680`), so the design's split is correct.
- `avc_config_from_sprop`/`_fmtp` really do depend on base64 and SDP parsing
  (`rtp_sdp.rs:128-147,280-285`), so they stay.
- `mpeg2_display.rs`, `audio_profile.rs` and `smooth_codec.rs` really are not
  bitstream parsers (§1).
- The design's PR order is acyclic; the only forced changes are the
  `rfc6381_mp4a` placement (item 14) and the `isobmff` feature timing (item 2).

## 12. Totals

| measure | count |
|---|---:|
| non-test lines that move | 10,949 |
| test lines that move | 6,483 |
| moving files | 18 (plus the `units.rs` and `ir/display.rs` splits) |
| external references that change | 723 |
| public fields to convert | 190 in candidate modules; 204 with the `ir` structs |
| struct literals needing constructors | 11 |
| `broadcast-core` additions | `read_ue`, `read_se` (minimum); optional cursor helpers |
| `container-probe` files edited | 2 (`adts.rs`, `mp3.rs`); neither is deleted |
| `media-doctor` files edited | 2 imports (`codec_common.rs`, `codec_signalling.rs`) |

The three biggest risks, in order: the `broadcast-common`/`isobmff` dependency
rule (§11 items 1-2), the private-field conversion (§11 items 5 and 22), and
the re-export deletion blast radius (§11 item 24).
