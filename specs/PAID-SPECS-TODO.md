# Paid Specs TODO

These specifications require purchase and are **not** committed to the repository.
Per repo policy: consult locally (gitignore `iso_iec_*`), commit only hand-transcriptions.

| ISO Number | Title | What Epic Needs It | Approx Cost | Note |
|---|---|---|---|---|
| ISO/IEC 14496-12 | Information technology — Coding of audio-visual objects — Part 12: ISO base media file format (ISOBMFF) | zenith-fMP4 / media-doctor box parsing | ~CHF 188 | Base container for MP4/fMP4/CMAF; boxes/atoms layout. Consult locally — gitignore `iso_iec_*`, commit only hand-transcriptions per repo policy. |
| ISO/IEC 14496-15 | Information technology — Coding of audio-visual objects — Part 15: Carriage of NAL unit structured video in the ISO base media file format | zenith-fMP4 / H.264/265 sample entry | ~CHF 188 | AVC/HEVC track box layouts (`avcC`, `hvcC`). Same policy. |
| ISO/IEC 23000-19 | Information technology — Multimedia application format (MPEG-A) — Part 19: Common media application format (CMAF) | zenith-fMP4 / CMAF segmenting | ~CHF 188 | CMAF track/chunk/segment constraints on top of ISOBMFF. Same policy. |
| ISO/IEC 23009-1 | Information technology — Dynamic adaptive streaming over HTTP (DASH) — Part 1: Media presentation description and segment formats | media-doctor / DASH manifest parsing | ~CHF 226 | MPD XML schema + segment timeline. Same policy. |
| ISO/IEC 14496-3 | Information technology — Coding of audio-visual objects — Part 3: Audio (AAC/HE-AAC) | codec epic / AAC decoder config | ~CHF 226 | AudioSpecificConfig + ADTS framing. Same policy. |
