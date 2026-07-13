# CENC/CBCS Encrypt Path Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax. Full design context: `docs/superpowers/specs/2026-07-13-cenc-encrypt-design.md`. Read it AND the named reference files before coding.

**Goal:** Add the IR→encrypted-CMAF path (cenc AES-CTR + cbcs AES-CBC-pattern) to `transmux`, closing epic #564.

**Architecture:** Factor the symmetric CTR cipher out of `cenc_decrypt.rs` into a shared `cenc_crypto.rs`; add CBC-pattern encrypt; a `CencEncryptor` implementing `broadcast_common::Encrypt` encrypts samples in place and records per-sample crypto onto `Track.encryption`; the CMAF muxer emits `encv`/`sinf` + `senc`/`saio`/`saiz` from that field; HLS/DASH packagers emit signalling.

**Tech Stack:** Rust, `no_std`+`alloc`, RustCrypto `aes`/`ctr`/`cbc` (feature `cenc`), existing `cenc.rs` box types, `nal.rs` NAL helpers.

## Global Constraints

- MSRV 1.86; build/test `--locked`; must build `--no-default-features` AND `--all-features`.
- Feature-gate all new code on `cenc` (matching `cenc_decrypt.rs`).
- No magic numbers outside `#[cfg(test)]` — named consts/enums; cite ISO/IEC 23001-7 §/14496-12 § in module + item docs.
- Every wire struct: symmetric `Serialize` + byte-exact round-trip test. No `self.raw` passthrough.
- Public spec/field enums get `name()` + `impl_spec_display!` (#204) or a `label_coverage` SKIP entry; `#[non_exhaustive]` on public config enums.
- Existing `cenc*.rs` decrypt tests MUST stay green (cipher-core refactor is behaviour-preserving).
- Gate suite (run by Claude, not the subagent's word): `cargo test -p transmux --all-features --locked`, `--no-default-features`, `clippy -D warnings`, `fmt --check`, doc `-D warnings`.

---

### Task 1: Cipher core refactor — `cenc_crypto.rs`

**Files:**
- Create: `transmux/src/cenc_crypto.rs`
- Modify: `transmux/src/cenc_decrypt.rs` (delegate `decrypt_sample_cenc`/`cbcs_pattern_decrypt` to the shared module), `transmux/src/lib.rs` (`mod cenc_crypto;`)

**Interfaces:**
- Produces:
  - `pub(crate) fn apply_ctr(iv: &[u8], key: &[u8;16], subsamples: &[SubSampleEntry], data: &mut [u8]) -> Result<()>` — AES-128-CTR over protected ranges (empty subsamples ⇒ whole sample). Symmetric: same fn encrypts and decrypts.
  - `pub(crate) fn cbcs_pattern(key: &[u8;16], chain_iv: &mut [u8;16], crypt: u8, skip: u8, range: &mut [u8], op: CbcsOp)` where `enum CbcsOp { Encrypt, Decrypt }` — one continuous CBC chain per the rule documented in `cenc_decrypt.rs` (chain across skip runs + subsample boundaries; trailing partial crypt block left clear; `0:0`⇒`1:0`).
  - `pub(crate) fn cbcs_sample(tenc, entry, key, data, op) -> Result<()>` — resolves IV (`resolve_cbcs_iv` moves here) + walks subsamples.
- Consumes: `cenc::{SampleEncryptionEntry, SubSampleEntry, TrackEncryptionBox}`.

- [ ] Step 1: Move `decrypt_sample_cenc` body → `apply_ctr`; `cbcs_pattern_decrypt`+`resolve_cbcs_iv`+`decrypt_sample_cbcs` → `cbcs_pattern`/`cbcs_sample` with a `CbcsOp` param (Decrypt = current code path). Add `Aes128CbcEnc = cbc::Encryptor<aes::Aes128>`; on `Encrypt`, capture the *plaintext*-derived next-chain from the produced ciphertext block (CBC encrypt: chain = ciphertext just produced — read after encrypt).
- [ ] Step 2: Rewrite `cenc_decrypt.rs` `decrypt_sample` to call `cenc_crypto::apply_ctr` / `cbcs_sample(..., CbcsOp::Decrypt)`. Delete the moved private fns.
- [ ] Step 3: `cargo test -p transmux --all-features --locked cenc` — all existing decrypt tests PASS (regression guard). No new behaviour.
- [ ] Step 4: Commit `refactor(transmux): factor CENC cipher core into cenc_crypto (#564)`.

**Gate:** existing decrypt suite green; `cbcs` encrypt of a block then decrypt returns the input (unit test in `cenc_crypto`).

---

### Task 2: `cenc_encrypt.rs` — `CencEncryptor` + `Encrypt` impl

**Files:**
- Create: `transmux/src/cenc_encrypt.rs`, `transmux/tests/cenc_encrypt.rs`
- Modify: `transmux/src/media.rs` (add `Track.encryption: Option<TrackEncryption>` + the struct), `transmux/src/lib.rs`
- Reference (read first): `transmux/src/cenc_decrypt.rs` (dual `TrackCrypto`), `transmux/src/nal.rs` (NAL walk helpers, #517), `broadcast-common/src/mux.rs` (`Encrypt` trait).

**Interfaces:**
- Produces:
  - `media::TrackEncryption { scheme: CencScheme, tenc: cenc::TrackEncryptionBox, samples: Vec<cenc::SampleEncryptionEntry> }` (+ `Track.encryption: Option<_>`, default `None`).
  - `pub enum IvGen { Counter { base: u64 }, Explicit(Vec<Vec<u8>>) }` (Default = `Counter{base:0}`).
  - `pub enum SubsamplePolicy { Video, WholeSample }` (`#[non_exhaustive]`).
  - `pub struct EncryptConfig { scheme, kid:[u8;16], key:[u8;16], iv: IvGen, pattern: Option<(u8,u8)>, subsample: SubsamplePolicy }`.
  - `pub struct CencEncryptor;` impl `broadcast_common::Encrypt<Media, EncryptConfig, Error>` (`encrypt(&self,&mut Media,&EncryptConfig)`).
  - `CencScheme` re-exported from a shared spot so `media`+both crypto modules share it (move to `cenc_crypto` or `cenc`; re-export at old path to avoid breaking).
- Consumes: Task 1 `cenc_crypto::{apply_ctr, cbcs_sample, CbcsOp}`.

- [ ] Step 1 (TDD): write `tests/cenc_encrypt.rs` self-round-trip for **cenc**: demux `../fixtures/mp4/cenc.mp4`? No — that's already encrypted. Use cleartext source `../fixtures/ts/h264/main.ts` → `ts_demux` → `Media`; `CencEncryptor.encrypt(cenc cfg)`; then `apply_ctr` with the same IVs to reverse in-place → assert byte-identical to pre-encrypt samples. (Full mux→decrypt round-trip is Task 3/4; this task tests the IR-level cipher+subsample+entry recording only.) Verify it FAILS to compile (types absent).
- [ ] Step 2: Add `media::TrackEncryption` + `Track.encryption`. Build `cenc_encrypt.rs`: subsample map via `nal.rs` (video: clear length-prefix+NAL-header+slice-header, protect slice payload; guard: protected+clear sum == sample len), `WholeSample`: empty subsample map. IV from `IvGen` (Counter: `(base+idx).to_be_bytes()` 8-byte). Build `tenc` from cfg (`default_is_protected=1`, iv_size 8, pattern for cbcs, `default_constant_IV` when iv_size 0). Encrypt via cipher core; push `SampleEncryptionEntry`; set `Track.encryption`.
- [ ] Step 3: cenc round-trip test PASSES. Add the **cbcs** variant (pattern default 1:9). PASSES.
- [ ] Step 4: `Explicit` IV path + error tests (IV>16, Explicit count≠samples, missing constant_IV for cbcs iv_size 0). `name()`+`impl_spec_display!` for any new public enum (or SKIP). `--no-default-features` builds (cenc off ⇒ module absent).
- [ ] Step 5: Commit `feat(transmux): CencEncryptor + Encrypt impl (cenc/cbcs) (#564)`.

**Gate:** IR-level encrypt→reverse byte-identical both schemes; error cases bite; `label_coverage` green.

---

### Task 3: Muxer emission — `encv`/`sinf` init + `senc`/`saio`/`saiz` fragments

**Files:**
- Modify: `transmux/src/init_segment.rs` (sample-entry → `encv`/`enca`+`sinf` when `Track.encryption`), `transmux/src/movie_fragment.rs` (per-`traf` `senc`+`saiz`+`saio`, back-patch `saio` offset)
- Reference: `transmux/src/cenc.rs` (`OriginalFormatBox`/`SchemeTypeBox`/`ProtectionSchemeInfoBox`/`SampleEncryptionBox`/`SampleAuxInfoSizesBox`/`SampleAuxInfoOffsetsBox` serialize APIs), `cenc.rs` `parse_box`s for the round-trip test.

**Interfaces:**
- Consumes: `Track.encryption` (Task 2), `cenc.rs` box serializers.
- Produces: CMAF init+media segments carrying full CENC boxes; no new public API (internal muxer behaviour keyed on `Track.encryption`).

- [ ] Step 1: init_segment — when `Track.encryption` Some, rename codec 4cc → `encv`/`enca`, append child `sinf`(`frma`=orig 4cc, `schm`=scheme+`0x00010000`, `schi`>`tenc`). Recompute sample-entry + parent sizes.
- [ ] Step 2: movie_fragment — after `trun`, for protected track append `senc` (flags `0x000002` when subsampled), `saiz` (per-sample aux sizes), `saio` (offset placeholder). After all `moof` sizes final, back-patch `saio.offset[0]` to the first sample's aux-info byte position. Choose anchor = absolute file offset (match `mp4decrypt` expectation) — document.
- [ ] Step 3: box byte-exact round-trip test (build→serialize→parse→equal) for the emitted `encv`/`sinf`/`senc`/`saiz`/`saio`.
- [ ] Step 4: Commit `feat(transmux): emit CENC sinf/senc/saio/saiz in CMAF mux (#564)`.

**Gate:** emitted boxes re-parse equal; init+media segment structurally valid (feed to `validate.rs` if it checks CENC, else `mp4dump`).

---

### Task 4: End-to-end round-trip + `mp4decrypt` golden interop

**Files:**
- Modify: `transmux/tests/cenc_encrypt.rs` (add full-pipeline + interop cases)
- Reference: `transmux/tests/cenc_fragmented_fixture.rs` (fixture paths, key material, `mp4decrypt` invocation pattern — reuse verbatim).

**Interfaces:** Consumes Tasks 2+3 (encrypt + mux); `CencDecryptor::from_fmp4`+`decrypt` (existing).

- [ ] Step 1: full self round-trip per scheme — cleartext `Media` → encrypt → mux CMAF → `CencDecryptor::from_fmp4` + `decrypt(KeyMap)` → byte-identical samples. (Fragmented source: `../fixtures/transmux/h264_aac_frag.mp4`; progressive: `../fixtures/ts/h264/main.ts` via ts→cmaf.)
- [ ] Step 2: golden interop per scheme — write our encrypted CMAF to a temp file, run `mp4decrypt --key <kid_hex>:<key_hex> in out` (skip test cleanly if binary absent, mirroring existing pattern), parse `out`, assert samples byte-identical to cleartext. Fixtures & key material reuse `cenc_fragmented_fixture.rs`.
- [ ] Step 3: Commit `test(transmux): CENC encrypt e2e round-trip + mp4decrypt interop (#564)`.

**Gate:** both schemes green on Claude's own run; interop actually invokes `mp4decrypt` locally (not just skipped).

---

### Task 5: HLS/DASH signalling + docs

**Files:**
- Modify: `transmux/src/hls.rs` (`EXT-X-KEY`), `transmux/src/dash.rs` (`ContentProtection`+`cenc:pssh`), `transmux/src/lib.rs` crate-doc coverage, `CHANGELOG.md`, README coverage table
- Reference: `transmux/src/sample_aes.rs` (HLS key-tag precedent), `cenc.rs` `ProtectionSystemSpecificHeaderBox` (pssh #480), `transmux/docs/codec/cenc-23001-7.md`.

**Interfaces:** Consumes `Track.encryption`; optional caller-supplied `pssh` system entries (add to `EncryptConfig` or a packager option — decide: packager-level `Vec<(system_id, data)>`).

- [ ] Step 1: DASH — emit `<ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc|cbcs" cenc:default_KID=...>` + one per caller pssh (base64 of `ProtectionSystemSpecificHeaderBox`). Unit test XML output.
- [ ] Step 2: HLS — emit `#EXT-X-KEY:METHOD=SAMPLE-AES,...,KEYFORMAT="urn:mpeg:dash:mp4protection:2011",KEYID=0x<kid>` for cbcs CMAF-HLS; document cenc = DASH-only. Unit test playlist output.
- [ ] Step 3: crate-doc + CHANGELOG `[Unreleased]` + README coverage; doc `-D warnings` green.
- [ ] Step 4: Commit `feat(transmux): HLS EXT-X-KEY + DASH ContentProtection for CENC (#564)`.

**Gate:** signalling unit tests bite; full 6-gate suite green; every #564 AC met.

---

## Self-review

- Spec coverage: units 1–5 → tasks 1–5. Round-trip + mp4decrypt gate → Task 4. Box byte-exact → Task 3. pssh reuse → Task 5. ✓
- Types consistent: `TrackEncryption`, `CencScheme`, `IvGen`, `EncryptConfig`, `CbcsOp`, `apply_ctr`, `cbcs_sample` used identically across tasks. ✓
- Ordering: cipher core (1) → encryptor+IR (2) → emission (3) → e2e/interop (4) → signalling (5). Each task independently gated. ✓
- Open impl decision deferred to Task 3 executor: `saio` anchor absolute vs moof-relative — must match `mp4decrypt`; verified empirically in Task 4 (if interop fails, flip anchor).
