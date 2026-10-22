# Audit report, group C (Apple HLS Authoring Spec sections 11-16 + Appendix), origin = hls-runtime + multimux

Checkout: /Volumes/External/Projects/rust-broadcast/.worktree/audit @ 90a30b58e. Read-only, no cargo run. Paths are relative to that root.

Cross-cutting facts that drive many rows (all read in code):
- multimux serves **HTTP/1 only, plaintext**: `hyper` built with only `http1` (multimux/Cargo.toml:82-87), connections served by `http1::Builder::new()` (multimux/src/origin/mod.rs:629, :726). There is no TLS acceptor (`tokio-rustls` in Cargo.toml:140 is only for `rtsps://` ingest) and no HTTP/2, so no RFC 9218 priorities.
- **No response compression anywhere**: `tower-http` features are `timeout, limit, cors` only (Cargo.toml:92); no `Content-Encoding`/gzip in multimux/src or hls-runtime/src (grep clean).
- The origin renders **one variant, muxed A/V, one media playlist** (hls-runtime/src/server/engine.rs:1278-1289: `variants: vec![Variant{..}]`; the only extra attr is VIDEO-RANGE). No EXT-X-KEY, SESSION-KEY, HDCP-LEVEL, ALLOWED-CPC, REQ-VIDEO-LAYOUT, STABLE-*-ID, PROGRAM-DATE-TIME is emitted by the served path (`MediaSegment` has no PDT field, broadcast-hls/src/lib.rs:535-584; engine media playlist at engine.rs:1722-1790 uses `extra_tags` for MAP/DATERANGE only).
- Header middleware: multimux/src/origin/mod.rs:865-914 (`add_response_headers`), cache policy :932-1000.

## Section 11 Privacy

| Item | Requirement | Class | Evidence | Note |
|---|---|---|---|---|
| 11.1 | Multivariant playlist SHOULD use TLS | GAP-OPTIONAL (PLATFORM/deployer) | origin/mod.rs:629,726 (http1 only, no acceptor) | The origin cannot terminate TLS itself; needs a reverse proxy. Cannot be enforced or even detected by the origin. |
| 11.2 | Media playlists SHOULD use TLS | GAP-OPTIONAL (PLATFORM) | same | same |
| 11.3 | Media segments SHOULD use TLS | GAP-OPTIONAL (PLATFORM) | same | same |
| 11.4 | Plaintext segment URLs SHOULD NOT reveal titles | PARTIAL | engine.rs:1320-1328 (`init-{t}-{inst}-{gen}.mp4`, `seg-{t}-{inst}-{msn}.ext`, `part-...`) | Generated file names are neutral. But the stream/route name is operator-chosen and is the first URL path segment (config.rs:1329, :1394 validates only path-safety); nothing warns or checks for revealing strings. |

## Section 12 Security

| Item | Requirement | Class | Evidence | Note |
|---|---|---|---|---|
| 12.1 | TLS >= 1.2 with forward secrecy | GAP-OPTIONAL (PLATFORM) | origin/mod.rs:726 | No TLS in the origin; deployer's proxy decides. |
| 12.2 | No insecure TLS primitives | GAP-OPTIONAL (PLATFORM) | same | same |
| 12.3 | RSA 2048 / EC 256 key sizes | GAP-OPTIONAL (PLATFORM) | same | same |
| 12.4 | Segment URLs SHOULD NOT be completely static (per device/over time) | PARTIAL | engine.rs:1320-1328 (msn + per-build random `instance` token, engine.rs:1309); config.rs:591-605 + origin/mod.rs:795-880 (`SignedUrl` output auth via `broadcast_auth`) | URLs change over time (msn) and per origin instance, but not per device. A signed-URL verifier exists as an auth gate, but the playlist emits bare relative URIs (no signature/token appended), so HLS players that resolve relative URIs would not carry a per-device signature. No test shows a signed-URL HLS session end to end. |

## Section 13 Content protection

The served path (hls-runtime `HlsOrigin` + multimux outputs) has **no encryption path**: no EXT-X-KEY is ever rendered by the origin (git grep of hls-runtime/src/server, multimux/src/output, multimux/src/route.rs for KEY/SAMPLE-AES/HDCP/CPC returns nothing). The building blocks live in libraries only (`broadcast_hls::cenc_ext_x_key`, `transmux::sample_aes::ExtXKey`, `transmux::cenc_encrypt`).

| Item | Requirement | Class | Evidence | Note |
|---|---|---|---|---|
| 13.1 | Protection SHOULD follow FairPlay Streaming | GAP-OPTIONAL | transmux/src/sample_aes.rs:609-619 | Library can build FPS key tags; origin never serves protected content. |
| 13.2 | FPS content MUST use METHOD=SAMPLE-AES | PARTIAL | sample_aes.rs:609-612 (`fairplay_sample_aes` -> SAMPLE-AES); test sample_aes.rs:1008-1012 | Library-level only (MET-VERIFIED there); not reachable from multimux. |
| 13.3 | FPS KEYFORMAT MUST be com.apple.streamingkeydelivery | PARTIAL | sample_aes.rs:617; but broadcast-hls/src/lib.rs:41,100 `cenc_ext_x_key` emits KEYFORMAT `urn:mpeg:dash:mp4protection:2011` | Two key-tag builders disagree; the CENC one (the one the fMP4 pipeline would use) is not an FPS tag. |
| 13.4 | IV SHOULD NOT be used with FPS | MET-UNVERIFIED | sample_aes.rs:613 (`iv: None`); broadcast-hls lib.rs:56 (never emits IV) | Library only. |
| 13.5 | HD content SHOULD use HDCP-LEVEL TYPE-0 | GAP-OPTIONAL | engine.rs:1278-1289; `Variant` has no HDCP field (broadcast-hls lib.rs:3163; grep for hdcp = none) | Master never carries HDCP-LEVEL; no operator knob. Could be a config pass-through. |
| 13.6 | >HD SHOULD use HDCP-LEVEL TYPE-1 | GAP-OPTIONAL | same | same |
| 13.7 | CE video MUST use 1:9 encrypt:skip | PARTIAL | transmux/src/cenc_encrypt.rs:421-422 (pattern is a caller parameter); test cenc_encrypt.rs:1179-1180 asserts (1,9) for that test's call | Not enforced as a default or validated against an Apple profile; not on served path. |
| 13.8 | FPS MAY use ALLOWED-CPC | NOT-APPLICABLE | broadcast-hls lib.rs:2309 (parse only) | Optional (MAY). Attribute is parsed/round-tripped, never emitted by the origin. |
| 13.9 | CE MUST NOT use content-sensitive encryption | NOT-APPLICABLE (ENCODER/packager) | transmux/src/cenc_encrypt.rs | Encryptor applies a fixed pattern; origin does not decide. Not checked line by line. |
| 13.10 | MAY mix encrypted/unencrypted samples | NOT-APPLICABLE | n/a | Optional (MAY). |
| 13.11 | SAMPLE-AES-CTR SHALL NOT be used on Apple | GAP-OPTIONAL | broadcast-hls/src/lib.rs:76 (`CencScheme::Cenc => "SAMPLE-AES-CTR"`) | The library emits the prohibited method for `cenc` with no Apple-profile guard; harmless today only because the origin never calls it. |

## Section 14 Low-Latency HLS

14.1 requires all of RFC 8216bis App. B.1 (draft text checked: specs/ietf_draft_pantos_hls_rfc8216bis.txt:5574-5760). Broken down per B.1 clause:

| Item | Requirement | Class | Evidence | Note |
|---|---|---|---|---|
| 14.1 (overall) | Meet every B.1 requirement | PARTIAL | see rows below | Fails PDT, HTTP/2-3, priorities, gzip; rest met or vacuous. |
| 14.1-B1a | All Media Playlists have EXT-X-PROGRAM-DATE-TIME | GAP (MUST-level via 14.1) | engine.rs:1722-1790 (no PDT anywhere; `MediaSegment` lacks the field, broadcast-hls lib.rs:535-584) | Origin never emits PDT even when `trunk.time_anchor()` is known (anchor only drives DATERANGE, engine.rs:1730). |
| 14.1-B1b | Each non-partial segment has an independent frame | MET-UNVERIFIED | engine.rs:1565,1690 (`independent` from part meta); segmenter cuts on sync samples | Test origin_llhls.rs:164 feeds sync every 30 frames but asserts only structure, not the property. |
| 14.1-B1c | EXT-X-PART without ENDLIST => EXT-X-PRELOAD-HINT | MET-VERIFIED | engine.rs:1700-1710 (next_part_hint), :1762 (`preload_hint_part`); test engine.rs:2429 ("preload hint for the next part") | Only when the open segment has >=1 part; at a segment boundary with no open parts no hint is rendered (`has_open_parts`, engine.rs:1608). Minor gap window. |
| 14.1-B1d | Different Init in hinted part => PRELOAD-HINT TYPE=MAP | GAP | engine.rs:1762 (`preload_hint_type` left default Part); init changes only announced via DISCONTINUITY+MAP on segments | Origin never emits TYPE=MAP hint; on an init-generation change mid-segment this MUST is not met. |
| 14.1-B1e | RENDITION-REPORT per other rendition | MET-UNVERIFIED (vacuous) | engine.rs:1278 single variant; no `rendition_reports` set | With one variant there is nothing to report. mediastreamvalidator flags "No rendition reports" and the oracle test explicitly ignores it (media-doctor/tests/mediastreamvalidator_oracle.rs:934-947). Becomes a GAP the moment a second variant/rendition or the I-frame variant (#1413) is advertised. |
| 14.1-B1f | HTTP/2 or HTTP/3 delivery | GAP | Cargo.toml:82-87; origin/mod.rs:629,726 | HTTP/1.1 only. |
| 14.1-B1g | RFC 9218 priority urgency in responses (playlists 1, segments 2-6, bitrate ordering) | GAP | grep for priority/urgency in multimux/src = none | No `Priority` header; moot on HTTP/1 but required by the profile when using H3, recommended on H2. |
| 14.1-B1h | All variants offered by each server | MET-UNVERIFIED | engine.rs:1278 | Single variant, trivially. |
| 14.1-B1i | HTTP Range support if BYTERANGE used | MET-VERIFIED | multimux/src/http.rs:196-300 (`range_response`); tests origin/resource.rs:1156-1546 (200/206/416/suffix/open-ended/multi/If-Range) | Applies to init/seg/part (resource.rs:208). |
| 14.1-B1j | Playlist requests idempotent | MET-UNVERIFIED | engine.rs:2005-2030 (resolve_playlist -> render, `drain()` only advances caches) | No test of idempotence. |
| 14.1-B1k | Playlists delivered gzip | GAP | no Content-Encoding in multimux/src; tower-http without compression (Cargo.toml:92) | See also 10.1 (other group). |
| 14.1-B1l | All renditions in sync within a Part Target | MET-UNVERIFIED (vacuous) | single variant, one trunk | n/a until multi-rendition. |
| 14.1-B1m | Blocking requests held at CDN; Age header | NOT-APPLICABLE (PLATFORM/CDN) | n/a | Origin cannot influence; origin sets `Cache-Control: no-cache` on playlists (origin/mod.rs:1003). |
| 14.1-B1n | Recommended caching (blocking playlists 6x TD, non-blocking TD/2, errors 1-4x TD, blocking segments 6x TD) | PARTIAL | origin/mod.rs:932-1000 | Playlists always `no-cache` (no distinction for blocking requests); segments immutable 1y or `max-age=10` (token-less); errors `no-cache`. Safe but forgoes the profile's recommended CDN lifetimes. SHOULD-level. |
| 14.1-B1o | Blocking reload semantics (_HLS_msn/_HLS_part, bad-request bounds) | MET-VERIFIED | engine.rs:1996-2030, :245; llhls.rs:133-200; tests engine.rs:2446 (preload part blocks), origin_llhls.rs:274 | Not strictly in B.1 text but required by CAN-BLOCK-RELOAD=YES which is advertised (engine.rs:1974 / :1760). Hold time cap is BLOCKING_RELOAD_TIMEOUT=5s (http.rs:36). |
| 14.2a | Part Target >= P95 RTT | NOT-APPLICABLE (PLATFORM/deployer) | config.rs:1971-1980 validates only part <= target duration | Origin has no RTT knowledge. Could document. |
| 14.2b | Part Target SHOULD be >= 3x P95 RTT | NOT-APPLICABLE (PLATFORM) | same | same |
| 14.2c | Recommended Part Target 1 s | PARTIAL | config.rs:1831 default `part_target_ms: 500` | Default is 0.5 s, not the 1 s recommendation; operator-overridable. Origin test uses 0.5 s (engine.rs:2410 "PART-HOLD-BACK=1.5"). |
| 14.3 | PART-HOLD-BACK >= 3x Part Target | MET-VERIFIED | engine.rs:250,1757 (3.0 multiplier); floor re-enforced in renderer broadcast-hls/src/lib.rs:759-763; test engine.rs:2410 (0.5 -> 1.5) | |
| 14.4 | Window > 2 min SHOULD offer delta updates | GAP (conditional) | grep CAN-SKIP/_HLS_skip in hls-runtime server + multimux = none; `BlockingReloadQuery` (llhls.rs:133) has no `_HLS_skip`; `can_skip_until` only set in client-side code | Origin never advertises CAN-SKIP-UNTIL. Default window is 8 segments (config.rs:1832) so below 2 min unless operator raises it; no guard ties the two. |
| 14.5 | Delta updates + DATERANGE SHOULD use CAN-SKIP-DATERANGES | NOT-APPLICABLE | n/a | Conditional on 14.4 being offered; vacuous. |

## Section 15 SharePlay

| Item | Requirement | Class | Evidence | Note |
|---|---|---|---|---|
| 15.1 | Live timeline from PROGRAM-DATE-TIME must agree for all participants | GAP | engine.rs:1722-1790 (no PDT emitted) | Without PDT clients cannot use the PDT timeline at all. All clients see one shared trunk so any PDT would agree, but none exists. |
| 15.2 | VOD timeline from playlist start must agree | MET-UNVERIFIED | engine.rs (single shared render per origin); catch-up playlists multimux/src/output/catchup.rs:237,308 | Same bytes to every client; no test. Live-only origin, VOD only via catch-up. |
| 15.3 | Interstitials SHOULD match in duration | NOT-APPLICABLE (CONTENT) | multimux does not emit interstitials; ssai-runtime/src/playlist.rs is separate and not a dependency of multimux/hls-runtime (git grep on Cargo.toml = none) | |

## Section 16 Spatial video

The origin has no notion of spatial/stereo content: it never inspects the init segment for `vexu`/MV-HEVC and emits no layout attributes.

| Item | Requirement | Class | Evidence | Note |
|---|---|---|---|---|
| 16.1 | Spatial variants MUST carry REQ-VIDEO-LAYOUT | GAP-OPTIONAL | engine.rs:1272-1289 (only VIDEO-RANGE via extra_attrs); VIDEO-LAYOUT only in broadcast-hls tests/parse | Origin could detect MV-HEVC/vexu in the init and emit it, or take an operator override. For `hls_pull` passthrough the source attribute is dropped because the master is regenerated. |
| 16.2 | Spatial content MUST include vexu boxes | NOT-APPLICABLE (CONTENT/ENCODER) | n/a | Origin passes init bytes through; could GAP-OPTIONAL verify presence. |
| 16.3 | Spatial<->non-spatial switch marked DISCONTINUITY or dual sample descriptions | MET-UNVERIFIED | engine.rs:~494-500 (init generation change forces `discontinuous = true`) | Generic mechanism covers it; no spatial-specific test. |
| 16.4 | Stereo/immersive SHOULD carry parallax metadata for subtitles | NOT-APPLICABLE (CONTENT) | n/a | |
| 16.5 | Spatial with mono sections MUST include CH-MONO | GAP-OPTIONAL | as 16.1 | |
| 16.6 (visionOS) | Stereo MUST be MV-HEVC | NOT-APPLICABLE (ENCODER) | n/a | |
| 16.7 (visionOS) | Immersive MUST use CH-STEREO + PROJ-AIV | GAP-OPTIONAL | as 16.1 | Also "immersive MUST point to AIME via EXT-X-SESSION-DATA" (Additional spatial specs): origin emits no SESSION-DATA. |
| 6.18/6.19 (visionOS trick play, adjacent) | I-frame list 160px | NOT-APPLICABLE | n/a | Outside group; I-frame work tracked in #1413. |

## Appendix guidance with testable origin behaviour

| Item | Requirement | Class | Evidence | Note |
|---|---|---|---|---|
| Declared vs measured: AVERAGE-BANDWIDTH | Live avg < 110% of declared; VOD within 10% | PARTIAL | engine.rs:742-751, :1270-1271 (cumulative mean since origin start, clamped to BANDWIDTH); tests hls-runtime/tests/avg_bandwidth.rs:86-190 | Average is cumulative since start, not ~1 h, and a master fetched early reflects a short sample. Verified arithmetic; the 110% live bound itself is not tested against a varying stream. |
| Declared vs measured: BANDWIDTH | Live peak < 125% of declared; VOD within 10% | PARTIAL | engine.rs:503-508 (max per-segment bitrate seen so far, rounded up), :1258-1260 (5 Mbit placeholder until first segment), :1263 | Peak only ever ratchets up, so a master fetched early under-declares a later peak (violates the "< 125%" bound); before any segment the 5 Mbit/s value is invented (comment admits placeholder). Per-segment peak is not the same as the table's "peak bit rate". |
| Declared vs measured: peak vs average < 200% (VOD) | | NOT-APPLICABLE (ENCODER) | n/a | Encoder rate control. Origin could verify (GAP-OPTIONAL). |
| MVP and resources must not expire | | PARTIAL (mostly NOT-APPLICABLE, PLATFORM) | origin/mod.rs master route; segment window evicts (engine.rs window) | Master URL persists while the route is configured; segments roll out of the live window and 404. That is normal for live; for downloadable VOD (catch-up/DVR) retention is operator config. No test. |
| Content Steering: STABLE-RENDITION-ID/STABLE-VARIANT-ID, steering manifest availability | | NOT-APPLICABLE | git grep steering in hls-runtime/multimux = none | Conditional on supporting steering ("If the content supports"); origin does not, so vacuous. Steering manifest MIME application/vnd.apple.steering-list never served. |
| Interstitials: assets follow main content structure; X-ASSET-LIST MIME application/json | | NOT-APPLICABLE | ssai-runtime/src/playlist.rs (renders X-ASSET-LIST) is not wired into multimux/hls-runtime | The served origin emits SCTE-35 DATERANGE only (engine.rs:1730-1745); no interstitials, no asset-list endpoint. |
| Download performance: fMP4 + contiguous BYTERANGE, avoid small files | | GAP-OPTIONAL | engine.rs:1565 (each seg/part is its own file URI; no BYTERANGE except the I-frame playlist, engine.rs:1946-1954) | Every segment and part is a separate HTTP resource; range support exists (http.rs) so single-file BYTERANGE playlists would be possible. |
| Low-Latency server profile reference | App. B.1 | PARTIAL | see 14.1 rows | |

## Counts per class (every row in the tables above, 14.1 sub-rows counted individually)

- MET-VERIFIED: 4 (14.1-B1c, 14.1-B1i, 14.1-B1o, 14.3)
- MET-UNVERIFIED: 8 (13.4, 14.1-B1b, B1e, B1h, B1j, B1l, 15.2, 16.3)
- PARTIAL: 12 (11.4, 12.4, 13.2, 13.3, 13.7, 14.1, 14.1-B1n, 14.2c, AVERAGE-BANDWIDTH, BANDWIDTH, MVP expiry, LL profile reference)
- GAP: 7 (14.1-B1a, B1d, B1f, B1g, B1k, 14.4, 15.1)
- GAP-OPTIONAL: 14 (11.1-11.3, 12.1-12.3, 13.1, 13.5, 13.6, 13.11, 16.1, 16.5, 16.7, download performance)
- NOT-APPLICABLE: 15 (13.8-13.10, 14.1-B1m, 14.2a, 14.2b, 14.5, 15.3, 16.2, 16.4, 16.6, 6.18/6.19, peak-vs-average, content steering, interstitials)

## Top 10 gaps by severity (MUST-level first)

1. 14.1-B1a / 15.1: no EXT-X-PROGRAM-DATE-TIME in any served media playlist (B.1 requires it; SharePlay timeline depends on it). Fix: add `program_date_time: Option<String>` to `broadcast_hls::MediaSegment`, emit it from `HlsOrigin::render_playlist` using `trunk.time_anchor()` (fall back to wall clock at first segment) with continuity across DISCONTINUITY.
2. 14.1-B1f: HTTP/1.1 only. Fix: enable hyper `http2` + `server-auto` and an optional TLS/ALPN (rustls) listener in `serve_hyper_util`, or document that an H2/H3 proxy is mandatory for LL-HLS conformance.
3. 14.1-B1k (and 10.1): playlists not gzip. Fix: add tower-http `compression-gzip` (or manual gzip of `HlsBody::Playlist`) honoring `Accept-Encoding`, with `Vary: Accept-Encoding` appended next to the existing Vary (origin/mod.rs:911).
4. 14.1-B1d: no `EXT-X-PRELOAD-HINT:TYPE=MAP` when the hinted part uses a new init. Fix: in `render_playlist` compare `generation_for(open_seq)` with the generation of the hinted part's segment and emit a second MAP hint (`preload_hint_type` already modelled in broadcast-hls).
5. 14.1-B1g: no RFC 9218 `Priority` response header. Fix: set `priority: u=1` on playlists and `u=2..6` on segments/parts (monotone by variant bitrate) in `add_response_headers`.
6. 14.4: CAN-SKIP-UNTIL/`_HLS_skip` delta updates never offered while window length is unbounded config. Fix: parse `_HLS_skip` in `BlockingReloadQuery`, render EXT-X-SKIP via `broadcast_hls::SkipInfo`, advertise CAN-SKIP-UNTIL=6xTD when window_secs > 120, and add config validation tying them.
7. Appendix BANDWIDTH: peak ratchets up so early masters under-declare; 5 Mbit/s placeholder is invented. Fix: render BANDWIDTH from the max of observed and a configured/ingest-declared bitrate, or return 503 for the master until measured; add a test with a bitrate step.
8. 13.11 / 13.3: key-tag builders disagree and `cenc` yields SAMPLE-AES-CTR. Fix: add an Apple-profile mode to `cenc_ext_x_key` (cbcs only, KEYFORMAT com.apple.streamingkeydelivery) and a guard rejecting SAMPLE-AES-CTR; wire an encrypted route end to end with a mediastreamvalidator oracle on the origin's real output.
9. 11.x/12.x TLS: no TLS in the origin. Fix: optional rustls listener (min TLS 1.2, PFS suites only) or an explicit startup warning/doc that TLS must be terminated upstream, since Apple says it may require TLS.
10. 16.1/16.5/16.7 + 13.5/13.6: no REQ-VIDEO-LAYOUT / HDCP-LEVEL / SESSION-DATA emission. Fix: per-route config for `video_layout` and `hdcp_level` pushed into `Variant::extra_attrs`, and auto-detect MV-HEVC/vexu from the init box in transmux.

Also notable test-quality gap: media-doctor/tests/mediastreamvalidator_oracle.rs:886-947 validates a **hand-built** VOD LL-HLS playlist, not the origin's own live output, and ignores the "No rendition reports" finding; the origin's real LL-HLS playlist (with PDT absent) has never been run through Apple's validator in the repo. Part-target and part-hold-back checks pass only on the one 0.5 s configuration.

## Could not assess

- Whether the signed-URL auth scheme (`OutputAuthSpec::SignedUrl`) is usable by an HLS client for segment fetches (12.4): read config + gate only, did not trace `broadcast_auth::signed_url` or run a player.
- 13.7/13.9 on the encryptor path: read only the pattern parameter plumbing, not the full `cenc_encrypt` default behaviour.
- Real-TCP behaviours (SACK, ECN, TLS 1.3 0-RTT, Age header at the CDN) are deployer/platform, not in code.
- Whether the segmenter always starts non-partial segments on an IDR (14.1-B1b): relied on sync-sample cutting in the pipeline without reading transmux's segmenter code.
- 14.4 threshold: default window (8 segments) is below 2 min; actual operator configs not examined.
