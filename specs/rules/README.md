# `specs/rules/` — curated behavioural-rules summaries

Per-spec **semantic** rule summaries the workspace's crates depend on: the prose, constraints,
ordering, frequency, validity, and reserved-bit policy — **not** just the syntax tables. Each file
references the full pdf2md text in `specs/fulltext/` (gitignored: the copyrighted raw spec text)
with **§ + line citations**, so a code design decision can cite the documented prose instead of
asserting it (see memory `full-spec-comprehension-extraction`).

These files **are committed** (curated summaries, not the copyrighted full text). To regenerate a
`specs/fulltext/*.md` from its PDF, use the pdf2md skill (`--engine textlayer --report`, expect
exit 0). `specs/MEDIA-SPECS-LOCAL.md` is the PDF manifest.

| Rules file | Spec | Full text (gitignored) | Drives |
|---|---|---|---|
| `h222_0-rules.md` | ITU-T H.222.0 / ISO 13818-1 (MPEG-2 Systems) | `itu_t_h222_0_202308_mpeg2_systems.md` | `mpeg-ts`, `mpeg-pes`, `ts-fix`, `dvb-conformance` |
| `isobmff-rules.md` | ISO/IEC 14496-12:2015 (ISOBMFF) | `iso_iec_14496-12_isobmff_2015.md` | transmux / fMP4-mux, `mp4-emsg` |
| `mp4-esds-rules.md` | ISO/IEC 14496-1:2010 + 14496-14:2003 (`esds`/ES_Descriptor) | `iso_iec_14496-1_systems_es_descriptor_2010.md`, `iso_iec_14496-14_mp4_2003.md` | transmux codec-config lift |
| `dash-mpd-rules.md` | ISO/IEC 23009-1:2012 (DASH MPD — timing only) | `iso_iec_23009-1_dash_2012.md` | `timed-metadata`, future DASH crate |
| `emsg-rules.md` | DASH Event Message Box (`emsg`) — ISO/IEC 23009-1:2022 ed.5 §5.10 (+ DASH-IF IOP Part 10 v5.0.0 for usage) | `iso_iec_23009-1_dash_2022.md`, `dashif_iop_part10_v5_emsg.md` | `mp4-emsg` (implemented), `timed-metadata` |

## Known gaps (tracked, not silently skipped)

- **Codec-config records not yet curated** (referenced as "lift per spec" by the rules above): AAC
  `AudioSpecificConfig` (14496-3 §1.6), `avcC`/`hvcC` (**14496-15**). 14496-15's vendored PDF is
  **scanned** (OCR-only, not value-verifiable) — needs a **text-layer 14496-15** before curating;
  cross-check against FFmpeg `movenc.c`, never treat the scan as primary. Codec-config is a later
  story (the media pivot is container-layer first).
