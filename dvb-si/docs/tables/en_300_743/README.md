# ETSI EN 300 743 v1.6.1 — DVB Subtitling

Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)

> Wire-structure reference, table-per-file for deep-linking. Each linked file
> carries one syntax/enum table **plus its field semantics** — enough to drive a
> spec-accurate Rust parser (symmetric Parse/Serialize; coded enums get TOML
> drift-guards when implemented). Transcribed via BlazeDocs (table oracle; not
> pdftotext), spot-checked vs the PDF render. No parser implemented yet.

## Tables

- [Table 3 — PES data field](tables/03-pes-data-field.md)
- [Table 4 — TS carriage of subtitle streams](tables/04-ts-carriage-of-subtitle-streams.md)
- [Table 5 — Subtitling type usage](tables/05-subtitling-type-usage.md)
- [Table 6 — Generic subtitling segment](tables/06-generic-subtitling-segment.md)
- [Table 7 — Segment types](tables/07-segment-types.md)
- [Table 8 — Display definition segment](tables/08-display-definition-segment.md)
- [Table 9 — Page composition segment](tables/09-page-composition-segment.md)
- [Table 10 — Page state](tables/10-page-state.md)
- [Table 11 — Region composition segment](tables/11-region-composition-segment.md)
- [Table 12 — Region level of compatibility](tables/12-region-level-of-compatibility.md)
- [Table 13 — Intended region pixel depth](tables/13-intended-region-pixel-depth.md)
- [Table 14 — Object type](tables/14-object-type.md)
- [Table 15 — Object provider flag](tables/15-object-provider-flag.md)
- [Table 16 — CLUT definition segment](tables/16-clut-definition-segment.md)
- [Table 17 — Object data segment](tables/17-object-data-segment.md)
- [Table 18 — Object coding method](tables/18-object-coding-method.md)
- [Table 19 — Recommended encoding of object_data_segment](tables/19-recommended-encoding-of-object-data-segment.md)
- [Table 20 — Pixel-data sub-block](tables/20-pixel-data-sub-block.md)
- [Table 21 — Data type](tables/21-data-type.md)
- [Table 22 — 2-bits per pixel code string](tables/22-2-bits-per-pixel-code-string.md)
- [Table 23 — switch_3 for 2-bits per pixel code](tables/23-switch-3-for-2-bits-per-pixel-code.md)
- [Table 24 — 4-bits per pixel code string](tables/24-4-bits-per-pixel-code-string.md)
- [Table 25 — switch_3 for 4-bits per pixel code](tables/25-switch-3-for-4-bits-per-pixel-code.md)
- [Table 26 — 8-bits per pixel code string](tables/26-8-bits-per-pixel-code-string.md)
- [Table 27 — Progressive pixel block](tables/27-progressive-pixel-block.md)
- [Table 28 — End of display set segment](tables/28-end-of-display-set-segment.md)
- [Table 29 — Disparity signalling segment](tables/29-disparity-signalling-segment.md)
- [Table 30 — disparity_shift_update_sequence](tables/30-disparity-shift-update-sequence.md)
- [Table 31 — Alternative CLUT segment](tables/31-alternative-clut-segment.md)
- [Table 32 — CLUT parameters](tables/32-clut-parameters.md)
- [Table 33 — Output bit-depth coding](tables/33-output-bit-depth-coding.md)
- [Table 34 — Dynamic range and colour gamut coding](tables/34-dynamic-range-and-colour-gamut-coding.md)
- [Table 42 — 2-bit/pixel_code_string()](tables/42-2-bit-pixel-code-string.md)
- [Table 43 — 4-bit/pixel_code_string()](tables/43-4-bit-pixel-code-string.md)
- [Table 44 — 8-bit/pixel_code_string()](tables/44-8-bit-pixel-code-string.md)
