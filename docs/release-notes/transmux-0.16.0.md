# transmux 0.16.0 — 2026-07-14

Minor, one breaking struct-literal change (see Compatibility). Completes the
CENC/CBCS **encrypt** path (issue #564) — the write-side counterpart to the
existing decrypt — turning a cleartext `Media` into standards-compliant
encrypted CMAF, plus HLS/DASH DRM signalling.

## Added (#564)
- **`CencEncryptor`** (`broadcast_common::Encrypt`) — protects a cleartext
  `Media` in place. `EncryptConfig { scheme, kid, key, iv, pattern, subsample }`:
  `cenc` (AES-128-CTR) / `cbcs` (AES-128-CBC pattern); `IvGen::{Counter, Explicit,
  Constant}`; `SubsamplePolicy::{Video, WholeSample}` (NAL-aware clear/protected
  split via the existing `nal.rs` helpers). Populates `Track::encryption`.
- **Box emission** — `init_segment::protect_init_segment` (`encv`/`enca` +
  `sinf`/`frma`/`schm`/`schi`/`tenc`) and `movie_fragment::protect_media_segment`
  (per-`traf` `senc`/`saiz`/`saio`, `moof`-relative `saio` anchor), rebuilt from
  `cenc.rs` typed serializers — no raw passthrough.
- **DASH/HLS signalling** — DASH `<ContentProtection>` (generic-CENC
  `default_KID` + caller-supplied `cenc:pssh`, reusing pssh #480) and HLS
  `#EXT-X-KEY` (cbcs; cenc is DASH-only). New `examples/cenc_encrypt.rs`.

## Correctness — external oracle
Both schemes are proven byte-identical through Bento4's real `mp4decrypt`
(including real multi-subsample video), not just self round-trip. The interop
gate caught two real bugs, both fixed here:
- **cbcs CBC chain must reset per subsample** (chain *within* a subsample, reset
  to the sample IV at each subsample boundary) — a latent bug in the shipped
  decrypt path too; triangulated against Bento4 + Shaka (ISO/IEC 23001-7 is
  unowned, so the reference implementations are the source of truth). The
  single-subsample `h264_cbcs.mp4` fixture regression stays byte-exact.
- **cbcs 16-byte constant IV** (`IvGen::Constant`) — the standard real-world
  convention; Bento4 silently no-ops on the previous 8-byte per-sample IV.

## Security (review hardening)
The encrypt API now errors instead of silently shipping unprotected/corrupt
output: rejects per-sample IV length ∉ {8, 16} (empty IV → all-zero CTR counter
→ two-time-pad), cbcs `crypt=0 / skip≠0` (silent plaintext, encrypt *and*
hostile-file decrypt), and cbcs pattern components > 15 (silent 4-bit
truncation). `protect_media_segment`'s `moof`/`saio` consistency `debug_assert`
promoted to a returned `Error`.

## Compatibility
- **Breaking:** `dash::ContentProtectionSystem` gained a `pssh: Option<Vec<u8>>`
  field — a full-field struct literal must add it (hence the minor bump, not a
  patch).
- Documented limitations: DASH `ContentProtection` signals one scheme/KID per
  `AdaptationSet`; `protect_*` are explicit post-processing calls (not
  auto-wired into `CmafMux::package`); `protect_media_segment` handles a
  single-fragment media segment.
- MSRV 1.86. Keys/KID/IV always caller-supplied — no DRM/license logic.
