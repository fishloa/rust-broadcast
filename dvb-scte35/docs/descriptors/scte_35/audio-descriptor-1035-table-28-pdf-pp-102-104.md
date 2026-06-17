## audio_descriptor() — §10.3.5, Table 28, PDF pp. 102-104

Implementation of a splice_descriptor() that dynamically signals the audios
actually in use in the stream, for programmers/MVPDs that do not support
dynamic signaling and for legacy audio formats that do not support it (see
[SCTE 248] §9.1.5). Shall only be used with a time_signal command and a
segmentation descriptor with the type Program_Start or
Program_Overlap_Start.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `audio_descriptor() {` |  |  |
| &nbsp;&nbsp;splice_descriptor_tag | 8 | uimsbf |
| &nbsp;&nbsp;descriptor_length | 8 | uimsbf |
| &nbsp;&nbsp;identifier | 32 | uimsbf |
| &nbsp;&nbsp;audio_count | 4 | uimsbf |
| &nbsp;&nbsp;reserved | 4 | bslbf |
| &nbsp;&nbsp;`for (i=0; i<audio_count; i++) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;component_tag | 8 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;ISO_code | 24 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Bit_Stream_Mode | 3 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Num_Channels | 4 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;Full_Srvc_Audio | 1 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| `}` |  |  |

- **splice_descriptor_tag** — shall be **0x04**.
- **identifier** — shall be **0x43554549** (ASCII "CUEI").
- **audio_count** — the number of audio PIDs in the program.
- **component_tag** — an optional 8-bit value identifying the elementary PID
  stream containing the audio channel that follows; if used, the value shall
  be the same as in the stream_identifier_descriptor(). If not used, the
  value shall be **0xFF** and the stream order shall be inferred from the
  PMT audio order.
- **ISO_code** — a 3-byte language code defining the language of this audio
  service, corresponding to a registered language code in the Code column of
  the [ISO 639-2] registry.
- **Bit_Stream_Mode** — as per ATSC A/52 Table 5.7.
- **Num_Channels** — as per ATSC A/52 Table A4.5.
- **Full_Srvc_Audio** — 1 bit (from ATSC A/52 Annex A.4.3): '1' if this
  audio service is sufficiently complete to be presented to the listener
  without being combined with another audio service; '0' if it is not
  sufficiently complete (e.g. a visually impaired narrative service that
  must be combined with the music/effects/dialogue service).

