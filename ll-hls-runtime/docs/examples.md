# HLS Playlist Examples — draft-pantos-hls-rfc8216bis-22 §9

Source: `specs/ietf_draft_pantos_hls_rfc8216bis.txt`, draft-pantos-hls-rfc8216bis-22
(1 May 2026), section 9 "Playlist Examples" (source lines 4608-5023).

Extracted **verbatim** from the spec text as committed test vectors. The only
edits made to any playlist/manifest body below are:

1. Removal of the uniform 3-space RFC body-text left margin (so the bodies are
   valid m3u8/JSON starting at column 0).
2. Removal of page-break artifacts (`Pantos ... [Page NN]` / `Internet-Draft
   ... May 2026` running headers/footers and the surrounding blank lines /
   form-feed character introduced by pagination) that fall **between**
   examples. None of the page breaks in this range land inside a playlist
   body, so no code block required interior surgery.

No other whitespace was touched: line-continuation backslashes (`\`) and the
extra indentation the spec authors used to visually align continuation lines
(e.g. the 3 extra spaces before `DEFAULT=YES` in §9.6/§9.7) are preserved
exactly as printed. Per the spec's own note at the top of section 9:

> In some examples a '\' is used for readability to indicate that the tag
> continues on the following line.

so those backslashes and line breaks are editorial artifacts of the spec's
prose formatting, not literal m3u8 syntax — a real HLS parser would see the
tag as broken across lines with a trailing `\`. They are preserved here
because the instruction is byte-fidelity to the spec text, not reconstruction
of a "real" single-line file.

Two examples (§9.8, §9.9) are explicit **fragments**, not complete playlists
— the spec itself says so ("In this example, only the EXT-X-SESSION-DATA is
shown"; a CHARACTERISTICS attribute value in isolation) — and neither begins
with `#EXTM3U`. They are included for completeness but flagged as fragments
in the summary table. §9.12 and §9.13 also each carry a Content Steering
Manifest, which is JSON, not m3u8.

---

## 9.1. Simple Media Playlist

A minimal VOD Media Playlist: three segments, no encryption, `#EXT-X-ENDLIST`.

```
#EXTM3U
#EXT-X-TARGETDURATION:10
#EXT-X-VERSION:3
#EXTINF:9.009,
http://media.example.com/first.ts
#EXTINF:9.009,
http://media.example.com/second.ts
#EXTINF:3.003,
http://media.example.com/third.ts
#EXT-X-ENDLIST
```

## 9.2. Live Media Playlist Using HTTPS

Live (no `EXT-X-ENDLIST`) Media Playlist with `EXT-X-MEDIA-SEQUENCE` and
HTTPS segment URIs.

```
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:8
#EXT-X-MEDIA-SEQUENCE:2680

#EXTINF:7.975,
https://priv.example.com/fileSequence2680.ts
#EXTINF:7.941,
https://priv.example.com/fileSequence2681.ts
#EXTINF:7.975,
https://priv.example.com/fileSequence2682.ts
```

## 9.3. Playlist with Encrypted Media Segments

AES-128 encryption via `EXT-X-KEY`, with a key rotation partway through
(second `EXT-X-KEY` applies to the segment(s) that follow it).

```
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-MEDIA-SEQUENCE:7794
#EXT-X-TARGETDURATION:15

#EXT-X-KEY:METHOD=AES-128,URI="https://priv.example.com/key.php?r=52"

#EXTINF:2.833,
http://media.example.com/fileSequence52-A.ts
#EXTINF:15.0,
http://media.example.com/fileSequence52-B.ts
#EXTINF:13.333,
http://media.example.com/fileSequence52-C.ts

#EXT-X-KEY:METHOD=AES-128,URI="https://priv.example.com/key.php?r=53"

#EXTINF:15.0,
http://media.example.com/fileSequence53-A.ts
```

## 9.4. Multivariant Playlist

Four `EXT-X-STREAM-INF` variants (three video renditions + an audio-only
variant with `CODECS`), no alternative renditions.

```
#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=1280000,AVERAGE-BANDWIDTH=1000000
http://example.com/low.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2560000,AVERAGE-BANDWIDTH=2000000
http://example.com/mid.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=7680000,AVERAGE-BANDWIDTH=6000000
http://example.com/hi.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=65000,CODECS="mp4a.40.5"
http://example.com/audio-only.m3u8
```

## 9.5. Multivariant Playlist with I-Frames

Adds `EXT-X-I-FRAME-STREAM-INF` (I-frame-only trick-play playlists) alongside
each `EXT-X-STREAM-INF` variant.

```
#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=1280000
low/audio-video.m3u8
#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=86000,URI="low/iframe.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=2560000
mid/audio-video.m3u8
#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=150000,URI="mid/iframe.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=7680000
hi/audio-video.m3u8
#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=550000,URI="hi/iframe.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=65000,CODECS="mp4a.40.5"
audio-only.m3u8
```

## 9.6. Multivariant Playlist with Alternative Audio

Three `EXT-X-MEDIA` audio renditions in group `"aac"` (English/Deutsch/
Commentary), referenced by `AUDIO="aac"` on each `EXT-X-STREAM-INF`. Note:
per the spec, "the CODECS attributes have been condensed for space" — the
literal `CODECS="..."` is in the spec text, not a placeholder introduced here.

```
#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aac",NAME="English",\
   DEFAULT=YES,AUTOSELECT=YES,LANGUAGE="en",\
   URI="main/english-audio.m3u8"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aac",NAME="Deutsch",\
   DEFAULT=NO,AUTOSELECT=YES,LANGUAGE="de",\
   URI="main/german-audio.m3u8"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="aac",NAME="Commentary",\
   DEFAULT=NO,AUTOSELECT=NO,LANGUAGE="en",\
   URI="commentary/audio-only.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=1280000,CODECS="...",AUDIO="aac"
low/video-only.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2560000,CODECS="...",AUDIO="aac"
mid/video-only.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=7680000,CODECS="...",AUDIO="aac"
hi/video-only.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=65000,CODECS="mp4a.40.5",AUDIO="aac"
main/english-audio.m3u8
```

## 9.7. Multivariant Playlist with Alternative Video

Three `EXT-X-MEDIA TYPE=VIDEO` renditions (Main/Centerfield/Dugout) per
bitrate tier (low/mid/hi), referenced via `VIDEO="low"`/`"mid"`/`"hi"` on the
corresponding `EXT-X-STREAM-INF`. `CODECS` again condensed to `"..."` per the
spec's own note.

```
#EXTM3U
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="low",NAME="Main",\
   DEFAULT=YES,URI="low/main/audio-video.m3u8"
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="low",NAME="Centerfield",\
   DEFAULT=NO,URI="low/centerfield/audio-video.m3u8"
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="low",NAME="Dugout",\
   DEFAULT=NO,URI="low/dugout/audio-video.m3u8"

#EXT-X-STREAM-INF:BANDWIDTH=1280000,CODECS="...",VIDEO="low"
low/main/audio-video.m3u8

#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="mid",NAME="Main",\
   DEFAULT=YES,URI="mid/main/audio-video.m3u8"
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="mid",NAME="Centerfield",\
   DEFAULT=NO,URI="mid/centerfield/audio-video.m3u8"
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="mid",NAME="Dugout",\
   DEFAULT=NO,URI="mid/dugout/audio-video.m3u8"

#EXT-X-STREAM-INF:BANDWIDTH=2560000,CODECS="...",VIDEO="mid"
mid/main/audio-video.m3u8

#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="hi",NAME="Main",\
   DEFAULT=YES,URI="hi/main/audio-video.m3u8"
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="hi",NAME="Centerfield",\
   DEFAULT=NO,URI="hi/centerfield/audio-video.m3u8"
#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID="hi",NAME="Dugout",\
   DEFAULT=NO,URI="hi/dugout/audio-video.m3u8"

#EXT-X-STREAM-INF:BANDWIDTH=7680000,CODECS="...",VIDEO="hi"
hi/main/audio-video.m3u8
```

## 9.8. Session Data in a Multivariant Playlist

**Fragment, not a complete playlist** — the spec shows only the
`EXT-X-SESSION-DATA` tags in isolation ("In this example, only the
EXT-X-SESSION-DATA is shown"). No `#EXTM3U` header is present in the source.

```
#EXT-X-SESSION-DATA:DATA-ID="com.example.lyrics",URI="lyrics.json"

#EXT-X-SESSION-DATA:DATA-ID="com.example.title",LANGUAGE="en",\
   VALUE="This is an example"
#EXT-X-SESSION-DATA:DATA-ID="com.example.title",LANGUAGE="es",\
   VALUE="Este es un ejemplo"
```

## 9.9. CHARACTERISTICS Attribute Containing Multiple Characteristics

**Fragment, not a playlist line at all** — just an isolated attribute-value
example (comma-separated characteristics), shown to illustrate the
`CHARACTERISTICS` attribute's value grammar, not a tag or playlist.

```
CHARACTERISTICS=
"public.accessibility.transcribes-spoken-dialog,public.easy-to-read"
```

## 9.10. EXT-X-DATERANGE Carrying SCTE-35 Tags

Two `EXT-X-DATERANGE` tags sharing an `ID`, showing an SCTE-35 "out" splice
followed later by the matching "in" splice. The `...` lines (both as a
playlist line and mid-hex-string line-continuations) are exactly as printed
in the spec — `...` on its own line stands in for elided Media Segment
declarations, and the SCTE35 hex payloads are wrapped mid-string with a
trailing `\` continuation exactly as the spec sets them.

```
#EXTM3U
...
#EXT-X-DATERANGE:ID="splice-6FFFFFF0",\
   START-DATE="2014-03-05T11:15:00Z",PLANNED-DURATION=59.993,\
   SCTE35-OUT=0xFC002F000000000000FF000014056FFFFFF000E081622DCAFF0\
   00052636200000000000A0008029896F50000008700000000

... Media Segment declarations for 60s worth of media

#EXT-X-DATERANGE:ID="splice-6FFFFFF0",\
   DURATION=59.993,\
   SCTE35-IN=0xFC002A000000000000FF00000F056FFFFFF000408162802E610\
   0000000000A0008029896F50000008700000000
...
```

## 9.11. Low-Latency Playlist

Tail of a Low-Latency HLS Media Playlist: `EXT-X-PART` partial segments,
an `EXT-X-DISCONTINUITY` mid-roll ad break, an `EXT-X-PRELOAD-HINT`, and an
`EXT-X-RENDITION-REPORT`. As in §9.10, the leading `...` stands in for
elided earlier content (the spec notes "EXT-X-PART tags have been removed
from earlier Parent Segments").

```
#EXTM3U
#EXT-X-TARGETDURATION:4
...
#EXTINF:4.00008,
fileSequence268.mp4
#EXTINF:4.00008,
fileSequence269.mp4
#EXTINF:4.00008,
fileSequence270.mp4
#EXT-X-PART:DURATION=2.00004,INDEPENDENT=YES,URI="filePart271.0.mp4"
#EXT-X-PART:DURATION=2.00004,URI="filePart271.1.mp4"
#EXTINF:4.00008,
fileSequence271.mp4
#EXT-X-PART:DURATION=2.00004,INDEPENDENT=YES,URI="filePart272.0.mp4"
#EXT-X-PART:DURATION=0.50001,URI="filePart272.1.mp4"
#EXTINF:2.50005,
fileSequence272.mp4
#EXT-X-DISCONTINUITY
#EXT-X-PART:DURATION=2.00004,INDEPENDENT=YES,URI="midRoll273.0.mp4"
#EXT-X-PART:DURATION=2.00004,URI="midRoll273.1.mp4"
#EXTINF:4.00008,
midRoll273.mp4
#EXT-X-PART:DURATION=2.00004,INDEPENDENT=YES,URI="midRoll274.0.mp4"
#EXT-X-PRELOAD-HINT:TYPE=PART,URI="midRoll274.1.mp4"
#EXT-X-RENDITION-REPORT:URI="/1M/LL-HLS.m3u8",LAST-MSN=274,LAST-PART=1
```

## 9.12. Content Steering Playlist and Manifest

A Multivariant Playlist (`https://example.com/videos/video12/mv.m3u8`) using
`EXT-X-CONTENT-STEERING` and two Pathways (`CDN-A`/`CDN-B`), each with an
`AUDIO` group and `STABLE-VARIANT-ID`/`STABLE-RENDITION-ID`, followed by its
corresponding **JSON** Steering Manifest. The spec notes "Line breaks have
been added for legibility" for this example.

```
#EXTM3U
#EXT-X-CONTENT-STEERING:SERVER-URI="/steering?video=00012",\
   PATHWAY-ID="CDN-A"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="A",NAME="English",DEFAULT=YES,\
   URI="eng.m3u8",LANGUAGE="en",STABLE-RENDITION-ID="Audio-37262"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="B",NAME="ENGLISH",DEFAULT=YES,\
   URI="https://b.example.com/videos/video12/audio-eng.m3u8",\
   LANGUAGE="en",STABLE-RENDITION-ID="Audio-37262"
#EXT-X-STREAM-INF:BANDWIDTH=1280000,AUDIO="A",PATHWAY-ID="CDN-A",\
   STABLE-VARIANT-ID="Video-128"
128/video.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=7680000,AUDIO="A",PATHWAY-ID="CDN-A",\
   STABLE-VARIANT-ID="Video-768"
768/video.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=1280000,AUDIO="B",PATHWAY-ID="CDN-B",\
   STABLE-VARIANT-ID="Video-128"
https://backup.example.com/videos/video12/128/video.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=7680000,AUDIO="B",PATHWAY-ID="CDN-B",\
   STABLE-VARIANT-ID="Video-768"
https://backup.example.com/videos/video12/768/video.m3u8

{
  "VERSION": 1,
  "TTL": 300,
  "RELOAD-URI": "https://example.com/steering?video=00012&session=123",
  "PATHWAY-PRIORITY": [
    "CDN-A",
    "CDN-B"
  ]
}
```

## 9.13. Content Steering Manifest with Pathway Clone

Extends §9.12's Steering Manifest with a `PATHWAY-CLONES` entry (Pathway
Clone `CDN-A-CLONE` derived from base Pathway `CDN-A`, with `HOST`/`PARAMS`
URI replacement plus `PER-VARIANT-URIS`/`PER-RENDITION-URIS` overrides).
**JSON, not m3u8.** The spec follows the manifest with prose plus three
resulting URIs, both reproduced below.

```
{
  "VERSION": 1,
  "TTL": 300,
  "PATHWAY-PRIORITY": [
    "CDN-A-CLONE",
    "CDN-A"
  ],
  "PATHWAY-CLONES": [
    {
      "BASE-ID": "CDN-A",
      "ID": "CDN-A-CLONE",
      "URI-REPLACEMENT": {
        "HOST": "b2.example.com",
        "PARAMS": {
          "token": "dkfs1239414"
        },
        "PER-VARIANT-URIS": {
          "Video-768":
            "https://pv.example.com/videos/video12/768/video.m3u8"
        },
        "PER-RENDITION-URIS": {
          "Audio-37262":
            "https://pr.example.com/videos/video12/audio-eng.m3u8?a=1"
        }
      }
    }
  ]
}
```

The Pathway Clone with ID "CDN-A-CLONE" will have the URIs:

```
https://b2.example.com/videos/video12/128/video.m3u8?token=dkfs1239414
https://pv.example.com/videos/video12/768/video.m3u8
https://pr.example.com/videos/video12/audio-eng.m3u8?a=1
```

---

## Summary table

| # | Name | Subsection | Kind | Notable tags exercised |
|---|------|------------|------|-------------------------|
| 1 | Simple Media Playlist | 9.1 | Media | `EXT-X-TARGETDURATION`, `EXT-X-VERSION`, `EXTINF`, `EXT-X-ENDLIST` |
| 2 | Live Media Playlist Using HTTPS | 9.2 | Media | `EXT-X-MEDIA-SEQUENCE`, HTTPS segment URIs, no `EXT-X-ENDLIST` (live) |
| 3 | Playlist with Encrypted Media Segments | 9.3 | Media | `EXT-X-KEY` (AES-128), key rotation mid-playlist |
| 4 | Multivariant Playlist | 9.4 | Multivariant | `EXT-X-STREAM-INF`, `BANDWIDTH`, `AVERAGE-BANDWIDTH`, `CODECS` |
| 5 | Multivariant Playlist with I-Frames | 9.5 | Multivariant | `EXT-X-I-FRAME-STREAM-INF`, `EXT-X-STREAM-INF` |
| 6 | Multivariant Playlist with Alternative Audio | 9.6 | Multivariant | `EXT-X-MEDIA` (`TYPE=AUDIO`), `AUDIO` attribute, `DEFAULT`/`AUTOSELECT`/`LANGUAGE` |
| 7 | Multivariant Playlist with Alternative Video | 9.7 | Multivariant | `EXT-X-MEDIA` (`TYPE=VIDEO`), `VIDEO` attribute, multiple renditions per group |
| 8 | Session Data in a Multivariant Playlist | 9.8 | Fragment (no `#EXTM3U`) | `EXT-X-SESSION-DATA`, `DATA-ID`, `LANGUAGE`, `VALUE` |
| 9 | CHARACTERISTICS Attribute Containing Multiple Characteristics | 9.9 | Fragment (attribute value only) | `CHARACTERISTICS` attribute grammar |
| 10 | EXT-X-DATERANGE Carrying SCTE-35 Tags | 9.10 | Media | `EXT-X-DATERANGE`, `SCTE35-OUT`, `SCTE35-IN`, `PLANNED-DURATION` |
| 11 | Low-Latency Playlist | 9.11 | Media (LL-HLS) | `EXT-X-PART`, `EXT-X-DISCONTINUITY`, `EXT-X-PRELOAD-HINT`, `EXT-X-RENDITION-REPORT` |
| 12 | Content Steering Playlist and Manifest | 9.12 | Multivariant + JSON Steering Manifest | `EXT-X-CONTENT-STEERING`, `PATHWAY-ID`, `STABLE-VARIANT-ID`, `STABLE-RENDITION-ID`; manifest `PATHWAY-PRIORITY`/`RELOAD-URI` |
| 13 | Content Steering Manifest with Pathway Clone | 9.13 | JSON Steering Manifest only | `PATHWAY-CLONES`, `URI-REPLACEMENT` (`HOST`/`PARAMS`/`PER-VARIANT-URIS`/`PER-RENDITION-URIS`) |
