# Media Segments — draft-pantos-hls-rfc8216bis-22, Section 3

Transcription source: `specs/ietf_draft_pantos_hls_rfc8216bis.txt`,
draft-pantos-hls-rfc8216bis-22 (1 May 2026), lines 347-627, Section 3 "Media
Segments" in full (subsections 3.1 "Supported Media Segment Formats" through
3.1.5 "IMSC Subtitles", and 3.2 "Partial Segments"). This is a verbatim
transcription for reference; section numbers are cited throughout. Normative
keywords (MUST/SHOULD/MAY) are preserved exactly as written in the source.

---

## 1. What a Media Segment is (Section 3, general rules)

> A Media Playlist contains a series of Media Segments that make up the
> overall presentation. A Media Segment is specified by a URI and optionally
> a byte range.

These rules apply to **every** Media Segment regardless of container format.

- The duration of each Media Segment is indicated in the Media Playlist by
  its `EXTINF` tag (Section 4.4.4.1). The value is the total presentation
  duration of samples in the segment if the segment contains a single media
  type. In the case of multiple media types, use values for a single media
  type, preferring video media over audio over subtitles.

- Each segment in a Media Playlist has a unique integer Media Sequence
  Number. The Media Sequence Number of the first segment in the Media
  Playlist is either 0 or declared in the Playlist (Section 4.4.3.2). The
  Media Sequence Number of every other segment is equal to the Media
  Sequence Number of the segment that precedes it plus one.

- **Continuity (MUST):** Each Media Segment MUST carry the continuation of
  the encoded bitstream from the end of the segment with the previous Media
  Sequence Number, where values in a series such as timestamps and
  Continuity Counters MUST continue uninterrupted. The only exceptions are
  the first Media Segment ever to appear in a Media Playlist and Media
  Segments that are explicitly signaled as discontinuities (Section 4.4.4.3).
  Unmarked media discontinuities can trigger playback errors.

- **Video decodability (SHOULD):** Any Media Segment that contains video
  SHOULD include enough information to initialize a video decoder and decode
  a continuous set of frames that includes the final frame in the Segment;
  network efficiency is optimized if there is enough information in the
  Segment to decode all frames in the Segment. For example, any Media
  Segment containing H.264 video SHOULD contain an Instantaneous Decoding
  Refresh (IDR); frames prior to the first IDR will be downloaded but
  possibly discarded.

### Media Initialization Section — general definition (Section 3.1)

> All Media Segments MUST be in a format described in this section. Transport
> of other media file formats is not defined.

> Some media formats require a common sequence of bytes to initialize a
> parser before a Media Segment can be parsed. This format-specific sequence
> is called the Media Initialization Section. The Media Initialization
> Section can be specified by an `EXT-X-MAP` tag (Section 4.4.4.5). The Media
> Initialization Section MUST NOT contain sample data.

This general definition applies to all formats; the per-format sections below
state whether an initialization section (and hence `EXT-X-MAP`) is required,
optional, or not applicable for that format.

---

## 2. Quick-reference: EXT-X-MAP requirement per format

| Format (spec §)                        | Media Initialization Section                          | EXT-X-MAP required? |
|-----------------------------------------|---------------------------------------------------------|----------------------|
| MPEG-2 Transport Stream (§3.1.1)        | PAT followed by PMT                                     | Conditional — required unless the Segment itself starts with PAT+PMT (see §3 below) |
| Fragmented MP4 / ISO BMFF (§3.1.2)      | An ISO Base Media File that can initialize a parser for the Segment (`ftyp` + Movie Box, no samples) | **MUST** — every fMP4 Segment in a Media Playlist MUST have an `EXT-X-MAP` tag applied to it |
| Packed Audio (§3.1.3)                   | None                                                     | Not applicable — "has no Media Initialization Section" |
| WebVTT (§3.1.4)                         | The WebVTT header                                       | Conditional — required unless the Segment itself starts with a WebVTT header |
| IMSC Subtitles (§3.1.5)                 | As specified in §3.1.2 (it is an fMP4 Segment)          | Same as fMP4 (§3.1.2) |

Full detail for each format is in Section 3, below.

---

## 3. MPEG-2 Transport Streams (Section 3.1.1)

MPEG-2 Transport Streams are specified by [ISO_13818].

**Media Initialization Section:** the Media Initialization Section of an
MPEG-2 Transport Stream Segment is a Program Association Table (PAT)
followed by a Program Map Table (PMT).

**Structural constraints:**

- Transport Stream Segments MUST contain a single MPEG-2 Program; playback
  of Multi-Program Transport Streams is not defined.
- Each Transport Stream Segment MUST contain a PAT and a PMT, **or** have an
  `EXT-X-MAP` tag (Section 4.4.4.5) applied to it.
- The first two Transport Stream packets in a Segment without an `EXT-X-MAP`
  tag SHOULD be a PAT and a PMT.

**EXT-X-MAP for TS — exact statement:** the spec does not say EXT-X-MAP is
either strictly required or forbidden for TS segments; it states a
disjunction: a TS Segment MUST contain a PAT and PMT, *or* have an EXT-X-MAP
tag applied. In other words, EXT-X-MAP is optional for TS segments only
if the segment itself starts with an in-band PAT+PMT; if it doesn't carry
its own PAT/PMT, EXT-X-MAP becomes the only way to satisfy the MUST.

---

## 4. Fragmented MPEG-4 / fMP4 (Section 3.1.2)

MPEG-4 Fragments are specified by the ISO Base Media File Format [ISOBMFF].
Unlike regular MPEG-4 files that have a Movie Box (`moov`) that contains
sample tables and a Media Data Box (`mdat`) containing the corresponding
samples, an MPEG-4 Fragment consists of a Movie Fragment Box (`moof`)
containing a subset of the sample table and a Media Data Box containing
those samples. Use of MPEG-4 Fragments does require a Movie Box for
initialization, but that Movie Box contains only non-sample-specific
information such as track and sample descriptions.

A Fragmented MPEG-4 (fMP4) Segment is a "segment" as defined by Section 3 of
[ISOBMFF], including the constraints on Media Data Boxes in Section 8.16 of
[ISOBMFF].

**Media Initialization Section:** the Media Initialization Section for an
fMP4 Segment is an ISO Base Media File that can initialize a parser for that
Segment.

Broadly speaking, fMP4 Segments and Media Initialization Sections are
[ISOBMFF] files that also satisfy the constraints described in this section.

**Media Initialization Section structure (MUST):**

- MUST contain a File Type Box (`ftyp`) containing a brand that is
  compatible with `iso6` or higher.
- The File Type Box MUST be followed by a Movie Box.
- The Movie Box MUST contain a Track Box (`trak`) for every Track Fragment
  Box (`traf`) in the fMP4 Segment, with matching `track_ID`.
- Each Track Box SHOULD contain a sample table, but its sample count MUST be
  zero.
- Movie Header Boxes (`mvhd`) and Track Header Boxes (`tkhd`) MUST have
  durations of zero.
- The Movie Box MUST contain a Movie Extends Box (`mvex`); it SHOULD follow
  the last Track Box.
- Note: a Common Media Application Format [CMAF] Header meets all these
  requirements.

**fMP4 Segment structure (MUST):**

- In an fMP4 Segment, every Track Fragment Box MUST contain a Track Fragment
  Decode Time Box (`tfdt`).
- fMP4 Segments MUST use movie-fragment-relative addressing.
- fMP4 Segments MUST NOT use external data references.
- Note: a CMAF Segment meets these requirements.

**I-frame playlists (MAY):** an fMP4 Segment in a Playlist containing the
`EXT-X-I-FRAMES-ONLY` tag (Section 4.4.3.6) MAY omit the portion of the
Media Data Box following the intra-coded frame (I-frame) sample data.

**Box order:** this specification makes no additional restrictions on
[ISOBMFF] boxes or box order. However, fMP4 Segments that indicate
compatibility with an additional standard, such as [CMAF], SHOULD comply
with whatever rules that standard requires.

**EXT-X-MAP for fMP4 — exact statement:**

> Each fMP4 Segment in a Media Playlist MUST have an EXT-X-MAP tag applied
> to it.

This is an unconditional MUST — unlike TS, there is no in-band alternative
stated for fMP4 Segments.

---

## 5. Packed Audio (Section 3.1.3)

A Packed Audio Segment contains encoded audio samples and ID3 tags that are
simply packed together with minimal framing and no per-sample timestamps.
Supported Packed Audio formats are Advanced Audio Coding (AAC) with Audio
Data Transport Stream (ADTS) framing [ISO_13818_7], MP3 [ISO_13818_3], AC-3
[AC_3], and Enhanced AC-3 [AC_3].

**Media Initialization Section:**

> A Packed Audio Segment has no Media Initialization Section.

(Not applicable to this format; the spec does not describe `EXT-X-MAP` usage
for Packed Audio at all.)

**Timestamp signalling (MUST / SHOULD NOT):**

- Each Packed Audio Segment MUST signal the timestamp of its first sample
  with an ID3 Private frame (PRIV) tag [ID3] at the beginning of the
  segment.
- The ID3 PRIV owner identifier MUST be
  `com.apple.streaming.transportStreamTimestamp`.
- The ID3 payload MUST be a 33-bit MPEG-2 Program Elementary Stream
  timestamp expressed as a big-endian eight-octet number, with the upper 31
  bits set to zero.
- Clients SHOULD NOT play Packed Audio Segments without this ID3 tag.

---

## 6. WebVTT (Section 3.1.4)

A WebVTT Segment is a section of a WebVTT [WebVTT] file. WebVTT Segments
carry subtitles.

**Media Initialization Section:** the Media Initialization Section of a
WebVTT Segment is the WebVTT header.

**Cue content constraints (MUST / MAY):**

- Each WebVTT Segment MUST contain all subtitle cues that are intended to be
  displayed during the period indicated by the segment `EXTINF` duration.
  The start time offset and end time offset of each cue MUST (with the
  single exception noted below) indicate the total display time for that
  cue, even if part of the cue time range is outside the Segment period.
- A WebVTT Segment MAY contain no cues; this indicates that no subtitles are
  to be displayed during that period.
- Exception (MAY): under certain conditions, like live streaming, where it
  is not possible to know the cue duration at the time of the segment
  creation and the subtitle cue interval is split over multiple Segments,
  the cue time range in each Segment MAY be limited to the WebVTT time range
  covered by the Segment.

**EXT-X-MAP for WebVTT — exact statement:**

> Each WebVTT Segment MUST either start with a WebVTT header or have an
> EXT-X-MAP tag applied to it.

Same disjunctive pattern as TS: in-band header satisfies the requirement, or
`EXT-X-MAP` must be applied.

**Timestamp synchronization (SHOULD / MUST):**

- In order to synchronize timestamps between audio/video and subtitles, an
  `X-TIMESTAMP-MAP` WebVTT metadata header [WebVTT-metadata-header] SHOULD
  be among a set of non-blank lines immediately after the `WEBVTT` header
  line.
- When present, this set of non-blank lines MUST be followed by two or more
  line terminators, followed by the rest of the body.
- This header maps WebVTT cue timestamps to media timestamps in other
  Renditions of the Variant Stream. Its format is:

  ```
  X-TIMESTAMP-MAP=LOCAL:<cue time>,MPEGTS:<media time>
  e.g., X-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:900000
  ```

  indicating the media time to which the cue time MUST be mapped. The cue
  timestamp in the LOCAL attribute MAY fall outside the range of time
  covered by the segment.
- The MPEGTS media timestamp MUST use a 90kHz timescale, even when
  non-WebVTT Media Segments use a different timescale.
- If a WebVTT Segment does not have the `X-TIMESTAMP-MAP`, the client MUST
  assume that the WebVTT cue time of 0 maps to a media timestamp of 0.
- When synchronizing WebVTT with PES timestamps, clients SHOULD account for
  cases where the 33-bit PES timestamps have wrapped and the WebVTT cue
  times have not. When the PES timestamp wraps, the WebVTT Segment SHOULD
  have an `X-TIMESTAMP-MAP` header that maps the current WebVTT time to the
  new (low valued) PES timestamp.

---

## 7. IMSC Subtitles (Section 3.1.5)

An IMSC Segment is a Fragmented MPEG-4 (Section 3.1.2) Media Segment that
carries subtitle media according to MPEG-4 Part 30 [MP4_TIMED_TEXT]. This
subtitle media MUST comply with the Text Profile of IMSC1 [IMSC1].

**Media Initialization Section:**

> The Media Initialization Section of an IMSC Segment is specified in
> Section 3.1.2.

I.e., IMSC Segments are fMP4 Segments and inherit the fMP4 initialization
rules verbatim (Section 4 above) — including the unconditional MUST that
every fMP4 Segment in a Media Playlist has an `EXT-X-MAP` tag applied to it.

**Content constraints (MUST):**

- Each IMSC Segment MUST contain all subtitle samples that are intended to
  be displayed during the period indicated by the segment `EXTINF`
  duration.
- Each Segment MUST contain definitions for all styles which are applied to
  any part of any sample in the Segment.

---

## 8. Partial Segments (Section 3.2)

Partial Segments are a cross-cutting mechanism, not a distinct container
format — a Partial Segment MUST be in one of the Supported Media Segment
Formats described in Section 3.1 (i.e., any format above), so all of the
per-format rules in Sections 3-7 above apply to a Partial Segment's own
content according to its container type.

**Purpose:** one component of viewer delay in a live stream is publishing
latency: a Segment cannot be distributed until it has been completely
encoded and packaged. A long Segment encoded in real-time introduces a
delay equal to its duration. Partial Segments provide a parallel channel
for distributing media at the live edge of the Media Playlist, where the
media is divided into a larger number of smaller pieces, such as CMAF
Chunks. These subsets are called Partial Segments. Because each Partial
Segment has a short duration, it can be packaged, published, and added to
the Media Playlist much earlier than its Parent Segment.

**Rules (MUST):**

- A Partial Segment MUST be in one of the Supported Media Segment Formats
  described in Section 3.1.
- A Partial Segment is associated with a regular Media Segment, called its
  Parent Segment, by appearing before it in the Media Playlist, and after
  the previous Media Segment. Partial Segments are identified by the
  `EXT-X-PART` tag (Section 4.4.4.9).
- A Partial Segment MUST contain a subset of the media samples in its
  Parent Segment.
- A Parent Segment and its entire set of Partial Segments MUST contain the
  same set of media samples, with the same timing and metadata.
- Each Partial Segment has a Part Index, which is an integer indicating the
  position of the Partial Segment within its Parent Segment. The first
  Partial Segment has a Part Index of zero.
- Each Partial Segment also has a Media Sequence Number, which is equal to
  the Media Sequence Number of its Parent Segment.

---

## Notes on scope / what is NOT stated in this section

- Section 3 does not state anything about segment-level **encryption**
  (e.g. `EXT-X-KEY`) — encryption is out of scope for this section of the
  spec; do not infer encryption rules from it.
- Section 3 does not itself define the `EXT-X-MAP` tag's own attribute
  syntax (URI/BYTERANGE) — that is Section 4.4.4.5, referenced but not
  reproduced here per the assigned range.
- Section 3 does not name any segment formats beyond the five enumerated in
  3.1.1-3.1.5 (MPEG-2 TS, fMP4, Packed Audio, WebVTT, IMSC Subtitles). No
  other container types are described as "Supported Media Segment Formats"
  in this range.
