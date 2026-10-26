# Provenance of the DASH fixtures

Everything under `fixtures/dash/` other than the original `manifest.mpd` family (see `README.md`) was produced in P0 of
`docs/dash-runtime/DESIGN.md` by an **external muxer** from a fixture this workspace already owns. No media byte was
written by hand; the only hand-written documents are the Annex G example MPDs (our own text) and one derived MPD
(`multiperiod-continuous/`, below).

## Licence and sources

| Source | Origin | Licence |
| --- | --- | --- |
| `fixtures/ts/h264_aac_40s.ts` | `fixtures/ts/h264_aac.ts` (this workspace's own H.264 + AAC capture) looped to about 48 s (`fixtures/PROVENANCE.md`, "Smooth Streaming segment fixture") | Own work, MIT OR Apache-2.0 (quoted from `fixtures/PROVENANCE.md`) |
| `fixtures/ts/h264_aac.ts` | this workspace's own H.264 + AAC capture (`fixtures/PROVENANCE.md`) | Own work, MIT OR Apache-2.0 |

Every derived file below inherits that licence (MIT OR Apache-2.0). No third-party media is included.

Tools: `ffmpeg 8.1.2` (libx264, native aac), `MP4Box`/`gpac` `GPAC 26.07-revrelease`, `xmllint` (libxml2). Generated 2026-10-26.

## Intermediate sources (not committed)

`$W` is a scratch directory outside the repository, `$R` the repository root.

```bash
ENC="-c:v libx264 -preset medium -profile:v main -g 50 -keyint_min 50 -sc_threshold 0 \
     -force_key_frames expr:gte(t,n_forced*2) -bf 0 -b:v 120k -c:a aac -b:a 48k -ar 44100"
# src-raw.mp4: 12 s, 2 s GOPs, video starts at 0.040 s (the TS had a 24 ms A/V start offset)
ffmpeg -y -i $R/fixtures/ts/h264_aac_40s.ts -t 12 $ENC $W/src-raw.mp4
# src.mp4: same with both streams re-based to start at 0
ffmpeg -y -i $R/fixtures/ts/h264_aac_40s.ts -t 12 -vf setpts=PTS-STARTPTS -af asetpts=PTS-STARTPTS $ENC $W/src.mp4
for s in src-raw src; do
  ffmpeg -y -i $W/$s.mp4 -map 0:v -c copy $W/$s-v.mp4
  ffmpeg -y -i $W/$s.mp4 -map 0:a -c copy $W/$s-a.mp4
done
```

## Fixtures

Every directory carries `oracle.txt`: the output of the external oracles run against it (`xmllint` against the ISO/IEC
23009-1 Ed.6 `DASH-MPD.xsd` held in the private tree, `gpac -i <mpd> inspect`, `MP4Box -diso` for `tfdt`/`sidx`, `ffprobe`
on the media, and a box walk cross-checking `SegmentBase` byte ranges). `ffprobe`'s own DASH demuxer answered "Invalid data"
for every MPD tried here (the on-demand, SegmentList and live ones and ffmpeg's own freshly written `manifest.mpd`), so GPAC is the MPD-level oracle.

### `annex-g/`

`example-F1.mpd` ... `example-F9.mpd` and `example-F8.patch.mpp`: the fresh minimal example MPDs written for this repository in
`docs/specs/dash/23009-1/annex-g-examples.md` (section "Fresh minimal example MPDs"), extracted verbatim from the code
fences by script with an XML declaration prepended. Our own text, so licence MIT OR Apache-2.0. No media: the URLs use
the reserved `.test` TLD or dangling relative names. All nine MPDs validate against the Ed.6 XSD; the patch document is checked for
well-formedness only (its XSD includes a remote IANA schema).

### `ondemand-sidx/` (SegmentBase + sidx, isoff-on-demand)

```bash
cp $W/src-v.mp4 v.mp4; cp $W/src-a.mp4 a.mp4
MP4Box -quiet -dash 2000 -rap -frag-rap -profile onDemand -out ondemand.mpd v.mp4 a.mp4
```

One single-file fragmented MP4 per Representation (`BaseURL` + `SegmentBase@indexRange` + `Initialization@range`), six
subsegments of 2 s each in a `sidx`. The oracle's box walk confirms `Initialization@range` = `ftyp..moov` and `indexRange` = the `sidx` box exactly.

### `ondemand-gpac-pto-quirk/` (same command, source with a 0.040 s video start)

```bash
cp $W/src-raw-v.mp4 v.mp4; cp $W/src-raw-a.mp4 a.mp4
MP4Box -quiet -dash 2000 -rap -frag-rap -profile onDemand -out ondemand.mpd v.mp4 a.mp4
```

Real GPAC 26.07 output that a client must survive: the video `SegmentBase` carries
`presentationTimeOffset="5354157010056287232"` (nonsense, about 4.2e14 s at the 12 800 timescale) because the source starts with an
empty edit of 40 ms. The `sidx` says `earliest_presentation_time=512` while the first `tfdt` is 0, so the sidx anchor
and the tfdt differ by the edit-list shift. Kept as-is; do not "fix" it.

### `segmentlist/` (SegmentList)

```bash
ffmpeg -y -i $W/src.mp4 -c copy -f dash -use_template 0 -use_timeline 0 -seg_duration 2 \
  -adaptation_sets "id=0,streams=v id=1,streams=a" \
  -init_seg_name 'init-$RepresentationID$.m4s' -media_seg_name 'seg-$RepresentationID$-$Number%03d$.m4s' list.mpd
```

`SegmentList@duration` with an `Initialization@sourceURL` and six `SegmentURL`s per Representation. Real quirks in the output:
`frameRate="49/2"` for a 24.5 fps source, and the audio `sidx earliest_presentation_time` is 1024 ticks (AAC priming) smaller
than the `tfdt` from segment 2 on (`tfdt=89088` vs `epct=88064`).

### `timeline-gpac/` (static SegmentTimeline, `$Number$`)

```bash
cp $W/src-v.mp4 v.mp4; cp $W/src-a.mp4 a.mp4
MP4Box -quiet -dash 2000 -profile live -segment-timeline -segment-name 'seg-$RepresentationID$-$Number$' -out timeline.mpd v.mp4 a.mp4
```

Real GPAC timeline output where the **audio timeline disagrees with the media**: the MPD says `S t=0 d=88064 r=3` then
`88063`, `88881` but the audio `tfdt`s are 0, 89088, 177152, 265216, 353280, 441343. The video timeline (`S t=0 d=25600 r=5`)
matches its `tfdt` exactly. The `Initialization` name is `seg-$RepresentationID$-.mp4` (GPAC's literal, trailing dash and all).

### `multiperiod-restart/` (two Periods, each Period's media restarts at tfdt 0)

```bash
ffmpeg -y -i $W/src.mp4 -t 6 -c copy p1.mp4
ffmpeg -y -ss 6 -i $W/src.mp4 -c copy p2.mp4
gpac -i p1.mp4:#Period=1 -i p2.mp4:#Period=2:#PStart=6 -o multiperiod.mpd:segdur=2
```

Two Periods (`Period@start` PT0 and PT6S) with `SegmentTemplate@duration` addressing, and every Representation id distinct
across Periods (1, 2, 3, 4). Media in Period 2 restarts at `tfdt` 0, so the Period boundary is a timeline reset, not a
continuation: there is no `presentationTimeOffset` and no continuity signalling. Quirks: Period 1 is declared
`duration=PT6.013S` (the audio is longer than the video) so it overlaps Period 2's `start=PT6S` by 13 ms, and the template
`duration="88200"` for audio does not match the real segment durations (88064 / 89088 ticks).

### `multiperiod-continuous/` (two Periods on one continuous media timeline, PTO set, continuity signalled)

Derived, MPD only: no new media. It re-addresses the **video** segments of `timeline-gpac/` (referenced through a
`BaseURL` of `../timeline-gpac/`, which also exercises relative BaseURL resolution) as two Periods: `a`
(`start=PT0S`, segments 1-3) and `b` (`start=PT6S`, segments 4-6, `startNumber=4`, `presentationTimeOffset=76800` = 6 s at
12 800), with `SupplementalProperty urn:mpeg:dash:period-continuity:2015 value="a"` on the same-`id` AdaptationSet in `b`. The
numbers are read from the real segments: `tfdt` of segment 4 is 76 800 (`oracle.txt` of `timeline-gpac/`), so `S@t` = PTO =
`tfdt`. The audio Representation is omitted on purpose, because GPAC's audio timeline (above) does not match its `tfdt`s and a
continuity fixture needs exact numbers. Written by hand from the GPAC output, in the same spirit as `manifest-inheritance.mpd`.

### `timeline-ffmpeg-quirk/` (reproduces the `S@t` vs `tfdt` offset of `manifest.mpd`)

```bash
ffmpeg -y -i $R/fixtures/ts/h264_aac.ts -c copy -tag:v avc1 -tag:a mp4a -seg_duration 1 -f dash manifest.mpd
```

The same command family as the committed `manifest.mpd`, re-run with ffmpeg 8.1.2. It reproduces the quirk: the video
`SegmentTimeline` starts at `S t="2090"` while the first segment's `tfdt` is 0 and its `sidx earliest_presentation_time` is
2090 (the committed file shows 2070, from an older ffmpeg; the cause of the offset was not analysed). `S@t` is a presentation
time (it equals the `sidx` value), not the `tfdt`. The audio timeline is exact (`t=0`).

### `live-number/` and `live-timeline/` (real dynamic MPDs, 4-segment window)

```bash
ffmpeg -y -re -i $R/fixtures/ts/h264_aac_40s.ts -c copy -tag:v avc1 -tag:a mp4a -f dash \
  -use_template 1 -use_timeline <0|1> -seg_duration 2 -window_size 4 -extra_window_size 0 \
  -adaptation_sets "id=0,streams=v id=1,streams=a" \
  -init_seg_name 'init-$RepresentationID$.m4s' -media_seg_name 'seg-$RepresentationID$-$Number%03d$.m4s' live.mpd
# killed with SIGKILL 16 s after start (the MPD is still `type="dynamic"`; a clean exit rewrites it as static);
# the two zero-byte `*.tmp` segment files left by the kill were deleted.
```

Both are a capture of ffmpeg's live output mid-run, with `availabilityStartTime`/`publishTime` of the capture day.
`live-number/` is `$Number$` + `@duration`; `live-timeline/` is `$Number$` + `SegmentTimeline`. Real quirks:

- `live-number`: `startNumber="1"` although only segments 5-8 exist on disk (the window moved on); a client must derive the
  current number from the clock. `@duration="2000000"` (timescale 1 000 000) while the video segments are 2.04 s, 3.04 s and 1.0 s long (the
  looped source has irregular GOPs).
- `live-timeline`: video and audio disagree on numbering (`startNumber="3"` for video, `5` for audio); the video `S t="365853"`
  while the first video `tfdt` is 363 763 (offset 2 090, the same as in `timeline-ffmpeg-quirk/`; `sidx earliest_presentation_time`=365 853).
- `timeShiftBufferDepth` is 12.1 s / 8.1 s, `minimumUpdatePeriod` 500 s / 3 s.

The gpac oracle line for these runs with `shift_utc` computed so the player's clock sits 3 s before `publishTime` (the files
are old; without the shift gpac looks for segments that no longer exist).

## Re-verifying

`python3 tools/check-dash-fixtures.py` checks `SHA256SUMS`, XML well-formedness of every `.mpd`/`.mpp`/`.xml`, and that every
file an MPD in a media-bearing directory references exists. `python3 tools/check-dash-fixtures.py --update` rewrites
`SHA256SUMS` after an intentional change.
