# ISO/IEC 14496-1 §11.2 — FlexMux buffer descriptors

_Referenced normatively by ISO/IEC 13818-1 Table 2-80 (`FmxBufferSize_descriptor`)._

`private/specs/` does **not** vendor ISO/IEC 14496-1, and the standard is
paywalled, so this transcription is **secondary**: the two structures below
are reconstructed from three independent open-source implementations of the
same clause, not from the standard text itself. Every consumer in this repo
that relies on it cites that provenance.

## Provenance

- `videolan/bitstream` — `mpeg/psi/desc_22.h`: `DESC_HEADER_SIZE + 3` for the
  default record, `DESC_ENTRY_SIZE == 4` for an entry, validator
  `(length - 3) % 4 == 0`; per-entry getters read an 8-bit channel
  (`p_desc_n[0]`) and a 24-bit buffer size (`p_desc_n[1..3]`).
- `junka/tsanalyze` — `include/descriptor.h`, explicitly commented
  "see ISO/IEC 14496-1": `DefaultFlexMuxBufferDescriptor { uint24_t
  FB_DefaultBufferSize; }` and `FlexMuxBufferDescriptor { MuxChannel:8;
  FB_BufferSize:24 }`.
- `wangf1978/DumpTS` — `src/descriptors_13818_1.h`: the 4-byte entry stride.
- `Duckbox-Developers/dvbsnoop` — `src/descriptors/mpeg_descriptor.c`
  transcribes the ISO/IEC 13818-1 `FmxBufferSize_descriptor()` syntax
  verbatim, annotated "defined in subclause 11.2 of ISO/IEC 14496-1".

## DefaultFlexMuxBufferDescriptor()

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| `FB_DefaultBufferSize` | 24 | uimsbf |

**Total: 3 bytes.**

## FlexMuxBufferDescriptor()

| Syntax | No. of bits | Mnemonic |
|---|---|---|
| `MuxChannel` | 8 | uimsbf |
| `FB_BufferSize` | 24 | uimsbf |

**Total: 4 bytes.**

## Consequence for Table 2-80

A `FmxBufferSize_descriptor` body is one 3-byte
`DefaultFlexMuxBufferDescriptor()` followed by `(descriptor_length - 3) / 4`
4-byte `FlexMuxBufferDescriptor()` entries. The default record is **always
present** — the syntax has no optional marker — so a body shorter than 3
bytes, or one whose length after the default is not a multiple of 4, is not
a valid instance of this descriptor.
