# Table L.1: Player conformance points

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Name | Codec | Frame formats | Colorimetry | Resolutions (notes 1 and 2) | Frame rates (notes 1 and 3)  |
| --- | --- | --- | --- | --- | --- |
|  avc_hd_50_level40 | H.264/AVC Main and High Profile up to level 4.0 | Interlaced and progressive | Recommendation ITU-R BT.709 [13] | Up to 1 920 x 1 080 | 50 Hz family  |
|  avc_hd_60_level40 | H.264/AVC Main and High Profile up to level 4.0 | Interlaced and progressive | Recommendation ITU-R BT.709 [13] | Up to 1 920 x 1 080 | 60 Hz family  |
|  avc_hd_50 | H.264/AVC Main and High Profile up to level 4.2 | Interlaced and progressive | Recommendation ITU-R BT.709 [13] | Up to 1 920 x 1 080 | 50 Hz family  |
|  avc_hd_60 | H.264/AVC Main and High Profile up to level 4.2 | Interlaced and progressive | Recommendation ITU-R BT.709 [13] | Up to 1 920 x 1 080 | 60 Hz family  |
|  hevc_hd_50_8 | HEVC Main Profile up to level 4.1 | Progressive | Recommendation ITU-R BT.709 [13] | Up to 1 920 x 1 080 | 50 Hz family  |
|  hevc_hd_60_8 | HEVC Main Profile up to level 4.1 | Progressive | Recommendation ITU-R BT.709 [13] | Up to 1 920 x 1 080 | 60 Hz family  |
|  hevc_hd_50_10 | HEVC Main and Main 10 Profile up to level 4.1 | Progressive | Recommendation ITU-R BT.709 [13] | Up to 1 920 x 1 080 | 50 Hz family  |
|  hevc_hd_60_10 | HEVC Main and Main 10 Profile up to level 4.1 | Progressive | Recommendation ITU-R BT.709 [13] | Up to 1 920 x 1 080 | 60 Hz family  |
|  hevc_uhd | HEVC Main and Main 10 Profile up to level 5.1 | Progressive | Recommendations ITU-R BT.709 [13] and BT.2020 [36] | Up to 3 840 x 2 160 | 50 Hz and 60 Hz families  |
|  hevc_uhd_hlg10 | HEVC Main 10 Profile up to level 5.1 | Progressive | Recommendation ITU-R BT.2100 [45] HLG (note 4) | Up to 3 840 x 2 160 | 50 Hz and 60 Hz families  |
|  hevc_uhd_pq10 | HEVC Main 10 Profile up to level 5.1 | Progressive | Recommendation ITU-R BT.2100 [45] PQ (note 4) | Up to 3 840 x 2 160 | 50 Hz and 60 Hz families  |
|  hevc_uhd_hfr_hlg10 | HEVC Main 10 Profile up to level 5.2 | Progressive | Recommendation ITU-R BT.2100 [45] HLG (note 5) | Up to 3 840 x 2 160 | 50 Hz and 60 Hz families, and 100 Hz, 120/1 001 Hz, 120 Hz  |
|  hevc_uhd_hfr_pq10 | HEVC Main 10 Profile up to level 5.2 | Progressive | Recommendation ITU-R BT.2100 [45] PQ (note 5) | Up to 3 840 x 2 160 | 50 Hz and 60 Hz families, and 100 Hz, 120/1 001 Hz, 120 Hz  |
|  hevc_uhd2_hdr | HEVC Main 10 Profile up to level 6.1 | Progressive | Recommendation ITU-R BT.2100 [45] (note 5) | Up to 7 680 x 4 320 | 50 Hz and 60 Hz families  |
|  NOTE 1: Only resolution and frame rate combinations which fall within the capabilities of the specified level are required. The highest frame rates may not be possible in combination with the highest resolutions. NOTE 2: Specific resolutions for interoperability testing are defined in ETSI TS 103 285 [i.34], clause 10.3. NOTE 3: "50 Hz family" refers to 6.25 Hz, 12.5 Hz, 25 Hz and 50 Hz; "60 Hz family" refers to 6 000/1 001 Hz, 6 Hz, 7 500/1 001 Hz, 7.5 Hz, 12 000/1 001 Hz, 12 Hz, 15 000/1 001 Hz, 15 Hz, 24 000/1 001 Hz, 24 Hz, 30 000/1 001 Hz, 30 Hz, 60 000/1 001 Hz and 60 Hz. NOTE 4: These HDR player conformance points also support BT.709 and BT.2020 SDR transfer characteristics at the same resolutions and frame rates as for HDR. NOTE 5: These HDR player conformance points also support BT.2020 SDR transfer characteristics at the same resolutions and frame rates as for HDR.  |   |   |   |   |   |

Requirements for the player conformance points and the relationships with relevant broadcast IRDs in clause 5 are specified in clauses L.2.2 to L.2.17.
