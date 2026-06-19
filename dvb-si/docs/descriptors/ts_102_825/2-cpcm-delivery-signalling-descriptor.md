# Table 2 — CPCM delivery signalling descriptor

_Source: ETSI TS 102 825-9 V1.2.1 (2011-02) §4.1.5 (PDF p. 9). Verified against the
PDF render. Carried as an EN 300 468 extended descriptor (descriptor_tag 0x7F,
descriptor_tag_extension 0x01)._

| Syntax | Number of bits | Identifier |
|---|---|---|
| cpcm_delivery_signalling_descriptor() { |  |  |
| cpcm_version | 8 | bslbf |
| for (i=0; i<N; i++) { |  |  |
| selector_byte | 8 | bslbf |
| } |  |  |
| } |  |  |

Semantics:

- **cpcm_version**: the encoding version of the CPCM USI data structure conveyed in
  the `selector_byte`s that follow. v1 is `0x01` (defined in TS 102 825-4).
- **selector_byte**: the version-dependent selector field. Its syntax/semantics are
  coded per the encoding version in `cpcm_version`; for v1 the structure is the
  CPCM delivery signalling (USI) defined in **TS 102 825-4** (Table 8, CPCM USI).
  At the descriptor level the selector is a version-tagged opaque payload — this
  crate exposes `cpcm_version` + the `selector_bytes`; decoding the v1 USI body is
  the consumer's (or a CPCM stack's) concern.
