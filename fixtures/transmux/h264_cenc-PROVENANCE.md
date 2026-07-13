# `h264_cenc.mp4` / `h264_cbcs.mp4` — provenance

Real CENC/CBCS-encrypted CMAF fixtures for issue #564 (transmux full CENC/CBCS box support),
produced with **Bento4's `mp4encrypt`** (an independent, actively-maintained open-source MP4
encryption tool — not this project's own code) against the already-committed real fixture
`fixtures/transmux/h264_aac_frag.mp4`.

## Why Bento4, not a hand-built fixture

ISO/IEC 23001-7 (Common Encryption) is a paid ISO standard not owned by this project (see
`specs/MEDIA-SPECS-LOCAL.md`). Per the project's "no implementation without truthful source"
rule, the box-level syntax for `tenc`/`senc`/`saio`/`saiz`/`sinf`/`frma`/`schm`/`schi` is
grounded in two independent, cross-corroborating open-source implementations instead:
**Shaka Packager** (`packager/media/formats/mp4/box_definitions.{h,cc}`, Apache-2.0, Google)
and **Bento4** (`Ap4CommonEncryption.cpp`, `Ap4{Tenc,Senc,Saio,Saiz}Atom.{h,cpp}`, Axiomatic
Systems). Producing the fixture with Bento4's own `mp4encrypt`/`mp4decrypt` — rather than
constructing bytes by hand from the doc — means the fixture is a genuine third-party artifact:
this crate's parser is graded against real output from an implementation the crate's own code
did not write, and `mp4decrypt` (also Bento4, but the read side, independently exercised)
serves as the golden-interop oracle specified in #564's acceptance criteria.

## Commands used (reproducible)

```bash
brew install bento4

# CENC (AES-CTR, full-sample + subsample)
mp4encrypt --method MPEG-CENC \
  --key 1:000102030405060708090a0b0c0d0e0f:100102030405060708090a0b0c0d0e0f \
  --property 1:KID:000102030405060708090a0b0c0d0e0f \
  fixtures/transmux/h264_aac_frag.mp4 fixtures/transmux/h264_cenc.mp4

# CBCS (AES-CBC, pattern 1:9 crypt:skip)
mp4encrypt --method MPEG-CBCS \
  --key 1:000102030405060708090a0b0c0d0e0f:100102030405060708090a0b0c0d0e0f \
  --property 1:KID:000102030405060708090a0b0c0d0e0f \
  fixtures/transmux/h264_aac_frag.mp4 fixtures/transmux/h264_cbcs.mp4
```

Only track 1 (video, H.264) is encrypted in both fixtures — Bento4 warns and skips track 2
(AAC audio), which is expected default `mp4encrypt` behavior without a second `--key`/track
selector. This is fine for the crate's purposes: it exercises the encrypted-track box tree
(`sinf`/`tenc`/`senc`/`saiz`/`saio` in both `trak` and the corresponding `traf`) while leaving
one real unencrypted track alongside it, matching a genuine mixed-protection multiplex.

## Key material (test-only, not secret)

- **Key:** `000102030405060708090a0b0c0d0e0f` (16 bytes, 0x00..0x0F)
- **IV / second key half:** `100102030405060708090a0b0c0d0e0f`
- **KID:** `000102030405060708090a0b0c0d0e0f` (same as the key, for easy recognition — not
  a real-world practice, purely a test fixture convenience)

## Verified independently

- `mp4dump fixtures/transmux/h264_cenc.mp4` shows the expected box tree: `sinf` (`frma`
  original_format=avc1, `schm` scheme_type=cenc, `schi`/`tenc` default_isProtected=1,
  default_Per_Sample_IV_Size=16, default_KID matching the key above) in `trak`, and
  `saiz`/`saio`/`senc` in each `traf`.
- `mp4dump fixtures/transmux/h264_cbcs.mp4` shows the same tree with `schm` scheme_type=cbcs.
- `mp4decrypt --key <key>:<iv> fixtures/transmux/h264_{cenc,cbcs}.mp4 <out>.mp4` round-trips
  cleanly (exit 0) for both — confirming the fixtures are genuinely decryptable, not just
  structurally plausible.
