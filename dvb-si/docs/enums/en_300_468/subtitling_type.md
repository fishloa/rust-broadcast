# subtitling type

_ETSI EN 300 468 Table 26 — subtitling_type codes_

> Values rendered from the co-located drift-guard [`subtitling_type.toml`](./subtitling_type.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x01 | `EbuTeletextSubtitles` | EBU teletext subtitles |
| 0x02 | `AssociatedEbuTeletext` | associated EBU teletext |
| 0x03 | `VbiData` | VBI data |
| 0x10 | `DvbSubtitlesNormal` | DVB subtitles (normal), no aspect ratio critical |
| 0x11 | `DvbSubtitlesNormal4x3` | DVB subtitles (normal), 4:3 |
| 0x12 | `DvbSubtitlesNormal16x9` | DVB subtitles (normal), 16:9 |
| 0x13 | `DvbSubtitlesNormal2p21x1` | DVB subtitles (normal), 2.21:1 |
| 0x14 | `DvbSubtitlesNormalHd` | DVB subtitles (normal), HD |
| 0x15 | `DvbSubtitlesNormalPlanoStereoscopicHd` | DVB subtitles (normal), plano-stereoscopic disparity, HD |
| 0x16 | `DvbSubtitlesNormalUhd` | DVB subtitles (normal), UHD |
| 0x20 | `DvbSubtitlesHardOfHearing` | DVB subtitles (hard of hearing), no aspect ratio critical |
| 0x21 | `DvbSubtitlesHardOfHearing4x3` | DVB subtitles (hard of hearing), 4:3 |
| 0x22 | `DvbSubtitlesHardOfHearing16x9` | DVB subtitles (hard of hearing), 16:9 |
| 0x23 | `DvbSubtitlesHardOfHearing2p21x1` | DVB subtitles (hard of hearing), 2.21:1 |
| 0x24 | `DvbSubtitlesHardOfHearingHd` | DVB subtitles (hard of hearing), HD |
| 0x25 | `DvbSubtitlesHardOfHearingPlanoStereoscopicHd` | DVB subtitles (hard of hearing), plano-stereoscopic disparity, HD |
| 0x26 | `DvbSubtitlesHardOfHearingUhd` | DVB subtitles (hard of hearing), UHD |
| 0x30 | `OpenSignLanguage` | open (in-vision) sign language interpretation |
| 0x31 | `ClosedSignLanguage` | closed sign language interpretation |
