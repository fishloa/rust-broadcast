# Streaming-first Phase 1 (extract `isobmff`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every ISO BMFF box type lives in a new crate `isobmff` that depends only on `broadcast-common`; transmux and every other crate import it from there; `mp4-emsg` is absorbed.

**Architecture:** The mature box code moves unchanged in bytes and behaviour. Six sequential slices, each leaving the whole workspace building and every golden byte-identical: first cut the seams inside transmux, then move leaf boxes, emsg, codec configuration boxes, the sample-entry and `moov` cluster, and finally repoint the hand-rolled walkers and clean up. No re-export shim at any point: a slice that moves a file updates every importer in the same change.

**Tech Stack:** Rust 1.95, `broadcast-common` (`Parse`/`Serialize`, `len`, `bits`, shared error kinds), `thiserror`, optional `serde`.

**Spec:** `docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md` (§4.1, §5 Phase 1, §10 Q6).

**Map:** `docs/superpowers/plans/2026-11-24-streaming-first-phase1-recon.md` (file-by-file classification, dependency edges, consumers, guard tables). Section numbers below ("recon §n") refer to it. Its line numbers are from the commit it was made on; find items by name.

**Tracking issue:** #4057.

## Decisions (made by the orchestrator; binding for every slice)

1. **Errors.** `isobmff::Error` is its own `#[non_exhaustive]` thiserror enum. It wraps the broadcast-common kinds it needs (`BufferTooShort`, `OutputBufferTooSmall`, `InvalidValue`, `InvalidField`, `FieldOverflow`, `UnsupportedVersion`) with `#[error(transparent)] X(#[from] …)`, and has its own variants for what the box code raises that no shared kind fits: `InvalidInput(&'static str)`, `UnexpectedBox`, `LargesizeBufferTooShort`, `UuidBufferTooShort`, `BoxSizeUnderflow`, `SampleDataBudgetExceeded`, `ZeroWidthTrunBudgetExceeded`, `UnsupportedFeature`, and emsg's `NotEmsg`, `InvalidSize`, `InvalidString`. Each keeps the fields and the `Display` text it has in transmux or mp4-emsg today.
   `transmux::Error` keeps every variant it has. It gains `impl From<isobmff::Error> for transmux::Error` that maps **variant for variant** onto the existing transmux variants (never a single wrapper variant), so the ~1000 existing `Error::X` matches in transmux, media-plane and dash-runtime keep compiling and keep their meaning. The emsg variants map onto transmux's existing `EmsgSerialize`.
2. **Byte-cursor helpers.** `transmux/src/wire_cursor.rs` moves to `broadcast-common` as a public module `wire` (an additive minor for broadcast-common). It is used by box code and by twelve non-box transmux files, so it belongs below both. The helpers return broadcast-common's own error kinds; each crate converts with `?`. `mp4-emsg`'s private `read_u32`/`read_u64`/`read_u24_at` copies are deleted in favour of it. Before adding a helper, check `broadcast-common` for one that already does the job and extend that instead.
3. **`TrackId`.** `isobmff::TrackId` is the 14496-12 `track_ID`: a `u32` newtype with `new`, `value`, `next`, `Ord`, `Hash`, `serde(transparent)`, and `Debug`/`Display` that print the bare number, exactly as `transmux::ir::TrackId` does today (golden tests hash `Debug` output). `transmux::TrackId` stays the IR identity and gets `From` conversions in both directions.
4. **What moves beyond the spec's list.** `frag_offsets` moves (pure box-level addressing rules). The `mdcv`/`clli` body types in `hdr.rs` move. `validate.rs` (the CMAF conformance validator) stays in transmux for now. `protect_init_segment`, `protect_media_segment`, `FragmentProtection`, `ConstantIvSenc`, `media_fragment_run` and `MediaRunSample` stay in transmux (they are the encryptor's and the pipeline's policy); the box-level `protect_sample_entry` moves.
5. **Codec strings.** `rfc6381_*` formatting stays in transmux. `HEVCDecoderConfigurationRecord::rfc6381`, `VvcDecoderConfigurationRecord::{dimensions, rfc6381}`, `AvcSps::{decode, rfc6381}`, `HevcNalUnit::{decode_sps, rfc6381}` and `EsdsBox::audio_specific_config` become transmux-side extension traits or free functions over the moved types' public fields.
6. **Bitstream constructors.** `Ec3SpecificBox::from_syncframes`, `Ac3SyncframeInfo::into_dac3`, `Ec3SyncframeInfo::into_dec3` and `DtsCoreFrameInfo::into_ddts` stay in transmux as functions that build the moved box through its public fields.
7. **container-probe** keeps its two small box walkers: it must depend only on broadcast-common at runtime. It takes `isobmff` as a **dev-dependency** and a drift test pins its header constants and its walk against `isobmff` on the ISO BMFF fixtures, as it already does for mpeg-ts. The other six walkers move onto `isobmff`.
8. **`mp4-emsg`.** The crate directory is deleted in slice 3; its history moves to `docs/history/mp4-emsg/` (as `rtcp-packet`'s did). `timed-metadata`'s public `Error::Emsg` then carries `isobmff::Error`: recorded as breaking under its `[Unreleased]`. No version numbers change in this phase and nothing is released; the release workflow `release-mp4-emsg.yml` is deleted with the crate.
9. **`serde`.** `isobmff` has an optional `serde` feature carrying the moved types' existing `cfg_attr` derives and `ser_timescale`. transmux's `serde` feature enables `isobmff/serde`.
10. **Visibility.** A `pub(crate)` helper that a staying file needs becomes `pub` in `isobmff` with a doc line. Nothing else changes visibility.

## Global Constraints

- "Unchanged" means wire bytes and behaviour. Signatures change only where a decision above says so. No redesign, no renames beyond the crate path, no drive-by fixes.
- `isobmff` depends on `broadcast-common` (and `thiserror`, optional `serde`) and nothing else in the workspace. It is `no_std` + `alloc` with a default `std` feature, like the crates it replaces parts of.
- No re-export of a moved item from transmux, at any slice. Every importer is repointed in the same change.
- Every golden and fixture round-trip in the workspace stays byte-identical.
- A moved file keeps its in-file unit tests. A test file under `transmux/tests` moves only if it compiles against `isobmff` alone; otherwise it stays.
- Guard tables: when a file moves, every `workspace-guards/tests/guards/*_data.rs` row and `broadcast-common/tests/it/ordering_guard.rs` path that names it is updated in the same change, with its reason kept (recon §8). The ordering guard skips silently on a missing path, so check each path you change resolves.
- Rules that apply to all code touched: no numeric literal other than 0 and 1 outside a constant definition (the `magic_numbers` guard enforces it); names say the field or role; no `#[allow]`/`#[expect]`; no `.unwrap()`/`.expect()`/`panic!` in non-test code; zero warnings; no comments that narrate code; a constant or import used only under a feature carries that feature's `cfg`; a generated file is changed through its generator.
- Spec citations name the edition where the file's other citations do; a clause number comes only from an existing citation or from `private/specs-verified/`.
- No version number changes. CHANGELOG entries go under `[Unreleased]`: `### Breaking` for a moved public path, `### Added` for a new public item.
- Commit after every green step. No Claude attribution.
- Never `pkill`/`killall`. Never run cargo over the whole workspace except where a gate says so.

**Gate for every slice** (run all; loop until green):
1. `cargo fmt --all --check`
2. For `isobmff`, `transmux` and every other crate whose files changed: `RUSTFLAGS="-D warnings" cargo clippy -p <crate> --all-features --all-targets --locked -- -D warnings`; `cargo test -p <crate> --all-features --locked`; `RUSTFLAGS="-D warnings" cargo build -p <crate> --no-default-features --locked` and once per declared feature (`--no-default-features --features <one>`); `RUSTFLAGS="-D warnings" cargo build -p <crate> --examples --all-features --locked`; `RUSTDOCFLAGS="-D warnings" cargo doc -p <crate> --all-features --no-deps --locked`.
3. `cargo build -p media-doctor --examples --locked` (it builds transmux with `cli` and without `cenc`).
4. `cargo test -p workspace-guards --test guards --locked`; `cargo test -p broadcast-common --all-features --locked --test it ordering_guard`.
5. `python3 tools/check-citations.py`; `python3 tools/gen-crate-index.py` then `git diff --exit-code docs/CRATES.md` (commit the regenerated file if it changed).
6. Test counts: record `cargo test -p transmux --all-features --locked` pass count before the slice; after it, transmux's count plus isobmff's count equals the count before plus any tests the slice adds. State the three numbers.
7. Byte identity: `cargo test -p transmux --all-features --locked golden` and `cargo test -p multimux --all-features --locked --test it golden` pass with no golden file changed (`git status --porcelain fixtures private` is empty).

## Review Focus

1. **A consumer that matches a box error by variant** (media-plane `ts_session.rs` builds `transmux::Error::LargesizeBufferTooShort`; dash-runtime builds `transmux::Error::InvalidValue`): must still compile and still match after the box code returns `isobmff::Error`. Slice 2 adds a test in transmux that converts every `isobmff::Error` variant and asserts the transmux variant and its `Display` text.
2. **Golden hashes that include a `TrackId`**: `Debug` of `isobmff::TrackId` must equal today's. Slice 2 adds a test asserting `format!("{:?}", …)`, `format!("{}", …)` and the serde form for both id types.
3. **A feature combination that leaves a moved constant or import dead** (`transmux` with `cli` and no `cenc`; `isobmff` with no `std`; `serde` on one crate and not the other): gate step 2 builds each feature alone; slice 2 adds `isobmff` to `tools/check-bare-features.sh`'s list if that script enumerates crates.
4. **A guard that silently stops checking a moved file** (the ordering guard skips a missing path; a per-crate guard table has no `isobmff` row): slice 2 adds `isobmff` to every guard table that has a transmux or mp4-emsg row or is covered by `table_covers_every_workspace_member`, and each later slice moves the rows for the files it moves.
5. **`no_std` consumers of emsg** (`mpeg-ts` dev-dependency, `timed-metadata` without `std`): after slice 3 they build with `--no-default-features`; gate step 2 covers it for both crates.

---

### Slice 1: seams inside transmux (no new crate)

**Files:** `transmux/src/**` only, plus `transmux/CHANGELOG.md`.

**Produces:** a transmux in which every later slice is a file move.

- [ ] **Step 1: Record the baseline.** `cargo test -p transmux --all-features --locked` pass count; `git grep -c "use crate::init_segment" -- transmux/src`.
- [ ] **Step 2: Extract the box helper layer** from `init_segment.rs` (the flat-table, child-walk and container-serialise helpers and `OpaqueBox`: `bounded_entry_count`, `check_entry_count`, `parse_flat_table`, `serialize_flat_table`, `flat_table_len`, `ChildBox`, `walk_children`, `ChildOrder`, `serialize_children`, `serialize_container`, `SerializeBox`, `box_body`, `OpaqueBox`) into a new module `transmux/src/box_support.rs`. Repoint `hevc_config`, `sample_groups`, `timing`, `subtitle_entries`, `track_reference` and `init_segment` itself. Gate step 2 for transmux; commit.
- [ ] **Step 3: Split each mixed module at the box boundary, in place**, one commit each, box half keeping the original file name and the bitstream half going to a new sibling file:
  - `ac3.rs` (box: `Ac3SpecificBox`, `Ec3SpecificBox`, `Ec3Substream` and their constants) / `ac3_syncframe.rs` (BSI parsers, CRC, splitters, and the three constructors of decision 6 as free functions).
  - `dts.rs` (box: `DtsSpecificBox` and fourcc constants) / `dts_core.rs` (`DtsCoreFrameInfo`, splitters, `into_ddts` as a function).
  - `mpegh.rs` (box: `MHADecoderConfigurationRecord`, `mhaC`) / `mhas.rs` (MHAS walker, `find_mpegh3da_config`).
  - `dovi.rs` (box: the record, `dvcC`/`dvvC`, profile enums) / `dovi_descriptor.rs` (TS descriptor).
  - `hdr.rs`: `ChromaticityXy`, `MasteringDisplayColourVolume`, `ContentLightLevel` to `hdr_boxes.rs`; the SEI carrier stays in `hdr.rs`.
  - `nalu_types.rs` (wrappers and `VALID_LENGTH_SIZES` stay in the file) with `decode`/`rfc6381`/`decode_sps` moved to extension traits in `nalu_ext.rs`.
  - `hevc_config::rfc6381`, `vvc_config::{dimensions, rfc6381}`, `mp4esds::audio_specific_config` to transmux-side extension functions (decision 5); make `EsdsBox::require_decoder_specific_info` (or the accessor that method uses) `pub`.
  - `cenc.rs`: `ConstantIvSenc` moves next to its consumer in transmux (`cenc_encrypt.rs`).
  - `init_segment.rs`: `protect_init_segment` moves to a transmux file that keeps taking the IR `TrackEncryption` and calls the box-level `protect_sample_entry`.
  - `movie_fragment.rs`: `media_fragment_run`, `MediaRunSample`, `protect_media_segment`, `FragmentProtection` to `fragment_build.rs`.
  - `avc_parameter_set.rs`: replace its one use of `annexb::iter_length_prefixed_nals_with` with a local length-prefixed walk over its own bytes, or take the iterator from the caller.
- [ ] **Step 4: Cut the small cross-cutting edges.** `visual_ext` gets its own named rounding constant in place of `units::ROUNDING_DIVISOR`; `vvc_config` and `avs3_audio` get local bit-error helpers over `broadcast_common::bits` in place of `crate::bitreader::{bit_error, bytes_needed, write_bits_checked}`.
- [ ] **Step 5: Verify the seam.** For every file recon §1 marks MOVE, plus the box half of every SPLIT file: `grep -n "crate::" <file>` shows only `crate::error`, `crate::wire_cursor`, `crate::TrackId`/`crate::ir::TrackId`, `crate::ser_timescale` and other MOVE/box-half modules. List any other edge in the final message.
- [ ] **Step 6: Gate** (all seven steps; step 6's three numbers: before, after, tests added = 0). Commit `refactor(transmux): cut the box/bitstream seams ahead of the isobmff move`.

### Slice 2: the crate, the shared pieces, and the leaf boxes

**Files:** create `isobmff/` (Cargo.toml, README.md, CHANGELOG.md, `src/lib.rs`, `src/error.rs`, `src/track_id.rs`, the moved modules, `tests/it/`), root `Cargo.toml` members; `broadcast-common/src/wire.rs` + `lib.rs`; move from transmux: `box_types.rs`, `box_support.rs`, `segments.rs`, `timing.rs`, `visual_ext.rs`, `cenc.rs` (boxes), `sample_groups.rs`, `frag_offsets.rs`, `movie_fragment.rs` (boxes); repoint transmux, dvr-runtime, dash-runtime, hls-runtime, multimux, media-doctor, media-plane, fuzz `transmux_cenc_boxes`; guard tables; `Cargo.lock`, side lockfiles (`demo/Cargo.lock`, `tools/differential-sweep/Cargo.lock` via `cargo update -p <crate> --offline` in each directory when cargo asks).

**Interfaces produced:** `isobmff::{Error, Result, TrackId}`; `isobmff::box_types::*`; the moved modules under their existing module names; `broadcast_common::wire::*`; `impl From<isobmff::Error> for transmux::Error`; `impl From<isobmff::TrackId> for transmux::TrackId` and the reverse.

- [ ] **Step 1: Failing tests first**, in transmux (`transmux/tests/it/isobmff_bridge.rs`):
  - `every_isobmff_error_maps_to_its_transmux_variant`: one case per `isobmff::Error` variant, asserting the transmux variant and that `to_string()` is unchanged from today's text for that failure.
  - `track_ids_print_and_serialise_alike`: `Debug`, `Display` and (feature `serde`) the JSON form are equal for both id types; the conversions round-trip.
  And in broadcast-common (`broadcast-common/tests/it/wire.rs`): each helper reads the value at an offset, and returns `BufferTooShort` with `need`/`have` when the slice is short.
- [ ] **Step 2: `broadcast_common::wire`** (decision 2). Move `wire_cursor`'s helpers; transmux's twelve non-box users and the box files import it from there; delete `transmux/src/wire_cursor.rs`.
- [ ] **Step 3: Crate skeleton** with `Error` (decision 1), `TrackId` (decision 3), `ser_timescale` under the `serde` feature, crate-root docs citing ISO/IEC 14496-12:2026, `#![no_std]` + `alloc` with default `std`.
- [ ] **Step 4: Move the nine modules** with `git mv`, fix paths, repoint every importer listed in recon §6 for these modules. transmux's `error.rs` gains the `From` impl; `transmux::Error`'s own variants are untouched.
- [ ] **Step 5: Guard tables.** Add an `isobmff` row to every table that needs one (recon §8: `truncation`, `label` with the moved enums' skip entries, `non_exhaustive`, `lint_suppressions`, `handroll` shape 1 with `allow: []`, `dep_policy` purposes, `magic_numbers`, and any table `table_covers_every_workspace_member` reports). Move the transmux rows that name moved files.
- [ ] **Step 6: Move the tests** that compile against `isobmff` alone for these modules (recon §7 "box" and "box-after-cut" for box_types, timing, visual_ext, cenc boxes, sample groups, frag offsets, trun budgets). Each moved test keeps its assertions.
- [ ] **Step 7: Fuzz.** `fuzz_targets/transmux_cenc_boxes.rs` targets `isobmff`; rename it `isobmff_cenc_boxes.rs` and update `fuzz/Cargo.toml` and the CI fuzz list that names it.
- [ ] **Step 8: CRATE-ACCEPTANCE for the new crate** as far as this slice's contents allow: README with a coverage table of the boxes it holds, two examples (`parse_boxes.rs`: walk the top-level boxes of a file given on the command line and print type and size; `build_ftyp.rs`: build and serialise an `ftyp`), CHANGELOG `[Unreleased]`, `docs/CRATES.md` regenerated, root `CLAUDE.md` crate list entry.
- [ ] **Step 9: Gate.** Commit `feat(isobmff): new crate holding the ISO BMFF box layer` (several commits are fine; this is the last).

### Slice 3: absorb `mp4-emsg`

**Files:** `mp4-emsg/**` (deleted), `isobmff/src/emsg.rs` (+ version types), `docs/history/mp4-emsg/CHANGELOG.md`, transmux (`pipeline.rs` re-export deleted, `error.rs`), timed-metadata, ssai-runtime, playout-runtime, mpeg-ts (dev-dep), fuzz (`emsg.rs`, `timed_metadata_convert.rs`, `Cargo.toml`), `.github/workflows/ci.yml` crate lists, `.github/workflows/release-mp4-emsg.yml` (deleted), `tools/prepare-release.py`, `fixtures/scte35/PROVENANCE.md` commands, guard tables, root `Cargo.toml`, `CLAUDE.md`, `docs/CRATES.md`.

- [ ] **Step 1:** `git mv` `mp4-emsg/src/{emsg,version}.rs` into `isobmff/src/`; its tests into `isobmff/tests/it/`; its examples into `isobmff/examples/`. Fold `mp4_emsg::Error` into `isobmff::Error` (decision 1). Replace its private `read_u32`/`read_u64`/`read_u24_at` with `broadcast_common::wire`.
- [ ] **Step 2:** Public paths become `isobmff::emsg::{EmsgBox, PresentationTime, EmsgVersion, …}` with the same names and constants. Repoint every dependent in recon §4. `transmux::pipeline` no longer re-exports emsg types; its users import from `isobmff`.
- [ ] **Step 3:** `timed-metadata`: `Error::Emsg(#[from] isobmff::Error)`; CHANGELOG `### Breaking`. Its `std`/`serde` features forward to `isobmff/std`, `isobmff/serde`.
- [ ] **Step 4:** Delete the crate directory, its workspace member line, its release workflow, its guard-table rows (folding their content into the `isobmff` rows), and its mentions in CI crate lists and `tools/prepare-release.py`. Move its CHANGELOG to `docs/history/mp4-emsg/CHANGELOG.md` with a first line saying the crate was absorbed into `isobmff`. Editing workflow YAML: change only the exact list entries; never reformat the file.
- [ ] **Step 5: Gate**, with gate step 2 also run for `timed-metadata`, `ssai-runtime`, `playout-runtime`, `mpeg-ts`. `git grep -n "mp4-emsg\|mp4_emsg"` returns only `docs/history/` and CHANGELOG history lines. Commit `refactor!: absorb mp4-emsg into isobmff`.

### Slice 4: codec configuration boxes

**Files:** move from transmux: `avc_config.rs`, `hevc_config.rs`, `vvc_config.rs`, `nalu_types.rs` (wrappers), `opus.rs`, `flac.rs`, `ac4.rs`, `ac3.rs` (box half), `dts.rs` (box half), `mpegh.rs` (box half), `dovi.rs` (box half), `hdr_boxes.rs`, `mp4esds.rs`, `avs2.rs`, `avs3_audio.rs`, and the config-box parts of `av1.rs` (`av1C`), `vp9.rs` (`vpcC`) and `avs3.rs` (`av3c`/`lavc`); the sample-entry parts of those three stay until slice 5. Repoint transmux (`rtp_sdp`, `nal`, `sps`, `hdr`, `flv`, `ts_demux`, the `*_ext` extension files), media-plane, smooth-runtime, multimux, hls-runtime, media-doctor, demo; guard tables (`non_exhaustive` expected set, `label` skips, ordering guard path for the `OTI_*` constants).

- [ ] **Step 1:** If `av1.rs`, `vp9.rs` or `avs3.rs` cannot be split into config box and sample entry without touching `sample_entries`, leave that file whole for slice 5 and say so; do not move half of a type.
- [ ] **Step 2:** Move, repoint, update guard rows. `pub(crate)` items a staying file uses (`has_high_profile_ext`, the `OTI_*` and `STREAM_TYPE_AUDIO` constants, `parse_sequence_header_record` if avs2 and avs3 end up in different slices) become `pub` with a doc line.
- [ ] **Step 3:** Move the tests that compile against `isobmff` alone (recon §7: `esds_config`, `codec_config_boxes`, `hevc_array_rules`, and the box halves of the Dolby/DTS/MPEG-H/Dolby Vision/HDR tests where a file is purely box-level). A mixed test file stays in transmux.
- [ ] **Step 4: Gate.** Commit `refactor!: move the codec configuration boxes to isobmff`.

### Slice 5: sample entries and the `moov` tree

**Files:** move from transmux: `sample_entries.rs`, `subtitle_entries.rs`, `avc_parameter_set.rs`, `track_reference.rs`, `init_segment.rs` (everything left in it after slice 1), and whatever of `av1.rs`, `vp9.rs`, `avs3.rs` remained. Repoint transmux (`pipeline.rs`, `pipeline/stsd.rs`, `media.rs`, `progressive*.rs`, `validate.rs`, `smooth*.rs`, `cenc_*`, `fragment_build.rs`), media-doctor, multimux, hls-runtime, smooth-runtime, fuzz `transmux_sample_entries` (renamed `isobmff_sample_entries`).

- [ ] **Step 1:** Move the cluster in one change (it is strongly connected through `SampleEntryVariant`). `no_dom` guard: `XmlSubtitleSampleEntry` is an `stpp` box, not XML parsing; carry its allowance to the `isobmff` row with the same reason.
- [ ] **Step 2:** Move the tests that compile against `isobmff` alone (`init_segment`, `init_segment_sweep`, `container_child_order`, `box_walk_untrusted_sizes`, the stsd/saiz box tests, `captions` if box-only).
- [ ] **Step 3:** `transmux/src/lib.rs`: delete every `pub use` of a moved item; `git grep -n "pub use" transmux/src/lib.rs` shows no `isobmff::` path.
- [ ] **Step 4: Gate.** Additionally: `cargo tree -p isobmff --edges normal --all-features` lists only `broadcast-common`, `thiserror`, `serde` and their own dependencies. Commit `refactor!: move the sample entries and the moov tree to isobmff`.

### Slice 6: walkers, test support, clean-up

**Files:** `dash-runtime/src/ingest/session.rs`, `dvr-runtime/src/recover.rs`, `hls-runtime/src/{client,server}/engine.rs`, `media-doctor/src/container_codec.rs`, `media-plane/src/segmenter/program.rs` (tests), `test-harness/src/fmp4_split.rs`, `transmux/tests/it/support/box_{iter,walk}.rs`, `container-probe` (dev-dep + drift test), docs.

- [ ] **Step 1:** Each of the six walkers uses `isobmff`'s header parser, box iterator and four-cc constants in place of its own literals and header arithmetic (recon §5 lists what each hand-rolls). Behaviour unchanged; each crate's existing tests are the check. Add `isobmff` to each crate's `Cargo.toml`; a crate that then no longer needs a transmux item for box work drops that import (it keeps transmux for the IR).
- [ ] **Step 2:** `container-probe`: add `isobmff` as a dev-dependency and a drift test (`container-probe/tests/it/isobmff_drift.rs`): its header constants equal `isobmff`'s, and on every ISO BMFF fixture its box walk visits the same `(type, offset, size)` sequence as `isobmff`'s iterator. Its runtime dependencies are unchanged (`cargo tree -p container-probe --edges normal` still shows only broadcast-common).
- [ ] **Step 3:** Replace `transmux/tests/it/support/box_iter.rs` and `box_walk.rs` with `isobmff`'s iterator.
- [ ] **Step 4:** Finish CRATE-ACCEPTANCE for `isobmff`: README coverage table for everything it now holds, crate-root docs, at least two examples that read a real file from the command line, a release workflow `release-isobmff.yml` modelled on an existing independent crate's (tag `isobmff-v*`; it is not run), fuzz targets listed in CI. Update `CLAUDE.md` (crate list: add `isobmff`, remove `mp4-emsg`, shorten the transmux entry to what it still owns) and the design doc's §4.1 if any decision above changed what a crate owns.
- [ ] **Step 5: Gate**, with gate step 2 for every crate touched. Commit `refactor: repoint the box walkers at isobmff`.

## Lanes

The slices are sequential: each starts from main after the previous one has landed. One lane per slice; slice 5 may be split by the orchestrator if its first round stalls. The orchestrator audits each slice on its own gate run (gate steps 1 to 7), checks `git grep` for re-exports and for hand-rolled header arithmetic introduced by the move, and lands it before writing the next brief from the tree as it then is.
