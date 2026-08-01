# HLS Playlist Tags — §4.4 Reference

Transcribed from `draft-pantos-hls-rfc8216bis-22` (1 May 2026), section 4.4
"Playlist Tags" in its entirety (§4.4.1 through §4.4.6, including §4.4.5.1.1).
Source: `specs/ietf_draft_pantos_hls_rfc8216bis.txt`, lines 823-3206.

This is a literal transcription for implementation reference. Prose is quoted
or paraphrased only where the spec itself is prose (constraints, MUST/SHOULD
rules); attribute tables are transcribed field-by-field from the spec text.
Where the source text is ambiguous or appears to contain an error, this is
flagged inline with **[SOURCE AMBIGUITY]** rather than silently resolved.

> **Tag count note:** the task brief that requested this document asserts
> "there are 33 tags" across the six subsections listed below. Enumerating the
> tags actually named in the brief's own subsection breakdown, and cross-
> checked against every `#EXT-X-...`/`#EXTM3U`/`#EXTINF` tag defined in §4.4 of
> the source text, yields **32 tags**, not 33. Every tag in the source's
> subsection headings (§4.4.1.1 – §4.4.6.6) is transcribed below; no tag was
> found in the source and omitted. See the "Tag count" section at the end of
> this document.

---

## 4.4.1. Basic Tags (§4.4.1)

> "These tags are allowed in both Media Playlists and Multivariant
> Playlists."

### EXTM3U — §4.4.1.1

**Format:** `#EXTM3U`

- Indicates that the file is an Extended M3U [M3U] Playlist file.
- **MUST** be the first line of every Media Playlist and every Multivariant
  Playlist.
- No attributes.

### EXT-X-VERSION — §4.4.1.2

**Format:** `#EXT-X-VERSION:<n>`

- `n` is an integer indicating the protocol compatibility version number.
- Applies to the entire Playlist file.
- Indicates the compatibility version of the Playlist file, its associated
  media, and its server.
- **MUST** appear in all Playlists containing tags or attributes that are not
  compatible with protocol version 1, to support interoperability with older
  clients. Section 8 specifies the minimum value of the compatibility version
  number for any given Playlist file.
- A Playlist file **MUST NOT** contain more than one `EXT-X-VERSION` tag. If a
  client encounters a Playlist with multiple `EXT-X-VERSION` tags, it **MUST**
  fail to parse it.

---

## 4.4.2. Media or Multivariant Playlist Tags (§4.4.2)

> "The tags in this section can appear in either Multivariant Playlists or
> Media Playlists.
>
> Tags in this section MUST NOT appear more than once in a Playlist. If one
> does, clients MUST fail to parse the Playlist. The only exception to this
> rule is EXT-X-DEFINE, which MAY appear more than once."

### EXT-X-INDEPENDENT-SEGMENTS — §4.4.2.1

**Format:** `#EXT-X-INDEPENDENT-SEGMENTS`

- No attributes.
- Indicates that all media samples in a Media Segment can be decoded without
  information from other segments.
- Applies to every Media Segment in the Playlist.
- If it appears in a Multivariant Playlist, it applies to every Media Segment
  in every Media Playlist in the Multivariant Playlist.
- **MUST NOT** appear more than once in a Playlist; if it does, clients
  **MUST** fail to parse the Playlist.
- If this tag appears in a Multivariant Playlist, it **SHOULD NOT** appear in
  any Media Playlist referenced by that Multivariant Playlist. A tag that
  appears in both **MUST** have the same value; otherwise, clients **SHOULD**
  ignore the value in the Media Playlist(s).

### EXT-X-START — §4.4.2.2

**Format:** `#EXT-X-START:<attribute-list>`

- OPTIONAL tag.
- Indicates a preferred point at which to start playing a Playlist. By
  default, clients **SHOULD** start playback at this point when beginning a
  playback session.
- **MUST NOT** appear more than once in a Playlist; if it does, clients
  **MUST** fail to parse the Playlist.
- If this tag appears in a Multivariant Playlist, it **SHOULD NOT** appear in
  any Media Playlist referenced by that Multivariant Playlist. A tag that
  appears in both **MUST** have the same value; otherwise, clients **SHOULD**
  ignore the value in the Media Playlist(s).

| Attribute   | Value type                       | Required/Optional | Meaning |
|-------------|-----------------------------------|--------------------|---------|
| TIME-OFFSET | signed-decimal-floating-point (seconds) | REQUIRED | A positive number indicates a time offset from the beginning of the Playlist. A negative number indicates a negative time offset from the end of the last Media Segment in the Playlist. The absolute value **SHOULD NOT** be larger than the Playlist duration; if it exceeds the duration it indicates either the end of the Playlist (if positive) or the beginning (if negative). If the Playlist does not contain `EXT-X-ENDLIST`, the TIME-OFFSET **SHOULD NOT** be within three Target Durations of the end of the Playlist file. |
| PRECISE     | enumerated-string (YES / NO)      | OPTIONAL; absence treated as NO | If YES, clients **SHOULD** start playback at the Media Segment containing the TIME-OFFSET, but **SHOULD NOT** render media samples in that segment whose presentation times are prior to the TIME-OFFSET. If NO, clients **SHOULD** attempt to render every media sample in that segment. |

### EXT-X-DEFINE — §4.4.2.3

**Format:** `#EXT-X-DEFINE:<attribute-list>`

- OPTIONAL tag. This is the sole exception to the "no more than once per
  Playlist" rule of §4.4.2 — it MAY appear more than once.

| Attribute  | Value type    | Required/Optional | Meaning |
|------------|---------------|--------------------|---------|
| NAME       | quoted-string | see below | Specifies the Variable Name. All characters in the quoted-string MUST be from the set: `[a-z]`, `[A-Z]`, `[0-9]`, `-`, and `_`. |
| VALUE      | quoted-string | REQUIRED if the tag has a NAME attribute | Specifies the Variable Value. The quoted-string MAY be empty. |
| IMPORT     | quoted-string | see below | Specifies the Variable Name and indicates that its value is that of the variable of the same name in the Multivariant Playlist. Valid character set is the same as NAME. `EXT-X-DEFINE` tags containing IMPORT **MUST NOT** occur in Multivariant Playlists; only allowed in Media Playlists. If the IMPORT value does not match any Variable Name declared in the Multivariant Playlist, or the Media Playlist was not loaded from a Multivariant Playlist, the parser **MUST** fail to parse the Playlist. |
| QUERYPARAM | quoted-string | see below | Specifies the Variable Name and indicates that its value is the value associated with the query parameter of the same name in the URI of the Playlist. Valid character set is the same as NAME. The value associated with the query parameter **MUST** be percent-decoded before performing the variable replacement. The decoded value **MUST NOT** contain any of the characters disallowed in quoted-strings. If the QUERYPARAM value does not match any query parameter in the URI, or the matching parameter has no associated value, the parser **MUST** fail to parse the Playlist. If more than one parameter matches, any of the associated values MAY be used. If the URI is redirected, the client **MUST** look for the query parameter in the 30x response URI. |

- An `EXT-X-DEFINE` tag **MUST** contain either a NAME, an IMPORT, or a
  QUERYPARAM attribute, but only one of the three. Otherwise, the client
  **MUST** fail to parse the Playlist.
- An `EXT-X-DEFINE` tag **MUST NOT** specify the same Variable Name as any
  other `EXT-X-DEFINE` tag in the same Playlist. Parsers that encounter
  duplicate Variable Name declarations **MUST** fail to parse the Playlist.
- Variable Names are case-sensitive.
- An `EXT-X-DEFINE` tag does NOT implicitly persist across Playlist reloads.

---

## 4.4.3. Media Playlist Tags (§4.4.3)

> "Media Playlist tags describe global parameters of the Media Playlist.
> There MUST NOT be more than one Media Playlist tag of each type in any
> Media Playlist.
>
> A Media Playlist tag MUST NOT appear in a Multivariant Playlist."

### EXT-X-TARGETDURATION — §4.4.3.1

**Format:** `#EXT-X-TARGETDURATION:<s>`

- `s` is a decimal-integer indicating the Target Duration in seconds. Its
  value **MUST** be at least 1.
- **REQUIRED** tag.
- Specifies the Target Duration, an upper bound on the duration of all Media
  Segments in the Playlist. The EXTINF duration of each Media Segment in a
  Playlist file, when rounded to the nearest integer, **MUST** be less than or
  equal to the Target Duration. Longer segments can trigger playback stalls
  or other errors.
- Applies to the entire Playlist file.

### EXT-X-MEDIA-SEQUENCE — §4.4.3.2

**Format:** `#EXT-X-MEDIA-SEQUENCE:<number>`

- `number` is a decimal-integer.
- Indicates the Media Sequence Number of the first Media Segment that appears
  in a Playlist file.
- If the Media Playlist file does not contain this tag, the Media Sequence
  Number of the first Media Segment in the Media Playlist **SHALL** be
  considered to be 0.
- A client **MUST NOT** assume that segments with the same Media Sequence
  Number in different Media Playlists contain matching content (see
  Section 6.3.2).
- A URI for a Media Segment is not required to contain its Media Sequence
  Number.
- **MUST** appear before the first Media Segment in the Playlist.
- See Section 6.2.1 and Section 6.3.5 for more information on setting the
  tag.

### EXT-X-DISCONTINUITY-SEQUENCE — §4.4.3.3

**Format:** `#EXT-X-DISCONTINUITY-SEQUENCE:<number>`

- `number` is a decimal-integer.
- Allows synchronization between different Renditions of the same Variant
  Stream or different Variant Streams that have `EXT-X-DISCONTINUITY` tags in
  their Media Playlists.
- If the Media Playlist does not contain this tag, the Discontinuity Sequence
  Number of the first Media Segment in the Playlist **SHALL** be considered
  to be 0.
- **MUST** appear before the first Media Segment in the Playlist.
- **MUST** appear before any `EXT-X-DISCONTINUITY` tag.
- See Section 6.2.1 and Section 6.2.2 for more information about setting the
  value of the tag.

### EXT-X-ENDLIST — §4.4.3.4

**Format:** `#EXT-X-ENDLIST`

- No attributes.
- Indicates that no more Media Segments will be added to the Media Playlist
  file.
- MAY occur anywhere in the Media Playlist file.

### EXT-X-PLAYLIST-TYPE — §4.4.3.5

**Format:** `#EXT-X-PLAYLIST-TYPE:<type-enum>`

- `type-enum` is either `EVENT` or `VOD`.
- OPTIONAL tag.
- Provides mutability information about the Media Playlist file. Applies to
  the entire Media Playlist file.
- Section 6.2.1 defines the implications of the tag.
- If the value is `EVENT`, Media Segments can only be added to the end of the
  Media Playlist.
- If the value is `VOD` (Video On Demand), the Media Playlist cannot change.
- If the tag is omitted from a Media Playlist, the Playlist can be updated
  according to the rules in Section 6.2.1 with no additional restrictions.
  For example, a live Playlist (Section 6.2.2) MAY be updated to remove Media
  Segments in the order that they appeared.

### EXT-X-I-FRAMES-ONLY — §4.4.3.6

**Format:** `#EXT-X-I-FRAMES-ONLY`

- No attributes.
- Indicates that each Media Segment in the Playlist describes a single
  I-frame. I-frames are encoded video frames whose decoding does not depend
  on any other frame. I-frame Playlists can be used for trick play, such as
  fast forward, rapid reverse, and scrubbing.
- Applies to the entire Playlist.
- In a Playlist with this tag, the Media Segment duration (EXTINF tag value)
  is the time between the presentation time of the I-frame in the Media
  Segment and the presentation time of the next I-frame in the Playlist, or
  the end of the presentation if it is the last I-frame in the Playlist.
- Media resources containing I-frame segments **MUST** begin with either a
  Media Initialization Section (Section 3) or be accompanied by an
  `EXT-X-MAP` tag indicating the Media Initialization Section so that clients
  can load and decode I-frame segments in any order. The byte range of an
  I-frame segment with an `EXT-X-BYTERANGE` tag applied to it (§4.4.4.2)
  **MUST NOT** include its Media Initialization Section; clients can assume
  that the Media Initialization Section is defined by the `EXT-X-MAP` tag, or
  is located between the start of the resource and the offset of the first
  I-frame segment in that resource.
- **Version requirement:** REQUIRES a compatibility version number of 4 or
  greater.

### EXT-X-PART-INF — §4.4.3.7

**Format:** `#EXT-X-PART-INF:<attribute-list>`

- Provides information about the Partial Segments in the Playlist.
- **REQUIRED** if a Playlist contains one or more `EXT-X-PART` tags.

| Attribute   | Value type                          | Required/Optional | Meaning |
|-------------|--------------------------------------|--------------------|---------|
| PART-TARGET | decimal-floating-point (seconds)     | REQUIRED | Indicates the Part Target Duration. |

### EXT-X-SERVER-CONTROL — §4.4.3.8

**Format:** `#EXT-X-SERVER-CONTROL:<attribute-list>`

- Allows the Server to indicate support for Delivery Directives
  (Section 6.2.5).

| Attribute            | Value type                       | Required/Optional | Meaning |
|----------------------|-----------------------------------|--------------------|---------|
| CAN-SKIP-UNTIL       | decimal-floating-point (seconds, the "Skip Boundary") | OPTIONAL; MAY appear in any Media Playlist | Indicates that the Server can produce Playlist Delta Updates (§6.2.5.1) in response to the `_HLS_skip` Delivery Directive. The Skip Boundary **MUST** be at least six times the Target Duration. |
| CAN-SKIP-DATERANGES  | enumerated-string (YES)           | OPTIONAL; REQUIRES the presence of CAN-SKIP-UNTIL | YES if the Server can produce Playlist Delta Updates (§6.2.5.1) that skip older `EXT-X-DATERANGE` tags in addition to Media Segments. |
| HOLD-BACK            | decimal-floating-point (seconds)  | OPTIONAL; absence implies three times the Target Duration; MAY appear in any Media Playlist | Server-recommended minimum distance from the end of the Playlist at which clients should begin to play or to which they should seek, unless PART-HOLD-BACK applies. Value **MUST** be at least three times the Target Duration. |
| PART-HOLD-BACK       | decimal-floating-point (seconds)  | REQUIRED if the Playlist contains the `EXT-X-PART-INF` tag | Server-recommended minimum distance from the end of the Playlist at which clients should begin to play or to which they should seek when playing in Low-Latency Mode. Value **MUST** be at least twice the Part Target Duration; value **SHOULD** be at least three times the Part Target Duration. If different Renditions have different Part Target Durations then PART-HOLD-BACK **SHOULD** be at least three times the maximum Part Target Duration. |
| CAN-BLOCK-RELOAD     | enumerated-string (YES)           | OPTIONAL; absence implies no support | YES if the server supports Blocking Playlist Reload (§6.2.5.2). |

- The spec adds a design-rationale paragraph (not a normative constraint):
  Target Duration and hold-back parameters (including Part Target Duration
  and PART-HOLD-BACK) have a significant impact on latency and bit rate
  adaptation. A shorter Target Duration reduces latency but also reduces
  available buffer, handicaps adaption, and increases delivery overhead,
  increasing the likelihood of playback stall. A longer Hold Back can
  mitigate the playback stall likelihood but increases latency. A longer
  Target Duration improves other factors at the cost of increased latency.
  Content producers should choose values for the Target Duration and Hold
  Back parameters that balance the competing demands of delivery overhead,
  latency, and playback reliability according to their own criteria.

---

## 4.4.4. Media Segment Tags (§4.4.4)

> "Each Media Segment is specified by a series of Media Segment tags
> followed by a URI. Some Media Segment tags apply to just the next segment;
> others apply to all subsequent segments until another instance of the same
> tag.
>
> A Media Segment tag MUST NOT appear in a Multivariant Playlist. Clients
> MUST fail to parse Playlists that contain both Media Segment tags and
> Multivariant Playlist tags (Section 4.4.6)."

### EXTINF — §4.4.4.1

**Format:** `#EXTINF:<duration>,[<title>]`

- Applies only to the next Media Segment. **REQUIRED** for each Media
  Segment.
- `duration` is a decimal-floating-point or decimal-integer number (as
  described in Section 4.2) that specifies the duration of the Media Segment
  in seconds.
- `title` is the remainder of the line following the comma — an optional
  human-readable informative title of the Media Segment expressed as UTF-8
  text. If title is zero length or entirely whitespace, the title is
  considered to be missing.
- Durations **SHOULD** be decimal-floating-point, with enough accuracy to
  avoid perceptible error when segment durations are accumulated. However, if
  the compatibility version number is less than 3, durations **MUST** be
  integers. Durations that are reported as integers **SHOULD** be rounded to
  the nearest integer.

### EXT-X-BYTERANGE — §4.4.4.2

**Format:** `#EXT-X-BYTERANGE:<n>[@<o>]`

- Applies only to the next URI line that follows it in the Playlist.
- `n` is a decimal-integer indicating the length of the sub-range in bytes.
- `o`, if present, is a decimal-integer indicating the start of the sub-range
  as a byte offset from the beginning of the resource. If `o` is not present,
  the sub-range begins at the next byte following the sub-range of the
  previous Media Segment.
- If `o` is not present, a previous Media Segment **MUST** appear in the
  Playlist file and **MUST** be a sub-range of the same media resource, or
  the Media Segment is undefined and the client **MUST** fail to parse the
  Playlist.
- Indicates that a Media Segment is a sub-range of the resource identified by
  its URI. A Media Segment without this tag consists of the entire resource
  identified by its URI.
- **Version requirement:** REQUIRES a compatibility version number of 4 or
  greater.

### EXT-X-DISCONTINUITY — §4.4.4.3

**Format:** `#EXT-X-DISCONTINUITY`

- No attributes.
- Indicates a discontinuity between the Media Segment that follows it and the
  one that preceded it.
- **MUST** be present if there is a change in any of:
  - file format
  - number, type, and identifiers of tracks
  - timestamp sequence
- **SHOULD** be present if there is a change in any of:
  - encoding parameters
  - encoding sequence
- See Section 3, Section 6.2.1, and Section 6.3.3 for more information.

### EXT-X-KEY — §4.4.4.4

**Format:** `#EXT-X-KEY:<attribute-list>`

- Media Segments MAY be encrypted; this tag specifies how to decrypt them.
- Applies to every Media Segment and to every Media Initialization Section
  declared by an `EXT-X-MAP` tag that appears between it and the next
  `EXT-X-KEY` tag in the Playlist file with the same KEYFORMAT attribute or a
  METHOD of NONE (or the end of the Playlist file). Any Media Segment or
  Media Initialization Section that precedes the first `EXT-X-KEY` tag is
  unencrypted. Two or more `EXT-X-KEY` tags with different KEYFORMAT
  attributes MAY apply to the same Media Segment if they ultimately produce
  the same decryption key.
- If the Media Playlist file does not contain an `EXT-X-KEY` tag, then Media
  Segments are not encrypted.
- See Section 5 for the format of the Key file, and Section 5.2,
  Section 6.2.3, and Section 6.3.6 for additional information on Media
  Segment encryption.

| Attribute | Value type | Required/Optional | Meaning |
|-----------|------------|--------------------|---------|
| METHOD | enumerated-string | REQUIRED | Specifies the encryption method. The required methods are: `NONE`, `AES-128`, and `SAMPLE-AES`. Clients MAY additionally support `SAMPLE-AES-CTR` and `AES-256-GCM`. See method semantics below. |
| URI | quoted-string | REQUIRED unless METHOD is NONE | A URI that specifies how to obtain the key. |
| IV | hexadecimal-sequence | OPTIONAL; may be disallowed depending on METHOD | Specifies a 128-bit unsigned integer Initialization Vector to be used with the key. **Version requirement:** REQUIRES a compatibility version number of 2 or greater. See Section 5.2 for when the IV attribute is used. |
| KEYFORMAT | quoted-string | OPTIONAL; absence implies "identity" | Specifies how the key is represented in the resource identified by the URI; see Section 5. **Version requirement:** REQUIRES a compatibility version number of 5 or greater. |
| KEYFORMATVERSIONS | quoted-string | OPTIONAL; absence implies "1" | One or more positive integers separated by `/` (e.g., "1", "1/2", "1/2/5"), indicating which version(s) of KEYFORMAT this instance complies with. **Version requirement:** REQUIRES a compatibility version number of 5 or greater. |

METHOD semantics (transcribed in full, as precision matters):

- **NONE** — Media Segments are not encrypted. If the encryption method is
  NONE, other attributes **MUST NOT** be present.
- **AES-128** — Media Segments are completely encrypted using the Advanced
  Encryption Standard (AES) [AES] with a 128-bit key, Cipher Block Chaining
  (CBC), and Public-Key Cryptography Standards #7 (PKCS7) padding [RFC5652].
  CBC is restarted on each segment boundary, using either the Initialization
  Vector (IV) attribute value or the Media Sequence Number as the IV; see
  Section 5.2.
- **AES-256-GCM** — Media Segments are completely encrypted using AES [AES]
  with a 256-bit key and Galois/Counter Mode (GCM) [AES_GCM]. This mode uses
  a 128-bit IV, and produces a 128-bit GCM authentication tag in addition to
  the cipher-text. GCM is restarted on each segment boundary. Each encrypted
  segment starts with the 16-octet IV, followed by the AES cipher-text, and
  ending with the 16-octet GCM authentication tag. If an `EXT-X-KEY` tag uses
  the AES-256-GCM encryption method then it **MUST NOT** have an IV
  attribute.
- **SAMPLE-AES** — an alternative to whole-segment encryption is Sample
  Encryption: only media sample data (such as audio packets or video frames)
  is encrypted; the rest of the Media Segment is unencrypted. Sample
  Encryption allows parts of the Segment to be processed without (or before)
  decrypting the media itself. Media Segments are Sample Encrypted using AES
  [AES]. How these media streams are encrypted and encapsulated in a segment
  depends on the media encoding and the media format of the segment. fMP4
  Media Segments are encrypted using the 'cbcs' scheme of Common Encryption
  [COMMON_ENC]. Encryption of other Media Segment formats containing H.264
  [H_264], AAC [ISO_14496], AC-3 [AC_3], and Enhanced AC-3 [AC_3] media
  streams is described in the HTTP Live Streaming (HLS) Sample Encryption
  specification [SampleEnc]. The IV attribute MAY be present; see Section 5.2.
- **SAMPLE-AES-CTR** — similar to SAMPLE-AES. However, fMP4 Media Segments
  are encrypted using the 'cenc' scheme of Common Encryption [COMMON_ENC].
  Encryption of other Media Segment formats is not defined for
  SAMPLE-AES-CTR. The IV attribute **MUST NOT** be present.

### EXT-X-MAP — §4.4.4.5

**Format:** `#EXT-X-MAP:<attribute-list>`

- Specifies how to obtain the Media Initialization Section (Section 3)
  required to parse the applicable Media Segments.
- Applies to every Media Segment that appears after it in the Playlist until
  the next `EXT-X-MAP` tag or until the end of the Playlist.

| Attribute | Value type | Required/Optional | Meaning |
|-----------|------------|--------------------|---------|
| URI | quoted-string | REQUIRED | A URI that identifies a resource that contains the Media Initialization Section. |
| BYTERANGE | quoted-string | OPTIONAL; absence implies the byte range is the entire resource indicated by URI | Specifies a byte range into the resource identified by URI. This range **SHOULD** contain only the Media Initialization Section. Format is similar to §4.4.4.2, however offset is REQUIRED: `"<n>@<o>"`. |

- An `EXT-X-MAP` tag **SHOULD** be supplied for Media Segments in Playlists
  with the `EXT-X-I-FRAMES-ONLY` tag when the first Media Segment (i.e.,
  I-frame) in the Playlist (or the first segment following an
  `EXT-X-DISCONTINUITY` tag) does not immediately follow the Media
  Initialization Section at the beginning of its resource.
- **Version requirement:** use of `EXT-X-MAP` in a Media Playlist that
  contains `EXT-X-I-FRAMES-ONLY` REQUIRES a compatibility version number of 5
  or greater. Use of `EXT-X-MAP` in a Media Playlist that DOES NOT contain
  `EXT-X-I-FRAMES-ONLY` REQUIRES a compatibility version number of 6 or
  greater.
- If the Media Initialization Section declared by an `EXT-X-MAP` tag is
  encrypted with a METHOD of AES-128, the IV attribute of the `EXT-X-KEY` tag
  that applies to the `EXT-X-MAP` is REQUIRED.

### EXT-X-PROGRAM-DATE-TIME — §4.4.4.6

**Format:** `#EXT-X-PROGRAM-DATE-TIME:<date-time-msec>`

- Applies only to the next Media Segment.
- `date-time-msec` is an ISO/IEC 8601 [ISO_8601] date/time representation,
  such as `YYYY-MM-DDThh:mm:ss.SSSZ`. It **SHOULD** indicate a time zone and
  fractional parts of seconds, to at least millisecond accuracy. If no time
  zone is indicated, the client **SHOULD** treat the time zone as UTC.
- Associates the first sample of a Media Segment with an absolute date
  and/or time.
- Example given in source: `#EXT-X-PROGRAM-DATE-TIME:2010-02-19T14:54:23.031+08:00`
- See Section 6.2.1 and Section 6.3.3 for more information.

### EXT-X-GAP — §4.4.4.7

**Format:** `#EXT-X-GAP`

- No attributes.
- Applies only to the next Media Segment.
- Indicates that the segment URI to which it applies does not contain media
  data and **SHOULD NOT** be loaded by clients.
- See Section 6.2.1 and Section 6.3.3 for more information.

### EXT-X-BITRATE — §4.4.4.8

**Format:** `#EXT-X-BITRATE:<rate>`

- `rate` is a decimal-integer of kilobits per second.
- OPTIONAL tag.
- Identifies the approximate segment bit rate of the Media Segment(s) to
  which it applies. Applies to every Media Segment between it and the next
  `EXT-X-BITRATE` tag in the Playlist file (or the end of the Playlist file)
  that does not have an `EXT-X-BYTERANGE` tag applied to it.
- If present, its value **MUST** be no less than 90% of the segment bit rate
  of each Media Segment to which it is applied and no greater than 110% of
  the segment bit rate of each Media Segment to which it is applied.

### EXT-X-PART — §4.4.4.9

**Format:** `#EXT-X-PART:<attribute-list>`

- OPTIONAL tag. Identifies a Partial Segment.

| Attribute   | Value type            | Required/Optional | Meaning |
|-------------|------------------------|--------------------|---------|
| URI         | quoted-string          | REQUIRED | The URI for the Partial Segment. |
| DURATION    | decimal-floating-point (seconds) | REQUIRED | The duration of the Partial Segment. |
| INDEPENDENT | enumerated-string (YES) | OPTIONAL | YES if the Partial Segment contains an independent frame. Every Partial Segment containing an independent frame **SHOULD** carry it, to increase the efficiency with which clients can join and switch Renditions. |
| BYTERANGE   | quoted-string          | OPTIONAL | Indicates that the Partial Segment is a sub-range of the resource specified by URI. Same format as `EXT-X-BYTERANGE`: `"<n>[@<o>]"`. If `o` is not present, the sub-range begins at the next byte following the sub-range of the previous Partial Segment belonging to the same Parent Segment. |
| GAP         | enumerated-string (YES) | REQUIRED for such Partial Segments (i.e., conditionally required) | YES if the Partial Segment is not available. |

- When the Media Segment Tags (§4.4.4) `EXT-X-DISCONTINUITY`, `EXT-X-KEY`,
  `EXT-X-MAP`, or `EXT-X-PROGRAM-DATE-TIME` are applied to a Parent Segment,
  they also necessarily apply to the first Partial Segment, so they **MUST**
  appear before the first `EXT-X-PART` tag of that Parent Segment.
- The duration of a Partial Segment **MUST** be less than or equal to the
  Part Target Duration. The duration of each Partial Segment **MUST** be at
  least 85% of the Part Target Duration, with the exception of Partial
  Segments with the INDEPENDENT=YES or GAP=YES attribute, Partial Segments
  that are immediately followed by a Partial Segment with a GAP=YES
  attribute, and the final Partial Segment of any Parent Segment.
- Playlists that contain the `EXT-X-I-FRAMES-ONLY` tag **SHOULD NOT** use
  Partial Segments.

---

## 4.4.5. Media Metadata Tags (§4.4.5)

> "Media Metadata tags provide information about the playlist that is not
> associated with specific Media Segments. There MAY be more than one Media
> Metadata tag of each type in any Media Playlist. The only exception to this
> rule is an EXT-X-SKIP, which MUST NOT appear more than once."

### EXT-X-DATERANGE — §4.4.5.1

**Format:** `#EXT-X-DATERANGE:<attribute-list>`

- Associates a Date Range (i.e., a range of time defined by a starting and
  ending date) with a set of attribute/value pairs.

| Attribute | Value type | Required/Optional | Meaning |
|-----------|-------------|--------------------|---------|
| ID | quoted-string | REQUIRED | Uniquely identifies a Date Range in the Playlist. |
| CLASS | quoted-string | OPTIONAL | Specifies some set of attributes and their associated value semantics. All Date Ranges with the same CLASS attribute value **MUST** adhere to these semantics. |
| START-DATE | quoted-string, ISO/IEC 8601 [ISO_8601] date/time | REQUIRED, unless the `EXT-X-DATERANGE` tag is preceded by another `EXT-X-DATERANGE` tag in the same Playlist with the same ID attribute and a START-DATE attribute | The date/time at which the Date Range begins. |
| CUE | enumerated-string-list of Trigger Identifiers | OPTIONAL | Collectively indicates when to trigger an action associated with the Date Range. The time to trigger the action MAY be at a point of playback outside the Date Range expressed by the START-DATE and duration. Defined Trigger Identifiers: PRE, POST, ONCE. |
| END-DATE | quoted-string, ISO/IEC 8601 [ISO_8601] date/time | OPTIONAL | The date/time at which the Date Range ends. **MUST** be equal to or later than the value of START-DATE. |
| DURATION | decimal-floating-point (seconds) | OPTIONAL | The duration of the Date Range. **MUST NOT** be negative. A single instant in time (e.g., crossing a finish line) **SHOULD** be represented with a duration of 0. |
| PLANNED-DURATION | decimal-floating-point (seconds) | OPTIONAL | The expected duration of the Date Range. **MUST NOT** be negative. **SHOULD** be used to indicate the expected duration of a Date Range whose actual duration is not yet known. |
| X-\<extension-attribute\> | quoted-string, hexadecimal-sequence, or signed-decimal-floating-point | OPTIONAL, however a particular CLASS MAY treat specific X- attributes as required | The "X-" prefix is a namespace for attributes defined outside the core HLS specification. Attribute name MUST be a legal AttributeName. Reverse-DNS syntax SHOULD be used to avoid collisions. Whoever defines the attribute MAY specify that the value is to be interpreted in a more restricted way (e.g., a quoted-string as an enumerated-string-list, or that a signed-decimal-floating-point value be positive). Example: `X-COM-EXAMPLE-AD-ID="XYZ123"`. When a particular CLASS defines an X- attribute, that definition is specific to that class; a different CLASS MAY define the same X- attribute with different semantics, though consistency across CLASS definitions is preferred. |
| SCTE35-CMD, SCTE35-OUT, SCTE35-IN | (see §4.4.5.1.1) | OPTIONAL | Used to carry SCTE-35 data; see §4.4.5.1.1. |
| END-ON-NEXT | enumerated-string, value MUST be YES | OPTIONAL | Indicates that the end of the range containing it is equal to the START-DATE of its Following Range. The Following Range is the Date Range of the same CLASS that has the earliest START-DATE after the START-DATE of the range in question. |

Additional constraints (prose, transcribed in full):

- A CUE attribute containing PRE indicates that an action is to be triggered
  before playback of the primary asset begins, regardless of where playback
  begins in the primary asset.
- A CUE attribute containing POST indicates that an action is to be triggered
  after the primary asset has been played to its end without error.
- The presence of a CUE attribute that contains ONCE indicates that an action
  is to be triggered once. It **SHOULD NOT** be triggered again, even if the
  user replays the portion of the primary asset that includes the trigger
  point.
- A CUE attribute **MUST NOT** include both PRE and POST.
- An `EXT-X-DATERANGE` tag with an END-ON-NEXT=YES attribute **MUST** have a
  CLASS attribute. Other `EXT-X-DATERANGE` tags with the same CLASS attribute
  **MUST NOT** specify Date Ranges that overlap.
- An `EXT-X-DATERANGE` tag with an END-ON-NEXT=YES attribute **MUST NOT**
  contain DURATION or END-DATE attributes.
- Any `EXT-X-DATERANGE` attribute whose value is an ISO/IEC 8601 date
  **SHOULD** indicate a time zone and fractional parts of seconds, to at
  least millisecond accuracy. If no time zone is indicated, the client
  **SHOULD** treat the time zone as UTC.
- A Date Range with neither a DURATION, an END-DATE, nor an END-ON-NEXT=YES
  attribute has an unknown duration, even if it has a PLANNED-DURATION.
- If a Playlist contains an `EXT-X-DATERANGE` tag, it **MUST** also contain
  at least one `EXT-X-PROGRAM-DATE-TIME` tag.
- A Server MAY augment a Date Range with additional attributes by adding
  subsequent `EXT-X-DATERANGE` tags with the same ID attribute to a Playlist.
  The ID attribute **MUST** be present in every `EXT-X-DATERANGE` tag, but any
  other required attribute MAY be omitted if it is present in the first
  `EXT-X-DATERANGE` tag with that ID. The client is responsible for
  consolidating the tags. The subsequent `EXT-X-DATERANGE` tags can appear in
  a subsequent playlist update, in the case of live or event streams. If a
  Playlist contains two `EXT-X-DATERANGE` tags with the same ID attribute
  value, then any AttributeName that appears in both tags **MUST** have the
  same AttributeValue.
- If a Date Range contains both a DURATION attribute and an END-DATE
  attribute, the value of the END-DATE attribute **MUST** be equal to the
  value of the START-DATE attribute plus the value of the DURATION attribute.
- Clients **SHOULD** ignore `EXT-X-DATERANGE` tags with illegal syntax.

#### 4.4.5.1.1. Mapping SCTE-35 into EXT-X-DATERANGE

> Full transcription of §4.4.5.1.1 (this is a substantive sub-spec, not a
> footnote).

Splice information carried in source media according to the SCTE-35
specification [SCTE35] MAY be represented in a Media Playlist using
`EXT-X-DATERANGE` tags.

- Each SCTE-35 `splice_info_section()` containing a `splice_null()`,
  `splice_schedule()`, `bandwidth_reservation()`, or `private_cmd()`
  **SHOULD** be represented by an `EXT-X-DATERANGE` tag with an SCTE35-CMD
  attribute whose value is the big-endian binary representation of the
  `splice_info_section()`, expressed as a hexadecimal-sequence.

- An SCTE-35 splice out/in pair signaled by a pair of `splice_insert()`
  commands **SHOULD** be represented by one or more `EXT-X-DATERANGE` tags
  carrying the same ID attribute, which **MUST** be unique to that splice
  out/in pair. The "out" `splice_info_section()` (with
  `out_of_network_indicator` set to 1) **MUST** be placed in an SCTE35-OUT
  attribute, with the same formatting as SCTE35-CMD. The "in"
  `splice_info_section()` (with `out_of_network_indicator` set to 0) **MUST**
  be placed in an SCTE35-IN attribute, with the same formatting as
  SCTE35-CMD.

- An SCTE-35 splice out/in pair signaled by a pair of `time_signal()`
  commands, each carrying a single `segmentation_descriptor()`, **SHOULD** be
  represented by one or more `EXT-X-DATERANGE` tags carrying the same ID
  attribute, which **MUST** be unique to that splice out/in pair. The "out"
  `splice_info_section()` **MUST** be placed in an SCTE35-OUT attribute; the
  "in" `splice_info_section()` **MUST** be placed in an SCTE35-IN attribute.

- Each `EXT-X-DATERANGE` tag with the same ID as an earlier `EXT-X-DATERANGE`
  tag can introduce new attributes but cannot change (or lose) attributes of
  the existing Date Range. When a SCTE-35 splice out/in pair is represented
  by more than one `EXT-X-DATERANGE` tag, each tag **MUST** have the same ID,
  the first occurrence **MUST** contain a START-DATE attribute (the program
  time of the splice), and a subsequent occurrence **MUST** specify the
  program duration of the splice by adding a DURATION or END-DATE attribute,
  as described below.

- Different types of segmentation, as indicated by the
  `segmentation_type_id` in the `segmentation_descriptor()`, **SHOULD** be
  represented by separate `EXT-X-DATERANGE` tags, even if two or more
  `segmentation_descriptor()`s arrive in the same `splice_info_section()`. In
  that case, each `EXT-X-DATERANGE` tag will have an SCTE35-OUT, SCTE35-IN,
  or SCTE35-CMD attribute whose value is the entire `splice_info_section()`.

- An SCTE-35 `time_signal()` command that does not signal a splice out or in
  point **SHOULD** be represented by an `EXT-X-DATERANGE` tag with an
  SCTE35-CMD attribute.

- The START-DATE of an `EXT-X-DATERANGE` tag containing an SCTE35-OUT
  attribute **MUST** be the date and time that corresponds to the program
  time of that splice.

- The START-DATE of an `EXT-X-DATERANGE` tag containing an SCTE35-CMD
  **MUST** be the date and time specified by the `splice_time()` in the
  command or the program time at which the command appeared in the source
  stream if the command does not specify a `splice_time()`.

- An `EXT-X-DATERANGE` tag containing an SCTE35-OUT attribute MAY contain a
  PLANNED-DURATION attribute. Its value **MUST** be the planned duration of
  the splice.

- The DURATION of an `EXT-X-DATERANGE` tag containing an SCTE35-IN attribute
  **MUST** be the actual (not planned) program duration between the
  corresponding out-point and that in-point.

- The END-DATE of an `EXT-X-DATERANGE` tag containing an SCTE35-IN attribute
  **MUST** be the actual (not planned) program date and time of that
  in-point.

- If the actual end date and time is not known when an SCTE35-OUT attribute
  is added to the Playlist, the DURATION attribute and the END-TIME attribute
  **MUST NOT** be present; the actual end date of the splice **SHOULD** be
  signaled by another `EXT-X-DATERANGE` tag once it has been established.

  > **[SOURCE AMBIGUITY]** The source text says "the DURATION attribute and
  > the END-TIME attribute MUST NOT be present" — but `EXT-X-DATERANGE` never
  > defines an attribute named `END-TIME` anywhere else in §4.4.5.1; the
  > attribute defined for range end is `END-DATE`. This appears to be a name
  > inconsistency in the source draft itself (quoting verbatim rather than
  > silently correcting to `END-DATE`).

- A canceled splice **SHOULD NOT** appear in the Playlist as an
  `EXT-X-DATERANGE` tag.

- An `EXT-X-DATERANGE` tag announcing a splice **SHOULD** be added to a
  Playlist at the same time as the last pre-splice Media Segment, or earlier
  if possible.

- The ID attribute of an `EXT-X-DATERANGE` tag MAY contain a
  `splice_event_id` and/or a `segmentation_event_id`, but it **MUST** be
  unique in the Playlist. If there is a possibility that an SCTE-35 id will
  be reused, the ID attribute value **MUST** include disambiguation, such as
  a date or sequence number.

### EXT-X-SKIP — §4.4.5.2

**Format:** `#EXT-X-SKIP:<attribute-list>`

- A server produces a Playlist Delta Update (Section 6.2.5.1), by replacing
  tags earlier than the Skip Boundary with an `EXT-X-SKIP` tag.
- When replacing Media Segments, the `EXT-X-SKIP` tag replaces the segment
  URI lines and all Media Segment Tags that are applied to those segments.
- This tag **MUST NOT** appear more than once in a Playlist (the sole
  exception noted in the §4.4.5 preamble).

| Attribute | Value type | Required/Optional | Meaning |
|-----------|-------------|--------------------|---------|
| SKIPPED-SEGMENTS | decimal-integer | REQUIRED | The number of Media Segments replaced by the `EXT-X-SKIP` tag. Replacing segments with the `EXT-X-SKIP` tag does not change the value of the `EXT-X-DISCONTINUITY-SEQUENCE` tag. |
| RECENTLY-REMOVED-DATERANGES | quoted-string, tab (0x9) delimited list of `EXT-X-DATERANGE` IDs | REQUIRED if the Client requested an update that skips `EXT-X-DATERANGE` tags | A list of `EXT-X-DATERANGE` IDs that have been removed from the Playlist recently. See Section 6.2.5.1 for more information. The quoted-string MAY be empty. |

### EXT-X-PRELOAD-HINT — §4.4.5.3

**Format:** `#EXT-X-PRELOAD-HINT:<attribute-list>`

- Allows a Client loading media from a live stream to reduce the time to
  obtain a resource from the Server by issuing its request before the
  resource is available to be delivered. The server will hold onto the
  request ("block") until it can respond.

| Attribute | Value type | Required/Optional | Meaning |
|-----------|-------------|--------------------|---------|
| TYPE | enumerated-string | REQUIRED | Specifies the type of the hinted resource. If the value is PART, the resource is a Partial Segment. If the value is MAP, the resource is a Media Initialization Section. |
| URI | quoted-string | REQUIRED | A URI identifying the hinted resource. **MUST** match the URI that will be subsequently added to the Playlist as a non-hinted resource (for example, the URI of an `EXT-X-PART` tag). The URI MAY be relative to the URI of the Playlist or it MAY be absolute. The hostname MAY differ from the hostname of the Playlist URI. |
| BYTERANGE-START | decimal-integer | OPTIONAL; absence implies a value of 0 | The byte offset of the first byte of the hinted resource, from the beginning of the resource identified by the URI attribute. |
| BYTERANGE-LENGTH | decimal-integer | OPTIONAL; absence indicates that the last byte of the hinted resource is the last byte of the resource identified by the URI attribute | The length of the hinted resource. In the absent case, you **SHOULD** use the recommended last-byte-pos [RFC8673] value of 2^53-1 (9007199254740991) in the HTTP Range request. |

- Note that when a hinted Partial Segment eventually appears in the Playlist
  as an `EXT-X-PART` tag, it MAY have a different Discontinuity Sequence
  Number, Media Initialization Section, or encryption configuration. In other
  words, the Partial Segment can be preceded by an EXTINF tag indicating the
  end of the previous Parent Segment and an `EXT-X-DISCONTINUITY`,
  `EXT-X-MAP`, or `EXT-X-KEY` tag.
- A Playlist containing an `EXT-X-ENDLIST` tag **MUST NOT** contain an
  `EXT-X-PRELOAD-HINT` tag.

### EXT-X-RENDITION-REPORT — §4.4.5.4

**Format:** `#EXT-X-RENDITION-REPORT:<attribute-list>`

- Carries information about an associated Rendition that is as up-to-date as
  the Playlist that contains it.

| Attribute | Value type | Required/Optional | Meaning |
|-----------|-------------|--------------------|---------|
| URI | quoted-string | REQUIRED | The URI for the Media Playlist of the specified Rendition. **MUST** be relative to the URI of the Media Playlist containing the `EXT-X-RENDITION-REPORT` tag. |
| LAST-MSN | decimal-integer | REQUIRED | The Media Sequence Number of the last Media Segment currently in the specified Rendition. If the Rendition contains Partial Segments then this value is the Media Sequence Number of the last Partial Segment. |
| LAST-PART | decimal-integer | REQUIRED if the Rendition contains a Partial Segment | The Part Index of the last Partial Segment currently in the specified Rendition whose Media Sequence Number is equal to the LAST-MSN attribute value. |

- A server MAY omit adding an attribute to an `EXT-X-RENDITION-REPORT` tag —
  even a mandatory attribute — if its value is the same as that of the
  Rendition Report of the Media Playlist to which the `EXT-X-RENDITION-REPORT`
  tag is being added. Doing so reduces the size of the Rendition Report.

---

## 4.4.6. Multivariant Playlist Tags (§4.4.6)

> "Multivariant Playlist tags define the Variant Streams, Renditions, and
> other global parameters of the presentation.
>
> Multivariant Playlist tags MUST NOT appear in a Media Playlist; clients
> MUST fail to parse any Playlist that contains both a Multivariant Playlist
> tag and either a Media Playlist tag or a Media Segment tag."

### EXT-X-MEDIA — §4.4.6.1

**Format:** `#EXT-X-MEDIA:<attribute-list>`

- Used to relate Media Playlists that contain alternative Renditions
  (§4.4.6.2.1) of the same content. For example, three `EXT-X-MEDIA` tags can
  be used to identify audio-only Media Playlists that contain English,
  French, and Spanish Renditions of the same presentation. Or, two
  `EXT-X-MEDIA` tags can be used to identify video-only Media Playlists that
  show two different camera angles.

| Attribute | Value type | Required/Optional | Meaning |
|-----------|-------------|--------------------|---------|
| TYPE | enumerated-string (AUDIO, VIDEO, SUBTITLES, CLOSED-CAPTIONS) | REQUIRED | Typically, closed-caption [CEA608] media is carried in the video stream. Therefore, an `EXT-X-MEDIA` tag with TYPE of CLOSED-CAPTIONS does not specify a Rendition; the closed-caption media is present in the Media Segments of every video Rendition. |
| URI | quoted-string | OPTIONAL (see §4.4.6.2.1); MUST NOT be present if TYPE is CLOSED-CAPTIONS | A URI that identifies the Media Playlist file. |
| GROUP-ID | quoted-string | REQUIRED | Specifies the group to which the Rendition belongs. See §4.4.6.1.1. |
| LANGUAGE | quoted-string, a language tag [RFC5646] | OPTIONAL | Identifies the primary language used in the Rendition. |
| ASSOC-LANGUAGE | quoted-string, a language tag [RFC5646] | OPTIONAL | Identifies a language that is associated with the Rendition. An associated language is often used in a different role than the language specified by LANGUAGE (e.g., written versus spoken, or a fallback dialect). |
| NAME | quoted-string | REQUIRED | A human-readable description of the Rendition. If LANGUAGE is present, this description **SHOULD** be in that language. See Appendix E. |
| STABLE-RENDITION-ID | quoted-string | OPTIONAL | A stable identifier for the URI within the Multivariant Playlist. All characters **MUST** be from the set `[a-z]`, `[A-Z]`, `[0-9]`, `+`, `/`, `=`, `.`, `-`, `_`. Allows the URI of a Rendition to change between two distinct downloads of the Multivariant Playlist. IDs are matched using a byte-for-byte comparison. All `EXT-X-MEDIA` tags in a Multivariant Playlist with the same URI value **SHOULD** use the same STABLE-RENDITION-ID. |
| DEFAULT | enumerated-string (YES, NO) | OPTIONAL; absence implies NO | If YES, the client **SHOULD** play this Rendition of the content in the absence of information from the user indicating a different choice. |
| AUTOSELECT | enumerated-string (YES, NO) | OPTIONAL; absence implies NO | If YES, the client MAY choose to play this Rendition in the absence of explicit user preference because it matches the current playback environment, such as chosen system language. If the AUTOSELECT attribute is present, its value **MUST** be YES if the value of DEFAULT is YES. |
| FORCED | enumerated-string (YES, NO) | OPTIONAL; absence implies NO; **MUST NOT** be present unless TYPE is SUBTITLES | YES indicates that the Rendition contains content that is considered essential to play. When selecting a FORCED Rendition, a client **SHOULD** choose the one that best matches the current playback environment (e.g., language). NO indicates that the Rendition contains content that is intended to be played in response to explicit user request. |
| INSTREAM-ID | quoted-string | REQUIRED if TYPE is CLOSED-CAPTIONS; OPTIONAL for all other TYPE values | Specifies a Rendition within the segments in the Media Playlist. For CLOSED-CAPTIONS, **MUST** have one of the values "CC1", "CC2", "CC3", "CC4", or "SERVICEn" where n **MUST** be an integer between 1 and 63 (e.g., "SERVICE9" or "SERVICE42"). "CC1"–"CC4" identify a Line 21 Data Services channel [CEA608]; "SERVICE" values identify a Digital Television Closed Captioning [CEA708] service block number. For all other types, the mechanism for carrying a Rendition and mapping from the INSTREAM-ID to the content within the segment is defined by the segment, sample or bitstream format; value is a string containing characters from `[A-Z]`, `[a-z]`, `[0-9]`, and `.`. If the value does not match any alternative content, the client **SHOULD** ignore this and treat it as if no INSTREAM-ID was provided. |
| BIT-DEPTH | non-negative decimal-integer | OPTIONAL; **MUST NOT** be present unless TYPE is AUDIO | Specifies the audio bit depth of the Rendition. Allows players to identify Renditions with a bit depth appropriate to the available hardware. |
| SAMPLE-RATE | non-negative decimal-integer | OPTIONAL; **MUST NOT** be present unless TYPE is AUDIO | Specifies the audio sample rate of the Rendition. Allows players to identify Renditions that may be played without sample rate conversion; useful for lossless encodings. |
| CHARACTERISTICS | quoted-string, one or more Media Characteristic Tags (MCTs) separated by `,` | OPTIONAL | A Media Characteristic Tag has the same format as the payload of a media characteristic tag atom [MCT]. Each MCT indicates an individual characteristic of the Rendition. See enumerated MCT values below. MAY include private MCTs. |
| CHANNELS | quoted-string, ordered slash-separated (`/`) list of parameters | see below; **MUST NOT** be present unless TYPE is AUDIO | See channel-parameter breakdown below. |

CHARACTERISTICS values (transcribed):

- A SUBTITLES Rendition MAY include: `public.accessibility.transcribes-spoken-dialog`,
  `public.accessibility.describes-music-and-sound`, and `public.easy-to-read`
  (which indicates that the subtitles have been edited for ease of reading).
- An AUDIO Rendition MAY include: `public.accessibility.describes-video`.
- Any Rendition MAY include `public.machine-generated` to indicate the
  Rendition was authored or translated programmatically.

CHANNELS attribute breakdown: the first parameter is a decimal-integer; each
succeeding parameter is a comma-separated list of Identifiers. An Identifier
is a string containing characters from the set `[A-Z]`, `[0-9]`, and `-`.

- **First parameter** — a count of audio channels expressed as a
  decimal-integer, indicating the maximum number of independent, simultaneous
  audio channels present in any Media Segment in the Rendition. For example,
  an AC-3 5.1 Rendition would have a `CHANNELS="6"` attribute.
- **Second parameter** — identifies the presence of spatial audio of some
  kind, for example, object-based audio, in the Rendition. This parameter is
  a comma-separated list of Audio Coding Identifiers (optional). Audio Coding
  Identifiers can be codec-specific. An Audio Coding Identifier can be used to
  signal the order of ambisonics: the value is a decimal-integer that
  represents the order of ambisonics followed by letters 'OA' (0x4F41); e.g.,
  a value of `3OA` indicates third order ambisonics. A parameter value
  consisting solely of the dash character (0x2D) indicates that the audio is
  only channel-based.
- **Third parameter** — supplementary indications of special channel usage
  necessary for informed selection and processing. Comma-separated list of
  Special Usage Identifiers (optional; if present, the second parameter
  **MUST** be non-empty). Defined Special Usage Identifiers:
  - **BINAURAL** — the audio is binaural (either recorded or synthesized). It
    **SHOULD NOT** be dynamically spatialized. Best suited for delivery to
    headphones.
  - **IMMERSIVE** — the audio is pre-processed content that **SHOULD NOT** be
    dynamically spatialized. Suitable to deliver to either headphones or
    speakers.
  - **DOWNMIX** — the audio is a downmix derivative of some other audio. If
    desired, the downmix may be used as a substitute for alternative
    Renditions in the same group with compatible attributes and a greater
    channel count. It MAY be dynamically spatialized.
  - **BED-\<integer\>** — the audio is prepared for routing to a specific
    speaker location. The value after the dash character (0x2D) indicates
    count of channels prepared for specific routing. Example: `BED-4`.
  - **DOF-\<integer\>** — the audio represents degrees of freedom. The value
    after the dash character (0x2D) indicates numerical value associated with
    degrees of freedom. Valid values for this special usage identifier are
    `DOF-3` or `DOF-6`.
  - Audio without a Special Usage Identifier MAY be dynamically spatialized.
  - No other CHANNELS parameters are currently defined.
- All audio `EXT-X-MEDIA` tags **SHOULD** have a CHANNELS attribute. If a
  Multivariant Playlist contains two Renditions with the same NAME encoded
  with the same codec but a different number of channels, then the CHANNELS
  attribute is **REQUIRED**; otherwise, it is OPTIONAL.

Example given in source for LANGUAGE/ASSOC-LANGUAGE use (Norwegian, Bokmael):

```
#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="subtitles",
NAME="Bokmael",AUTOSELECT=YES,LANGUAGE="nb",
ASSOC-LANGUAGE="no",URI="nb-subtitles.m3u8"
```

This allows automatic selection of the Bokmael subtitles in this Media
Playlist when the user picks an audio variant in Norwegian.

#### 4.4.6.1.1. Rendition Groups

A set of one or more `EXT-X-MEDIA` tags with the same GROUP-ID value and the
same TYPE value defines a Group of Renditions. Each member of the Group
**MUST** be an alternative Rendition of the same content; otherwise, playback
errors can occur.

All `EXT-X-MEDIA` tags in a Playlist **MUST** meet the following constraints:

- All `EXT-X-MEDIA` tags in the same Group **MUST** have different NAME
  attributes.
- A Group **MUST NOT** have more than one member with a DEFAULT attribute of
  YES.
- Each `EXT-X-MEDIA` tag with an AUTOSELECT=YES attribute **SHOULD** have a
  combination of LANGUAGE [RFC5646], ASSOC-LANGUAGE, FORCED, and
  CHARACTERISTICS attributes that is distinct from those of other
  AUTOSELECT=YES members of its Group.

A Playlist MAY contain multiple Groups of the same TYPE in order to provide
multiple encodings of that media type. If it does so, each Group of the same
TYPE **MUST** have the same set of members, and each corresponding member
**MUST** have identical attributes with the exception of the URI, CHANNELS,
BIT-DEPTH, STABLE-RENDITION-ID, INSTREAM-ID, and SAMPLE-RATE attributes.

Each member in a Group of Renditions MAY have a different sample format. For
example, an English Rendition can be encoded with AC-3 5.1 while a Spanish
Rendition is encoded with AAC stereo. However, any `EXT-X-STREAM-INF` tag
(§4.4.6.2) or `EXT-X-I-FRAME-STREAM-INF` tag (§4.4.6.3) that references such
a Group **MUST** have a CODECS attribute that lists every sample format
present in any Rendition in the Group, or client playback failures can occur.
In the example above, the CODECS attribute would include
`"ac-3,mp4a.40.2"`.

### EXT-X-STREAM-INF — §4.4.6.2

**Format:**
```
#EXT-X-STREAM-INF:<attribute-list>
<URI>
```

- Specifies a Variant Stream, which is a set of Renditions that can be
  combined to play the presentation. The attributes of the tag provide
  information about the Variant Stream.
- The URI line that follows the `EXT-X-STREAM-INF` tag specifies a Media
  Playlist that carries a Rendition of the Variant Stream. The URI line is
  **REQUIRED**. Clients that do not support multiple video Renditions
  **SHOULD** play this Rendition.

| Attribute | Value type | Required/Optional | Meaning |
|-----------|-------------|--------------------|---------|
| BANDWIDTH | decimal-integer (bits per second) | REQUIRED (every `EXT-X-STREAM-INF` tag **MUST** include it) | The peak segment bit rate of the Variant Stream. If all Media Segments in a Variant Stream have already been created, the BANDWIDTH value **MUST** be the largest sum of peak segment bit rates produced by any playable combination of Renditions. (For a Variant Stream with a single Media Playlist, this is just the peak segment bit rate of that Media Playlist.) An inaccurate value can cause playback stalls or prevent clients from playing the variant. If the Multivariant Playlist is to be made available before all Media Segments in the presentation have been encoded, the BANDWIDTH value **SHOULD** be the BANDWIDTH value of a representative period of similar content, encoded using the same settings. |
| AVERAGE-BANDWIDTH | decimal-integer (bits per second) | OPTIONAL | The average segment bit rate of the Variant Stream. If all Media Segments in a Variant Stream have already been created, the AVERAGE-BANDWIDTH value **MUST** be the largest sum of average segment bit rates produced by any playable combination of Renditions. (For a Variant Stream with a single Media Playlist, this is just the average segment bit rate of that Media Playlist.) An inaccurate value can cause playback stalls or prevent clients from playing the variant. If the Multivariant Playlist is to be made available before all Media Segments have been encoded, the value **SHOULD** be the AVERAGE-BANDWIDTH value of a representative period of similar content, encoded using the same settings. |
| SCORE | positive decimal-floating-point | OPTIONAL, but if any Variant Stream contains SCORE then all Variant Streams in the Multivariant Playlist **SHOULD** have a SCORE attribute | An abstract, relative measure of the playback quality-of-experience of the Variant Stream. The value can be based on any metric or combination of metrics that can be consistently applied to all Variant Streams. The value **SHOULD** consider all media in the Variant Stream, including video, audio and subtitles. A Variant Stream with a SCORE attribute **MUST** be considered by the Playlist author to be more desirable than any Variant Stream with a lower SCORE attribute in the same Multivariant Playlist. See Section 6.3.1. |
| CODECS | quoted-string, comma-separated list of formats [RFC6381] | SHOULD be included (every `EXT-X-STREAM-INF` tag SHOULD include a CODECS attribute) | Each format specifies a media sample type present in one or more Renditions specified by the Variant Stream. Valid format identifiers are those in the ISO Base Media File Format Name Space defined by "The 'Codecs' and 'Profiles' Parameters for 'Bucket' Media Types" [RFC6381]. Example: a stream containing AAC-LC audio and H.264 Main Profile Level 3.0 video would have CODECS value `"mp4a.40.2,avc1.4d401e"`. If a Variant Stream specifies one or more Renditions that include IMSC subtitles, the CODECS attribute **MUST** indicate this with a format identifier such as `"stpp.ttml.im1t"`. |
| SUPPLEMENTAL-CODECS | quoted-string, comma-separated list of elements, each a slash-separated list of fields | OPTIONAL | Describes media samples with both a backward-compatible base layer and a newer enhancement layer. The base layers are specified in CODECS and the enhancement layers by SUPPLEMENTAL-CODECS. Each element's first field **MUST** be a valid CODECS format; remaining fields (if present) **MUST** be compatibility brands [MP4RA] pertaining to that codec's bitstream. Each member of SUPPLEMENTAL-CODECS **MUST** have its base layer codec declared in CODECS. Example: Dolby Vision 8.4 content might have CODECS including `"hvc1.2.4.L153.b0"`, and SUPPLEMENTAL-CODECS including `"dvh1.08.07/db4h"`. |
| RESOLUTION | decimal-resolution | OPTIONAL but recommended if the Variant Stream includes video | Describes the optimal pixel resolution at which to display all the video in the Variant Stream. |
| FRAME-RATE | decimal-floating-point, rounded to three decimal places | OPTIONAL but recommended if the Variant Stream includes video; **SHOULD** be included if any video in a Variant Stream exceeds 30 frames per second | Describes the maximum frame rate for all the video in the Variant Stream. |
| HDCP-LEVEL | enumerated-string (TYPE-0, TYPE-1, NONE) | OPTIONAL; **SHOULD** be present if any content in the Variant Stream will fail to play without HDCP | Advisory attribute. TYPE-0 indicates that the Variant Stream could fail to play unless the output is protected by HDCP Type 0 [HDCP] or equivalent. TYPE-1 indicates HDCP Type 1 or equivalent is required. NONE indicates the content does not require output copy protection. Encrypted Variant Streams with different HDCP levels **SHOULD** use different media encryption keys. Clients without output copy protection **SHOULD NOT** load a Variant Stream with an HDCP-LEVEL attribute unless its value is NONE. |
| ALLOWED-CPC | quoted-string, comma-separated list of `KEYFORMAT:CPC-Label[/CPC-Label...]` entries | OPTIONAL | Allows a server to indicate that playback of a Variant Stream containing encrypted Media Segments is to be restricted to devices that guarantee a certain level of content protection robustness. Each entry consists of a KEYFORMAT attribute value followed by `:` followed by a sequence of Content Protection Configuration (CPC) Labels separated by `/`. Each CPC Label is a string containing characters from `[A-Z]`, `[0-9]`, and `-`. Example: `ALLOWED-CPC="com.example.drm1:SMART-TV/PC,com.example.drm2:HW"`. A CPC Label identifies a class of playback device that implements the KEYFORMAT with a certain level of content protection robustness. Each KEYFORMAT can define its own set of CPC Labels. The "identity" KEYFORMAT does not define any labels. A KEYFORMAT that defines CPC Labels **SHOULD** also specify its robustness requirements in a secure manner in each key response. A client MAY play the Variant Stream if it implements one of the listed KEYFORMAT schemes with content protection robustness matching one or more of the CPC Labels in the list; if it does not match any, it **SHOULD NOT** attempt to play the Variant Stream. If ALLOWED-CPC is not present or does not contain a particular KEYFORMAT then all clients that support that KEYFORMAT MAY play the Variant Stream. |
| VIDEO-RANGE | enumerated-string (SDR, HLG, PQ) | OPTIONAL; absence implies SDR | See transfer-characteristic mapping below. Clients that do not recognize the attribute value **SHOULD NOT** select the Variant Stream. |
| REQ-VIDEO-LAYOUT | quoted-string, comma-separated list of View Presentation Entries | OPTIONAL; **SHOULD** be present if any content in the Variant Stream will fail to display properly without specialized rendering | See View Presentation Entry breakdown below. |
| STABLE-VARIANT-ID | quoted-string | OPTIONAL | A stable identifier for the URI within the Multivariant Playlist. All characters **MUST** be from the set `[a-z]`, `[A-Z]`, `[0-9]`, `+`, `/`, `=`, `.`, `-`, `_`. Allows the URI of the Variant Stream to change between two distinct downloads of the Multivariant Playlist; IDs are matched by byte-for-byte comparison. All `EXT-X-STREAM-INF` tags in a Multivariant Playlist with the same URI value **SHOULD** use the same STABLE-VARIANT-ID. |
| AUDIO | quoted-string | OPTIONAL | **MUST** match the value of the GROUP-ID attribute of an `EXT-X-MEDIA` tag elsewhere in the Multivariant Playlist whose TYPE attribute is AUDIO. Indicates the set of audio Renditions that **SHOULD** be used when playing the presentation. See §4.4.6.2.1. |
| VIDEO | quoted-string | OPTIONAL | **MUST** match the value of the GROUP-ID attribute of an `EXT-X-MEDIA` tag elsewhere in the Multivariant Playlist whose TYPE attribute is VIDEO. Indicates the set of video Renditions that **SHOULD** be used when playing the presentation. See §4.4.6.2.1. |
| SUBTITLES | quoted-string | OPTIONAL | **MUST** match the value of the GROUP-ID attribute of an `EXT-X-MEDIA` tag elsewhere in the Multivariant Playlist whose TYPE attribute is SUBTITLES. Indicates the set of subtitle Renditions that can be used when playing the presentation. See §4.4.6.2.1. |
| CLOSED-CAPTIONS | quoted-string, or enumerated-string NONE | OPTIONAL | If a quoted-string, **MUST** match the value of the GROUP-ID attribute of an `EXT-X-MEDIA` tag elsewhere in the Playlist whose TYPE attribute is CLOSED-CAPTIONS, and indicates the set of closed-caption Renditions that can be used when playing the presentation (see §4.4.6.2.1). If the value is the enumerated-string NONE, all `EXT-X-STREAM-INF` tags **MUST** have this attribute with a value of NONE, indicating that there are no closed captions in any Variant Stream in the Multivariant Playlist. Having closed captions in one Variant Stream but not another can trigger playback inconsistencies. |
| PATHWAY-ID | quoted-string | OPTIONAL | Indicates that the Variant Stream belongs to the identified Content Steering (Section 7) Pathway. Absence indicates that the Variant Stream belongs to the default Pathway ".", so every Variant Stream can be associated with a named Pathway. A Content Producer **SHOULD** provide all Rendition Groups on all Pathways. A Variant Stream belonging to a particular Pathway **SHOULD** use Rendition Group(s) on that Pathway. |

VIDEO-RANGE transfer-characteristic mapping (transcribed):

- The value **MUST** be SDR if the video in the Variant Stream is encoded
  using one of the following reference opto-electronic transfer
  characteristic functions specified by the TransferCharacteristics code
  point: [CICP] 1, 6, 13, 14, 15. Note that different TransferCharacteristics
  code points can use the same transfer function.
- The value **MUST** be HLG if the video in the Variant Stream is encoded
  using a reference opto-electronic transfer characteristic function
  specified by the TransferCharacteristics code point 18, or consists of such
  video mixed with video qualifying as SDR (see above).
- The value **MUST** be PQ if the video in the Variant Stream is encoded
  using a reference opto-electronic transfer characteristic function
  specified by the TransferCharacteristics code point 16, or consists of such
  video mixed with video qualifying as SDR or HLG (see above).

REQ-VIDEO-LAYOUT breakdown (transcribed):

- Indicates whether the video content in the Variant Stream requires
  specialized rendering to be properly displayed. Its value is a
  quoted-string containing a comma-separated list of View Presentation
  Entries, where each entry specifies the rendering for some portion of the
  Variant Stream.
- Each View Presentation Entry consists of an unordered, slash-separated list
  of specifiers. Each specifier controls one aspect of the entry — the
  specifiers are disjoint and the values for a specifier are mutually
  exclusive. Each specifier can occur at most once in an entry.
- All specifier values are enumerated-strings. The enumerated-strings for a
  specifier will share a common prefix. If the specifier list contains an
  unrecognized enumerated-string then the client **MUST** ignore the tag and
  the following URI line. Otherwise, the client might display the content
  without applying the correct specialized rendering.
- **Video Channel Specifier** — an enumerated-string that defines the video
  channels; valid strings are `CH-STEREO` and `CH-MONO`. `CH-STEREO`
  (stereoscopic) indicates that both left and right eye images are present.
  `CH-MONO` (monoscopic) indicates that a single image is present.
- **Projection Specifier** — a video projection defines how a
  two-dimensional rectangular image must be transformed in order to display
  it faithfully to a viewer. An enumerated-string; valid values are
  `PROJ-RECT`, `PROJ-EQUI`, `PROJ-HEQU`, `PROJ-PRIM`. `PROJ-RECT`
  (rectilinear) indicates no projection. `PROJ-EQUI` (equirectangular)
  indicates a 360 degree spherical projection. `PROJ-HEQU`
  (half-equirectangular) indicates a 180 degree spherical projection.
  `PROJ-PRIM` (parametric immersive) indicates that the image is a parametric
  spherical projection. The absence of a Projection Specifier is identical to
  specifying `PROJ-RECT`. These projection specifiers signal the need for
  specialized rendering but details of the rendering are not in the playlist
  and are outside the scope of this specification (refer to ISO Base Media
  File Format and Apple HEVC Stereo Video [ISOBMFF-StereoVideo]).
- A REQ-VIDEO-LAYOUT attribute **MUST NOT** be empty, and each View
  Presentation Entry **MUST NOT** be empty.
- The client **SHOULD** assume that the order of entries reflects the most
  common presentation in the content. For example, if the content is
  predominantly stereoscopic, with some brief sections that are monoscopic,
  then the Multivariant Playlist **SHOULD** specify
  `REQ-VIDEO-LAYOUT="CH-STEREO,CH-MONO"`. On the other hand, if the content
  is predominantly monoscopic then the Multivariant Playlist **SHOULD**
  specify `REQ-VIDEO-LAYOUT="CH-MONO,CH-STEREO"`.
- By default a video variant is monoscopic and rectilinear, so an attribute
  consisting entirely of one or more of those specifiers, such as
  `REQ-VIDEO-LAYOUT="CH-MONO"` or `REQ-VIDEO-LAYOUT="CH-MONO/PROJ-RECT"` is
  unnecessary and **SHOULD NOT** be present. Eliminating it allows
  Multivariant Playlists with a mix of monoscopic and stereoscopic variants
  to be played by clients that do not handle the REQ-VIDEO-LAYOUT attribute.

#### 4.4.6.2.1. Alternative Renditions

When an `EXT-X-STREAM-INF` tag contains an AUDIO, VIDEO, SUBTITLES, or
CLOSED-CAPTIONS attribute, it indicates that alternative Renditions of the
content are available for playback of that Variant Stream.

When defining alternative Renditions, the following constraints **MUST** be
met to prevent client playback errors:

- All playable combinations of Renditions associated with an
  `EXT-X-STREAM-INF` tag **MUST** have an aggregate bandwidth less than or
  equal to the BANDWIDTH attribute of the `EXT-X-STREAM-INF` tag.
- If an `EXT-X-STREAM-INF` tag contains a RESOLUTION attribute and a VIDEO
  attribute, then every alternative video Rendition **MUST** have an optimal
  display resolution matching the value of the RESOLUTION attribute.
- Every alternative Rendition associated with an `EXT-X-STREAM-INF` tag
  **MUST** meet the constraints for a Variant Stream described in
  Section 6.2.4.

The URI attribute of the `EXT-X-MEDIA` tag is REQUIRED if the media type is
SUBTITLES, but OPTIONAL if the media type is VIDEO or AUDIO. If the media
type is VIDEO or AUDIO, a missing URI attribute indicates that the media data
for this Rendition is included in the Media Playlist of any
`EXT-X-STREAM-INF` tag referencing this `EXT-X-MEDIA` tag. If the media TYPE
is AUDIO and the URI attribute is missing, clients **MUST** assume that the
audio data for this Rendition is present in every video Rendition specified
by the `EXT-X-STREAM-INF` tag.

The URI attribute of the `EXT-X-MEDIA` tag **MUST NOT** be included if the
media type is CLOSED-CAPTIONS.

### EXT-X-I-FRAME-STREAM-INF — §4.4.6.3

**Format:** `#EXT-X-I-FRAME-STREAM-INF:<attribute-list>`

- Identifies a Media Playlist file containing the I-frames of a multimedia
  presentation. It stands alone, in that it does not apply to a particular
  URI in the Multivariant Playlist.
- All attributes defined for the `EXT-X-STREAM-INF` tag (§4.4.6.2) are also
  defined for `EXT-X-I-FRAME-STREAM-INF`, **except for** the FRAME-RATE,
  AUDIO, SUBTITLES, and CLOSED-CAPTIONS attributes. In addition, the
  following attribute is defined:

| Attribute | Value type | Required/Optional | Meaning |
|-----------|-------------|--------------------|---------|
| URI | quoted-string | REQUIRED (every `EXT-X-I-FRAME-STREAM-INF` tag **MUST** include a BANDWIDTH attribute and a URI attribute) | Identifies the I-frame Media Playlist file. That Playlist file **MUST** contain an `EXT-X-I-FRAMES-ONLY` tag. |

- The provisions in §4.4.6.2.1 also apply to `EXT-X-I-FRAME-STREAM-INF` tags
  with a VIDEO attribute.
- A Multivariant Playlist that specifies alternative VIDEO Renditions and
  I-frame Playlists **SHOULD** include an alternative I-frame VIDEO Rendition
  for each regular VIDEO Rendition, with the same NAME and LANGUAGE
  attributes.

### EXT-X-SESSION-DATA — §4.4.6.4

**Format:** `#EXT-X-SESSION-DATA:<attribute-list>`

- Allows arbitrary session data to be carried in a Multivariant Playlist.

| Attribute | Value type | Required/Optional | Meaning |
|-----------|-------------|--------------------|---------|
| DATA-ID | quoted-string | REQUIRED | Identifies a particular data value. **SHOULD** conform to a reverse DNS naming convention, such as `"com.example.movie.title"`; there is no central registration authority, so Playlist authors **SHOULD** take care to choose a value that is unlikely to collide with others. |
| VALUE | quoted-string | see below | Contains the data identified by DATA-ID. If LANGUAGE is specified, VALUE **SHOULD** contain a human-readable string written in the specified language. |
| URI | quoted-string | see below | A URI. The resource identified by the URI **MUST** be formatted as indicated by the FORMAT attribute; otherwise, clients may fail to interpret the resource. |
| FORMAT | enumerated-string (JSON, RAW) | OPTIONAL; absence implies JSON | **MUST** be ignored when URI attribute is missing. If JSON, the URI **MUST** reference a JSON [RFC8259] format file. If RAW, the URI SHALL be treated as a binary file. |
| LANGUAGE | quoted-string, a language tag [RFC5646] | OPTIONAL | Identifies the language of the VALUE. |

- Each `EXT-X-SESSION-DATA` tag **MUST** contain either a VALUE or URI
  attribute, but not both.
- A Playlist MAY contain multiple `EXT-X-SESSION-DATA` tags with the same
  DATA-ID attribute. A Playlist **MUST NOT** contain more than one
  `EXT-X-SESSION-DATA` tag with the same DATA-ID attribute and the same
  LANGUAGE attribute.

### EXT-X-SESSION-KEY — §4.4.6.5

**Format:** `#EXT-X-SESSION-KEY:<attribute-list>`

- Allows encryption keys from Media Playlists to be specified in a
  Multivariant Playlist. This allows the client to preload these keys
  without having to read the Media Playlist(s) first.
- All attributes defined for the `EXT-X-KEY` tag (§4.4.4.4) are also defined
  for `EXT-X-SESSION-KEY`, **except that** the value of the METHOD attribute
  **MUST NOT** be NONE. If an `EXT-X-SESSION-KEY` is used, the values of the
  METHOD, KEYFORMAT, and KEYFORMATVERSIONS attributes **MUST** match any
  `EXT-X-KEY` with the same URI value.
- `EXT-X-SESSION-KEY` tags **SHOULD** be added if multiple Variant Streams or
  Renditions use the same encryption keys and formats. An `EXT-X-SESSION-KEY`
  tag is not associated with any particular Media Playlist.
- A Multivariant Playlist **MUST NOT** contain more than one
  `EXT-X-SESSION-KEY` tag with the same METHOD, URI, IV, KEYFORMAT, and
  KEYFORMATVERSIONS attribute values.
- The `EXT-X-SESSION-KEY` tag is optional.

(Attribute table: identical to `EXT-X-KEY`, §4.4.4.4 above, except METHOD
value NONE is disallowed here.)

### EXT-X-CONTENT-STEERING — §4.4.6.6

**Format:** `#EXT-X-CONTENT-STEERING:<attribute-list>`

- Allows a server to provide a Content Steering (Section 7) Manifest. It is
  OPTIONAL. It **MUST NOT** appear more than once in a Multivariant Playlist.

| Attribute | Value type | Required/Optional | Meaning |
|-----------|-------------|--------------------|---------|
| SERVER-URI | quoted-string | REQUIRED | A URI to a Steering Manifest (Section 7.2). MAY contain an asset identifier if the Steering Server requires it to produce the Steering Manifest. MAY use the "data" URI scheme to provide the manifest in-line in the Multivariant Playlist; in that case, subsequent manifest reloads MAY be redirected to a remote Steering Server using the RELOAD-URI parameter (see Section 7.2). |
| PATHWAY-ID | quoted-string | OPTIONAL | Identifies the Pathway that **MUST** be applied by any client that supports Content Steering (see Section 7.5) until the initial Steering Manifest has been obtained. Its value **MUST** be a legal Pathway ID according to Section 7.2 that is specified by the PATHWAY-ID attribute of one or more Variant Streams in the Multivariant Playlist. |

---

## Tag count

Enumerated per subsection, as tags actually appear as `####` headings above
(each corresponds one-to-one with a `#EXT...`-format tag defined in the
source text, lines 823-3206):

| Subsection | Tags | Count |
|---|---|---|
| §4.4.1 Basic Tags | EXTM3U, EXT-X-VERSION | 2 |
| §4.4.2 Media or Multivariant Playlist Tags | EXT-X-INDEPENDENT-SEGMENTS, EXT-X-START, EXT-X-DEFINE | 3 |
| §4.4.3 Media Playlist Tags | EXT-X-TARGETDURATION, EXT-X-MEDIA-SEQUENCE, EXT-X-DISCONTINUITY-SEQUENCE, EXT-X-ENDLIST, EXT-X-PLAYLIST-TYPE, EXT-X-I-FRAMES-ONLY, EXT-X-PART-INF, EXT-X-SERVER-CONTROL | 8 |
| §4.4.4 Media Segment Tags | EXTINF, EXT-X-BYTERANGE, EXT-X-DISCONTINUITY, EXT-X-KEY, EXT-X-MAP, EXT-X-PROGRAM-DATE-TIME, EXT-X-GAP, EXT-X-BITRATE, EXT-X-PART | 9 |
| §4.4.5 Media Metadata Tags | EXT-X-DATERANGE, EXT-X-SKIP, EXT-X-PRELOAD-HINT, EXT-X-RENDITION-REPORT | 4 |
| §4.4.6 Multivariant Playlist Tags | EXT-X-MEDIA, EXT-X-STREAM-INF, EXT-X-I-FRAME-STREAM-INF, EXT-X-SESSION-DATA, EXT-X-SESSION-KEY, EXT-X-CONTENT-STEERING | 6 |
| **Total** | | **32** |

This totals **32**, not the 33 stated in the task brief. Every subsection
heading present in the source (§4.4.1.1 through §4.4.6.6, plus the
§4.4.5.1.1 SCTE-35 sub-spec, which defines no new tag of its own — it only
describes how to populate `EXT-X-DATERANGE` attributes) has been transcribed
above. No tag-numbered heading in the source's table of contents for §4.4 was
found and skipped; the discrepancy appears to be in the brief's count itself,
not in a missing tag in this document.
