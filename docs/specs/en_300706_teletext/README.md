# ETSI EN 300 706 v1.2.1 — Enhanced Teletext (packet-coding wire reference)

Source: specs/etsi_en_300_706_v01.02.01_enhanced_teletext.pdf §9-12 packet codings (PDF pp. 23-46). Character-set / display glyph tables (35+) deferred (rendering, not parsing).

> Wire-structure reference, table-per-file for deep-linking. Each linked file
> carries one syntax/enum table **plus its field semantics** — enough to drive a
> spec-accurate Rust parser (symmetric Parse/Serialize; coded enums get TOML
> drift-guards when implemented). Transcribed via BlazeDocs (table oracle; not
> pdftotext), spot-checked vs the PDF render. No parser implemented yet.

## Tables

- [Table 1 — Summary of packet types, their function and application](tables/01-summary-of-packet-types-their-function-and-application.md)
- [Table 2 — Control bits in the page header](tables/02-control-bits-in-the-page-header.md)
- [Table 3 — Page function and page coding bits (packets X/28/0 Format 1, X/28/3 and X/28/4)](tables/03-page-function-and-page-coding-bits-packets-x-28-0-format-1-x.md)
- [Table 4 — Coding of packet X/28/0 Format 1 for basic Level 1 pages](tables/04-coding-of-packet-x-28-0-format-1-for-basic-level-1-pages.md)
- [Table 5 — Coding of Packet X/28/0 Format 1 for Data Broadcasting Pages](tables/05-coding-of-packet-x-28-0-format-1-for-data-broadcasting-pages.md)
- [Table 6 — Coding of Packet X/28/0 Format 1 for other types of pages](tables/06-coding-of-packet-x-28-0-format-1-for-other-types-of-pages.md)
- [Table 7 — Page function and page coding bits (packets X/28/0 Format 2 and X/28/2)](tables/07-page-function-and-page-coding-bits-packets-x-28-0-format-2-a.md)
- [Table 8 — Coding of Packet X/28/1](tables/08-coding-of-packet-x-28-1.md)
- [Table 9 — Coding of Packet X/28/3 for DRCS Downloading Pages](tables/09-coding-of-packet-x-28-3-for-drcs-downloading-pages.md)
- [Table 10 — Coding of packet X/28/4 for basic Level 1 pages](tables/10-coding-of-packet-x-28-4-for-basic-level-1-pages.md)
- [Table 11 — Coding of Packet M/29/0](tables/11-coding-of-packet-m-29-0.md)
- [Table 12 — Coding of Packet M/29/1](tables/12-coding-of-packet-m-29-1.md)
- [Table 13 — Coding of Packet M/29/4](tables/13-coding-of-packet-m-29-4.md)
- [Table 14 — Coding of Packet X/27/0-3](tables/14-coding-of-packet-x-27-0-3.md)
- [Table 15 — Coding of Packets X/27/4 and X/27/5, Format 1](tables/15-coding-of-packets-x-27-4-and-x-27-5-format-1.md)
- [Table 16 — Fixed Link Functions of Packets X/27/4 and X/27/5](tables/16-fixed-link-functions-of-packets-x-27-4-and-x-27-5.md)
- [Table 17 — Coding of Format 2 packets X/27/4 - X/27/7](tables/17-coding-of-format-2-packets-x-27-4-x-27-7.md)
- [Table 18 — Coding of Packet 8/30 Format 1](tables/18-coding-of-packet-8-30-format-1.md)
- [Table 19 — Coding of Packet 8/30 Format 2](tables/19-coding-of-packet-8-30-format-2.md)
