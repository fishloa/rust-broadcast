# Vendored ETSI specification PDFs

Canonical source-of-truth standards for the `rust-dvb` crates. The
structured markdown under each crate's `docs/` is transcribed from these.
Kept in-repo so the parsers can always be checked against the spec they
claim to implement.

These are freely-downloadable ETSI deliverables, redistributed here for
reference. ETSI retains copyright; see <https://www.etsi.org/>.

| File | Standard | Used by |
|---|---|---|
| `etsi_en_300_468_v01.19.01_dvb_si.pdf` | EN 300 468 v1.19.1 — DVB Service Information | `dvb-si` (all SI tables, incl. SAT §5.2.11) |
| `etsi_en_301_192_v01.07.01_dvb_databcast.pdf` | EN 301 192 v1.7.1 — DVB data broadcasting | `dvb-si` (INT, table_id 0x4C) |
| `etsi_ts_102_006_v01.07.01_dvb_ssu.pdf` | TS 102 006 v1.7.1 — System Software Update | `dvb-si` (UNT, table_id 0x4B) |
| `etsi_ts_102_323_v01.04.01_dvb_tvanytime.pdf` | TS 102 323 v1.4.1 — TV-Anytime carriage | `dvb-si` (RCT 0x76, CIT 0x77, RNT 0x79, container 0x75) |
| `etsi_ts_102_809_v01.03.01_dvb_app_signalling.pdf` | TS 102 809 v1.3.1 — Signalling and carriage of interactive applications | `dvb-si` (AIT 0x74, protection message 0x7B) |
| `etsi_ts_102_772_v01.01.01_dvb_mpe_ifec.pdf` | TS 102 772 v1.1.1 — MPE inter-burst FEC | `dvb-si` (MPE-IFEC section 0x7A) |
| `etsi_en_303_560_v01.01.01_dvb_ttml_fonts.pdf` | EN 303 560 v1.1.1 — TTML subtitling systems | `dvb-si` (downloadable font info 0x7C) |
| `etsi_ts_102_727_v01.01.01_dvb_mhp.pdf` | TS 102 727 v1.1.1 — Multimedia Home Platform 1.2.2 | `dvb-si` (XAIT_location descriptor 0x7D, §10.17.6) |
| `etsi_tr_101_202_v01.02.01_dvb_data_broadcasting_guidelines.pdf` | TR 101 202 v1.2.1 — Data broadcasting implementation guidelines | `dvb-si` (`carousel` module — DVB profile semantics; the message byte-syntax itself is ISO/IEC 13818-6, see `dvb-si/docs/iso_13818_6_carousel.md`) |
| `etsi_en_302_307_1_v01.04.01_dvb_s2.pdf` | EN 302 307-1 — DVB-S2 | `dvb-bbframe` |
| `etsi_en_302_307_2_v01.04.01_dvb_s2x.pdf` | EN 302 307-2 — DVB-S2X | `dvb-bbframe` |
| `etsi_en_302_755_v01.04.01_dvb_t2.pdf` | EN 302 755 v1.4.1 — DVB-T2 | `dvb-bbframe`, `dvb-t2mi` |
| `etsi_ts_102_773_v01.04.01_dvb_t2_modulator_interface.pdf` | TS 102 773 v1.4.1 — T2-MI | `dvb-t2mi` |

Download URLs (browser User-Agent required; bare `curl` is blocked by ETSI):
- https://www.etsi.org/deliver/etsi_en/300400_300499/300468/01.19.01_60/en_300468v011901p.pdf
- https://www.etsi.org/deliver/etsi_ts/102000_102099/102006/01.07.01_60/ts_102006v010701p.pdf
- https://www.etsi.org/deliver/etsi_en/301100_301199/301192/01.07.01_60/en_301192v010701p.pdf
- https://www.etsi.org/deliver/etsi_ts/102300_102399/102323/01.04.01_60/ts_102323v010401p.pdf
- https://www.etsi.org/deliver/etsi_ts/102800_102899/102809/01.03.01_60/ts_102809v010301p.pdf
- https://www.etsi.org/deliver/etsi_ts/102700_102799/102772/01.01.01_60/ts_102772v010101p.pdf
- https://www.etsi.org/deliver/etsi_en/303500_303599/303560/01.01.01_60/en_303560v010101p.pdf
- https://www.etsi.org/deliver/etsi_ts/102700_102799/102727/01.01.01_60/ts_102727v010101p.pdf
- https://www.etsi.org/deliver/etsi_tr/101200_101299/101202/01.02.01_60/tr_101202v010201p.pdf
- https://www.etsi.org/deliver/etsi_en/302300_302399/30230701/01.04.01_60/en_30230701v010401p.pdf
- https://www.etsi.org/deliver/etsi_en/302300_302399/30230702/01.04.01_60/en_30230702v010401p.pdf
- https://www.etsi.org/deliver/etsi_en/302700_302799/302755/01.04.01_60/en_302755v010401p.pdf
- https://www.etsi.org/deliver/etsi_ts/102700_102799/102773/01.04.01_60/ts_102773v010401p.pdf
