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


---


# Fixture provenance

All fixtures in this directory are unmodified files taken verbatim from the
**W3C `imsc-tests` repository** (`https://github.com/w3c/imsc-tests`), the
official conformance test suite for the IMSC family of specifications
(referenced from `https://github.com/w3c/imsc/`). Fetched from the `main`
branch at commit `08f10c5d5c36ab1105202ed964aee8c4ae939ed5` (2026-04-20), on
2026-07-30.

**Licence**: dual-licensed under the W3C Test Suite License and the W3C
3-clause BSD License (licensee's choice) — see
`https://github.com/w3c/imsc-tests/blob/main/LICENSE.md`. Both permit
copying and redistribution "in any medium for any purpose and without fee or
royalty" provided a link/URL to the original document and the W3C copyright
notice are retained. That attribution is given here. No right to create
*modifications* of the test files is granted by this licence — accordingly
every file below is committed byte-for-byte as fetched; none has been edited.

These are real, hand-authored-by-spec-editors conformance test documents
(several originally contributed by Institut für Rundfunktechnik, per their
own file headers, under Apache 2.0, itself compatible with redistribution
here), not synthetic happy-path samples — they carry the real oddities
(mixed profile designator styles across TTML1/TTML2 syntax eras, EBU-TT-D
dual-conformance metadata, negative shadow offsets, multi-byte Japanese
text with ruby annotations) that a hand-written fixture would not.

## Files

| File | Source path (in `w3c/imsc-tests`) | What it exercises |
|---|---|---|
| `imsc1-document-example-822.ttml` | `imsc1/ttml/document/DocumentExample822.ttml` | Baseline Text-Profile document: `tt:tt`-prefixed root, `p`/`span`/`br`, `tts:backgroundColor`, `tts:textAlign`, `ttm:title`/`ttm:desc`/`ttm:copyright` metadata. Derived from the DFXP 1.0 §8.2.2 spec example. |
| `imsc1-time-expressions-001.ttml` | `imsc1/ttml/timing/TimeExpressions001.ttml` | Exhaustive `<time-expression>` syntax coverage: `s`/`m`/`h`/`f`/`t` metrics, plain clock-time, fractional clock-time, clock-time-with-frames, `ttp:frameRate`+`ttp:frameRateMultiplier`+`ttp:tickRate` all set together, `seq` time container. |
| `imsc1-animation-001.ttml` | `imsc1/ttml/animation/Animation001.ttml` | `<set>` animation element changing `tts:backgroundColor` mid-interval. |
| `imsc1-backgroundcolor-rgba-001.ttml` | `imsc1/ttml/backgroundColor/backgroundcolor-rgba-001.ttml` | `#rrggbbaa` color syntax, `ttp:cellResolution`, referential `style` chaining (`style` element + `style=IDREFS`), dual EBU-TT-D/IMSC1 conformance via `ebuttm:conformsToStandard`. IRT-contributed (Apache-2.0 origin, see file header). |
| `imsc1.1-ruby-001.ttml` | `imsc1_1/ttml/ruby/ruby001.ttml` | `tts:ruby` (`container`/`base`/`text`) — a TTML2 feature only reachable through the IMSC 1.1 (not 1.0.1) Text Profile designator; real Japanese ruby text. |
| `imsc1.1-textemphasis-001.ttml` | `imsc1_1/ttml/textEmphasis/textEmphasis001.ttml` | `tts:textEmphasis` (`filled`/`open` × `circle`/`dot`/`sesame`), Japanese text, IMSC 1.1 designator. |
| `imsc1.1-textshadow-001.ttml` | `imsc1_1/ttml/textShadow/textShadow001.ttml` | `tts:textShadow` with **negative** offset components (`10% -20% 5% lime`) — exercises `<length>`'s signed-value path. |
| `imsc1.1-displayaspectratio-001.ttml` | `imsc1_1/ttml/displayAspectRatio/displayAspectRatio001.ttml` | `ttp:displayAspectRatio` on `tt`, IMSC 1.1 `ttp:contentProfiles` designator. |
| `imsc1-activearea-001.ttml` | `imsc1/ttml/activeArea/ActiveArea001.ttml` | `ittp:activeArea` (IMSC Extension namespace) with 3 regions, one of them outside the active area. **It does NOT exercise the §7.12.1.3 max-4-presented-regions cap (only 3 regions are present) nor the §7.12.1.2 no-overlap rule (the three y-extents — 10-20%, 80-90%, 92-98% — are disjoint).** An earlier revision of this row claimed it exercised both; that was a coverage overclaim, caught by the fidelity audit. No fixture in this set covers either constraint — see the gaps note below. |
| `imsc1-alttext-smpte-backgroundimage-001.ttml` | `imsc1/ttml/altText/altText1.ttml` | Image Profile via the older `smpte:backgroundImage` attribute mechanism (as opposed to the `image001` fixture's `<image>` element mechanism) + `ittm:altText` metadata element. Note: this fixture references an external `altText1-img.png` resource that was **not** fetched (only the `image001` pair below includes its image resource) — the `.ttml` is still useful standalone for parsing `smpte:backgroundImage`/`ittm:altText` syntax; a decoder exercising the referenced-resource path would need to fetch `imsc1/ttml/altText/altText1-img.png` separately. |
| `imsc1.1-image-profile-001.ttml` + `imsc1.1-image-profile-001-img.png` | `imsc1_1/ttml/image/image001.ttml` + `imsc1_1/ttml/image/image001-img.png` | Image Profile via the TTML2 `<image>` element (`src`/`type`/`tts:extent` all required per IMSC 1.1 §9.4.4) — a matched TTML+PNG pair, so this is the one fixture where the referenced image resource is actually present and byte-verifiable (640×120 8-bit RGBA PNG, verified with `file`). |

## What was deliberately excluded (and why)

- The bulk of the `imsc1`/`imsc1_1`/`imsc1_2`/`imsc1_3` test trees (321 TTML
  files + hundreds of reference PNGs total) — same licence, so redistribution
  would have been permitted, but committing the whole suite is unnecessary
  weight for a spec-prep task; the 11-file selection above was chosen to
  cover the syntax breadth documented in `private/specs-verified/w3c/ttml-subtitle/ttml2-syntax.md` and
  `private/specs-verified/w3c/ttml-subtitle/imsc11-profiles.md` (Text Profile, Image Profile via both
  mechanisms, animation, ruby/textEmphasis/textShadow as IMSC-1.1-only
  reachable TTML2 features, the full time-expression grammar, an IMSC
  extension attribute, and referential styling).
- `imsc1/ttml/altText/altText1-img.png` (the image resource referenced by
  `imsc1-alttext-smpte-backgroundimage-001.ttml`) — not fetched; noted above.
- DASH-IF test assets (`testassets.dashif.org`, `livesim2.dashif.org`) and
  the `Dash-Industry-Forum` GitHub org's `Test-Vectors` repo — both endpoints
  are reachable (checked), but a shallow pass (repo listing, `Test-Vectors`
  git metadata) found no obviously-indexed bare `.ttml`/IMSC XML document
  comparable to the W3C `imsc-tests` suite; DASH-IF's TTML-related test
  content is more likely muxed inside CMAF/fMP4 segments (a `transmux`
  concern, not this crate's), not exposed as standalone XML. This was not an
  exhaustive search of DASH-IF's holdings — the W3C `imsc-tests` suite above
  was already an authoritative, sufficient, and clearly-licensed source, so
  the search stopped there rather than being pursued further.
- No TTML/DFXP files pre-existed anywhere else in this repository (checked
  via `find`/`grep` across the whole workspace before fetching externally).

## Coverage gaps in this fixture set (established by the fidelity audit)

The 11 documents were checked against the spec for what they actually exercise, not what
they were assumed to. Nothing here covers:

- **The IMSC 1.1 §7.12.1.3 four-presented-region cap.** No fixture presents more than 3
  regions.
- **The §7.12.1.2 region-no-overlap rule.** Every fixture's regions have disjoint extents.
- **`itts:forcedDisplay`** and **`itts:fillLineGap`.**

These are all "must reject" structural constraints, so a validator implementing them has no
fixture to prove it bites. Either pull the relevant documents from the rest of the
`w3c/imsc-tests` suite (321 files; only 11 were taken here) or hand-construct violating
documents — and if hand-constructed, say so at the test site, because a hand-made document
only exercises the violation its author imagined.


## Notes

# `fixtures/subtitles/`

WebVTT/SRT fixtures for `caption-convert` (and any other consumer of plain
WebVTT/SRT text).

## `header-meta.vtt`

Hand-written (synthetic, no third-party content), following W3C WebVTT
(<https://www.w3.org/TR/webvtt1/>) SS4.1's own canonical example of a
`WEBVTT` header carrying free-form metadata text lines (`Kind:`/`Language:`)
before the first blank line — used by real WebVTT producers (e.g. YouTube's
caption exporter) as an informal MIME-header-style convention, distinct from
the `X-TIMESTAMP-MAP` header this crate already handled (RFC 8216 SS3.5,
issue #974).

Exercises audit finding CV-W1 / issue #1109 (reopens #974): `parse_webvtt`
used to fail the *whole document* when the header contained anything other
than `X-TIMESTAMP-MAP`, because the header text fell through into the
cue-block grouper and was misread as a cue identifier with no timing line
after it.

Independent oracle: ffmpeg 8.1.2 accepts this file as valid WebVTT and
converts it to the same two cues:

```sh
ffprobe -v error -show_entries stream=codec_name \
  -of default=noprint_wrappers=1 header-meta.vtt
# codec_name=webvtt

ffmpeg -v error -i header-meta.vtt -f srt -
# 1
# 00:00:01,000 --> 00:00:03,000
# Hello header test
#
# 2
# 00:00:04,000 --> 00:00:06,000
# Second cue
```

## `cap.vtt`, `sintel-en.srt`

Pre-existing fixtures — see `caption-convert/tests/it/webvtt_srt_fixture.rs`
and `caption-convert/tests/it/srt_real_fixture.rs` for their usage.


---


# Teletext fixtures — provenance

## `teletext-subtitle-synthetic.txt`

Synthetic-but-spec-real EN 300 706 Teletext subtitle page (magazine 8, page
0x88): see the file's own header comment for the full generation recipe and
page plan. Bytes are constructed with this crate's own verified
`encode_hamming_8_4`/`encode_odd_parity`, then stored **bit-reversed**
(wire/LSB-first order — issue #1041): EN 300 706 transmits each byte
LSB-first, so a real DVB capture's PES bytes are the bit-reversal of the
spec's own canonical values. Used by `timed-metadata/tests/it/webvtt_teletext_fixture.rs` and
`caption-convert/tests/it/teletext_to_webvtt_fixture.rs`.

## `teletext-subtitle-boxed.ts` + `teletext-subtitle-boxed.tsduck-oracle.srt`

Independent-tool oracle for issue #1041 (`transmux/tests/it/timed_metadata_teletext_tsduck_oracle.rs`).
`teletext-subtitle-synthetic.txt`'s oracle is this project's own reading of EN
300 706 and a hand-generated fixture; this pair instead cross-checks this
crate's Teletext decode against **TSDuck 3.44-4676's own independent Teletext
demux** (`tsp -P teletext`), which implements EN 300 472/706 (including its
own bit-reversal, `REVERSE_8`) from scratch.

### `teletext-subtitle-boxed.ts`

A synthetic, from-scratch MPEG-2 TS: PAT (service 1, PMT PID `0x100`) + PMT
(`PCR_PID 0x101`, one `stream_type 0x06` component on PID `0x101` carrying a
`teletext_descriptor`, tag `0x56`) + PES (`stream_id 0xBD`, `data_identifier
0x10`, `data_unit_id 0x03` SUBTITLE, `data_unit_length 0x2C`) frames carrying
the same magazine-8/page-0x88 plan as the `.txt` fixture above (header/erase,
row 20 "HELLO WORLD", row 22 "THIS IS A TEST", header/erase), each row's text
wrapped in EN 300 706's Start Box (`0x0B`)/End Box (`0x0A`) spacing-attribute
pair (clause 12.2) — the real-world convention TSDuck's own SRT extraction
requires to recognise boxed subtitle text (`tsTeletextDemux.cpp`,
`processTeletextPage`'s `0x0B`/`0x0A` scan); without it TSDuck reports zero
frames even though the address/Hamming/parity decode is correct.

Generated in two steps:

1. **PSI (PAT/PMT) via TSDuck's own compiler** (independent of this crate),
   `tstabcomp` (TSDuck 3.44-4676):

   ```
   tstabcomp --compile pat.xml -o pat.bin
   tstabcomp --compile pmt.xml -o pmt.bin
   ```

   `pat.xml`:
   ```xml
   <tsduck>
     <PAT transport_stream_id="1">
       <service service_id="1" program_map_PID="0x100" />
     </PAT>
   </tsduck>
   ```
   `pmt.xml` (the teletext_descriptor's exact byte layout —
   `lang(3="eng") + (teletext_type<<3|magazine) + BCD page` — is verified
   against `dvb-si/src/descriptors/teletext.rs` / EN 300 468 §6.2.44, and
   injected via TSDuck's `generic_descriptor` so this crate's own transcription,
   not TSDuck's differently-shaped `page_number` convenience field, is what's
   under test; confirmed round-trips as `teletext_type=0x02, magazine=0,
   page=136 (0x88), full page=888` via `tstables pmt.bin`):
   ```xml
   <tsduck>
     <PMT service_id="1" PCR_PID="0x101">
       <component stream_type="0x06" elementary_PID="0x101">
         <generic_descriptor tag="0x56">656e671088</generic_descriptor>
       </component>
     </PMT>
   </tsduck>
   ```

2. **TS/PES packetisation**: TSDuck has no XML-driven ES/PES builder, so the
   PAT/PMT sections above are wrapped into 188-byte TS packets, and the
   Teletext PES frames are built, by a small from-scratch Python script
   (standard MPEG-2 TS/PES framing: `pointer_field` + section for PAT/PMT;
   `00 00 01 BD` + `PES_packet_length` + `data_alignment_indicator=1` +
   PTS-only optional header + `PES_data_field` for each Teletext frame) — not
   committed (one-off), but fully specified above: any TS/PES muxer
   reproduces byte-identical output from the same PAT/PMT sections and frame
   plan.

### `teletext-subtitle-boxed.tsduck-oracle.srt`

TSDuck's own decode of `teletext-subtitle-boxed.ts`, recorded verbatim:

```
tsp -I file teletext-subtitle-boxed.ts -P teletext --pid 0x0101 \
    -o teletext-subtitle-boxed.tsduck-oracle.srt -O drop
```

(`--pid` explicit, matching the PMT's declared PID; the plugin also finds it
automatically via the PMT's `teletext_descriptor` with `--service` or no PID
option at all — verified to produce the same output.)

All files here are generated from synthetic input (hand-authored page text and TSDuck-compiled PSI); released under the workspace licence.

