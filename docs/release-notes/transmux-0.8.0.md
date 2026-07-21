# transmux 0.8.0 — 2026-07-02

The release that turns transmux from a one-way TS→CMAF remuxer into an
**any-to-any media container muxing hub**: demux any supported container into one
neutral IR (`Media`/`Track`), mux from it into any supported container. Additive
over 0.6.0 (0.7.0 was never published); the batch `build_init_segment` /
`build_media_segment` / `Segmenter` / `TrackSpec` / `Sample` APIs are unchanged.

Requires **broadcast-common ≥ 8.2.0** (the `mux` traits).

## Highlights

- **Hub foundation** — `Media`/`Track` IR + the `broadcast_common` `Unpackage`/
  `Package` (and `Encrypt`/`Decrypt`) traits every spoke implements.
- **Demux inputs (`Unpackage`):** MPEG-2 TS (`TsDemux`), fMP4/CMAF (`Fmp4Demux`,
  now all codecs), MPEG Program Stream (`PsDemux`), WebM/Matroska (`WebmDemux`).
- **Mux outputs (`Package`):** CMAF/fMP4 (`CmafMux`), progressive MP4
  (`ProgressiveMux`), MPEG-2 TS (`TsMux`), CMAF-HLS (`HlsPackager`), TS-HLS
  (`TsHlsPackager`), DASH MPD (`DashPackager`), low-latency DASH (`LlSegmenter`/
  `LlDashPackager`), Microsoft Smooth Streaming (`SmoothPackager`).
- **Transforms:** `Repackage` (resegment / trim / track-select).
- **Crypto:** CENC (`cenc` AES-CTR) decrypt (`CencDecryptor`, behind the `cenc`
  feature).
- **RTP:** `RtpPacketizer`/`RtpDepacketizer` + SDP (H.264 FU-A/STAP-A, AAC hbr).
- **Full codec config coverage:** added H.266/VVC, VP8, Vorbis, MPEG-2 video
  (H.262), MPEG-1/2 audio (MP1/2/3), and MPEG-H input — joining AVC/HEVC/AV1/VP9
  and AAC/AC-3/E-AC-3/AC-4/DTS/Opus/FLAC. 17 codecs, demux + mux.

Every spoke is gated on byte-exact round-trips through the IR against real
fixtures; codec config records are byte-verified against reference muxer output.

## Feature flags
- `cenc` (default): CENC decrypt (pulls `aes`/`ctr`). `--no-default-features`
  drops it.

## Examples
`cargo run -p transmux --example transmux_hub` (TS→IR→CMAF) and
`transmux_any_to_any` (WebM→IR→DASH+CMAF).
