# Incoming Spec Manifest

Files downloaded 2026-06-27 for upcoming HLS/LL-HLS, codec, ID3, timed-metadata, MSE, QTFF, and container epics.

## Succeeded

| Filename | Standard | Source URL | Size | Format | Future Epic |
|---|---|---|---|---|---|
| `ietf_rfc8216_hls.txt` | IETF RFC 8216 — HTTP Live Streaming | https://www.rfc-editor.org/rfc/rfc8216.txt | 124 KB | TXT (PDF 404s at rfc-editor) | HLS / timed-metadata |
| `ietf_draft_pantos_hls_rfc8216bis.txt` | draft-pantos-hls-rfc8216bis-22 — HLS 2nd Edition (LL-HLS) | https://www.ietf.org/archive/id/draft-pantos-hls-rfc8216bis-22.txt | 270 KB | TXT | LL-HLS / timed-metadata |
| `itu_t_h264_avc.pdf` | ITU-T H.264 (08/2024) — Advanced Video Coding | https://www.itu.int/rec/dologin_pub.asp?lang=e&id=T-REC-H.264-202408-I!!PDF-E&type=items | 14.7 MB | PDF (854 pages, `%PDF`) | codec / zenith-fMP4 |
| `itu_t_h265_hevc.pdf` | ITU-T H.265 (01/2026) — HEVC | https://www.itu.int/rec/dologin_pub.asp?lang=e&id=T-REC-H.265-202601-I!!PDF-E&type=items | 11.7 MB | PDF (734 pages, `%PDF`) | codec / zenith-fMP4 |
| `id3v2.4.0_structure.txt` | ID3 v2.4.0 — Structure | https://id3.org/id3v2.4.0-structure | 95 KB | HTML (saved as .txt per instruction) | timed-metadata / ID3 |
| `id3v2.4.0_frames.txt` | ID3 v2.4.0 — Frames | https://id3.org/id3v2.4.0-frames | 217 KB | HTML (saved as .txt per instruction) | timed-metadata / ID3 |
| `apple_hls_timed_metadata.html` | Apple "Timed Metadata for HTTP Live Streaming" v1.2.1 | https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/HTTP_Live_Streaming_Metadata_Spec/Introduction/Introduction.html | 12 KB | HTML (single-page archive doc) | timed-metadata |
| `w3c_media_source_extensions.html` | W3C Media Source Extensions Level 2 | https://www.w3.org/TR/media-source-2/ | 904 KB | HTML | media-doctor / MSE |
| `apple_quicktime_file_format.pdf` | Apple QuickTime File Format (2001 classic spec) | https://developer.apple.com/standards/qtff-2001.pdf | 5.3 MB | PDF (`%PDF`) | zenith-fMP4 / ISOBMFF base |

**Total: ~33.3 MB across 9 files**

## Failed

| Filename | Standard | Attempted URL | Reason |
|---|---|---|---|
| `3gpp_ts_26244_3gp_file_format.pdf` | 3GPP TS 26.244 v19.0.0 — 3GP File Format | https://www.etsi.org/deliver/etsi_ts/126200_126299/126244/19.00.00_60/ts_126244v190000p.pdf | Cloudflare WAF returns HTTP 403 for all automated curl fetches to etsi.org/deliver. Older ETSI versions (v12, v14) also 403. The 3GPP FTP archive (3gpp.org) only distributes .docx (Word) not PDF. **Workaround:** download manually from https://www.etsi.org/deliver/etsi_ts/126200_126299/126244/19.00.00_60/ts_126244v190000p.pdf in a browser. |
