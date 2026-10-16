# MPEG-H_3dAudio_descriptor — ITU-T H.222.0 (08/2023) | ISO/IEC 13818-1:2023

Source: `private/specs/itu_t_h222_0_202308_mpeg2_systems.pdf`, clause 2.6.106
Table 2-121 and clause 2.6.107 semantics. **Verified on rendered page (PDF
p.156, printed p.134)**; the markdown extract mangles the `numCompatibleSets`
conditional. The descriptor is the body of the `extension_descriptor` (tag
`0x3F`) selected by `extension_descriptor_tag` `0x08` (Table 2-110).

## Table 2-121

| Syntax | Bits | Mnemonic |
|---|---|---|
| `mpegh3daProfileLevelIndication` | 8 | uimsbf |
| `interactivityEnabled` | 1 | bslbf |
| `compatibleProfileSetsPresent` | 1 | bslbf |
| `reserved` | 8 | bslbf |
| `referenceChannelLayout` | 6 | uimsbf |
| `if (compatibleProfileSetsPresent == '0') {` | | |
| &nbsp;&nbsp;`numCompatibleSets` | 8 | uimsbf |
| &nbsp;&nbsp;`for (n < numCompatibleSets) CompatibleSetIndication` | 8 | uimsbf |
| `}` | | |
| `for (i = 0; i < N; i++) reserved` | 8 | bslbf |

Semantics (2.6.107): `interactivityEnabled` `0` = no user interactivity of any
kind is available; `compatibleProfileSetsPresent` set to `'0'` **indicates the
presence** of compatible profile sets; `numCompatibleSets` shall be >= 1;
`referenceChannelLayout` is the ISO/IEC 23001-8 `ChannelConfiguration`.

> **Spec note:** the flag polarity is inverted relative to its name (`'0'` =
> present). The fixed part is 1 + 2 bytes: the `reserved` run (8 bits) sits
> between the two flag bits and the 6-bit `referenceChannelLayout`, so byte 1 =
> `interactivityEnabled | compatibleProfileSetsPresent | reserved[7:2]` and byte
> 2 = `reserved[1:0] | referenceChannelLayout`.

## Real-fixture cross-check

`private/fixtures/ts/mpegh-cicp01-baseline.ts` ES_info: `3F 04 08 10 7F C1` =
tag, length 4, `extension_descriptor_tag` 0x08, profile/level 0x10, `0x7F`
(interactivity 0, compatible-sets flag 1 = absent, reserved ones), `0xC1`
(reserved `11`, `referenceChannelLayout` = 1). The muxer reproduces these
bytes exactly for profile 0x10 / layout 1 (unit test
`mpegh_3daudio_descriptor_matches_real_fixture_bytes`).
