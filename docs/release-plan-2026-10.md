# Release plan - the 2026-10 breaking wave

Status: **PREPARATION ONLY.** Nothing in this document has been executed: no version is bumped, no tag exists (local or
remote), nothing is published. The owner says "release" later; this is the plan that step executes.
Prepared 2026-10-16 against `main` @ `9fe7a022` (CI green: run 38931894510). Companion documents:
[`release-audit-2026-10.md`](release-audit-2026-10.md) (gate evidence + must-fix list),
[`release-notes/2026-10-wave-overview.md`](release-notes/2026-10-wave-overview.md) (human summary), the 55 draft release
notes under [`release-notes/`](release-notes/) (`status: DRAFT` front matter), and
[`migration-notes-downstream.md`](migration-notes-downstream.md).

## 1. Summary

* **54 publishable crates are in the wave, all of them major-class** (47 are 0.x minor-epoch bumps, 7 are semver majors: `broadcast-common` 9 -> 10, the lockstep five 11 -> 12, `scte35-splice` 3 -> 4).
  Not because 54 crates broke their own API (about 31 did), but because of the **epoch-purity rule**: `broadcast-common` takes a major
  (`Serialize::to_bytes` removed), and 49 workspace crates depend on it, so every dependent's caret bucket moves and
  every dependent is major-class (0.x -> 0.y+1, >=1 -> +1 major). `mpeg-ts` 0.5 -> 0.6 (typed `Pid`) and `transmux` 0.26 -> 0.27
  (typed `TrackId`) then cascade to their dependents on top. Only `broadcast-auth`, `dvb-csa`, `rtsp-runtime` and `multimux-cli` are outside the
  `broadcast-common` fan-in, and each is major-class for another reason (own break, or the `mpeg-ts` / `multimux` epoch).
* **Not released:** `atsc3` (never published, abandoned per owner; only its `broadcast-common` requirement is edited by the bump), `test-alloc`, `test-bounded`,
  `test-fixtures` (`publish = false`). 58 workspace members = 54 + 4.
* **Lockstep five** (`dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-conformance`, `dvb-tools`) -> **12.0.0**, one `v12.0.0` tag.
* **Inventory reconciliation (owner's "57 of 58 changed"):** of the 54 publishable crates, **53 have file changes since their base tag** (only `multimux-cli` has none and releases purely for the `multimux` epoch); of those, `ssai-runtime` (tests only), `rmt-flute` (manifest + tests), `dvb-simulcrypt` and `atsc3-route` (manifest/tests/serde feature) have no behavioural change and release purely for the epoch. The other four workspace members (`atsc3`, `test-alloc`, `test-bounded`, `test-fixtures`) have no tag at all, which is how a diff-since-latest-tag count reaches 57.
* **Every one of the 54 release lanes exists** (49 independent lanes + `release.yml` for the lockstep five); see section 7. No missing lane.
* **Tag count: 50** (49 independent + 1 lockstep). Tags must be pushed one at a time, each waiting for the crates-io approval gate and the crates.io index (section 6/8).

Counts: major-class **54**, minor **0**, patch **0**.

## 2. Method and how to read the table

* **Base tag per crate:** the latest `<crate>-v*` tag; the lockstep five use the latest `v*` (`v11.1.0`). Every crate's tree version equals its latest tag and its live crates.io version (checked with `tools/crates-io.py version`, 2026-10-16: all 54 are live at the tree version), so the diff base is exactly what consumers have.
* **Own change class** comes from the public-API diff since the base tag: `cargo semver-checks -p <crate> --all-features` against the live crates.io release (cargo-semver-checks, already installed locally, analysis only) **cross-checked against each CHANGELOG's `[Unreleased]` BREAKING items and the source diff**. semver-checks is lint-based and has false negatives: it does not see a changed field/argument type (`u32` -> `TrackId`, `u16` -> `Pid`), an added `Drop` impl (`broadcast-auth::Credentials`), a changed return type (`caption-convert::matrix::lookup`) or a behavioural break, so a green semver-checks never downgrades a CHANGELOG BREAKING entry (the `semver-checks` column shows what the tool found; the CHANGELOG is the primary evidence).
* **`#[non_exhaustive]` check (trap f):** additions to `#[non_exhaustive]` enums/structs were not counted as breaking: `rist-runtime` `Error::PaddingBitSet`, `rtmp-runtime` `RtmpError::FieldOverflow`, `mpeg-ts` `Error::{PayloadTooLarge, FieldOverflow, SectionPayloadLengthMismatch}` (already in 0.5), `transmux` `InputDegradation`/`Media` fields. Removing a variant (`ule::Error::TsMapping`, `broadcast-loudness::Error::ChannelOutOfRange`, `scte35-splice::Error::UnexpectedCommandType`) **is** breaking even on a non-exhaustive enum.
* **Cascade:** a crate is major-class if it has a normal/build dependency, or a versioned dev-dependency, on a workspace sibling that is major-class. Path-only dev-dependencies (e.g. `caption-convert` -> `mpeg-ts`, `mpeg-pes`; `timed-metadata` -> `transmux`; `dvb-si` -> `dvb-t2mi`) are stripped by `cargo publish` and do not count. The `prepare-release.py --check` dry run over all 54 pairs (section 8) resolves every requirement and lists exactly which manifests change.
* **Versions:** 0.x major-class = `0.(y+1).0`; >=1 major-class = `(x+1).0.0`. If the owner prefers to land a smaller wave, the only legal alternative is to leave a crate *and everything above it* unreleased: no crate in this list can be skipped without leaving a published crate requiring `broadcast-common` ^9 next to ^10 (the #819/#858 failure).

## 3. Version table

| Crate | Live now | Base tag | Proposed | Class | Own change | semver-checks (all features) | Justification |
|---|---|---|---|---|---|---|---|
| `atsc3-route` | 0.2.0 | `atsc3-route-v0.2.0` | **0.3.0** | major-class | patch | none flagged | own: test/serde-feature only; epoch cascade from broadcast-common |
| `broadcast-auth` | 0.4.0 | `broadcast-auth-v0.4.0` | **0.5.0** | major-class | major | none flagged | own: Credentials gains Drop (cannot be destructured by move) |
| `broadcast-common` | 9.4.0 | `broadcast-common-v9.4.0` | **10.0.0** | major-class | major | MAJOR (1 major, 1 minor lint failures) | own: Serialize::to_bytes removed; MJD range changed (CHANGELOG BREAKING, semver-checks major) |
| `broadcast-hls` | 0.3.1 | `broadcast-hls-v0.3.1` | **0.4.0** | major-class | patch | none flagged | own: computed_version fix; epoch cascade from broadcast-common |
| `broadcast-loudness` | 0.4.0 | `broadcast-loudness-v0.4.0` | **0.5.0** | major-class | major | MAJOR (2 major, 0 minor lint failures) | own: Error::ChannelOutOfRange removed (semver-checks major) |
| `caption-convert` | 0.2.0 | `caption-convert-v0.2.0` | **0.3.0** | major-class | major | none flagged | own: matrix::lookup returns Option |
| `cc-data` | 0.6.0 | `cc-data-v0.6.0` | **0.7.0** | major-class | major | MAJOR (1 major, 0 minor lint failures) | own: CcData gains fields; Error::InvalidFixedBits payload changed (semver-checks major) |
| `compliance-probe` | 0.3.0 | `compliance-probe-v0.3.0` | **0.4.0** | major-class | minor | none flagged | own: PcrSample::pid typed (public field type; crate is unpublished-in-practice per CLAUDE.md but live 0.3.0); epoch cascade from broadcast-common |
| `container-probe` | 0.1.1 | `container-probe-v0.1.1` | **0.2.0** | major-class | patch | none flagged | own: prober verdict fix + allocation bound; epoch cascade from broadcast-common |
| `dvb-bbframe` | 11.1.0 | `v11.1.0` | **12.0.0** | major-class | major | none flagged | own: Bbheader::serialize returns Result |
| `dvb-ci` | 0.9.1 | `dvb-ci-v0.9.1` | **0.10.0** | major-class | major | MAJOR (3 major, 0 minor lint failures) | own: length_form fields on every APDU/SPDU/TPDU object, typed Pid (semver-checks major) |
| `dvb-ci-runtime` | 0.17.2 | `dvb-ci-runtime-v0.17.2` | **0.18.0** | major-class | major | none flagged | own: typed Pid on ManagedService/ManagedCa/Driver APIs; dvb-ci length_form |
| `dvb-conformance` | 11.1.0 | `v11.1.0` | **12.0.0** | major-class | major | none flagged | own: ConformanceEvent::pid is Option<Pid> |
| `dvb-csa` | 0.3.1 | `dvb-csa-v0.3.1` | **0.4.0** | major-class | patch | none flagged | own: constant-time ControlWord eq; epoch cascade from mpeg-ts |
| `dvb-mabr` | 0.2.0 | `dvb-mabr-v0.2.0` | **0.3.0** | major-class | patch | none flagged | own: src/manifest change, no CHANGELOG entry (defect); epoch cascade from broadcast-common |
| `dvb-si` | 11.1.0 | `v11.1.0` | **12.0.0** | major-class | major | MAJOR (9 major, 0 minor lint failures) | own: typed Pid, reserved-bit fields, non_exhaustive tables, CellCenter->CellCentre, fit_mask serializer errors |
| `dvb-simulcrypt` | 0.6.0 | `dvb-simulcrypt-v0.6.0` | **0.7.0** | major-class | patch | none flagged | own: manifest only; epoch cascade from broadcast-common |
| `dvb-stream` | 0.6.0 | `dvb-stream-v0.6.0` | **0.7.0** | major-class | major | none flagged | own: T2miStream::{new,from_socket,bind} take Pid |
| `dvb-subtitle` | 0.5.0 | `dvb-subtitle-v0.5.0` | **0.6.0** | major-class | major | MAJOR (1 major, 0 minor lint failures) | own: CamelCase consts renamed SCREAMING_SNAKE, Error::InvalidValue |
| `dvb-t2mi` | 11.1.0 | `v11.1.0` | **12.0.0** | major-class | major | none flagged | own: T2miPump::new / InnerTsRecovery::{new,new_for_plp} take Pid |
| `dvb-tools` | 11.1.0 | `v11.1.0` | **12.0.0** | major-class | major | tool error | own: t2mi --inner without --plp now exits non-zero on multi-PLP captures (CLI behaviour) |
| `dvb-vbi` | 0.4.2 | `dvb-vbi-v0.4.2` | **0.5.0** | major-class | major | MAJOR (1 major, 0 minor lint failures) | own: LineHeader/WssDataField/DataUnit gain fields |
| `hls-runtime` | 0.8.0 | `hls-runtime-v0.8.0` | **0.9.0** | major-class | major | none flagged | own: TrackId / SegmentSeq / PartIndex / Generation newtypes |
| `media-doctor` | 0.10.0 | `media-doctor-v0.10.0` | **0.11.0** | major-class | major | none flagged | own: Location::at_track(TrackId), PidFlag::pid is Pid |
| `media-plane` | 0.6.0 | `media-plane-v0.6.0` | **0.7.0** | major-class | major | none flagged | own: SegmentSeq/PartIndex/TrackId typed ids |
| `mp4-emsg` | 0.4.1 | `mp4-emsg-v0.4.1` | **0.5.0** | major-class | major | MAJOR (2 major, 0 minor lint failures) | own: EmsgBox.flags field, parse_with_flags removed |
| `mpeg-pes` | 0.5.0 | `mpeg-pes-v0.5.0` | **0.6.0** | major-class | major | MAJOR (2 major, 0 minor lint failures) | own: PesHeader::header_stuffing_len -> header_stuffing, new fields |
| `mpeg-ps` | 0.5.0 | `mpeg-ps-v0.5.0` | **0.6.0** | major-class | major | MAJOR (1 major, 0 minor lint failures) | own: SystemHeader.trailing_reserved field |
| `mpeg-ts` | 0.5.0 | `mpeg-ts-v0.5.0` | **0.6.0** | major-class | major | MAJOR (3 major, 0 minor lint failures) | own: TsHeader::pid is Pid, From<u16> for Pid removed, AdaptationField fields |
| `multimux` | 0.12.0 | `multimux-v0.12.0` | **0.13.0** | major-class | major | MAJOR (2 major, 0 minor lint failures) | own: Config::allow_open_ingest, WHIP/WHEP Location resources, typed ids |
| `multimux-cli` | 0.10.0 | `multimux-cli-v0.10.0` | **0.11.0** | major-class | none | tool error | no change; epoch cascade from multimux |
| `playout-runtime` | 0.2.1 | `playout-runtime-v0.2.1` | **0.3.0** | major-class | patch | none flagged | own: internal; epoch cascade from broadcast-common |
| `rdd29` | 0.4.0 | `rdd29-v0.4.0` | **0.5.0** | major-class | patch | none flagged | own: internal; epoch cascade from broadcast-common |
| `rist-runtime` | 0.3.0 | `rist-runtime-v0.3.0` | **0.4.0** | major-class | minor | none flagged | own: additive non_exhaustive Error::PaddingBitSet + NACK fixes; epoch cascade from broadcast-common |
| `rmt-flute` | 0.6.0 | `rmt-flute-v0.6.0` | **0.7.0** | major-class | patch | none flagged | own: manifest only; epoch cascade from broadcast-common |
| `rtcp-packet` | 0.5.0 | `rtcp-packet-v0.5.0` | **0.6.0** | major-class | patch | none flagged | own: fixes; epoch cascade from broadcast-common |
| `rtmp-runtime` | 0.7.2 | `rtmp-runtime-v0.7.2` | **0.8.0** | major-class | minor | none flagged | own: RtmpError::FieldOverflow (non_exhaustive, additive) + hardening; epoch cascade from broadcast-common |
| `rtp-packet` | 0.4.0 | `rtp-packet-v0.4.0` | **0.5.0** | major-class | minor | none flagged | own: av3a feature (additive); epoch cascade from broadcast-common |
| `rtsp-runtime` | 0.7.1 | `rtsp-runtime-v0.7.1` | **0.8.0** | major-class | major | none flagged | own: ClientSession::handle_data returns HandleOutcome |
| `scte104` | 0.5.0 | `scte104-v0.5.0` | **0.6.0** | major-class | major | MAJOR (4 major, 0 minor lint failures) | own: audio/alternate-break operations typed, message_size no longer a field |
| `scte35-splice` | 3.0.1 | `scte35-splice-v3.0.1` | **4.0.0** | major-class | major | MAJOR (2 major, 0 minor lint failures) | own: ClearPayload.legacy_command_length_sentinel, compact layout fix, Error variant removed |
| `srt-runtime` | 0.5.0 | `srt-runtime-v0.5.0` | **0.6.0** | major-class | minor | none flagged | own: SrtSocket::rendezvous (additive); epoch cascade from broadcast-common |
| `ssai-runtime` | 0.2.1 | `ssai-runtime-v0.2.1` | **0.3.0** | major-class | none | none flagged | no change; epoch cascade from broadcast-common |
| `st12-1` | 0.3.1 | `st12-1-v0.3.1` | **0.4.0** | major-class | minor | none flagged | own: FrameRate::max_frame, validate_for_rate (additive); epoch cascade from broadcast-common |
| `st2022` | 0.1.0 | `st2022-v0.1.0` | **0.2.0** | major-class | major | none flagged | own: PayloadHeader::extension typed |
| `st291` | 0.4.1 | `st291-v0.4.1` | **0.5.0** | major-class | patch | none flagged | own: internal; epoch cascade from broadcast-common |
| `st337` | 0.3.1 | `st337-v0.3.1` | **0.4.0** | major-class | patch | none flagged | own: internal; epoch cascade from broadcast-common |
| `st377-1` | 0.4.0 | `st377-1-v0.4.0` | **0.5.0** | major-class | patch | none flagged | own: internal; epoch cascade from broadcast-common |
| `timed-metadata` | 0.6.1 | `timed-metadata-v0.6.1` | **0.7.0** | major-class | minor | none flagged | own: webvtt text helpers (additive); epoch cascade from broadcast-common |
| `transmux` | 0.26.0 | `transmux-v0.26.0` | **0.27.0** | major-class | major | MAJOR (3 major, 0 minor lint failures) | own: TrackId/ProgramNumber newtypes, Pid, CodecConfig::Subtitle sample_entry, VttCueBox |
| `ts-fix` | 0.6.0 | `ts-fix-v0.6.0` | **0.7.0** | major-class | major | MAJOR (1 major, 0 minor lint failures) | own: PidFilter/AccessUnit take Pid |
| `ttml-subtitle` | 0.3.0 | `ttml-subtitle-v0.3.0` | **0.4.0** | major-class | major | none flagged | own: InlineContent::Unknown, XmlDeclaration fields, Drop impls |
| `ule` | 0.4.1 | `ule-v0.4.1` | **0.5.0** | major-class | major | MAJOR (1 major, 0 minor lint failures) | own: Error::TsMapping removed |
| `webrtc-runtime` | 0.3.0 | `webrtc-runtime-v0.3.0` | **0.4.0** | major-class | minor | none flagged | own: restart_ice added; behaviour changes (token origin gating, WHIP PATCH Content-Type); epoch cascade from broadcast-common |

## 4. The epoch-purity cascade

Seeds (own major-class change; all verified in the CHANGELOGs):
`broadcast-common` (to_bytes removed, MJD range), `mpeg-ts` (typed `Pid`, `From<u16>` removed), `transmux` (`TrackId`/`ProgramNumber`/`Pid`, `CodecConfig::Subtitle`, `VttCueBox`), `media-plane` (`SegmentSeq`/`PartIndex`/`TrackId`), `hls-runtime` (`Generation`, ids), `dvb-si`/`dvb-t2mi`/`dvb-bbframe`/`dvb-conformance`/`dvb-tools` (lockstep), `dvb-ci`, `dvb-ci-runtime`, `dvb-stream`, `ts-fix`, `media-doctor`, `multimux`, `rtsp-runtime`, `broadcast-auth`, `scte35-splice`, `scte104`, `mp4-emsg`, `mpeg-pes`, `mpeg-ps`, `st2022`, `ttml-subtitle`, `caption-convert`, `cc-data`, `dvb-subtitle`, `dvb-vbi`, `ule`, `broadcast-loudness`.

Cascade-only crates (no breaking change of their own; they ship because a sibling epoch moved): `atsc3-route`, `broadcast-hls`, `container-probe`, `compliance-probe`, `dvb-csa` (via `mpeg-ts`), `dvb-mabr`, `dvb-simulcrypt`, `playout-runtime`, `rdd29`, `rist-runtime`, `rmt-flute`, `rtcp-packet`, `rtmp-runtime`, `rtp-packet`, `srt-runtime`, `ssai-runtime`, `st12-1`, `st291`, `st337`, `st377-1`, `timed-metadata`, `webrtc-runtime`, `multimux-cli` (via `multimux`).
Their draft release notes say so explicitly and carry the dependency diff. Their CHANGELOGs already have entries except `dvb-mabr`, `multimux-cli`, `rmt-flute`, `ssai-runtime` (empty `[Unreleased]`; `prepare-release.py` cuts a default "Dependency/epoch bump only" line for them: see the audit's must-fix list for `dvb-mabr`, which also has a source change).

Second-order effects the script handles but a human should know about (from the `--check` run): `demo/Cargo.toml` (WASM demo, out of workspace) has requirements on `broadcast-common`, `dvb-conformance`, `dvb-subtitle`, `mpeg-pes`, `scte35-splice` that get bumped; `dvb-conformance` carries a second, stale `broadcast-common 9.3` requirement (a versioned dev-dependency) that is bumped to 10.0 as well; `atsc3` (unpublished) gets its `broadcast-common` requirement bumped so the path build still resolves, with no version bump and no tag. `bindings/python/` and `bindings/transmux-py/` are **not** workspace members and consume published crates by version: bump them after the crates are live (post-publish follow-up, as for the previous waves).

## 5. The lockstep five

| Crate | Now | Proposed |
|---|---|---|
| `dvb-si` | 11.1.0 | **12.0.0** |
| `dvb-t2mi` | 11.1.0 | **12.0.0** |
| `dvb-bbframe` | 11.1.0 | **12.0.0** |
| `dvb-conformance` | 11.1.0 | **12.0.0** |
| `dvb-tools` | 11.1.0 | **12.0.0** |

* One tag `v12.0.0`, `release.yml`: gates (workspace tests excluding `dvb-stream`, clippy, tag == version for all five, **`docs/release-notes/v12.0.0.md` must exist and be non-empty**), then `cargo publish` in the order `dvb-bbframe`, `dvb-si`, `dvb-conformance`, `dvb-t2mi`, `dvb-tools` (idempotent; a re-run skips what already shipped), then the GitHub Release from `v12.0.0.md`.
* External prerequisites of the five (versioned normal + dev edges): **only `broadcast-common` 10.0.0 and `mpeg-ts` 0.6.0** must be live first. That is why the lockstep tag is available as soon as `mpeg-ts` is live (`mpeg-ts` itself waits for `mp4-emsg`, because of its versioned dev-dependency on it); the generated order (`prepare-release.py --tag-order`, alphabetical within each ready batch) places it after the other batch-mates that become ready with `mpeg-ts` (position 34 of 50) and **before** `transmux`, `dvb-ci`, `dvb-ci-runtime`, `dvb-stream`, `ts-fix`, `media-doctor` and `multimux`, which all depend on `dvb-si`/`dvb-conformance`/`dvb-t2mi`.
* `dvb-tools` ships a binary but its binaries job is disabled in `release.yml` (ARM runner); use `attach-binaries.yml` (`workflow_dispatch`) after publish.

## 6. Topological tag order

> This list is GENERATED: `python3 tools/prepare-release.py --tag-needs $(cat docs/release-plan-2026-10.pairs)` prints it, and `tools/tests/test_release_lanes.py` fails if it drifts from `--tag-order`.

Computed from `cargo metadata` over normal + build edges **and versioned dev-dependency edges** (a versioned dev-dependency must resolve on the index at `cargo publish` time; path-only ones are ignored), with the lockstep five collapsed into one slot. No cycle exists. Every crate lists what must already be live on crates.io. **One tag at a time** (trap a): a push of more than three tags fires no workflows (#933), and each lane stops at the `crates-io` environment approval gate, so the loop is push -> approve -> wait for the index -> next.

1. `broadcast-common-v10.0.0`  
   needs live first: (nothing in this wave)
2. `broadcast-auth-v0.5.0`  
   needs live first: (nothing in this wave)
3. `broadcast-hls-v0.4.0`  
   needs live first: `broadcast-common`
4. `broadcast-loudness-v0.5.0`  
   needs live first: `broadcast-common`
5. `cc-data-v0.7.0`  
   needs live first: `broadcast-common`
6. `dvb-mabr-v0.3.0`  
   needs live first: `broadcast-common`
7. `dvb-simulcrypt-v0.7.0`  
   needs live first: `broadcast-common`
8. `dvb-vbi-v0.5.0`  
   needs live first: `broadcast-common`
9. `mp4-emsg-v0.5.0`  
   needs live first: `broadcast-common`
10. `mpeg-pes-v0.6.0`  
   needs live first: `broadcast-common`
11. `rdd29-v0.5.0`  
   needs live first: `broadcast-common`
12. `rmt-flute-v0.7.0`  
   needs live first: `broadcast-common`
13. `rtcp-packet-v0.6.0`  
   needs live first: `broadcast-common`
14. `rtp-packet-v0.5.0`  
   needs live first: `broadcast-common`
15. `rtsp-runtime-v0.8.0`  
   needs live first: `broadcast-auth`
16. `scte104-v0.6.0`  
   needs live first: `broadcast-common`
17. `srt-runtime-v0.6.0`  
   needs live first: `broadcast-common`
18. `st12-1-v0.4.0`  
   needs live first: `broadcast-common`
19. `st2022-v0.2.0`  
   needs live first: `broadcast-common`
20. `st337-v0.4.0`  
   needs live first: `broadcast-common`
21. `st377-1-v0.5.0`  
   needs live first: `broadcast-common`
22. `ttml-subtitle-v0.4.0`  
   needs live first: `broadcast-common`
23. `ule-v0.5.0`  
   needs live first: `broadcast-common`
24. `atsc3-route-v0.3.0`  
   needs live first: `broadcast-common`, `rmt-flute`
25. `dvb-subtitle-v0.6.0`  
   needs live first: `broadcast-common`, `mpeg-pes`
26. `mpeg-ps-v0.6.0`  
   needs live first: `broadcast-common`, `mpeg-pes`
27. `mpeg-ts-v0.6.0`  
   needs live first: `broadcast-common`, `mp4-emsg`
28. `rist-runtime-v0.4.0`  
   needs live first: `broadcast-common`, `rtcp-packet`
29. `st291-v0.5.0`  
   needs live first: `broadcast-common`, `rtp-packet`
30. `webrtc-runtime-v0.4.0`  
   needs live first: `broadcast-common`, `rtcp-packet`, `rtp-packet`
31. `container-probe-v0.2.0`  
   needs live first: `broadcast-common`, `mpeg-ps`, `mpeg-ts`, `st377-1`
32. `dvb-csa-v0.4.0`  
   needs live first: `mpeg-ts`
33. `scte35-splice-v4.0.0`  
   needs live first: `broadcast-common`, `mpeg-ts`
34. `v12.0.0`  
   needs live first: `broadcast-common`, `mpeg-ts`  
   five crates in one lane, publishes bbframe -> si -> conformance -> t2mi -> tools; **push by hand** (`push-release-tags.sh` refuses `v*` in explicit-tag mode; `--plan` stops here, exit 2)
35. `dvb-ci-v0.10.0`  
   needs live first: `v12.0.0`, `broadcast-common`, `mpeg-ts`
36. `dvb-stream-v0.7.0`  
   needs live first: `v12.0.0`, `broadcast-common`, `mpeg-ts`, `rtp-packet`
37. `ssai-runtime-v0.3.0`  
   needs live first: `broadcast-common`, `broadcast-hls`, `mp4-emsg`, `scte35-splice`
38. `timed-metadata-v0.7.0`  
   needs live first: `broadcast-common`, `broadcast-hls`, `cc-data`, `dvb-vbi`, `mp4-emsg`, `scte35-splice`
39. `transmux-v0.27.0`  
   needs live first: `v12.0.0`, `broadcast-common`, `broadcast-hls`, `cc-data`, `container-probe`, `mp4-emsg`, `mpeg-pes`, `mpeg-ps`, `mpeg-ts`, `rtcp-packet`, `rtp-packet`, `scte35-splice`
40. `ts-fix-v0.7.0`  
   needs live first: `v12.0.0`, `broadcast-common`, `mpeg-pes`, `mpeg-ts`, `scte35-splice`
41. `caption-convert-v0.3.0`  
   needs live first: `broadcast-common`, `cc-data`, `dvb-vbi`, `timed-metadata`
42. `dvb-ci-runtime-v0.18.0`  
   needs live first: `v12.0.0`, `broadcast-common`, `dvb-ci`, `mpeg-ts`
43. `media-doctor-v0.11.0`  
   needs live first: `v12.0.0`, `broadcast-common`, `broadcast-hls`, `container-probe`, `mpeg-pes`, `mpeg-ts`, `scte35-splice`, `timed-metadata`, `transmux`
44. `media-plane-v0.7.0`  
   needs live first: `broadcast-common`, `scte35-splice`, `timed-metadata`, `transmux`
45. `playout-runtime-v0.3.0`  
   needs live first: `broadcast-common`, `mp4-emsg`, `scte35-splice`, `ssai-runtime`
46. `rtmp-runtime-v0.8.0`  
   needs live first: `broadcast-common`, `transmux`
47. `compliance-probe-v0.4.0`  
   needs live first: `v12.0.0`, `broadcast-common`, `media-doctor`, `media-plane`, `mpeg-ts`, `scte35-splice`, `timed-metadata`
48. `hls-runtime-v0.9.0`  
   needs live first: `broadcast-auth`, `broadcast-common`, `broadcast-hls`, `container-probe`, `media-plane`, `timed-metadata`, `transmux`
49. `multimux-v0.13.0`  
   needs live first: `v12.0.0`, `broadcast-auth`, `broadcast-common`, `broadcast-hls`, `caption-convert`, `container-probe`, `hls-runtime`, `media-doctor`, `media-plane`, `mpeg-ts`, `rtmp-runtime`, `rtp-packet`, `rtsp-runtime`, `scte35-splice`, `srt-runtime`, `timed-metadata`, `transmux`, `webrtc-runtime`
50. `multimux-cli-v0.11.0`  
   needs live first: `multimux`

Equivalent grouped view (everything inside a group is independent of its own group, pushed still one by one): `broadcast-common` first; then the leaf tier that needs only it (`mp4-emsg`, `mpeg-pes`, `rtp-packet`, `rtcp-packet`, `dvb-vbi`, `cc-data`, `rmt-flute`, `st377-1`, ...); then `mpeg-ts` (gated by its dev-dependency on `mp4-emsg`) and the rest of that batch (`rist-runtime`, `st291`, `webrtc-runtime`, `container-probe`, `dvb-csa`, `scte35-splice`); then the lockstep tag (`v12.0.0`, position 34); then `transmux`; then the `transmux`/`dvb-si` dependents (`media-plane`, `hls-runtime`, `dvb-stream`, `ts-fix`, `dvb-ci`, `media-doctor`, ...); then `multimux`; `multimux-cli` last.

## 7. Release lanes

`.github/workflows/release.yml` (lockstep, tag `v*`) plus 49 `release-<name>.yml` lanes. Present and wired to the right crate/tag for **every** publishable crate (checked by parsing each lane: tag pattern, `cargo publish -p` target, release-note path):
`atsc3-route`, `broadcast-auth`, `broadcast-common`, `broadcast-hls`, `broadcast-loudness`, `caption-convert`, `cc-data`, `compliance-probe`, `container-probe`, `dvb-ci`, `dvb-ci-runtime`, `dvb-csa`, `dvb-mabr`, `dvb-simulcrypt`, `dvb-stream` (file `release-stream.yml`), `dvb-subtitle` (file `release-subtitle.yml`), `dvb-vbi`, `hls-runtime`, `media-doctor`, `media-plane`, `mp4-emsg`, `mpeg-pes`, `mpeg-ps`, `mpeg-ts`, `multimux`, `multimux-cli`, `playout-runtime`, `rdd29`, `rist-runtime`, `rmt-flute`, `rtcp-packet`, `rtmp-runtime`, `rtp-packet`, `rtsp-runtime`, `scte104`, `scte35-splice`, `srt-runtime`, `ssai-runtime`, `st12-1`, `st2022`, `st291`, `st337`, `st377-1`, `timed-metadata`, `transmux`, `ts-fix`, `ttml-subtitle`, `ule`, `webrtc-runtime`.

* **Missing lanes: none.**
* **Staged lanes, not in this plan's pair list:** `release-tshttp-runtime.yml` and `release-broadcast-client.yml` (both new crates at 0.1.0, copied from `release-hls-runtime.yml`, release notes `docs/release-notes/tshttp-runtime-0.1.0.md` and `broadcast-client-0.1.0.md`). They publish only on an owner-signed-off tag; `tshttp-runtime` must be live before `broadcast-client`.
* **Extra lane that must never fire:** `release-atsc3.yml` exists for an abandoned, never-published crate. Its release-note gate (`docs/release-notes/atsc3-0.1.0.md`) is deliberately unsatisfied, so a stray `atsc3-v0.1.0` tag fails safe; do not create that tag.
* **Tag-pattern collisions checked:** `dvb-ci-v*` does not match `dvb-ci-runtime-v*`, `multimux-v*` does not match `multimux-cli-v*`, `v*` does not match any `<crate>-v*`.
* Every lane gates on: `cargo test -p <crate> --all-features --locked`, clippy `-D warnings`, `--no-default-features` build, examples, **tag == manifest version**, **`docs/release-notes/<crate>-<ver>.md` exists and is non-empty**, then `cargo publish`, then a GitHub Release whose body is that note. Consequences: (1) the note files must have their `status: DRAFT` front matter removed before tagging, or the GitHub Release body starts with a YAML block; (2) `--locked` means **`Cargo.lock` must be refreshed and committed with the bump**; (3) every lane runs behind the `crates-io` environment, i.e. a human approves each of the 50 runs.

## 8. `tools/prepare-release.py`: which modes edit files

Read in full (`tools/prepare-release.py`, 418 lines). Modes:

| Invocation | Edits files? | What it does |
|---|---|---|
| `prepare-release.py <crate>=<ver> ...` (**default / apply**) | **YES** | bumps `version` in each `Cargo.toml`; rewrites every sibling requirement (every manifest incl. `fuzz/` and `demo/`, every dependency table, via regex `crate = { ... version = "x" }`); cuts each CHANGELOG `[Unreleased]` to `## [X.Y.Z] - <today>`; scaffolds `docs/release-notes/<crate>-<ver>.md` **only if absent** (our drafts exist, so they are kept); runs `cargo metadata --locked --offline`; prints tag order |
| `--check <crate>=<ver> ...` | no | dry run of the above. **Run for this plan:** all 54 pairs exit 0 and list the 55 manifests (54 crates + `atsc3`/`demo` requirement edits) and 54 CHANGELOGs it would touch |
| `--verify <crate>=<ver> ...` | no (network read) | `tools/crates-io.py check`: each crate live at exactly that version (the post-publish check, trap c) |
| `--restamp <crate>=<ver> ...` | **YES** | rewrites every CHANGELOG `## [ver] - date` heading and every release note `_Released <date>._` line to today; exits non-zero if a crate has no dated heading or no note |
| `--restamp-check <crate>=<ver> ...` | no | dry run of `--restamp` |

> **Update (release tooling PR):** apply mode now also rewrites README dependency snippets, strips the `status: DRAFT` front matter (pair notes, `v12.0.0.md`, `--extra-note`), regenerates `docs/CRATES.md`, and refreshes `Cargo.lock` (workspace-member versions only, verified); the tag order counts versioned dev-dependency edges. The paragraph below describes the script as first audited. The ordered commands are in [`RELEASE-RUNBOOK-2026-10.md`](RELEASE-RUNBOOK-2026-10.md); `tools/finalize-release-check.py` is the pre-tag gate and `tools/push-release-tags.sh --plan` pushes in order with a truthful exit code.

Not edited by the script (a human must): `Cargo.lock` (refresh with `cargo update --workspace` / a build and commit it; the script's own `--locked --offline` resolution check will fail until the lock matches the new versions: previous waves committed the lock in the same PR, e.g. `57cdbf7a`); the root `README.md` coverage table and per-crate README dependency snippets (`"11"`, `"0.5"` pins listed in the audit); `bindings/*`; the `status: DRAFT` front matter in the notes; the lockstep note `v12.0.0.md` (the script scaffolds per-crate notes only, not the `v*` note, and the lane gate needs it).
`dependency_order()` in the script counts only normal/build edges; the order in section 6 additionally counts versioned dev-dependency edges, and is the one to follow.

Exact command the release step runs (after the owner's "release"):

```bash
python3 tools/prepare-release.py $(cat docs/release-plan-2026-10.pairs)   # apply; then refresh Cargo.lock, finalise notes, review the diff
python3 tools/prepare-release.py --restamp-check <same pairs>             # all dated headings agree
```

The pair list is in section 9.

## 9. Pair list for `prepare-release.py`

```text
atsc3-route=0.3.0
broadcast-auth=0.5.0
broadcast-common=10.0.0
broadcast-hls=0.4.0
broadcast-loudness=0.5.0
caption-convert=0.3.0
cc-data=0.7.0
compliance-probe=0.4.0
container-probe=0.2.0
dvb-bbframe=12.0.0
dvb-ci=0.10.0
dvb-ci-runtime=0.18.0
dvb-conformance=12.0.0
dvb-csa=0.4.0
dvb-mabr=0.3.0
dvb-si=12.0.0
dvb-simulcrypt=0.7.0
dvb-stream=0.7.0
dvb-subtitle=0.6.0
dvb-t2mi=12.0.0
dvb-tools=12.0.0
dvb-vbi=0.5.0
hls-runtime=0.9.0
media-doctor=0.11.0
media-plane=0.7.0
mp4-emsg=0.5.0
mpeg-pes=0.6.0
mpeg-ps=0.6.0
mpeg-ts=0.6.0
multimux=0.13.0
multimux-cli=0.11.0
playout-runtime=0.3.0
rdd29=0.5.0
rist-runtime=0.4.0
rmt-flute=0.7.0
rtcp-packet=0.6.0
rtmp-runtime=0.8.0
rtp-packet=0.5.0
rtsp-runtime=0.8.0
scte104=0.6.0
scte35-splice=4.0.0
srt-runtime=0.6.0
ssai-runtime=0.3.0
st12-1=0.4.0
st2022=0.2.0
st291=0.5.0
st337=0.4.0
st377-1=0.5.0
timed-metadata=0.7.0
transmux=0.27.0
ts-fix=0.7.0
ttml-subtitle=0.4.0
ule=0.5.0
webrtc-runtime=0.4.0
```

## 10. Release-process traps (checked against this wave)

| Trap | Applies how |
|---|---|
| (a) >3 tags in one push fires **no** workflows | 50 tags: one at a time. `tools/push-release-tags.sh` does exactly this and waits on the sparse index, but refuses `v*` (push `v12.0.0` by hand) and exits 2 while a lane waits on the `crates-io` approval gate |
| (b) crates.io 429 | no *new* crate in this wave (all 54 exist), so the 5-per-10-minute new-crate limit does not apply; existing-crate version publishes are allowed a burst of 30 then 1 per minute, which one-at-a-time with approval easily stays under. If a 429 still appears: wait, re-run the lane (publish is idempotent) |
| (c) `push-release-tags.sh` exits 0 with missing tags | create all 50 tags **locally** at the release commit first, then `prepare-release.py --verify` after each push (it exits non-zero when a crate is not live); never trust the script's exit code |
| (d) epoch purity | the whole cascade above; `tools/check-published-dep-consistency.py` (checks 1-2) runs blocking on every release tag, see audit |
| (e) lockstep | `broadcast-common-v10.0.0` first and **live**, then `mp4-emsg-v0.5.0`, `mpeg-ts-v0.6.0`, then `v12.0.0` (no earlier than `mpeg-ts`; the script places it at position 34 of 50) |
| (f) `#[non_exhaustive]` | applied, see section 2 |
| lockstep gate runs the **whole workspace** test suite | `cargo test --workspace --exclude dvb-stream --all-features --locked` inside `release.yml`: it must be green on the release commit; run CI on the bump PR first |
| `Cargo.lock` | `--locked` everywhere: commit the refreshed lock |
| `status: DRAFT` front matter | remove from all 55 notes before tagging (it would render at the top of every GitHub Release) |
| python bindings | out of workspace: bump + publish their wheels only after the crates are live |
| `atsc3` | no tag, no lane run; version stays 0.1.0 |
