# Real WebVTT (`wvtt`) and TTML (`stpp`) ISOBMFF artifacts

Everything here was produced on 2026-10-15 by the real tools below from two
inputs **authored for this repository** (no third-party content):

| Input | What it exercises |
|---|---|
| `subs.vtt` | `NOTE` blocks, a `STYLE` block, cue identifiers, cue settings (`line`/`position`/`align`/`size`), `<b>` `<i>` `<v>` `<c.class>` tags, multi-line payloads, overlapping cues, gaps between cues, non-ASCII text (accents, CJK, emoji) and escaped `&amp;`/`&lt;` |
| `subs.ttml` | an IMSC-style TTML document with styles, a region, a span, a `<br/>`, non-ASCII and escaped text |

Both inputs and every output are ours (the tools only transform our input), so
they carry the repository licence.

## Producers

`MP4Box` and `gpac` are the same release: `GPAC version 26.07-revrelease`
(Homebrew, macOS arm64). `ffmpeg 8.1.2` was not used (it cannot write `wvtt`).
The exact commands are in [`gen.sh`](gen.sh); run it to regenerate everything.

| Artifact | Producer command (abridged) |
|---|---|
| `mp4box/wvtt_progressive.mp4` | `MP4Box -add subs.vtt:fmt=VTT:lang=en -new ...` |
| `mp4box/wvtt_fragmented.mp4` | `MP4Box -frag 4000` of the file above |
| `mp4box/dash/*` (MPD, init, 6 segments) | `MP4Box -dash 4000 -profile live -segment-name 'wvtt_$Number$' subs.vtt:lang=en` |
| `mp4box/stpp_progressive.mp4`, `stpp_fragmented.mp4` | `MP4Box -add subs.ttml:lang=en -new ...` then `-frag 4000` |
| `gpac/wvtt_progressive.mp4` | `gpac -i subs.vtt -o ...` |
| `gpac/wvtt_fragmented.mp4` | `gpac -i subs.vtt -o ...:frag:cdur=4` |
| `gpac/dash/*` (MPD, init, 6 segments) | `gpac -i subs.vtt:#Language=en -o manifest.mpd:segdur=4:profile=live:template='gp_$Number$'` |
| `gpac/stpp_progressive.mp4` | `gpac -i subs.ttml -o ...` |

## Independent oracles (`oracle/`)

Produced by the tools, never by transmux:

* `*.gpac_inspect.txt` - gpac's own parse of each file (`gpac -i FILE inspect:deep:full`):
  per sample `dts`, `cts`, `dur`, `sap`, `size` and `CRC32` (gpac's `gf_crc_32`,
  the MPEG-2 CRC-32). For the DASH sets the input is the `manifest.mpd`.
* `*.mp4box_raw.vtt` - MP4Box's own WebVTT export of each wvtt track
  (`MP4Box -raw 1 FILE -out ...`): the cue identifiers, settings, payloads and
  `NOTE` blocks as MP4Box reads them back.

## What the producers write (observed, both tools)

Sample entry `wvtt` with a `vttC` holding only the first header line
(`WEBVTT - ...`). Each sample is a plain sequence of boxes: `vtte` (8 bytes) for
a gap; `vttc` holding `iden`, `sttg`, `payl` **in that order**; `vtta` holding a
`NOTE` block (written ahead of the cue that follows it); overlapping cues are
split into consecutive samples, the overlap holding two `vttc`. These layouts are
the reason `VttCueBox` now keeps child order and unknown children, and why
`vtta` is modelled (`transmux::VttAdditionalTextBox`, `parse_wvtt_sample`).
