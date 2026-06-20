# DTS Neural descriptor — ETSI EN 300 468 V1.19.1 Annex L.1 (extension descriptor, tag_extension 0x0F)

_Source: ETSI EN 300 468 V1.19.1 (2025-02), Annex L (normative), §L.1 (PDF p. 225).
Verified against the PDF render. Extended descriptor (descriptor_tag 0x7F); follows
the associated audio descriptor in the PMT ES_info loop._

## Table L.1 — DTS Neural descriptor

| Syntax | No. of bits | Identifier |
|---|---|---|
| DTS_Neural_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| config_id | 8 | uimsbf |
| for (i=0; i<N; i++) { additional_info_byte | 8 | uimsbf |
| } |  |  |
| } |  |  |

Semantics:

- **config_id**: audio channel configuration of the host audio stream. For a stereo
  host stream coded per Table L.2; for a 5.1 host stream coded per Table L.3.
  Stored as the raw 8-bit value (the L.2/L.3 mappings are the label tables).
- **additional_info_byte**: optional reserved tail.
