# CENC/CBCS encrypt path — design (issue #564)

**Status:** approved design, pre-implementation
**Crate:** `transmux` (feature `cenc`)
**Spec sources:** ISO/IEC 23001-7 (Common Encryption) — transcribed in `transmux/docs/codec/cenc-23001-7.md`; ISO/IEC 14496-12 §8.8 (`moof`/`traf`/`trun`), §8.12 (`sinf`/`frma`/`schm`/`schi`).

## Why

`pssh` (#480), HLS Sample-AES (#479), the CENC/CBCS **decrypt** path (`cenc_decrypt.rs`), all boxes' parse+serialize (`cenc.rs`), and real cenc/cbcs fixtures (#693/#694) have shipped. The `Encrypt` impl remains deferred (`media.rs:24`) and the muxer emits **no** `sinf`/`senc`/`saio`/`saiz`/`encv`. This story adds the IR→encrypted-CMAF path and closes the epic.

## Scope

In: `cenc`+`cbcs` sample encryption, `encv`/`enca`+`sinf` init-segment emission, per-`traf` `senc`+`saio`+`saiz` emission, HLS `EXT-X-KEY` + DASH `ContentProtection`/`cenc:pssh` signalling, round-trip + `mp4decrypt` golden interop.

Out: DRM license logic, key servers, key rotation (`sbgp`/`sgpd seig`) — deferred; keys/KID/IV always caller-supplied.

## Architecture

Five units, each independently testable:

### 1. IR crypto carrier (`media.rs`)
Add one optional field so the muxer can emit crypto boxes without knowing how encryption happened:
```rust
// on Track
pub encryption: Option<TrackEncryption>,

pub struct TrackEncryption {
    pub scheme: CencScheme,                       // reuse cenc_decrypt::CencScheme
    pub tenc: cenc::TrackEncryptionBox,           // KID, iv_size, is_protected, pattern, constant_iv
    pub samples: Vec<cenc::SampleEncryptionEntry>,// per-sample IV + subsample map, decode order
}
```
This is byte-for-byte the shape decrypt already recovers into its private `TrackCrypto` — the two are duals. `CencScheme` moves to (or is re-exported from) a shared location so both `cenc_encrypt` and `cenc_decrypt` use it. Default `encryption: None` (clear content — muxer unchanged).

### 2. Cipher core (`cenc_crypto.rs`, factored)
CTR is symmetric, so the `cenc` keystream helper is shared verbatim between encrypt and decrypt. Extract the existing `decrypt_sample_cenc` counter/subsample walk into a shared `apply_ctr(entry, key, data)` (encrypt == decrypt). For `cbcs`, add `Aes128CbcEnc = cbc::Encryptor<aes::Aes128>` and an `cbcs_pattern_encrypt` mirroring `cbcs_pattern_decrypt`'s **continuous-chain** rule (chain across skip runs and subsample boundaries; trailing partial crypt block left clear; `0:0` ⇒ `1:0`). Decrypt is refactored to call the shared module — no behaviour change, existing decrypt tests must stay green (regression guard).

### 3. `cenc_encrypt.rs` — `CencEncryptor` + `Encrypt` impl
```rust
pub enum IvGen {
    Counter { base: u64 },       // default; per-sample 8-byte IV = BE(base + sample_index), zero-pad to 16
    Explicit(Vec<Vec<u8>>),      // caller-supplied per-sample IVs, one per sample
}
pub struct EncryptConfig {
    pub scheme: CencScheme,
    pub kid: [u8; 16],
    pub key: [u8; 16],
    pub iv: IvGen,               // default IvGen::Counter { base: [0;8] } via Default
    pub pattern: Option<(u8, u8)>, // cbcs crypt:skip blocks; default 1:9 for cbcs, ignored for cenc
    pub subsample: SubsamplePolicy, // Video (NAL-aware) | WholeSample
}
impl Encrypt for CencEncryptor {
    type Media = Media; type Config = EncryptConfig; type Error = Error;
    fn encrypt(&self, media, cfg) -> Result<()>;
}
```
Per protected track, per sample: build the subsample map (video → walk NAL units via `nal.rs` #517 helpers, marking the length-prefix + NAL header + slice-header bytes clear and the slice payload protected; audio/`WholeSample` → one all-protected range or empty map = whole-sample), pick the IV from `IvGen`, encrypt the protected bytes in place with the cipher core, push a `SampleEncryptionEntry`, and set `Track.encryption`. `tenc` is built from `cfg` (KID, `default_is_protected=1`, iv_size, pattern, and for `cbcs` a `default_constant_IV` when iv_size==0).

### 4. Muxer emission (`init_segment.rs`, `movie_fragment.rs`)
- **init_segment**: when `Track.encryption` is `Some`, transform the sample entry: replace the codec 4cc with `encv` (video) / `enca` (audio), keep the original 4cc in a child `sinf`>`frma`, add `sinf`>`schm` (`scheme_type`, `scheme_version=0x00010000`) and `sinf`>`schi`>`tenc` (serialize the `TrackEncryptionBox`). Sample-entry child-box order and the outer size are recomputed.
- **movie_fragment**: for a protected track's `traf`, after building `trun`, append `senc` (per-sample IV + subsamples, flags bit `0x2` set when subsamples present) and the `saiz`/`saio` pair. `saio.offset[0]` = absolute (or `moof`-relative per the chosen anchor) byte offset of the first sample's aux data inside `senc`; it is **back-patched** after all `moof` box sizes are final. `saiz` carries per-sample aux-info sizes (IV + 2 + n·6 when subsampled).

### 5. Signalling (`hls.rs`, `dash.rs`)
- **HLS**: emit `#EXT-X-KEY:METHOD=SAMPLE-AES,URI=...,KEYFORMAT="urn:mpeg:dash:mp4protection:2011",KEYID=0x<kid>` for `cbcs` fMP4 (the CMAF-HLS case). `cenc` (CTR) is not an HLS method — documented; DASH-only.
- **DASH**: add `<ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc|cbcs" cenc:default_KID="..."/>` plus one `<ContentProtection>` per DRM system carrying a base64 `cenc:pssh` built from the shipped `ProtectionSystemSpecificHeaderBox` (#480). PSSH/system-id is caller-supplied (no DRM logic).

## Data flow
`demux(clear) → Media → CencEncryptor.encrypt(&mut Media, cfg)` (sets `Track.encryption`, mutates sample bytes) `→ CMAF muxer` reads `Track.encryption` → emits `encv`+`sinf` init + `senc`/`saio`/`saiz` fragments → `hls`/`dash` packagers read `Track.encryption` → emit signalling.

## Error handling
Structured `Error`: reject IV > 16 bytes, `Explicit` IV count ≠ sample count, subsample range exceeding sample, missing `default_constant_IV` for `cbcs` iv_size==0, unknown scheme. `#[non_exhaustive]` on public config enums.

## Testing (the gate — ungameable)
1. **Self round-trip** (per scheme): demux a committed cleartext fixture (`ts/h264/main.ts` progressive; `transmux/h264_aac_frag.mp4` fragmented) → `encrypt` → mux to CMAF → `CencDecryptor::from_fmp4` + `decrypt` → **byte-identical** cleartext samples.
2. **Golden interop** (per scheme): pipe our encrypted CMAF through `mp4decrypt --key <kid>:<key>` (Bento4, verified present) → byte-identical to the cleartext samples. Skips cleanly if `mp4decrypt` absent (CI/public).
3. **Box byte-exact**: build `encv`/`sinf`/`senc`/`saio`/`saiz` structs → serialize → parse → equal, and serialize twice → identical.
4. Decrypt regression: existing `cenc*.rs` tests unchanged and green (cipher-core refactor guard).
5. 6-gate suite (fmt/clippy/build both feature sets/test/doc) run by Claude, not delegate.

## Delegation plan (token-wise)
Claude (Opus): design, brief, audit, gates, CHANGELOG/release. Authoring delegated to **Sonnet** subagents (spec-reasoning-heavy): one for cipher-core + `cenc_encrypt.rs`, one for muxer emission, one for signalling; **Haiku** for mechanical wiring/tests scaffolding. Claude runs every gate itself and only marks done on its own evidence (never subagent say-so).
