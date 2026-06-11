# Vendored specification PDFs

Canonical source-of-truth standards for the `rust-dvb` crates. The
structured markdown under each crate's `docs/` is transcribed from these.
Kept in-repo so the parsers can always be checked against the spec they
claim to implement.

These are freely-downloadable deliverables (ETSI, DVB Project, and SCTE —
all published at no cost), redistributed here for reference. Each publisher
retains copyright; see <https://www.etsi.org/>, <https://dvb.org/>,
<https://www.scte.org/>.

Paid ISO standards (e.g. ISO/IEC 13818-1) are **never** vendored: they are
consulted locally (gitignored as `specs/iso_iec_*.pdf`) and only the
provenance-documented hand transcriptions under `*/docs/` are committed.

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
| `etsi_tr_101_290_v01.04.01_dvb_measurement.pdf` | TR 101 290 v1.4.1 — Measurement guidelines for DVB systems | planned `dvb-conformance` (#57); tables in `dvb-si/docs/tr_101_290.md` |
| `etsi_tr_101_211_v01.09.01_dvb_si_guidelines.pdf` | TR 101 211 v1.9.1 — SI implementation guidelines | `dvb-si` (SI repetition rates — packetizer/SiMux #56, conformance #57) |
| `ansi_scte_35_2023r1_dpi_cueing.pdf` | ANSI/SCTE 35 2023r1 — Digital Program Insertion Cueing Message | planned `dvb-scte35` (#58); tables in `dvb-scte35/docs/scte_35.md` |
| `dvb_a001r18_draft_ts_101_154_v02.07.01_av_coding.pdf` | DVB A001r18 (draft TS 101 154 v2.7.1) — AV coding in broadcast/broadband | `dvb-si` (component/AC-4/audio-preselection semantics, #53) |
| `etsi_ts_103_190_2_v01.03.01_ac4.pdf` | TS 103 190-2 v1.3.1 — AC-4 immersive/personalized audio | `dvb-si` (AC-4 descriptor `ac4_toc` structure §6.2.1 — deferred typing, #102) |

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
- https://www.etsi.org/deliver/etsi_tr/101200_101299/101290/01.04.01_60/tr_101290v010401p.pdf
- https://www.etsi.org/deliver/etsi_tr/101200_101299/101211/01.09.01_60/tr_101211v010901p.pdf
- https://dutchguild.nl/event/13/attachments/82/203/SCTE_35_2023r1.pdf (public mirror; canonical source is the SCTE standards library request form)
- https://dvb.org/wp-content/uploads/2021/02/A001r18_Use-of-Video-and-Audio-Coding-in-Broadcast-and-Broadband-Applications_Draft_TS_101-154-v271_Nov-2021.pdf

## Consulted locally (gitignored, not redistributed)

- **Rec. ITU-T H.222.0 (06/2021)** = ISO/IEC 13818-1 (MPEG-2 Systems), free-of-charge ITU-T text. Downloaded from <https://www.itu.int/rec/T-REC-H.222.0>; kept as `specs/iso_iec_13818-1_2021_systems_itu_h222.0.pdf` (gitignored). Source of the `dvb-si/docs/iso_13818_1_systems.md` Table 2-34 (stream_type) transcription.
