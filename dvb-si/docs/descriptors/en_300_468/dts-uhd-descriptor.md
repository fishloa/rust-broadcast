# DTS-UHD descriptor — ETSI EN 300 468 V1.19.1 Annex G.5 (extension descriptor, tag_extension 0x21)

_Source: ETSI EN 300 468 V1.19.1 (2025-02), Annex G (normative), §G.5.1 (PDF pp. 192–193).
Verified against the PDF render. Extended descriptor (descriptor_tag 0x7F) per §6.2.16;
PMT ES_info loop, at most one instance per relevant loop._

## Table G.15 — DTS-UHD descriptor

| Syntax | No. of bits | Identifier |
|---|---|---|
| DTS-UHD_descriptor() { |  |  |
| descriptor_tag | 8 | uimsbf |
| descriptor_length | 8 | uimsbf |
| descriptor_tag_extension | 8 | uimsbf |
| DecoderProfileCode | 6 | uimsbf |
| FrameDurationCode | 2 | uimsbf |
| MaxPayloadCode | 3 | uimsbf |
| DTS_reserved | 2 | bslbf |
| StreamIndex | 3 | uimsbf |
| for (i=0; i<N; i++) { codec_selector_byte | 8 | uimsbf |
| } |  |  |
| } |  |  |

Semantics:

- **DecoderProfileCode**: `DecoderProfile = DecoderProfileCode + 2`. 0 ⇒ DTS-UHD Profile 2, 1 ⇒ Profile 3; other values reserved.
- **FrameDurationCode**: PCM-sample frame duration at the 48 kHz base, per Table G.16.
- **MaxPayloadCode**: max audio payload size, per Table G.17.
- **DTS_reserved**: shall be '00' for DVB; if non-zero a codec selector field is present.
- **StreamIndex**: stream priority (main = 0; aux 1..7; single-stream = 0).
- **codec_selector_byte**: codec-defined selector field; length implied by descriptor_length; ignored by receivers.

## Table G.16 — FrameDurationCode coding

| FrameDurationCode | Description |
|---|---|
| 0 | 512 samples |
| 1 | 1 024 samples |
| 2 | 2 048 samples |
| 3 | 4 096 samples |

## Table G.17 — MaxPayloadCode coding

| MaxPayloadCode | Description |
|---|---|
| 0 | 2 048 byte |
| 1 | 4 096 byte |
| 2 | 8 192 byte |
| 3 | 16 384 byte |
| 4 | 32 768 byte |
| 5 | 65 536 byte |
| 6 | 131 072 byte |
| 7 | reserved for future use |

(The DTS-UHD BroadcastChunk that pairs with this — Table 28 of ETSI TS 101 154 — is
audio-frame metadata, out of scope for this SI descriptor.)
