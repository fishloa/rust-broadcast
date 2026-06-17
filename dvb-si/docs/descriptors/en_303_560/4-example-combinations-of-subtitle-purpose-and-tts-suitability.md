## Table 4 — Example combinations of subtitle_purpose and TTS_suitability
_§5.2.1.1, PDF pp. 16-16_

| Subtitle_purpose | TTS_suitability: Suitable for TTS | TTS_suitability: Unknown suitability for TTS | TTS_suitability: Not suitable for TTS |
|---|---|---|---|
| Dialogue (e.g. subtitle_purpose 0x00-0x02, 0x10-0x12) | Suitable for Spoken Subtitles | Possibly suitable for Spoken Subtitles | Not suitable for Spoken Subtitles |
| Hard-of-hearing (e.g. subtitle_purpose 0x10-0x12) | Suitable for alternative audio source for hearing impaired | Possibly suitable for alternative audio source for hearing impaired | Not suitable as alternative audio source for hearing impaired |
| Audio description (e.g. subtitle_purpose 0x30) | Suitable for audio description source | Invalid combination | Invalid combination |
| Content-related commentary (e.g. subtitle_purpose 0x31) | Suitable for Spoken Commentary | Possibly suitable for Spoken Commentary | Not suitable for Spoken Commentary |

