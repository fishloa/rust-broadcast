# Server Responsibilities — HLS 2nd Edition (draft-pantos-hls-rfc8216bis-22, 1 May 2026), §6.2

Transcription of §6.2 "Server Responsibilities" (source lines 3258-3868 of
`specs/ietf_draft_pantos_hls_rfc8216bis.txt`). This is a normative checklist for
auditing an HLS **origin** (server-side) implementation — every MUST / MUST NOT /
SHOULD / SHOULD NOT / MAY / SHALL in the range is captured as a discrete,
independently checkable item, tagged with its source section number. Wording is
transcribed, not paraphrased, except where noted.

Verification counts (raw string occurrences in the source range vs. items
captured below): see the bottom of this document.

---

## 6.2.1 General Server Responsibilities

1. **[6.2.1-M1]** The server MUST divide the source media into individual Media
   Segments whose duration (when rounded to a whole second) is less than or
   equal to the Target Duration. Segments longer than that can trigger playback
   stalls and other errors.
2. **[6.2.1-S1]** The server SHOULD attempt to divide the source media at points
   that support effective decode of individual Media Segments, such as on packet
   and key frame boundaries.
3. **[6.2.1-M2]** The server MUST create a URI for every Media Segment that
   enables its clients to obtain the segment data.
4. **[6.2.1-MAY1]** If a server supports partial loading of resources (e.g., via
   HTTP Range requests), it MAY specify segments as sub-ranges of larger
   resources using the EXT-X-BYTERANGE tag.
5. **[6.2.1-S2]** The absence of media data (due to, for example, the temporary
   unavailability of an encoder with no change in the encoding parameters)
   SHOULD be signaled by adding one or more Media Segments to the Playlist
   whose Segment durations add up to the duration of absent media.
6. **[6.2.1-M3]** These (absent-media) Media Segments MUST have EXT-X-GAP tags
   applied to them.
7. **[6.2.1-M4]** Such Partial Segments MUST have a GAP=YES attribute.
8. **[6.2.1-MAY2]** Attempting to download these (gap) segments MAY produce an
   error, such as HTTP 404 or 410.
9. **[6.2.1-M5]** Any changes in encoding parameters (such as codec, resolution,
   file format, and tracks) or timing information MUST be signalled by the use
   of EXT-X-DISCONTINUITY.
10. **[6.2.1-MAY3]** Format changes MAY require decoder initialization and MAY
    result in a noticeable playback transition (two MAYs in one sentence).
11. **[6.2.1-M6]** A Media Segment MUST be available for immediate download at
    the full speed of the link to the Client when it is added to a Playlist
    unless it has been marked with an EXT-X-GAP tag; otherwise playback errors
    can occur.
12. **[6.2.1-S3]** Once download starts, its (the Media Segment's) transfer
    rate SHOULD NOT be constrained by the segment production process.
13. **[6.2.1-M7]** A Partial Segment MUST be similarly available at the time it
    is added to a Playlist.
14. **[6.2.1-S4]** HTTP servers SHOULD transfer text files — such as Playlists
    and WebVTT segments — using the "gzip" Content-Encoding if the client
    indicates that it is prepared to accept it.
15. **[6.2.1-M8]** The server MUST create a Media Playlist file (Section 4)
    that contains a URI for each Media Segment that the server wishes to make
    available, in the order in which they are to be played.
16. **[6.2.1-S5]** The value of the EXT-X-VERSION tag (Section 4.4.1.2) SHOULD
    NOT be greater than what is required for the tags and attributes in the
    Playlist (see Section 8).
17. **[6.2.1-M9]** Changes to the Playlist file MUST be made atomically from
    the point of view of the clients, or playback errors MAY occur (the "MAY"
    here describes the consequence of non-compliance, not a server option).
18. **[6.2.1-M10]** The server MUST NOT change the Media Playlist file, except
    to:
    - Append lines to it (Section 6.2.1).
    - Remove Media Segment URIs from the Playlist in the order that they
      appear, along with any tags that apply only to those segments
      (Section 6.2.2).
    - Remove Media Metadata tags that no longer apply to the presentation
      (Section 6.2.1).
    - Remove EXT-X-PART tags no longer at the live edge (Section 6.2.2).
    - Increment the value of the EXT-X-MEDIA-SEQUENCE or
      EXT-X-DISCONTINUITY-SEQUENCE tags (Section 6.2.2).
    - Add an EXT-X-ENDLIST tag to the Playlist (Section 6.2.1).
19. **[6.2.1-M11]** An EXT-X-PLAYLIST-TYPE tag with a value of VOD indicates
    that the Playlist file MUST NOT change.
20. **[6.2.1-M12]** An EXT-X-PLAYLIST-TYPE tag with a value of EVENT indicates
    that the Server MUST NOT change or remove any part of the Playlist file,
    with the exception of EXT-X-PART tags and Media Metadata tags as described
    above.
21. **[6.2.1-MAY4]** (EVENT playlists:) the Server MAY append lines to the
    Playlist.
22. **[6.2.1-M13]** The value of the EXT-X-TARGETDURATION tag in the Media
    Playlist MUST NOT change. (A typical Target Duration is 6 seconds —
    informative, not normative.)
23. Each Media Segment in a Media Playlist has an integer Discontinuity
    Sequence Number: the value of the EXT-X-DISCONTINUITY-SEQUENCE tag (or
    zero if none) plus the number of EXT-X-DISCONTINUITY tags in the Playlist
    preceding the URI line of the segment. (Definition, not a normative
    keyword item.)
24. **[6.2.1-MAY5]** The server MAY associate an absolute date and time with a
    Media Segment by applying an EXT-X-PROGRAM-DATE-TIME tag to it.
25. **[6.2.1-S6]** If a server provides this (PDT) mapping, it SHOULD apply an
    EXT-X-PROGRAM-DATE-TIME tag to every segment that has an
    EXT-X-DISCONTINUITY tag applied to it.
26. **[6.2.1-M14]** The Server MUST NOT add any EXT-X-PROGRAM-DATE-TIME tag to
    a Playlist that would cause the mapping between program date and Media
    Segment to become ambiguous.
27. **[6.2.1-MAY6]** One exception is permitted: the Server MAY introduce
    small (sub-second) overlaps to account for drift between the encoder
    clock and some independently produced date/time reference.
28. **[6.2.1-M15]** The later segment MUST only partially overlap the
    preceding segment.
29. **[6.2.1-M16]** The client MUST resolve these ambiguous date/times in
    favor of the later segment. (Client-side requirement, included for
    completeness since it constrains what the server may produce.)
30. **[6.2.1-M17]** The server MUST NOT remove an EXT-X-DATERANGE tag from a
    Playlist if any date in the range maps to a Media Segment in the
    Playlist.
31. **[6.2.1-M18]** The server MUST NOT reuse the ID attribute value of an
    EXT-X-DATERANGE tag for any new Date Range in the same Playlist.
32. **[6.2.1-M19]** Once the Following Range of a Date Range with an
    END-ON-NEXT=YES attribute is added to a Playlist, the Server MUST NOT
    subsequently add a Date Range with the same CLASS attribute whose
    START-DATE is between that of the END-ON-NEXT=YES range and its Following
    Range.
33. **[6.2.1-S7]** For Date Ranges with a PLANNED-DURATION attribute, the
    Server SHOULD signal the actual end of the range once it has been
    established (via a new EXT-X-DATERANGE tag with the same ID and either a
    DURATION or END-DATE attribute, or, for END-ON-NEXT=YES ranges, by adding
    a Following Range).
34. **[6.2.1-M20]** If the Media Playlist contains the final Media Segment of
    the presentation, then the Playlist file MUST contain the EXT-X-ENDLIST
    tag.
35. **[6.2.1-M21]** If a Media Playlist does not contain the EXT-X-ENDLIST
    tag, the server MUST make a new version of the Playlist file available
    that contains at least one new Media Segment.
36. **[6.2.1-M22]** It (the new Playlist version) MUST be made available no
    later than 1.5 times the Target Duration after the previous time the
    Playlist was updated with a Media Segment.
37. **[6.2.1-M23]** If a Media Playlist without an EXT-X-ENDLIST tag contains
    Partial Segments, the Server MUST add a new Partial Segment to the
    Playlist within one Part Target Duration after it added the previous
    Partial Segment.
38. **[6.2.1-S8]** If the server wishes to remove an entire presentation, it
    SHOULD provide a clear indication to clients that the Playlist file is no
    longer available (e.g., with an HTTP 404 or 410 response).
39. **[6.2.1-M24]** It (the server, on removing a presentation) MUST ensure
    that all Media Segments in the Playlist file remain available to clients
    for at least the duration of the Playlist file at the time of removal, to
    prevent interruption of in-progress playback.

## 6.2.2 Live Playlists

40. **[6.2.2-MAY1]** The server MAY limit the availability of Media Segments
    by removing Media Segments from the Playlist file (Section 6.2.1).
41. **[6.2.2-M1]** If Media Segments are to be removed, the Playlist file
    MUST contain an EXT-X-MEDIA-SEQUENCE tag.
42. **[6.2.2-M2]** Its (EXT-X-MEDIA-SEQUENCE) value MUST be incremented by 1
    for every Media Segment that is removed from the Playlist file.
43. **[6.2.2-M3]** It (EXT-X-MEDIA-SEQUENCE) MUST NOT decrease or wrap.
44. **[6.2.2-S1]** EXT-X-PART tags SHOULD be removed from the Playlist after
    they are greater than three Target Durations from the end of the
    Playlist.
45. **[6.2.2-M4]** Clients MUST be able to download the Partial Segment for
    at least three Target Durations after the EXT-X-PART tag is removed from
    the Playlist.
46. **[6.2.2-M5]** Media Segments and EXT-X-PART tags MUST be removed from the
    Playlist in the order that they appear in the Playlist.
47. **[6.2.2-M6]** The server MUST NOT remove a Media Segment from a Playlist
    file without an EXT-X-ENDLIST tag if that would produce a Playlist whose
    duration is less than three times the Target Duration.
48. Definition: the Availability Duration of a Media Segment is the duration
    of the segment plus the duration of the longest-duration Playlist
    distributed by the server containing that segment.
49. **[6.2.2-M7]** If the server removes a Media Segment URI from a Playlist
    that contains an EXT-X-ENDLIST tag, clients MUST be able to download the
    corresponding Media Segment until the time of removal plus the segment's
    Availability Duration.
50. **[6.2.2-M8]** If the server removes a Media Segment URI from a Playlist
    that does not contain an EXT-X-ENDLIST tag, clients MUST be able to
    download the segment until the time at which it first appeared in the
    Playlist plus the segment's Availability Duration.
51. **[6.2.2-M9]** If the server wishes to remove segments from a Media
    Playlist containing an EXT-X-DISCONTINUITY tag, the Media Playlist MUST
    contain an EXT-X-DISCONTINUITY-SEQUENCE tag.
52. **[6.2.2-M10]** If the server removes an EXT-X-DISCONTINUITY tag from the
    Media Playlist, it MUST increment the value of the
    EXT-X-DISCONTINUITY-SEQUENCE tag so that the Discontinuity Sequence
    Numbers of the segments still in the Media Playlist remain unchanged.
53. **[6.2.2-M11]** The value of the EXT-X-DISCONTINUITY-SEQUENCE tag MUST NOT
    decrease or wrap.
54. **[6.2.2-S2]** If a server plans to remove a Media Segment after it is
    delivered to clients over HTTP, it SHOULD ensure that the HTTP response
    contains an Expires header that reflects the planned time-to-live.
55. **[6.2.2-M12]** A Live Playlist MUST NOT contain the EXT-X-PLAYLIST-TYPE
    tag, as no value of that tag allows Media Segments to be removed.

### 6.2.2 reload/update rules summary (cross-referencing 6.2.1)

- Reload cadence when no EXT-X-ENDLIST: a new Playlist version with at least
  one new Media Segment MUST be available no later than **1.5 × Target
  Duration** after the last Media-Segment update (item 36 / [6.2.1-M22]).
- If Partial Segments are present and no EXT-X-ENDLIST: a new Partial Segment
  MUST be added within **one Part Target Duration** of the previous one (item
  37 / [6.2.1-M23]).
- Removal floor: a Media Segment MUST NOT be removed if doing so would leave
  the Playlist shorter than **3 × Target Duration** (item 47 / [6.2.2-M6]).
- EXT-X-PART removal: SHOULD be removed once more than **3 × Target
  Duration** from the end of the Playlist (item 44 / [6.2.2-S1]), but clients
  MUST still be able to download the Partial Segment for **3 × Target
  Duration** after its tag is removed (item 45 / [6.2.2-M4]).
- Media Sequence Number and Discontinuity Sequence Number are monotonic:
  increment-by-removal-count only, never decrease or wrap (items 42/43,
  52/53).

## 6.2.3 Encrypting Media Segments

56. **[6.2.3-MAY1]** Media Segments MAY be encrypted.
57. **[6.2.3-M1]** Every encrypted Media Segment MUST have an EXT-X-KEY tag
    (Section 4.4.4.4) applied to it with a URI that the client can use to
    obtain a Key file (Section 5) containing the decryption key.
58. A Media Segment can only be encrypted with one encryption METHOD, using
    one encryption key and IV (constraint, not a keyword item).
59. **[6.2.3-MAY2]** A server MAY offer multiple ways to retrieve that key by
    providing multiple EXT-X-KEY tags, each with a different KEYFORMAT
    attribute value.
60. **[6.2.3-MAY3]** The server MAY set the HTTP Expires header in the key
    response to indicate the duration for which the key can be cached.
61. **[6.2.3-M2]** Any unencrypted Media Segment in a Playlist MUST be in the
    scope of an EXT-X-KEY tag that specifies an encryption METHOD of NONE or
    precedes the first EXT-X-KEY tag.
62. **[6.2.3-SHALL1]** If the encryption METHOD is AES-128 or AES-256-GCM and
    the Playlist does not contain the EXT-X-I-FRAMES-ONLY tag, AES encryption
    as described in Section 4.4.4.4 SHALL be applied to individual Media
    Segments. (Note: this is the sole "SHALL" in the range — treated as
    equivalent in force to MUST per RFC 2119 usage, transcribed verbatim.)
63. **[6.2.3-M3]** If the encryption METHOD is AES-128 and the Playlist
    contains an EXT-X-I-FRAMES-ONLY tag, the entire resource MUST be
    encrypted using AES-128 CBC with PKCS7 padding [RFC5652].
64. **[6.2.3-MAY4]** Encryption MAY be restarted on 16-byte block boundaries,
    unless the first block contains an I-frame.
65. **[6.2.3-M4]** The IV used for encryption MUST be either the Media
    Sequence Number of the Media Segment or the value of the IV attribute of
    the EXT-X-KEY tag, as described in Section 5.2.
66. **[6.2.3-MAY5]** If the encryption METHOD indicates Sample Encryption,
    media samples MAY be encrypted prior to encapsulation in a Media Segment.
67. **[6.2.3-M5]** The server MUST NOT remove an EXT-X-KEY tag from the
    Playlist file if it applies to any Media Segment in the Playlist file.

## 6.2.4 Providing Variant Streams and Renditions

68. **[6.2.4-MAY1]** A server MAY offer multiple Media Playlist files to
    provide different encodings of the same presentation.
69. **[6.2.4-S1]** If it does so, it SHOULD provide a Multivariant Playlist
    file that lists each Variant Stream and Rendition to allow clients to
    switch between encodings dynamically.
70. Multivariant Playlists describe regular Variant Streams with
    EXT-X-STREAM-INF tags, I-frame Variant Streams with
    EXT-X-I-FRAME-STREAM-INF tags, and Renditions with EXT-X-MEDIA tags
    (descriptive, not a keyword item).
71. **[6.2.4-M1]** If an EXT-X-STREAM-INF tag or EXT-X-I-FRAME-STREAM-INF tag
    contains the CODECS attribute, the attribute value MUST include every
    media format [RFC6381] present in any Media Segment in any of the
    Renditions specified by the Variant Stream.
72. **[6.2.4-M2]** The server MUST meet the following constraints when
    producing Variant Streams (and alternative Renditions
    (Section 4.4.6.2.1)) in order to allow clients to switch between them:

    - **[6.2.4-M2a]** Each Variant Stream MUST present the same content.
    - **[6.2.4-M2b]** Matching content in Variant Streams and Renditions
      MUST have matching timestamps.
    - **[6.2.4-M2c]** Matching content in Variant Streams and Renditions
      MUST have matching Discontinuity Sequence Numbers (see
      Section 4.4.3.3).
    - **[6.2.4-M2d]** Each Media Playlist in each Variant Stream and
      Rendition MUST have the same Target Duration. The only exceptions are
      SUBTITLES Renditions and Media Playlists containing an
      EXT-X-I-FRAMES-ONLY tag, which **MAY** have different Target
      Durations if they have an EXT-X-PLAYLIST-TYPE of VOD.
    - **[6.2.4-M2e]** Content that appears in a Media Playlist of one
      Variant Stream but not in another MUST appear either at the
      beginning or at the end of the Media Playlist file and MUST NOT be
      longer than the smallest Target Duration declared in any Media
      Playlist in the Multivariant Playlist.
    - **[6.2.4-M2f]** If any Media Playlists have an EXT-X-PLAYLIST-TYPE
      tag, all Media Playlists MUST have an EXT-X-PLAYLIST-TYPE tag with
      the same value.
    - **[6.2.4-M2g]** If the Playlist contains an EXT-X-PLAYLIST-TYPE tag
      with the value of VOD, the first segment of every Media Playlist in
      every Variant Stream MUST start at the same media timestamp.
    - **[6.2.4-M2h]** If any Media Playlist in a Multivariant Playlist
      contains an EXT-X-PROGRAM-DATE-TIME tag, then all Media Playlists in
      that Multivariant Playlist MUST contain EXT-X-PROGRAM-DATE-TIME tags
      with consistent mappings of date and time to media timestamps.
    - **[6.2.4-M2i]** If any Playlist contains Date Ranges, then at least
      one Playlist in any playable combination of Renditions of any
      Variant Stream MUST contain Date Ranges. Any Playlist with Date
      Ranges MUST contain the same set of Date Ranges as the others that
      do. The EXT-X-DATERANGE tags of corresponding Date Ranges MUST have
      the same ID attribute value and contain the same set of
      attribute/value pairs.
    - **[6.2.4-M2j]** If any Media Playlist in a Multivariant Playlist
      contains an EXT-X-SERVER-CONTROL tag, then all Media Playlists in
      that Multivariant Playlist MUST contain that tag, with the same
      attributes and values.

73. **[6.2.4-S2]** In addition, for broadest compatibility, Variant Streams
    SHOULD contain the same encoded audio bitstream. This allows clients to
    switch between Variant Streams without audible glitching.

## 6.2.5 Delivery Directives Interface

74. **[6.2.5-MAY1]** A server MAY provide a set of services to its clients by
    implementing support for Delivery Directives. Delivery Directives are
    transmitted by the Client to the Server as Query Parameters in Playlist
    request URIs.
75. Servers advertise the availability of Delivery Directives using the
    EXT-X-SERVER-CONTROL tag (Section 4.4.3.8) (descriptive).
76. Query parameters for Playlist requests that begin with the string
    "_HLS_" are reserved by this specification. Currently-defined Delivery
    Directives are `_HLS_skip`, `_HLS_msn`, and `_HLS_part` (descriptive —
    the interface's parameter set).

### Query-parameter interface (exact names / value types)

| Parameter    | Value type                                   | Defined in |
|--------------|-----------------------------------------------|------------|
| `_HLS_skip`  | enumerated string: `YES` or `v2`               | §6.2.5.1   |
| `_HLS_msn`   | decimal-integer, `M`                           | §6.2.5.2   |
| `_HLS_part`  | decimal-integer, `N`                           | §6.2.5.2   |

### 6.2.5.1 Playlist Delta Updates

77. A Server advertises support for Playlist Delta Updates that skip older
    Media Segments by adding the CAN-SKIP-UNTIL attribute to the
    EXT-X-SERVER-CONTROL tag (descriptive — advertisement mechanism, no
    keyword).
78. A Server can also offer support for Playlist Delta Updates that skip
    older EXT-X-DATERANGE tags by adding the CAN-SKIP-DATERANGES attribute to
    the EXT-X-SERVER-CONTROL tag (descriptive, no keyword).
79. **[6.2.5.1-M1]** When a Server receives a request for a Playlist
    containing the CAN-SKIP-UNTIL attribute but no EXT-X-ENDLIST tag, and the
    requested URI contains an `_HLS_skip` directive whose value is `YES` or
    `v2`, it MUST respond with a Playlist Delta Update.
80. The Playlist Delta Update is a version of the Playlist in which Media
    Segments that are further from the end of the last (Parent) Media
    Segment in the Playlist than the Skip Boundary (Section 4.4.3.8), as well
    as their associated tags, are replaced by an EXT-X-SKIP tag
    (Section 4.4.5.2) (definition of the response shape).
81. **[6.2.5.1-M2]** When the `_HLS_skip` directive has a value of `v2`, the
    Playlist Delta Update additionally MUST NOT contain EXT-X-DATERANGE tags
    that were added to the Playlist more than CAN-SKIP-UNTIL seconds before
    the Playlist request.
82. **[6.2.5.1-M3]** The RECENTLY-REMOVED-DATERANGES attribute of the
    EXT-X-SKIP tag MUST list the date ranges that were removed from the
    Playlist within CAN-SKIP-UNTIL seconds of the Playlist request.
83. **[6.2.5.1-M4]** All tags that were not skipped MUST remain in the
    Playlist Delta Update.
84. **[6.2.5.1-M5]** A Server MUST ignore the `_HLS_skip` directive if the
    Playlist does not contain the CAN-SKIP-UNTIL attribute, or if it contains
    an EXT-X-ENDLIST tag.

### 6.2.5.2 Blocking Playlist Reload

85. **[6.2.5.2-MAY1]** A Server MAY offer Blocking Playlist Reloads, which
    enable immediate client discovery of Playlist updates as an alternative
    to polling.
86. A Server advertises support for Blocking Playlist Reload by adding the
    CAN-BLOCK-RELOAD=YES attribute to the EXT-X-SERVER-CONTROL tag
    (descriptive advertisement mechanism).
87. A Client requests a Blocking Playlist Reload using an `_HLS_msn`
    directive with a decimal-integer value `M` (interface description).
88. **[6.2.5.2-M1]** When the Playlist URI contains an `_HLS_msn` directive
    and no `_HLS_part` directive, the Server MUST defer responding to the
    request until the Playlist contains a Media Segment with a Media
    Sequence Number of `M` or later or it responds with an error.
89. The Playlist URI MAY also contain an `_HLS_part` directive with a
    decimal-integer value `N` — **[6.2.5.2-MAY2]**.
90. **[6.2.5.2-M2]** When the Playlist URI contains both an `_HLS_msn`
    directive and an `_HLS_part` directive, the Server MUST defer responding
    to the request until the Playlist contains the Partial Segment with Part
    Index `N` and with a Media Sequence Number of `M` or later or it
    responds with an error.
91. **[6.2.5.2-M3]** If the Client requests a Part Index greater than that of
    the final Partial Segment of the Parent Segment, the Server MUST treat
    the request as one for Part Index 0 of the following Parent Segment.
92. **[6.2.5.2-M4]** The Server MUST deliver the entire Playlist, even if the
    requested Media Segment is not the last one in the Playlist, and even if
    it is no longer in the Playlist.
93. **[6.2.5.2-M5]** A Server MUST ignore `_HLS_msn` and `_HLS_part` if the
    Playlist contains an EXT-X-ENDLIST tag.
94. **[6.2.5.2-S1]** If the `_HLS_msn` is greater than the Media Sequence
    Number of the last Media Segment in the current Playlist plus two, or if
    the `_HLS_part` exceeds the last Partial Segment in the current Playlist
    by the Advance Part Limit, then the server SHOULD immediately return Bad
    Request, such as HTTP 400.
    - Advance Part Limit = 3 / Part Target Duration, if Part Target Duration
      < 1 second; otherwise Advance Part Limit = 3.
95. **[6.2.5.2-M6]** If the Playlist URI contains an `_HLS_part` directive
    but no `_HLS_msn` directive, the Server MUST return Bad Request, such as
    HTTP 400.
96. **[6.2.5.2-S2]** A Server that cannot provide the requested Playlist
    after blocking for more than three Target Durations SHOULD return
    Service Unavailable, such as HTTP 503.
97. **[6.2.5.2-S3]** Playlists that contain the EXT-X-I-FRAMES-ONLY tag
    SHOULD support Blocking Playlist Reload using the `_HLS_msn` directive if
    other Renditions in the presentation contain CAN-BLOCK-RELOAD.

### 6.2.5 server response-behaviour summary (blocking reload + delta updates)

- `_HLS_skip=YES|v2` → server MUST return a delta-updated Playlist if
  CAN-SKIP-UNTIL is advertised and no EXT-X-ENDLIST is present ([6.2.5.1-M1]);
  otherwise the directive MUST be ignored ([6.2.5.1-M5]).
- `_HLS_msn=M` alone → block until a segment with sequence number ≥ `M`
  exists, or return an error ([6.2.5.2-M1]).
- `_HLS_msn=M&_HLS_part=N` → block until Part Index `N` of segment `M` (or
  later) exists, or return an error ([6.2.5.2-M2]); over-range `N` maps to
  Part Index 0 of the next Parent Segment ([6.2.5.2-M3]).
- `_HLS_part` without `_HLS_msn` → MUST return HTTP 400-class Bad Request
  ([6.2.5.2-M6]).
- Both directives MUST be ignored once EXT-X-ENDLIST is present
  ([6.2.5.2-M5]).
- Excessive look-ahead (`_HLS_msn` > last MSN + 2, or `_HLS_part` beyond the
  Advance Part Limit) SHOULD get an immediate HTTP 400-class response
  ([6.2.5.2-S1]).
- A block exceeding 3 × Target Duration with no answer SHOULD return HTTP
  503-class Service Unavailable ([6.2.5.2-S2]).
- The full Playlist is always returned, never a partial/diff response outside
  the delta-update mechanism ([6.2.5.2-M4]).

## 6.2.6 Providing Preload Hints

98. **[6.2.6-MAY1]** The Server MAY add EXT-X-PRELOAD-HINT tags
    (Section 4.4.5.3) to the Playlist to allow Clients playing the stream to
    request upcoming resources in advance.
99. **[6.2.6-M1]** A hinted resource MUST be available for request when its
    EXT-X-PRELOAD-HINT tag is added to the Playlist.
100. **[6.2.6-M2]** When processing requests for a URI or a byte range of a
    URI that includes one or more Partial Segments that are not yet
    completely available to be sent — such as requests made in response to
    an EXT-X-PRELOAD-HINT tag — the server MUST refrain from transmitting any
    bytes belonging to a Partial Segment until all bytes of that Partial
    Segment can be transmitted at the full speed of the link to the client.
101. **[6.2.6-M3]** If the requested range includes more than one Partial
    Segment then the server MUST enforce this delivery guarantee for each
    Partial Segment in turn.
102. **[6.2.6-S1]** The Server SHOULD NOT hint a byte range that it does not
    expect its clients to require in the near term.
103. **[6.2.6-S2]** The server SHOULD respond with "Not Found" (such as HTTP
    404) to a request for a resource that it cannot find and that is not
    specified by an EXT-X-PRELOAD-HINT tag in an active Media Playlist.
104. **[6.2.6-MAY2]** A server MAY choose not to publish previously-hinted
    resources if the planned segmentation changes, such as the case of early
    return from an ad.
105. **[6.2.6-S3]** The server SHOULD respond to client requests for those
    (unpublished-hint) resources with "Not Found" (such as HTTP 404).
106. **[6.2.6-S4]** If a Partial Segment is created as a sub-range of a
    larger resource and its length is not known at the time that its hint is
    added to the Playlist, the BYTERANGE-LENGTH attribute SHOULD be omitted.
107. **[6.2.6-S5]** The BYTERANGE-OFFSET SHOULD indicate the Partial
    Segment's starting offset into the larger resource.
108. **[6.2.6-S6]** The Server SHOULD NOT add more than one
    EXT-X-PRELOAD-HINT tag with the same TYPE to a Playlist.

---

## Verification counts

Raw string occurrences of the normative keywords in source lines 3258-3868
(`specs/ietf_draft_pantos_hls_rfc8216bis.txt`), counted with
`grep -o '\bKEYWORD\b'` (note: `MUST NOT` and `SHOULD NOT` occurrences are
included in the plain `MUST` / `SHOULD` word counts, since the string "MUST"
appears inside "MUST NOT"):

| Keyword      | Raw count |
|--------------|-----------|
| `MUST` (word, incl. "MUST NOT")     | 70 |
| `MUST NOT`                          | 14 (subset of the 70) |
| `SHOULD` (word, incl. "SHOULD NOT") | 21 |
| `SHOULD NOT`                        | 4 (subset of the 21) |
| `MAY`                                | 21 |
| `SHALL`                              | 1 |

**Items captured in this document: 70 MUST-class (MUST + MUST NOT) items, 21
SHOULD-class (SHOULD + SHOULD NOT) items, 21 MAY items, 1 SHALL item** — each
enumerated above with its section tag. These match the raw grep counts
exactly; every MUST/MUST NOT/SHOULD/SHOULD NOT/MAY/SHALL occurrence in the
assigned range (lines 3258-3868) is accounted for as a discrete, numbered
requirement.

No ambiguous items were found requiring a direct quote-and-flag: the one
"MAY...MAY" double-hit in §6.2.1 (item 10, line 3294-3295) and the one
consequential (non-optional) "MAY" in §6.2.1 item 17 (line 3331, "or playback
errors MAY occur" — describing a failure consequence, not granting the server
an option) are called out inline above since they could otherwise be
miscounted as server-facing permissions.
