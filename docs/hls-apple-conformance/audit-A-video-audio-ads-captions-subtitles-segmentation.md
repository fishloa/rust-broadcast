# Apple HLS Authoring Spec: conformance audit, Group A (sections 1, 2, 3, 4, 5, 7)

Checkout: detached origin/main at `.worktree/audit` (HEAD 90a30b58e). Paths below are relative to that tree. Read-only audit; no cargo run, code read only.

Reading guide. The origin that serves HLS is `multimux` (axum) over `hls_runtime::server::HlsOrigin` (`hls-runtime/src/server/engine.rs`, abbreviated EN) over a `media_plane::Trunk`. Segments are cut by `transmux` (`LlHlsSegmenter`, `StreamingTsHlsSegmenter`, `Segmenter`). Facts that drive many rows:

- F1. The origin master playlist has exactly ONE `EXT-X-STREAM-INF` variant (EN:1278-1290). `broadcast_hls::MasterPlaylist` has no model for `EXT-X-MEDIA` renditions (broadcast-hls/src/lib.rs:3217-3262, only `variants`, `iframe_variants`, `extra_tags`). No origin code emits `EXT-X-MEDIA`, `CLOSED-CAPTIONS`, `SUBTITLES`, `CHARACTERISTICS` or `AUTOSELECT` (`git grep` over multimux/src and hls-runtime/src: zero non-test hits).
- F2. All tracks of a program are muxed into every segment (LlHlsSegmenter and TS segmenter take every track; `CODECS` is the join of every track, EN:1227). There is no audio/video/subtitle rendition split and no track selection config in multimux.
- F3. The origin never emits `EXT-X-INDEPENDENT-SEGMENTS` (`MediaPlaylist { .. ..Default::default() }` at EN:1766-1783 and EN:1981; `MasterPlaylist` at EN:1278 likewise). The serializer supports the tag (broadcast-hls/src/lib.rs:1310, 3323) but no origin caller sets it.
- F4. Segment cut rule: first sync sample of the anchor (first video) track once the target duration is reached (transmux/src/segmenter.rs:488-492; transmux/src/ll_hls.rs:392-397; transmux/src/ts_hls.rs:423-424). "Sync" means a random-access point, which is wider than IDR (nal.rs:351-381, ts_demux.rs:2279-2296). See 7.4.
- F5. The only independent oracle is Apple `mediastreamvalidator` (media-doctor/tests/mediastreamvalidator_oracle.rs). It is macOS-only (skips elsewhere), gates only MUST-level findings (requirement level 1), and the two origin tests that feed it (:521 TS-HLS, :552 CMAF-HLS) exercise the `transmux`/`media-doctor run_bytes` packaging path, NOT `HlsOrigin`/multimux. I therefore do not count it as MET-VERIFIED evidence for any row except where noted.

Class legend: MET-V verified, MET-U unverified, GAP, PARTIAL, N/A (ENCODER/CONTENT/CLIENT/PLATFORM), GAP-OPT (encoder/content rule that the origin could verify or pass through but does not).

## Section 1: Video

| Item | Requirement | Class | Evidence (file:line) | Note |
|---|---|---|---|---|
| 1.1 | Video H.264/HEVC/DV/AV1 only | GAP-OPT | transmux/src/ir/codec.rs:524-535 (`is_video` lists VVC, VP8, VP9, MPEG-2); ts_mux.rs:438-446 | Origin repackages whatever codec arrives (MPEG-2, VP9, VVC, VP8 pass into CMAF/TS) and never rejects or flags a non-Apple codec. Dolby Vision: `dvh1`/`dvhe` are not recognised anywhere (init_segment.rs:3023-3026 only `avc1..`/`hvc1,hev1`/`av01`). |
| 1.2 | H.264 in fMP4 or TS | MET-U | EN:163-196 (Container Fmp4/MpegTs); pipeline.rs:610 | Only two containers are ever produced. No test asserts the container/codec pairing. |
| 1.3a | H.264 variants <= High@4.1 (SHOULD) | GAP-OPT | EN:1193 (CODECS from avcC) | avcC profile/level is available at `set_track_specs`; never compared. |
| 1.3b | H.264 <= High@5.2 | GAP-OPT | same | Same; no check. |
| 1.4 | Prefer High profile | N/A ENCODER | | |
| 1.5 | HEVC container MUST be fMP4 | GAP | ts_mux.rs:438 (`CodecConfig::Hevc => EsKind::Hevc`), ts_mux.rs:1434-1437; multimux/src/config.rs:1476-1490 | A `ts_hls` route carries HEVC in MPEG-TS (stream_type 0x24). The config validator only forbids mixing ts_hls with fMP4 outputs; it does not reject HEVC on a TS-HLS route. |
| 1.6a | HEVC <= Main10@4.0 (SHOULD) | GAP-OPT | EN:1193 | As 1.3a, from hvcC. |
| 1.6b | HEVC <= Main10@5.1 High tier | GAP-OPT | EN:1193 | As 1.3b. |
| 1.7 | HDR HEVC must be HDR10/HLG/DV | PARTIAL | EN:1198-1205, EN:1273-1277 | `VIDEO-RANGE` emitted for HLG/PQ from the colr transfer code; no verification that HDR content is one of the allowed set; SDR/unknown omits the attribute. |
| 1.8 | HDR10 static SEI in hvcC, not in samples | GAP | pipeline.rs:231-285 (init builds only `pasp`+`colr`; no `mdcv`/`clli`); `git grep mdcv/clli` in pipeline.rs/init_segment.rs: none | Init segment drops mastering/CLL boxes. For TS sources the SEI stays in sample data and is not hoisted. A source `hvcC` SEI array is preserved only if the config record round-trips. |
| 1.9 | Dolby Vision must be Profile 5, level <= 7 | GAP | `git grep dvh1/dvhe` (non-sample_entries): zero; pipeline.rs:231-285 emits no `dvcC`/`dvvC` | DV P5 (`dvh1`) input is not parsed as DV; `dvcC`/`dvvC` boxes are preserved only on the sample_entries passthrough path (sample_entries.rs:514-532), not on the pipeline-built init used by LlHlsSegmenter (ll_hls.rs:46, pipeline.rs:70). No `SUPPLEMENTAL-CODECS`. |
| 1.10 | Parameter sets in sample entry (avc1/hvc1, not avc3/hev1) | MET-V | pipeline.rs:610 (`avc1`), pipeline.rs:641-644 (`hvc1`, comment "Always emit") ; test transmux/tests/it/codec_tail.rs:308-332 (hev1 input -> hvc1 output with source hvcC) | HEVC verified by test; AVC `avc1` by code only (no equivalent avc3->avc1 test found). AV1: `av01`, N/A. |
| 1.11 | Don't use higher level than needed | N/A ENCODER | | |
| 1.12 | Provide H.264 for back-compat | N/A ENCODER | | Single-variant origin cannot add a codec ladder (F1). |
| 1.13 | IDR every 2 s (SHOULD) | GAP-OPT | transmux/src/segmenter.rs:212-217 (`MAX_UNCUT_SECS`=60 only guards memory) | Origin sees every keyframe position and segment length; it does not warn when GOP > 2 s. |
| 1.14 | All interlaced content deinterlaced | GAP-OPT | media-doctor/src/diagnostics/interlace.rs (offline diagnostic); transmux/src/interlace.rs | Detection exists as a standalone tool; not wired into the origin ingest. |
| 1.15 | Deinterlace 30i to 60p | N/A ENCODER | | |
| 1.16 | Live NTSC/ATSC 60/59.94 fps | N/A ENCODER | | |
| 1.17 | Live PAL 50 fps | N/A ENCODER | | |
| 1.18 | VOD natural frame rate | N/A CONTENT | | |
| 1.19 | No frame rate above 60 fps | GAP-OPT | EN:1219-1225 (FRAME-RATE from H.264 VUI only); media-doctor/src/diagnostics/fps_cadence.rs | Rate is known for H.264 VUI and HEVC VPS timing (commit e93c28b1e) but never compared to 60. |
| 1.20 | HDR also <= 30 fps (SHOULD) | N/A ENCODER | | |
| 1.21 | Single color space | GAP-OPT | EN:1198-1205 (colr read from first video track) | Origin reads colour info; no cross-variant/within-stream consistency check. |
| 1.22 | VOD keeps original color space | N/A CONTENT | | |
| 1.23 | Multi-codec streams each provide all bandwidths | N/A ENCODER | | Single variant (F1). |
| 1.24 | SDR streams MUST be provided | N/A ENCODER | | Origin cannot synthesise an SDR variant; GAP-OPT could warn if the only variant is PQ/HLG (EN:1273-1277 already knows). |
| 1.25 | Example bitrate ladder | N/A ENCODER | | Origin serves one variant (F1). |
| 1.26 | VOD avg segment bitrate within 10% of AVERAGE-BANDWIDTH | N/A | multimux/src/catchup.rs:604-612 | VOD/catch-up media playlists are emitted with no master and no BANDWIDTH at all; item concerns authored VOD ladders. |
| 1.27 | VOD peak within 10% of BANDWIDTH | N/A | same | As 1.26. |
| 1.28 | Live avg over ~1 h < 110% of AVERAGE-BANDWIDTH | PARTIAL | EN:742-753, EN:1265-1271; tests hls-runtime/tests/avg_bandwidth.rs:87-210 | AVERAGE-BANDWIDTH is the cumulative mean since the origin started (survives eviction, test :111), clamped to BANDWIDTH. It is a moving value, so a client that cached the master earlier can see > 110%; omitted until the first segment closes. |
| 1.29 | Live peak < 125% of BANDWIDTH | PARTIAL | EN:488-492 (peak = max segment bps ever, rounded up), EN:1258-1262 (`PLACEHOLDER_BANDWIDTH_BPS` 5 000 000 until first segment) | Peak is measured, rounded up, never decreasing (but can rise after the master was fetched). Before any segment closes the master advertises a fixed 5 Mb/s guess (EN:203, 1258), which is unrelated to the content. |
| 1.30 | VOD peak <= 200% of average (SHOULD) | GAP-OPT | EN:488-492 | Both peak and average are measured for live; no ratio check or log. |
| 1.31 | Variants may differ in frame rate | N/A | | Permission. |
| 1.32 | Default variant is the 2000 kb/s one | N/A ENCODER | | Single variant. |
| 1.33 | Identical aspect ratio across variants | N/A ENCODER | | Single variant. |
| 1.34 | UHD variants <= 15 Mb/s (SHOULD) | GAP-OPT | EN:488-492 | Peak known; not checked against 15 Mb/s. |
| 1.35 | HDR10 MDCV + CLL SEI present (SHOULD) | MET-U | ts_demux.rs:2279-2296 + annexb.rs:136 (NALs copied through; bijection test annexb.rs:354) | SEI NALs in samples pass through if the source has them; origin neither adds nor checks them (and drops the init boxes, see 1.8). |
| 1.36 | MV-HEVC only for stereo | N/A CONTENT | | MV-HEVC is not modelled in transmux. |
| 1.37 | AV1 level <= 6.2 | GAP-OPT | transmux/src/dash.rs:551 (`config.rfc6381()`) | Level available in `av1C`; not checked. |
| 1.38 | APMP mono must be HEVC | N/A CONTENT | | |
| 1.39 | AV1 container MUST be fMP4 | MET-U | ts_mux.rs:420-446 (`from_config` returns None for AV1, track silently dropped from TS) | Holds only because the TS muxer silently drops AV1 (no error, no log); CMAF path accepts `av01` (pipeline.rs:816). A TS-HLS route given AV1 yields a stream with no video and no diagnostic. |

## Section 2: Audio

| Item | Requirement | Class | Evidence (file:line) | Note |
|---|---|---|---|---|
| 2.1 | Audio as elementary stream or fMP4 | PARTIAL | F2; EN:163-196 | fMP4 output is fine. TS output multiplexes audio with video in the same `.ts`; no audio-only/elementary rendition is ever produced. |
| 2.2 | Stereo codecs APAC/AAC/HE-AAC/xHE-AAC/ALAC/FLAC | GAP-OPT | `git grep -w usac/xhe-aac/apac/alac` in transmux: zero; transmux/src/flac.rs exists | xHE-AAC, APAC, Apple Lossless have no codec identity in transmux; FLAC is carried in CMAF only. Origin does not restrict to the list. |
| 2.3 | Stereo AAC/HE-AAC MUST be provided | N/A ENCODER | | Origin cannot transcode; a broadcast AC-3-only source yields an AC-3-only stream with no check (GAP-OPT: could flag from CODECS). |
| 2.4 | No HE-AAC above 64 kb/s | N/A ENCODER | | |
| 2.5 | Multichannel codec list | GAP-OPT | ts_mux.rs:420-446 (accepts DTS, MPEG audio, MPEG-H) | Non-Apple codecs pass through. |
| 2.6 | Provide DD if DD+ delivered to non-DD+ devices | N/A ENCODER | | One audio set only (F2); no rendition groups. |
| 2.7 | Stereo at multiple bit rates (MAY) | N/A | | Permission. |
| 2.8 | Multichannel at multiple bit rates (MAY) | N/A | | Permission. |
| 2.9 | Bit-rate recommendations | N/A ENCODER | | |
| 2.10 | May change channel layout in stream | N/A | | Permission; see 2.11. (AirPlay amendment 2.10 says SHOULD NOT; not enforced.) |
| 2.11 | Layout change must come with EXT-X-DISCONTINUITY | PARTIAL | EN:515-523 + EN:575-582 (`InitStore::set` new generation on changed init; `discontinuous = true`); test hls-runtime/tests/origin_hardening.rs:914 | fMP4: verified (a changed init yields new generation and `EXT-X-DISCONTINUITY`). TS-HLS: no init, so a mid-stream ASC/channel change in ADTS gets no discontinuity unless the segmenter caller sets `mark_discontinuity` (ts_hls.rs:1141-1142), and nothing detects it. |
| 2.12 | DVS must carry CHARACTERISTICS describes-video | GAP-OPT | F1 | Origin cannot publish any alternate-audio rendition. If a source carries a DVS/audio-description PID (ISO 639 audio_type), it is merged or ignored; nothing maps to `EXT-X-MEDIA`. |
| 2.13 | DVS AUTOSELECT=YES | GAP-OPT | F1 | As 2.12. |
| 2.14 | DVS name indicates DVS (SHOULD) | GAP-OPT | F1 | As 2.12. |
| 2.15 | Alt audio/DVS spans entire duration | N/A CONTENT | | No alternates exist. |
| 2.16 | Alt audio may reuse regular segments | N/A | | Permission. |
| 2.17 | One Media Segment must not contain multiple audio streams | GAP | F2; transmux/src/ll_hls.rs:371-410 and ts_hls.rs (all tracks pushed to one segment); EN:1227 | A source with two audio tracks (e.g. an MPTS programme with two language PIDs, or RTSP with two audio) yields segments with two audio streams, and the master `CODECS` lists both. No selector, no check, no warning. |
| 2.18 | Loudness/DRC present unless levels equal (SHOULD) | N/A ENCODER | | |
| 2.19 | fMP4 loudness box `ludt` (SHOULD) | GAP-OPT | `git grep ludt/udta` in transmux/src: zero | `ludt` is never read or written; an fMP4 source's `ludt` is dropped on repackage. Measuring loudness would need decoding (out of scope). Pass-through is possible. |
| 2.20 | dialnorm for DD/DD+ without ludt | N/A ENCODER | | Bitstream field carried untouched. |
| 2.21 | AAC prog_ref_level/loudnessInfo | N/A ENCODER | | |
| 2.22 | xHE-AAC loudness metadata MUST be present | N/A ENCODER | | xHE-AAC unsupported anyway (2.2). |
| 2.23 | Loudness value must be actual | N/A ENCODER | | |
| 2.24 | HE-AAC in fMP4 MUST use explicit SBR signalling | PARTIAL | transmux/src/aac_asc.rs:371-400 (explicit signalling is parsed/preserved from a source ASC); ts_demux.rs:240 (ADTS to ASC via `parse_adts_header`, no SBR sync-extension detect: `git grep -i sbr transmux/src/ts_demux.rs` empty) | fMP4->fMP4 keeps an explicit ASC. TS(ADTS) HE-AAC -> CMAF produces an ASC from the ADTS header alone, so SBR is implicit and `CODECS` says `mp4a.40.2`. Moderate confidence (I did not read the ASC builder line by line). |
| 2.25 | xHE-AAC/ALAC/FLAC/APAC must be in fMP4 | MET-U | ts_mux.rs:420-446 (`Flac` and others return None, dropped from TS) | Holds by silent drop on TS (no error); xHE-AAC/ALAC/APAC unsupported. |
| 2.26 | Non-Dolby multichannel: also provide AAC (SHOULD) | N/A ENCODER | | |
| 2.27 | Audio description needs LANGUAGE | GAP-OPT | F1 | As 2.12. |
| 2.28 | Speech-enhanced audio MAY be marked | N/A | | Permission. |
| 2.29 | Multi-format codecs APAC/DD+ JOC | N/A | | Informational list. |
| 2.30 | APAC needs in-stream loudness/DRC | N/A ENCODER | | APAC unsupported. |
| 2.31 | APAC also needs stereo AAC (SHOULD) | N/A ENCODER | | |

## Section 3: Ads

| Item | Requirement | Class | Evidence (file:line) | Note |
|---|---|---|---|---|
| 3.1 | Ads inserted in playlists or as Interstitials (MAY) | PARTIAL | ssai-runtime/src/playlist.rs:29-30, 96-155 (interstitial DATERANGE renderer); multimux has no `ssai-runtime` dependency (`git grep ssai_runtime multimux`: CHANGELOG only); EN:1730-1747 (SCTE-35 to plain DATERANGE) | Library capability exists; the HLS origin does neither stitched insertion nor interstitial tags (only SCTE-35-derived `EXT-X-DATERANGE`, gated on a time anchor). |
| 3.2 | Inserted media same codecs (SHOULD) | GAP-OPT | transmux/src/splice.rs; transmux/examples/ssai_ad_stitch.rs | Splice/stitch exists in transmux (offline example); no codec-equality check at the join. Not wired to the origin. |
| 3.3 | Same aspect ratio (SHOULD) | GAP-OPT | same | No aspect check. |
| 3.4 | Inserted media <= variant bandwidths (SHOULD) | GAP-OPT | same | No check. |

## Section 4: Accessibility

| Item | Requirement | Class | Evidence (file:line) | Note |
|---|---|---|---|---|
| 4.1 | Provide captions (SHOULD) | PARTIAL | ts_demux.rs:2279-2296; annexb.rs:136 | In-band caption SEI/user_data passes through the segmenters; nothing ensures or checks presence. |
| 4.2 | Supported caption formats | N/A | | Informational list. |
| 4.3 | Closed captions must be in the video segments | MET-U | ts_demux.rs:2279-2296 + annexb.rs:136 (all NALs copied; bijection test annexb.rs:354) ; ts_mux.rs:1434 | A/53 SEI and MPEG-2 user_data ride in the video samples. No end-to-end test that an A/53 SEI survives demux, segmenter and mux. |
| 4.4 | CC presence declared via EXT-X-MEDIA with LANGUAGE | GAP | F1; `transmux::caption_cc_data` exists (transmux/src/lib.rs:359) but `git grep caption_cc_data multimux hls-runtime`: zero | Segments can carry CEA-608/708 while the master has no `CLOSED-CAPTIONS` group and no `EXT-X-MEDIA TYPE=CLOSED-CAPTIONS`. Players then cannot surface the captions. The origin owns the master, so it can detect cc_data and declare (LANGUAGE needs operator config). |
| 4.5 | HoH subtitle track CHARACTERISTICS | GAP-OPT | F1 | No subtitle renditions (see 5.x). |
| 4.6 | HoH subtitle AUTOSELECT=YES | GAP-OPT | F1 | As 4.5. |
| 4.7 | Subtitle EXT-X-MEDIA has LANGUAGE | GAP-OPT | F1 | As 4.5. |
| 4.8 | See 2.12-2.16 | n/a | | Cross-reference; same classes as 2.12-2.16. |

## Section 5: Subtitles

| Item | Requirement | Class | Evidence (file:line) | Note |
|---|---|---|---|---|
| 5.1 | Subtitles MAY be provided | N/A | | Permission. Origin currently cannot provide them: no WebVTT/IMSC segmenter, no subtitle playlist route (`git grep -i webvtt multimux/src hls-runtime/src`: only a smooth.rs test string). |
| 5.2 | Subtitles must be WebVTT or IMSC1-in-fMP4 | GAP-OPT | transmux/src/ts_mux.rs:420-436 (Subtitle tracks not TS-carriable); ir/codec.rs:514 (`is_muxable_in_bmff` excludes subtitle w/o entry) | A `wvtt`/`stpp` track from fMP4/DASH input is dropped from HLS output; DVB subtitle/teletext PES is carried verbatim as private data in TS-HLS (undeclared, not WebVTT/IMSC). `caption-convert` (608/Teletext to WebVTT) is not wired in. |
| 5.3 | WebVTT as text files with X-TIMESTAMP-MAP | GAP-OPT | `git grep -i x-timestamp-map` in origin/segmenter crates: none | No WebVTT segment is ever produced or passed through (HLS pull ingest handles media playlists only, source/hls_pull.rs). |
| 5.4 | IMSC1 in fMP4 | GAP-OPT | same | No `stpp` track passthrough to HLS output. |
| 5.5 | Subtitle playlist covers whole content | N/A | | No subtitle playlist exists. |
| 5.6 | Live subtitle target durations equal other media | N/A | | No subtitle playlist exists. |
| 5.7 | VOD subtitle target may be longer | N/A | | Permission. |
| 5.8 | Regular track includes forced cues | N/A CONTENT | | |
| 5.9 | Prefer forced subtitles over burn-in | N/A CONTENT | | |
| 5.10 | CODECS names subtitle format (stpp.ttml.im1t / wvtt) | N/A | EN:1193-1196 (`UnsupportedCodec` tracks are skipped from CODECS) | Moot, no subtitle renditions. |
| 5.11 | Forced subtitles AUTOSELECT=YES | N/A | F1 | No subtitle renditions. |

## Section 7: Media segmentation

| Item | Requirement | Class | Evidence (file:line) | Note |
|---|---|---|---|---|
| 7.1 | Media continuous across segments (except ads) | MET-V | EN:482-535 (`Window::push`: skipped/repeated numbers, lag gaps, or init change mark `EXT-X-DISCONTINUITY`); tests hls-runtime/tests/origin_hardening.rs:359, 424, 914; transmux/tests/it/hls_discontinuity.rs:102, 277 | Discontinuities are signalled, not hidden. Timeline continuity inside a healthy stream rests on 7.3. |
| 7.2 | TS: continuity counters and timestamps sequential | MET-U | transmux/src/ts_hls.rs:210-214, 689-692 and 1134 (one `TsContinuity` across all segments); ts_mux.rs:603-609 | Code carries per-PID CC across cuts (audit r05-W21 comment). I found no test asserting CC continuity across a segment boundary (transmux/tests grep "continuity_counter" only finds fixture builders). Timestamps: sequential by construction from the anchor clock. |
| 7.3 | fMP4: tfdt consistent with prev decode time + duration | MET-V | transmux/src/segmenter.rs:578-588 (`base_decode += tick sum`); test transmux/tests/it/segmenter.rs:186-206 (asserts `tfdt == total duration so far`, video and audio); transmux/tests/it/ll_hls.rs:771-783 (audio tfdt advances across parts) | Verified on `Segmenter` and `LlHlsSegmenter` (the one multimux uses). |
| 7.4 | Video segments MUST start with an IDR | GAP | transmux/src/nal.rs:351-381 (RAP = IDR OR recovery_point SEI OR SPS+I-slice; HEVC any IRAP incl. CRA/BLA); ts_demux.rs:2279-2296; segmenter.rs:488-492; ll_hls.rs:392-397; `transmux::LiveJoin` (live_join.rs) has no consumer (`git grep LiveJoin` outside live_join.rs: only fuzz/tests) | Three defects. (a) Open-GOP: segments can start on a non-IDR RAP (the doc comment at nal.rs:341-343 admits this). (b) HEVC CRA/BLA start segments. (c) The FIRST segment after connect, and after a source gap, starts with whatever sample arrives first: `push` buffers non-sync samples into an empty pending buffer (segmenter.rs:489-509, ll_hls.rs:383-410), so a mid-GOP join yields a segment that begins on a P-frame. I expect Apple's validator to flag this, but I did not test it; the oracle tests do not run this path (F5). Closed-GOP IDR-cadence sources otherwise comply; cut-on-keyframe is tested (transmux/tests/it/segmenter.rs header "keyframe-aligned segment starts"; tests/opengop_anchor.rs). |
| 7.5 | Target duration SHOULD be 6 s | PARTIAL | multimux/src/config.rs:1830 (`target_duration_secs: 4.0` default); hls-runtime builder requires explicit value (EN:845-853) | Configurable, default 4 s (LL-HLS-oriented), not 6 s. |
| 7.6 | Segment durations nominally 6 s | PARTIAL | transmux/src/segmenter.rs:488-492 (cut at first keyframe at or past target) | Actual duration = target + remainder to next keyframe; follows source GOP. |
| 7.7 | Segment must not exceed target by more than 0.5 s | PARTIAL | EN:1617-1629 (`TARGETDURATION = round(max(configured, max segment ever))`); test EN:2724 | The origin never exceeds the PUBLISHED target (it raises the target to the largest segment seen), which satisfies RFC 8216 4.3.3.1 but sidesteps this item: segments can exceed the configured target by any GOP length and nothing flags it. Side effect: `EXT-X-TARGETDURATION` can increase over the life of a live playlist, which RFC 8216 says must not change (outside this group; noted for the playlist-section auditor). |
| 7.8 | xHE-AAC segments start with IPF | N/A | | xHE-AAC not supported (2.2). |
| 7.9 | APAC segments start with ASP | N/A | | APAC not supported (2.2). |

## Counts per class

Computed by script from the Class column (one primary class per row; the 4.8 cross-reference row and permission-only rows are N/A).

| Class | S1 | S2 | S3 | S4 | S5 | S7 | Total |
|---|---|---|---|---|---|---|---|
| MET-VERIFIED | 1 | 0 | 0 | 0 | 0 | 2 | 3 |
| MET-UNVERIFIED | 3 | 1 | 0 | 1 | 0 | 1 | 6 |
| GAP | 3 | 1 | 0 | 1 | 0 | 1 | 6 |
| PARTIAL | 3 | 3 | 1 | 1 | 0 | 3 | 11 |
| GAP-OPTIONAL | 12 | 7 | 3 | 3 | 3 | 0 | 28 |
| NOT-APPLICABLE | 19 | 19 | 0 | 2 | 8 | 2 | 50 |
| Rows | 41 | 31 | 4 | 8 | 11 | 9 | 104 |

(Section 1 has 41 rows because 1.3a/b and 1.6a/b are split rows. N/A vs GAP-OPT is a judgement call on borderline encoder rules.)

## Top 10 gaps by severity (MUST-level first)

1. 7.4 (MUST) Segments not guaranteed to start on an IDR: first segment after a mid-GOP join, after a source gap, open-GOP RAP, HEVC CRA/BLA. Fix: feed samples through `transmux::LiveJoin` with `JoinPolicy::SelfContainedOnly` (drop until the first IDR) in `ProgramSegmenter`, and make the cut predicate IDR-only (HEVC: IDR_W_RADL/IDR_N_LP only, excluding CRA and BLA) when the route is configured Apple-strict.
2. 1.5 (MUST) HEVC carried in MPEG-TS on `ts_hls` routes. Fix: in `Config::validate`/`ProgramSegmenter` reject (or drop with a logged error) HEVC (and AV1) tracks on a TS-HLS route, mirroring the existing mutual-exclusion check at config.rs:1476.
3. 2.17 (MUST) Multiple audio streams muxed into one segment. Fix: add a per-route audio-track selector (default first audio, or by language) in the segmenter's track set, or split extra audio into `EXT-X-MEDIA` renditions.
4. 4.4 (MUST) Embedded CEA-608/708 never declared. Fix: use `transmux::caption_cc_data` during ingest to detect cc_data and emit `#EXT-X-MEDIA:TYPE=CLOSED-CAPTIONS,GROUP-ID,INSTREAM-ID,LANGUAGE` plus `CLOSED-CAPTIONS=` on the variant (language from config, since the bitstream has none).
5. 2.24 (MUST) HE-AAC from ADTS gets an implicit-SBR ASC in fMP4. Fix: in `ts_demux` ADTS to ASC, scan the first raw_data_block for the SBR/PS extension (or accept a config override) and emit AOT 5/29 explicit signalling; add a fixture test from `fixtures/ts/heaac`.
6. 2.11 for TS-HLS (MUST) Channel-layout/ASC change inside a TS stream gets no `EXT-X-DISCONTINUITY`. Fix: have `StreamingTsHlsSegmenter` compare the audio ASC/PMT per cut and call `mark_discontinuity` on change (the demux already re-probes on header change, ts_demux.rs:36).
7. 1.9 / 1.1 Dolby Vision unsupported (`dvh1`/`dvhe` not parsed; `dvcC`/`dvvC` not written by the pipeline init) and no allow-list of codecs. Fix: parse `dvh1`/`dvhe` entries, keep `dvcC`/`dvvC` in `build_trak`, emit `SUPPLEMENTAL-CODECS`; add a codec allow-list that rejects MPEG-2/VP8/VP9/VVC on HLS outputs.
8. 1.29/1.28 Master `BANDWIDTH` is a fixed 5 Mb/s placeholder until the first segment closes (EN:203, 1258) and both bandwidth attributes drift. Fix: return 503 or `Retry-After` on `master.m3u8` until a segment has been measured, or take an operator-supplied declared bitrate.
9. F3 (supports 7.x, RFC 4.4.2.1) `EXT-X-INDEPENDENT-SEGMENTS` never emitted although every cut is on a sync sample. Fix: set `independent_segments: true` on both playlists in EN (render_playlist and the master) once 7.4 is IDR-strict.
10. 1.8 / 1.35 HDR10 static metadata (`mdcv`/`clli`) is not written to the fMP4 init (pipeline.rs:231-285 only emits `pasp`/`colr`). Fix: carry `mdcv`/`clli` from the source sample entry or SEI into `avc_hevc_display_extra_boxes`.

Honourable mentions: 5.2-5.4 (no WebVTT/IMSC rendition path; wire `caption-convert` and an `EXT-X-MEDIA TYPE=SUBTITLES` group); 2.12-2.14/2.27 (no alternate-audio/DVS renditions, so `CHARACTERISTICS`/`AUTOSELECT` can never be emitted or passed through); 2.19 (`ludt` dropped); 7.2 CC-continuity has no boundary test; oracle coverage (F5) does not exercise `HlsOrigin`.

## Could not assess, and why

- Everything marked N/A ENCODER/CONTENT is by nature outside a repackager; I did not look for stream-level proof beyond what is listed.
- Actual segment bytes produced by `multimux` end to end (IDR start, CC continuity, ASC content) were not executed: the task excluded running cargo. 7.4(c), 2.24 and 1.8 findings are from code reading; each names the lines to re-check.
- Whether `mediastreamvalidator` (macOS-only) passes against `HlsOrigin` output: no test wires it to the origin (F5).
- Platform-amendment sections were scanned only for items that change an origin obligation in my group: 1.41 (AirPlay encrypted fMP4 needs `senc` or `saiz`+`saio`; transmux/src/cenc_encrypt.rs emits `senc`, but `HlsOrigin` itself never encrypts, so not rated), 2.10 (AirPlay: SHOULD NOT change layout; unenforced), 5.2 (AirPlay: WebVTT only; no subtitle path at all), 3.5 (frame-rate at ad joins; unchecked).
- Dolby Vision `dvcC` retention through the sample_entries passthrough path vs the pipeline init was judged from `pipeline.rs:231-285`; I did not trace whether any caller injects source extra boxes into the pipeline init.
