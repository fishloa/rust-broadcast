# ETSI TS 101 154 v2.10.1 — AV coding constraints in DVB (selected tables)

Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed.

> Wire-structure reference, table-per-file for deep-linking. Each linked file
> carries one syntax/enum table **plus its field semantics** — enough to drive a
> spec-accurate Rust parser (symmetric Parse/Serialize; coded enums get TOML
> drift-guards when implemented). Transcribed via BlazeDocs (table oracle; not
> pdftotext), spot-checked vs the PDF render. No parser implemented yet.

## Tables

- [Table 8 — Resolutions for Full-screen Display from 25 Hz H.264/AVC SDTV IRD and supported by 25 Hz H.264/AVC HDTV IRD, 50 Hz H.264/AVC HDTV IRD, 25 Hz SVC HDTV IRD and 50 Hz SVC HDTV IRD](tables/8-resolutions-for-full-screen-display-from-25-hz-h-264-avc-sdt.md)
- [Table 9 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for 30 Hz H.264/AVC SDTV](tables/9-time-scal-and-num-units-in-tick-for-progressive-and-interlac.md)
- [Table 10 — Resolutions for Full-screen Display from 30 Hz H.264/AVC SDTV IRD, and supported by 30 Hz H.264/AVC HDTV IRD, 60 Hz H.264/AVC HDTV IRD, 30 Hz SVC HDTV IRD and 60 Hz SVC HDTV IRD](tables/10-resolutions-for-full-screen-display-from-30-hz-h-264-avc-sdt.md)
- [Table 11 — Resolutions for Full-screen Display from H.264/AVC HDTV IRD and SVC HDTV IRD](tables/11-resolutions-for-full-screen-display-from-h-264-avc-hdtv-ird-.md)
- [Table 12 — Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for 25 Hz H.264/AVC HDTV, 50 Hz H.264/AVC HDTV, 25 Hz SVC HDTV, 50 Hz SVC HDTV and 25 Hz MVC Stereo HDTV](tables/12-time-scal-and-num-units-in-tick-for-progressive-and-interlac.md)
- [Table 19 — Progressive and Interlaced Frame Rates for HEVC Bitstreams and recommended values for signalling](tables/19-progressive-and-interlaced-frame-rates-for-hevc-bitstreams-a.md)
- [Table 20 — Resolutions for Full-screen Display from HEVC HDTV IRD](tables/20-resolutions-for-full-screen-display-from-hevc-hdtv-ird.md)
- [Table 21 — Resolutions for Full-screen Display from HEVC UHDTV IRD](tables/21-resolutions-for-full-screen-display-from-hevc-uhdtv-ird.md)
- [Table B.11 — Active Format Description for H264/AVC video](tables/B_11-active-format-description-for-h264-avc-video.md)
- [Table B.12 — Auxiliary Data for VC-1 video](tables/B_12-auxiliary-data-for-vc-1-video.md)
- [Table B.13 — Support for WSS](tables/B_13-support-for-wss.md)
- [Table E.1 — AD_descriptor](tables/E_1-ad-descriptor.md)
- [Table L.1 — Player conformance points](tables/L_1-player-conformance-points.md)
