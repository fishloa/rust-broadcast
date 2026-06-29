# `specs/rules/` — curated behavioural-rules summaries

Per-spec rule summaries the workspace's crates depend on — the prose, constraints, ordering,
frequency, validity, and reserved-bit policy, with **§ + line citations** into the full pdf2md text
in `specs/fulltext/` (gitignored copyrighted spec text). Committed (summaries, not the full text).
Regenerate a `specs/fulltext/*.md` with the pdf2md skill (`--engine textlayer --report`).
`specs/MEDIA-SPECS-LOCAL.md` is the PDF manifest.

| Rules file | Spec | Full text (gitignored) | Drives |
|---|---|---|---|
| `h222_0-rules.md` | ITU-T H.222.0 / ISO 13818-1 (MPEG-2 Systems) | `itu_t_h222_0_202308_mpeg2_systems.md` | `mpeg-ts`, `mpeg-pes`, `ts-fix`, `dvb-conformance` |
| `isobmff-rules.md` | ISO/IEC 14496-12:2015 (ISOBMFF) | `iso_iec_14496-12_isobmff_2015.md` | transmux / fMP4-mux, `mp4-emsg` |
| `mp4-esds-rules.md` | ISO/IEC 14496-1:2010 + 14496-14:2003 (`esds`/ES_Descriptor) | `iso_iec_14496-1_systems_es_descriptor_2010.md`, `iso_iec_14496-14_mp4_2003.md` | transmux codec-config lift |
| `dash-mpd-rules.md` | ISO/IEC 23009-1:2012 (DASH MPD — timing only) | `iso_iec_23009-1_dash_2012.md` | `timed-metadata`, future DASH crate |
| `emsg-rules.md` | DASH Event Message Box (`emsg`) — ISO/IEC 23009-1:2022 ed.5 §5.10 (+ DASH-IF IOP Part 10 v5.0.0 for usage) | `iso_iec_23009-1_dash_2022.md`, `dashif_iop_part10_v5_emsg.md` | `mp4-emsg` (implemented), `timed-metadata` |

| `nal-avcc-hvcc-rules.md` | ISO/IEC 14496-15:2017 §5.3–5.4, §8.3–8.4 (`avcC`/`hvcC` + sample entries) | `iso_iec_14496-15_avc_hevc_2017_excerpt.md` | transmux (H.264/H.265 → MP4) |

## Known gaps

- **AAC `AudioSpecificConfig`** (14496-3 §1.6) not yet curated — the audio counterpart of avcC/hvcC,
  needed for `mp4a`/ADTS transmux. PDF on disk (text-layer); a later story.
- **14496-15 footing**: its vendored PDF body is image-only; `nal-avcc-hvcc-rules.md` is
  vision-transcribed (excerpt in `specs/fulltext/`), cross-checked against FFmpeg `movenc.c`. A
  text-layer edition would let pdf2md value-verify it.
