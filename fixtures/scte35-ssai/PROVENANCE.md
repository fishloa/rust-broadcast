# SCTE-35 SSAI fixture set — provenance (issue #929 prep)

Fixture-acquisition pass for issue #929 (SSAI ad-stitcher). Per the
project's md-and-fixtures-first rule, this directory supplies the "real
cue-carrying source" the issue's scope requires before any splice-point
conditioning is implemented. **No SSAI code was written for this pass** —
this is fixture acquisition + verification only.

Both options from the prep brief were obtained:

1. `dash/` — a real DASH-IF live-simulator capture: CMAF video + audio
   segments with a genuine **inband SCTE-35 `emsg`** (DASH/CMAF path).
2. `ts/` — a real MPEG-2 TS carrying the *same* genuine SCTE-35 bytes
   alongside real H.264 video, for the TS path.

Both carry a `splice_insert()` command (the "genuine ad break" the brief
asked for), not a hand-built vector.

## Source

**DASH-IF `livesim2`** — `https://livesim2.dashif.org/livesim2/scte35_1/testpic_2s/`
(a DASH-IF reference live simulator; the repo already trusted this source
for `fixtures/shared/emsg_v1_scte35_livesim.bin`, extracted from the
`scte35_2` variant of the same asset).

Retrieved: 2026-08-09 (session date), via `curl`, exact requests below.

The underlying video/audio test content (`testpic_2s`) is not part of the
`livesim2` server code itself — the MPD's own `<ProgramInformation
moreInformationURL="https://github.com/dash-Industry-Forum/livesim-content">`
and `<Source>VoD source for DASH-IF livesim2</Source>` fields identify it as
coming from the **`Dash-Industry-Forum/livesim-content`** repository, which
is separately licensed from the `livesim2` server code (`livesim2` itself is
licensed "Other"/`NOASSERTION` on GitHub — its *content* repo is not, and it
is the content repo's bytes we are redistributing here, not the server).

**Licence: Apache License, Version 2.0** — confirmed via GitHub's repository
metadata (`"license": {"key": "apache-2.0", "spdx_id": "Apache-2.0"}` for
`Dash-Industry-Forum/livesim-content`) and by fetching the LICENSE file
directly:

> `https://raw.githubusercontent.com/Dash-Industry-Forum/livesim-content/master/LICENSE`
>
> ```
> Apache License
>                            Version 2.0, January 2004
>                         http://www.apache.org/licenses/
>
>    TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION
>    ...
> ```
> (full text at the URL above; permissive, redistribution-compatible —
> matches this workspace's own dual MIT/Apache-2.0 licensing).

This mirrors the licence basis already accepted in this repo for
`emsg_v1_scte35_livesim.bin`; nothing here relies on a new or weaker
licence claim.

## How the SCTE-35 cue actually lands (verified from the `livesim2` Go source)

Rather than assume the wiki's `scte35_x` parameter description was
sufficient, the actual server logic was read directly (shallow git clone,
not committed): `github.com/Dash-Industry-Forum/livesim2`,
`pkg/scte35/scte35.go`, `CreateEmsgAhead`:

```go
// CreateEmsgAhead generates an emsg SCTE-35 box if the segment covers the
// time 7s before the ad start. For scte35_1: dur 20s, starts 10s after the
// full minute.
modMinute := segStart % (60 * timescale)
minuteStart := segStart - modMinute
spliceInsertTimes = []uint64{minuteStart + 10*timescale}   // perMinute == 1
announceTime := spliceTime - 7*timescale
// emsg is embedded in the segment covering announceTime, not the segment
// covering spliceTime itself.
```

This matters: the `emsg` box is **not** in the segment covering the actual
splice instant — it is in the segment covering `spliceTime - 7s` (the
"announce" window). The fixture below spans both.

It also confirmed the command is a genuine `SetIsOut(true)` +
`SetHasDuration(true)` + `SetIsAutoReturn(true)` `splice_insert()`, built via
`github.com/Comcast/gots/v2/scte35` — i.e. real generator output, not a
value transcribed from a spec table.

## `dash/` — CMAF capture spanning the cue

Requested `scte35_1/testpic_2s` (1 cue/minute, 20s ad duration, at `hh:mm:10`)
for a live window computed from wall-clock time, waiting for the DASH
`availabilityStartTime`/segment-number formula (`floor(epoch_seconds /
segment_duration)`, `duration=2s`, `startNumber=0`, confirmed against the
live MPD) to bring the window into the 60s DVR window, then fetched with
`curl` immediately as each segment became live (never more than a few
seconds stale, to avoid the server's `410 Gone` eviction):

```
curl https://livesim2.dashif.org/livesim2/scte35_1/testpic_2s/V300/init.mp4
curl https://livesim2.dashif.org/livesim2/scte35_1/testpic_2s/V300/<N>.m4s   # N = 893151748..893151760
curl https://livesim2.dashif.org/livesim2/scte35_1/testpic_2s/A48/init.mp4
curl https://livesim2.dashif.org/livesim2/scte35_1/testpic_2s/A48/<N>.m4s    # same N range
```

Segments are committed renumbered `0.m4s .. 12.m4s` (relative index = `N -
893151748`) for a sane filename; the original absolute `$Number$` values are
recorded here for reproducibility. **Segment `3.m4s`** (original number
`893151751`, covering wall-clock `19:25:02`–`19:25:04` UTC on 2026-08-09) is
the one carrying the `emsg` — matching `announceTime = 19:25:10 − 7s =
19:25:03`, inside that segment's window, exactly as the source computed it.

`dash/manifest-template.mpd` is the live MPD as served at fetch time —
useful for track/timescale/codec context; it is **not** re-playable as-is
(dynamic MPD, wall-clock `$Number$` addressing tied to the moment of
capture), kept for reference only.

### Verified with our own tools (mp4-emsg + scte35-splice)

The raw `emsg` box was located and extracted byte-for-byte from
`dash/V300/3.m4s` (box starts 24 bytes into the fragment; `size=98`,
matches GPAC `MP4Box -diso`'s independent box-tree dump) and committed as
`emsg_splice_insert.bin`. Decoded with this workspace's own `mp4-emsg`:

```
$ cargo run -q -p mp4-emsg --example <parse> -- fixtures/scte35-ssai/emsg_splice_insert.bin
version           : RepresentationRelative
scheme_id_uri     : urn:scte:scte35:2013:bin
value             : ""
timescale         : 90000
presentation_time : Absolute(160767315900000)
event_duration    : 1800000
id                : 1786303510
is_scte35()       : true
message_data.len  : 40
message_data hex  : fc30250000713e89a000fff014056a78d4167fefff8ec17660fe001b77400000000000002af6d7c0
byte-exact round-trip: OK
```

`presentation_time / timescale = 1786303510` seconds since the Unix epoch =
**2026-08-09T19:25:10Z** — exactly `minuteStart (19:25:00) + 10s`, matching
the source formula above. The 40-byte `message_data` is the
`splice_info_section` itself, decoded with this workspace's own
`scte35-splice`:

```
$ cargo run -q -p scte35-splice --example <parse> -- <message_data.bin>
protocol_version : 0
pts_adjustment    : 1899923872
tier              : 0xFFF
command                   : splice_insert
splice_event_id           : 0x6A78D416
splice_event_cancel_indicator: false
out_of_network_indicator  : true      # ad break START
program_splice_flag       : true
splice_immediate_flag     : false
splice_time.pts_time      : Some(6690010720)   # = presentation_time mod 2^33 (verified: 160767315900000 % 2^33 = 6690010720)
break_duration.auto_return: true               # implicit ad-in after duration
break_duration.duration    : 1800000            # 20s @ 90kHz
unique_program_id         : 0
avail_num                 : 0
avails_expected           : 0
```

Both parses round-trip byte-exact through this crate's own `Serialize`
impls — this is a real, wire-accurate cue, not a hand-typed vector.
`pts_adjustment` (1,899,923,872) is non-zero — this is the actual encoder
output, not fabricated; SSAI conditioning should be tested against a
nonzero `pts_adjustment` too, since it's what real generators emit.

### Cue-to-IDR alignment (measured, not asserted)

Since transmux's own `Fmp4Demux` currently cannot build a track for this
real fixture (see **Bug found** below), the video track's sample-level
`dts`/`pts`/sync-sample flags were extracted independently, straight from
the `moof`/`traf`/`tfhd`/`tfdt`/`trun` boxes (ISO/IEC 14496-12 §8.8),
deliberately bypassing `moov`/`stsd`/`avcC` (the part that's broken) — a
from-scratch decode, not reliant on the buggy path or on any external tool's
interpretation. Every video sample in this capture happens to be a sync
sample (an all-intra "testpic" encode), so this reduces to: which sample's
`pts` is nearest the cue's absolute presentation time
(`160767315900000`, in the same 90kHz clock both the `emsg` and the video
track use)?

```
NEAREST IDR: pts=160767315906000  delta=6000 ticks = +0.067s relative to cue pts_time=160767315900000
```

**The cue is not exactly IDR-aligned in this capture** — the nearest
keyframe lands 6000 ticks (**67ms**) *after* the nominal splice instant, not
on it. That is the honest, measured answer, and it is realistic: this is
exactly the kind of small misalignment SSAI splice-point conditioning has to
detect and handle (snap-to-nearest-IDR, or hold/patch the frame gap), which
is why a synthetic, hand-aligned fixture would have hidden this question
entirely.

## `ts/` — MPEG-TS carrying the same real cue

`ts/video_with_scte35_splice_insert.ts`: the **same real video** (all 13
segments above, `ffmpeg -c copy` remux of the concatenated `V300` CMAF
track — container-only, no re-encode) plus the **same real
`splice_info_section` bytes** (byte-identical to `emsg_splice_insert.bin`'s
`message_data`, verified below), packaged onto a new elementary stream PID
using this workspace's own `dvb-si` (PMT builder) and `mpeg-ts`
(`SectionPacketiser`) — i.e. the video and the cue bytes are both real and
untouched; only the *TS-level packaging* (PMT edit, section packetisation)
is this workspace's own assembly, the same kind of "own work" fixture
construction already documented elsewhere in `fixtures/PROVENANCE.md`
(e.g. the DASH MPD fixture "generated by ffmpeg from this workspace's own
`fixtures/ts/h264_aac.ts`").

Construction, in order:
1. `ffmpeg -i <concatenated V300 CMAF> -c copy -f mpegts base_video.ts` —
   real video only, no cue, ffmpeg's own PAT/PMT/SDT.
2. Parsed the resulting PMT with `dvb_si::tables::pmt::PmtSection::parse`.
3. Added one `PmtStream { stream_type: StreamType::Scte35 (0x86),
   elementary_pid: 0x0101 }` to the existing single-video-stream PMT,
   version bumped, re-serialized with `PmtSection::serialize_into` (own
   crate, round-tripped through its own parser as a self-check).
4. Re-packetised the new PMT with `mpeg_ts::mux::SectionPacketiser`, same
   PID, continuity counter continued from the original PMT's, and replaced
   every original PMT packet slot with it (in place, one packet — the PMT
   fits a single TS packet before and after the edit).
5. Packetised the real 40-byte `splice_info_section` (identical bytes to
   `emsg_splice_insert.bin`'s payload) onto the new PID 0x0101, inserted
   once, immediately after the PMT's first occurrence (packet index 2).
   Real splicers repeat the message periodically ahead of the event; this
   fixture carries it once — the field that matters for SSAI testing is the
   embedded `pts_time`/`presentation_time`, not the TS packet's physical
   position, and that field is verified above to be the real, unmodified
   value.

### Verified with our own tools (dvb-tools)

```
$ cargo run -q -p dvb-tools -- pids fixtures/scte35-ssai/ts/video_with_scte35_splice_insert.ts
pid=0x0100  packets=3244  84.99%   # H.264 video (unchanged from the ffmpeg remux)
pid=0x0000  packets=260   6.81%    # PAT
pid=0x1000  packets=260   6.81%    # PMT (edited)
pid=0x0011  packets=52    1.36%    # SDT (ffmpeg's, unchanged)
pid=0x0101  packets=1     0.03%    # SCTE-35 (added)
-- total_packets=3817  bitrate=0.22 Mbit/s (PCR from pid 0x0100)

$ cargo run -q -p dvb-tools -- dump fixtures/scte35-ssai/ts/video_with_scte35_splice_insert.ts --json
...
"pmtSection": {
  "program_number": 1, "version_number": 1, "pcr_pid": 256,
  "streams": [
    { "stream_type": "H264",   "elementary_pid": 256 },
    { "stream_type": "Scte35", "elementary_pid": 257 }
  ]
}
...
-- packets=3817 sections=572 emitted=3 suppressed=569 crc_failures=0 malformed=0
```

`crc_failures=0` — the edited PMT and the inserted SCTE-35 section both pass
this workspace's own CRC-32/MPEG-2 validation. A byte-level check (extract
the TS packet on PID `0x0101`, strip the `pointer_field`) confirms the
embedded section is **byte-identical** to `emsg_splice_insert.bin`'s
`message_data` — the same genuine `splice_insert()` bytes as the DASH-side
fixture, just repackaged into TS. `dvb-tools dump` does not itself decode
SCTE-35 (a private table_id outside `dvb-si`'s DVB-SI table registry, by
design — that decode lives in `scte35-splice`); the splice command fields
are already fully decoded above via `scte35-splice` from the identical
bytes.

## Bug found while building this fixture (recorded, not fixed — out of this task's scope)

**`transmux`'s `Fmp4Demux` cannot currently build a track from this real
fixture.** Root cause, isolated:

- `dash/V300/init.mp4`'s `avc1` sample entry declares
  `AVCProfileIndication = 100` (H.264 High Profile).
- `transmux::avc_config::AVCDecoderConfigurationRecord::parse`
  (`transmux/src/avc_config.rs`, `has_high_profile_ext`/`is_high_profile`
  gate around line 162) treats the ISO/IEC 14496-15 §5.3.3.1.1
  backward-compatibility extension (`chroma_format` /
  `bit_depth_luma_minus8` / `bit_depth_chroma_minus8` /
  `numOfSequenceParameterSetExt`) as **mandatory** whenever the profile is
  in the High-profile family (100/110/122/244).
- This real encoder's `avcC` (41-byte body: version/profile/compat/level +
  length-size byte + 1 SPS + 1 PPS, and **nothing after the PPS**) omits
  that extension entirely — a real-world non-conformance (or at least an
  optional-in-practice field) that a hand-built/synthetic avcC fixture would
  never exercise, since a hand-built fixture is only ever as broken as its
  author remembered to make it.
- `read_u8` past the end of the 41-byte body returns `Err` as designed (no
  panic), but that `Err` propagates through
  `AVCSampleEntry::bare_parse` → `SampleDescriptionBox::parse`, where
  `transmux::init_segment::parse_stbl_children`'s
  `.unwrap_or_else(|_| SampleDescriptionBox { entries: Vec::new(), .. })`
  (`transmux/src/init_segment.rs`, the `b"stsd"` arm, ~line 2174) **silently
  swallows the error into zero `stsd` entries**, which makes
  `track_spec_from_trak` fail with `UnexpectedBox { expected: "stsd entry"
  }`, which makes the **entire video track** get dropped into
  `Media::skipped` rather than just losing the (optional-in-practice)
  extension fields.
- ffmpeg (`-c copy` remux, used for the `ts/` fixture above) reads this
  exact `init.mp4` without complaint, corroborating that the extension is
  being treated as more mandatory here than real-world muxers assume.

Reproducer (against the committed fixture, no scratch files needed):

```rust
// cat fixtures/scte35-ssai/dash/V300/{init.mp4,0.m4s,1.m4s,...,12.m4s} > combined.mp4
use broadcast_common::Unpackage;
use transmux::Fmp4Demux;
let data = std::fs::read("combined.mp4").unwrap();
let media = Fmp4Demux::new().unpackage(&data).unwrap();
assert_eq!(media.tracks.len(), 0); // currently 0 — should be 1
assert_eq!(media.skipped.len(), 1); // SkippedTrack { reason: "unexpected box: expected stsd entry" }
```

Not fixed here — this task is fixture acquisition only, and a fix to
`transmux`'s High-profile `avcC` handling (most likely: treat the extension
as present only if enough bytes remain, rather than assuming its presence
from `profile_indication` alone) is implementation work for a separate
issue/PR. Recommend filing one; this fixture is the reproducer.

## What was tried and did not pan out

- **GPAC `scte35dec` filter (`mode=m2ts`)**, both against the downloaded
  CMAF directly and against a live `-i https://livesim2.dashif.org/...`
  ingest: produced a TS with no SCTE-35 PID in either case (the filter
  graph needs more specific wiring than `gpac -i <src> scte35dec:mode=m2ts
  @ -o out.ts` to actually surface the inband `emsg` as a splice section —
  not pursued further once the `dvb-si`/`mpeg-ts`-based TS assembly above
  (using the *actual* decoded real cue bytes, not a GPAC-synthesized one)
  produced a correct, verified result by a more direct route).
- Considered Unified Streaming / Bitmovin / Akamai public demo streams for
  a second independent source; not pursued once `livesim2` (already
  trusted in this repo) yielded a fully verified fixture on both the DASH
  and TS paths, satisfying "at minimum one, ideally both" without taking on
  a second source's licence-review burden.

## Files

```
fixtures/scte35-ssai/
├── PROVENANCE.md
├── emsg_splice_insert.bin          # real emsg box, 98 bytes (from dash/V300/3.m4s)
├── dash/
│   ├── manifest-template.mpd       # live MPD as served at capture time (reference only)
│   ├── V300/{init.mp4,0.m4s..12.m4s}   # real H.264 video, avc1 High Profile, 90kHz
│   └── A48/{init.mp4,0.m4s..12.m4s}    # real AAC audio, mp4a
└── ts/
    └── video_with_scte35_splice_insert.ts   # real video + real cue, TS-packaged
```
