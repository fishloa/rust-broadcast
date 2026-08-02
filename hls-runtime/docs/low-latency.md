# Low-Latency HLS — draft-pantos-hls-rfc8216bis-22

Transcription source: `specs/ietf_draft_pantos_hls_rfc8216bis.txt`,
draft-pantos-hls-rfc8216bis-22 (1 May 2026). Low-latency HLS is not a single
contiguous section of the spec; it is assembled from tag definitions in
Section 4 and behavioural (server/client responsibility) text in Section 6.
The ranges transcribed here are:

| Section | Title | Source lines |
|---|---|---|
| 4.4.3.7 | EXT-X-PART-INF | 1213-1234 |
| 4.4.3.8 | EXT-X-SERVER-CONTROL | 1237-1325 |
| 4.4.4.9 | EXT-X-PART | 1690-1756 |
| 4.4.5.2 | EXT-X-SKIP | 2057-2095 |
| 4.4.5.3 | EXT-X-PRELOAD-HINT | 2096-2157 |
| 4.4.5.4 | EXT-X-RENDITION-REPORT | 2159-2207 |
| 6.2.5 | Delivery Directives Interface | 3687-3707 |
| 6.2.5.1 | Playlist Delta Updates | 3708-3756 |
| 6.2.5.2 | Blocking Playlist Reload | 3757-3816 |
| 6.2.6 | Providing Preload Hints | 3817-3863 |
| 6.3.7 | Requesting Playlist Delta Updates | 4208-4234 |
| 6.3.8 | Issuing Blocking Requests | 4235-4285 |

Two supporting ranges outside the assigned list were also pulled in because
the tag/mechanism text directly depends on them: Section 3.2 "Partial
Segments" (lines 586-627, the definition of Parent Segment/Part Index/Media
Sequence Number that 4.4.4.9 assumes), the Partial-Segment-removal rules in
6.2.1/6.2.2 (lines 3309-3495), and Appendix B.1 "Low-Latency Server
Configuration Profile" (lines 5574-5707), which states the Rendition-Report
and Preload-Hint-chaining obligations in profile form. These are marked as
such below, not folded silently into the numbered sections.

Normative keywords (MUST/SHOULD/MAY/REQUIRED/OPTIONAL) are preserved exactly
as written in the source. Where precision matters the source text is quoted
verbatim rather than paraphrased.

---

## Does any low-latency tag state an EXT-X-VERSION requirement?

**Direct answer: no.** Of the six low-latency tag definitions transcribed
below (EXT-X-PART-INF, EXT-X-SERVER-CONTROL, EXT-X-PART, EXT-X-SKIP,
EXT-X-PRELOAD-HINT, EXT-X-RENDITION-REPORT), only **EXT-X-SKIP** is named in
Section 8's version-compatibility table, and even then indirectly — Section 8
attaches the version requirement to the *tag*, not to "low-latency" as a
mode:

> A Playlist MUST indicate an EXT-X-VERSION of 9 or higher if it contains:
>
> *  The EXT-X-SKIP tag.
>
> A Playlist MUST indicate an EXT-X-VERSION of 10 or higher if it contains:
>
> *  An EXT-X-SKIP tag that replaces EXT-X-DATERANGE tags in a Playlist
>    Delta Update.

(Section 8, source lines 4558-4567; also already transcribed in this crate's
`docs/version-compatibility.md`, rows for versions 9 and 10.)

Section 8 (lines 4490-4607, grep-checked exhaustively for "compatibility
version" occurrences across the whole document) contains **no** "MUST
indicate an EXT-X-VERSION of N or higher" clause naming EXT-X-PART-INF,
EXT-X-SERVER-CONTROL, EXT-X-PART, EXT-X-PRELOAD-HINT, or
EXT-X-RENDITION-REPORT. The tag-definition text for each of those five (in
Sections 4.4.3.7, 4.4.3.8, 4.4.4.9, 4.4.5.3, 4.4.5.4, all reproduced below)
likewise contains no "REQUIRES a compatibility version number of..." sentence
— that phrasing appears elsewhere in Section 4 (e.g. for EXT-X-BYTERANGE,
EXT-X-I-FRAMES-ONLY, the IV attribute of EXT-X-KEY) but never attached to any
of these five tags.

The one data point in the other direction is circumstantial, not normative:
the Section 9.11 "Low-Latency Playlist" example (lines 4842-4901) elides its
EXT-X-VERSION line entirely (`#EXT-X-TARGETDURATION:4` followed by `...`), so
the example neither confirms nor contradicts anything about which version a
real low-latency playlist should declare — it simply doesn't show the tag.

**Conclusion as transcribed:** the spec's version-9 requirement is scoped
specifically to the presence of the EXT-X-SKIP tag (Playlist Delta Updates),
not to Partial Segments, Blocking Playlist Reload, or Preload Hints/Rendition
Reports as a group. A playlist using EXT-X-PART/EXT-X-PART-INF,
EXT-X-SERVER-CONTROL, EXT-X-PRELOAD-HINT, or EXT-X-RENDITION-REPORT but not
EXT-X-SKIP has no stated EXT-X-VERSION floor from any of these six sections.
(Whether some *other* attribute used alongside them, e.g. variable
substitution at version 8, forces a version bump is a separate, unrelated
trigger — not a low-latency-specific one.)

---

## 1. Tag reference

### 1.1 EXT-X-PART-INF — §4.4.3.7 (lines 1213-1234)

> The EXT-X-PART-INF tag provides information about the Partial Segments in
> the Playlist. It is REQUIRED if a Playlist contains one or more EXT-X-PART
> tags.

Format: `#EXT-X-PART-INF:<attribute-list>`

| Attribute | Type | Required? | Meaning |
|---|---|---|---|
| `PART-TARGET` | decimal-floating-point (seconds) | REQUIRED | "indicating the Part Target Duration" |

No other attributes are defined for EXT-X-PART-INF in this section.

### 1.2 EXT-X-SERVER-CONTROL — §4.4.3.8 (lines 1237-1325)

> The EXT-X-SERVER-CONTROL tag allows the Server to indicate support for
> Delivery Directives (Section 6.2.5).

Format: `#EXT-X-SERVER-CONTROL:<attribute-list>`

| Attribute | Type | Required? | Meaning |
|---|---|---|---|
| `CAN-SKIP-UNTIL` | decimal-floating-point (seconds) = the Skip Boundary | OPTIONAL; MAY appear in any Media Playlist | "Indicates that the Server can produce Playlist Delta Updates (Section 6.2.5.1) in response to the `_HLS_skip` Delivery Directive." The Skip Boundary **MUST be at least six times the Target Duration**. |
| `CAN-SKIP-DATERANGES` | enumerated-string (`YES`) | OPTIONAL; REQUIRES the presence of `CAN-SKIP-UNTIL` | "value ... is YES if the Server can produce Playlist Delta Updates (Section 6.2.5.1) that skip older EXT-X-DATERANGE tags in addition to Media Segments." |
| `HOLD-BACK` | decimal-floating-point (seconds) | OPTIONAL; MAY appear in any Media Playlist; absence implies three times the Target Duration | "server-recommended minimum distance from the end of the Playlist at which clients should begin to play or to which they should seek, unless PART-HOLD-BACK applies." Value **MUST be at least three times the Target Duration**. |
| `PART-HOLD-BACK` | decimal-floating-point (seconds) | REQUIRED if the Playlist contains the EXT-X-PART-INF tag | "server-recommended minimum distance from the end of the Playlist at which clients should begin to play or to which they should seek when playing in Low-Latency Mode." Value **MUST be at least twice the Part Target Duration**; value **SHOULD be at least three times the Part Target Duration**. If different Renditions have different Part Target Durations, PART-HOLD-BACK **SHOULD be at least three times the maximum Part Target Duration**. |
| `CAN-BLOCK-RELOAD` | enumerated-string (`YES`) | OPTIONAL; absence implies no support | "value ... is YES if the server supports Blocking Playlist Reload (Section 6.2.5.2)." |

The section closes with an explicit statement of the numeric interplay
between these values (quoted in full because it is exactly the kind of
relationship the implementation must get right):

> The Target Duration, and hold-back parameters, including the corresponding
> Partial Segment parameters Part Target Duration and PART-HOLD-BACK, have a
> significant impact on latency and bit rate adaptation. A shorter Target
> Duration reduces latency but also reduces available buffer, handicaps
> adaption and increases delivery overhead, increasing the likelihood of
> playback stall. A longer Hold Back can mitigate the playback stall
> likelihood, but increases the latency. A longer Target Duration improves
> other factors at the cost of increased latency.
>
> Content producers should choose values for the Target Duration and Hold
> Back parameters that balance the competing demands of delivery overhead,
> latency, and playback reliability according to their own criteria.

### 1.3 EXT-X-PART — §4.4.4.9 (lines 1690-1756)

> The EXT-X-PART tag identifies a Partial Segment. It is OPTIONAL.

Format: `#EXT-X-PART:<attribute-list>`

| Attribute | Type | Required? | Meaning |
|---|---|---|---|
| `URI` | quoted-string | REQUIRED | "the URI for the Partial Segment." |
| `DURATION` | decimal-floating-point (seconds) | REQUIRED | duration of the Partial Segment. |
| `INDEPENDENT` | enumerated-string (`YES`) | OPTIONAL (but SHOULD be carried by every Partial Segment containing an independent frame) | "value ... is YES if the Partial Segment contains an independent frame ... to increase the efficiency with which clients can join and switch Renditions." |
| `BYTERANGE` | quoted-string, same format as EXT-X-BYTERANGE (`"<n>[@<o>]"`) | OPTIONAL | "Indicates that the Partial Segment is a sub-range of the resource specified by the URI attribute." If `o` is absent, the sub-range begins at the next byte following the sub-range of the previous Partial Segment belonging to the same Parent Segment. |
| `GAP` | enumerated-string (`YES`) | REQUIRED for Partial Segments that are not available | "value ... is YES if the Partial Segment is not available." |

Additional rules stated directly under the attribute list:

- "When the Media Segment Tags (Section 4.4.4) EXT-X-DISCONTINUITY,
  EXT-X-KEY, EXT-X-MAP, or EXT-X-PROGRAM-DATE-TIME are applied to a Parent
  Segment, they also necessarily apply to the first Partial Segment, so they
  **MUST appear before the first EXT-X-PART tag** of that Parent Segment."
- "The duration of a Partial Segment **MUST be less than or equal to** the
  Part Target Duration."
- "The duration of each Partial Segment **MUST be at least 85%** of the Part
  Target Duration, with the exception of Partial Segments with the
  INDEPENDENT=YES or GAP=YES attribute, Partial Segments that are
  immediately followed by a Partial Segment with a GAP=YES attribute, and the
  final Partial Segment of any Parent Segment."
- "Playlists that contain the EXT-X-I-FRAMES-ONLY tag **SHOULD NOT** use
  Partial Segments."

### 1.4 EXT-X-SKIP — §4.4.5.2 (lines 2057-2095)

> A server produces a Playlist Delta Update (Section 6.2.5.1), by replacing
> tags earlier than the Skip Boundary with an EXT-X-SKIP tag.
>
> When replacing Media Segments, the EXT-X-SKIP tag replaces the segment URI
> lines and all Media Segment Tags that are applied to those segments. This
> tag **MUST NOT** appear more than once in a Playlist.

Format: `#EXT-X-SKIP:<attribute-list>`

| Attribute | Type | Required? | Meaning |
|---|---|---|---|
| `SKIPPED-SEGMENTS` | decimal-integer | REQUIRED | "the number of Media Segments replaced by the EXT-X-SKIP tag. Replacing segments with the EXT-X-SKIP tag does not change the value of the EXT-X-DISCONTINUITY-SEQUENCE tag." |
| `RECENTLY-REMOVED-DATERANGES` | quoted-string, tab (0x9) delimited list of EXT-X-DATERANGE IDs | REQUIRED if the Client requested an update that skips EXT-X-DATERANGE tags (MAY be empty) | "EXT-X-DATERANGE IDs that have been removed from the Playlist recently. See Section 6.2.5.1 for more information." |

Also stated in §4.4.5 (Media Metadata Tags, line 1758-1764): "There MAY be
more than one Media Metadata tag of each type in any Media Playlist. The only
exception to this rule is an EXT-X-SKIP, which MUST NOT appear more than
once" — restating the single-occurrence rule.

### 1.5 EXT-X-PRELOAD-HINT — §4.4.5.3 (lines 2096-2157)

> The EXT-X-PRELOAD-HINT tag allows a Client loading media from a live stream
> to reduce the time to obtain a resource from the Server by issuing its
> request before the resource is available to be delivered. The server will
> hold onto the request ("block") until it can respond.

Format: `#EXT-X-PRELOAD-HINT:<attribute-list>`

| Attribute | Type | Required? | Meaning |
|---|---|---|---|
| `TYPE` | enumerated-string (`PART` or `MAP`) | REQUIRED | "If the value is PART, the resource is a Partial Segment. If the value is MAP, the resource is a Media Initialization Section." |
| `URI` | quoted-string | REQUIRED | "a URI identifying the hinted resource. It MUST match the URI that will be subsequently added to the Playlist as a non-hinted resource (for example, the URI of an EXT-X-PART tag). The URI MAY be relative to the URI of the Playlist or it MAY be absolute. The hostname MAY differ from the hostname of the Playlist URI." |
| `BYTERANGE-START` | decimal-integer | OPTIONAL; absence implies 0 | "the byte offset of the first byte of the hinted resource, from the beginning of the resource identified by the URI attribute." |
| `BYTERANGE-LENGTH` | decimal-integer | OPTIONAL; absence indicates the last byte of the hinted resource is the last byte of the URI resource | When absent, "you SHOULD use the recommended last-byte-pos [RFC8673] value of 2^53-1 (9007199254740991) in the HTTP Range request." |

Additional rules stated directly under the attribute list:

- "Note that when a hinted Partial Segment eventually appears in the
  Playlist as an EXT-X-PART tag, it MAY have a different Discontinuity
  Sequence Number, Media Initialization Section, or encryption configuration.
  ... the Partial Segment can be preceded by an EXTINF tag indicating the end
  of the previous Parent Segment and an EXT-X-DISCONTINUITY, EXT-X-MAP, or
  EXT-X-KEY tag."
- "A Playlist containing an EXT-X-ENDLIST tag **MUST NOT** contain an
  EXT-X-PRELOAD-HINT tag."

### 1.6 EXT-X-RENDITION-REPORT — §4.4.5.4 (lines 2159-2207)

> The EXT-X-RENDITION-REPORT tag carries information about an associated
> Rendition that is as up-to-date as the Playlist that contains it.

Format: `#EXT-X-RENDITION-REPORT:<attribute-list>`

| Attribute | Type | Required? | Meaning |
|---|---|---|---|
| `URI` | quoted-string | REQUIRED | "the URI for the Media Playlist of the specified Rendition. It MUST be relative to the URI of the Media Playlist containing the EXT-X-RENDITION-REPORT tag." |
| `LAST-MSN` | decimal-integer | REQUIRED | "the Media Sequence Number of the last Media Segment currently in the specified Rendition. If the Rendition contains Partial Segments then this value is the Media Sequence Number of the last Partial Segment." |
| `LAST-PART` | decimal-integer | REQUIRED if the Rendition contains a Partial Segment | "the Part Index of the last Partial Segment currently in the specified Rendition whose Media Sequence Number is equal to the LAST-MSN attribute value." |

> A server MAY omit adding an attribute to an EXT-X-RENDITION-REPORT tag —
> even a mandatory attribute — if its value is the same as that of the
> Rendition Report of the Media Playlist to which the EXT-X-RENDITION-REPORT
> tag is being added. Doing so reduces the size of the Rendition Report.

---

## 2. Mechanisms

### 2.1 Partial Segments — lifecycle

**Definition (§3.2, lines 586-627, supporting context outside the assigned
range):**

> One component of viewer delay in a live stream is publishing latency: a
> Segment cannot be distributed until it has been completely encoded and
> packaged. ... Partial Segments provide a parallel channel for distributing
> media at the live edge of the Media Playlist, where the media is divided
> into a larger number of smaller pieces, such as CMAF Chunks. ... Because
> each Partial Segment has a short duration, it can be packaged, published,
> and added to the Media Playlist much earlier than its Parent Segment.

- A Partial Segment **MUST** be in one of the Supported Media Segment
  Formats (§3.1). It is associated with its Parent Segment "by appearing
  before it in the Media Playlist, and after the previous Media Segment."
- A Partial Segment **MUST** contain a subset of the media samples in its
  Parent Segment; "A Parent Segment and its entire set of Partial Segments
  **MUST** contain the same set of media samples, with the same timing and
  metadata."
- Each Partial Segment has a **Part Index** (integer, position within its
  Parent Segment, first Partial Segment = 0) and a **Media Sequence Number**
  equal to that of its Parent Segment.

**Advertisement:** EXT-X-PART-INF (§4.4.3.7) is REQUIRED once any EXT-X-PART
tag is present, and carries PART-TARGET (the Part Target Duration).
EXT-X-SERVER-CONTROL's PART-HOLD-BACK attribute is REQUIRED under the same
condition (§4.4.3.8: "PART-HOLD-BACK is REQUIRED if the Playlist contains the
EXT-X-PART-INF tag").

**Availability:** "A Partial Segment MUST be similarly available at the time
it is added to a Playlist" (§6.2.1, line 3315-3316 — the same immediate-
availability rule stated for full Media Segments in the preceding sentence).

**Ongoing publication cadence (§6.2.1, lines 3450-3453, outside the assigned
range but directly governing Partial Segment lifecycle):**

> If a Media Playlist without an EXT-X-ENDLIST tag contains Partial
> Segments, the Server **MUST** add a new Partial Segment to the Playlist
> within one Part Target Duration after it added the previous Partial
> Segment.

**Removal (§6.2.1/§6.2.2, lines 3344, 3487-3495, outside the assigned range
but the section that states exactly when Parts must be removed):**

The list of the only mutations a server may make to a Media Playlist file
(§6.2.1) explicitly includes:

> Remove EXT-X-PART tags no longer at the live edge (Section 6.2.2).

And §6.2.2 states the concrete threshold and the corresponding client-side
availability guarantee:

> EXT-X-PART tags **SHOULD** be removed from the Playlist after they are
> greater than three Target Durations from the end of the Playlist. Clients
> **MUST** be able to download the Partial Segment for at least three Target
> Durations after the EXT-X-PART tag is removed from the Playlist.
>
> Media Segments and EXT-X-PART tags **MUST** be removed from the Playlist
> in the order that they appear in the Playlist; otherwise, client playback
> can malfunction.

An EXT-X-PLAYLIST-TYPE of `EVENT` forbids removing/changing playlist content
with one stated exception that includes Parts (§6.2.1, lines 3368-3371):

> An EXT-X-PLAYLIST-TYPE tag with a value of EVENT indicates that the Server
> MUST NOT change or remove any part of the Playlist file, with the
> exception of EXT-X-PART tags and Media Metadata tags as described above;
> the Server MAY append lines to the Playlist.

Playlists with EXT-X-I-FRAMES-ONLY: "Playlists that contain the
EXT-X-I-FRAMES-ONLY tag SHOULD NOT use Partial Segments" (§4.4.4.9).

### 2.2 Blocking Playlist Reload — §6.2.5.2 (server, lines 3757-3816) and §6.3.8 (client, lines 4235-4285)

**Advertisement:** CAN-BLOCK-RELOAD=YES on EXT-X-SERVER-CONTROL.

**Query parameters (both reserved under the `_HLS_` prefix, §6.2.5, line
3704-3706: "Query parameters for Playlist requests that begin with the
string `_HLS_` are reserved by this specification. Currently-defined
Delivery Directives are `_HLS_skip`, `_HLS_msn` and `_HLS_part`."):**

- **`_HLS_msn`** — decimal-integer value `M`. "When the Playlist URI
  contains an `_HLS_msn` directive and no `_HLS_part` directive, the Server
  **MUST** defer responding to the request until the Playlist contains a
  Media Segment with a Media Sequence Number of M or later or it responds
  with an error."
- **`_HLS_part`** — decimal-integer value `N`, MAY additionally appear.
  "When the Playlist URI contains both an `_HLS_msn` directive and an
  `_HLS_part` directive, the Server **MUST** defer responding to the request
  until the Playlist contains the Partial Segment with Part Index N and with
  a Media Sequence Number of M or later or it responds with an error."

**Server required behaviour:**

- "If the Client requests a Part Index greater than that of the final
  Partial Segment of the Parent Segment, the Server **MUST** treat the
  request as one for Part Index 0 of the following Parent Segment."
- "The Server **MUST** deliver the entire Playlist, even if the requested
  Media Segment is not the last one in the Playlist, and even if it is no
  longer in the Playlist."
- "A Server **MUST** ignore `_HLS_msn` and `_HLS_part` if the Playlist
  contains an EXT-X-ENDLIST tag."
- "If the `_HLS_msn` is greater than the Media Sequence Number of the last
  Media Segment in the current Playlist plus two, or if the `_HLS_part`
  exceeds the last Partial Segment in the current Playlist by the Advance
  Part Limit, then the server **SHOULD** immediately return Bad Request,
  such as HTTP 400. The Advance Part Limit is three divided by the Part
  Target Duration if the Part Target Duration is less than one second, or
  three otherwise."
- "If the Playlist URI contains an `_HLS_part` directive but no `_HLS_msn`
  directive, the Server **MUST** return Bad Request, such as HTTP 400."
- "A Server that cannot provide the requested Playlist after blocking for
  more than three Target Durations **SHOULD** return Service Unavailable,
  such as HTTP 503."
- "Playlists that contain the EXT-X-I-FRAMES-ONLY tag **SHOULD** support
  Blocking Playlist Reload using the `_HLS_msn` directive if other
  Renditions in the presentation contain CAN-BLOCK-RELOAD."

**Client required/recommended behaviour (§6.3.8):**

- "Clients **MUST NOT** request Blocking Playlist Reloads unless the
  Playlist contains an EXT-X-SERVER-CONTROL tag with a CAN-BLOCK-RELOAD=YES
  attribute."
- "If Blocking Playlist Reloads are supported, Clients **SHOULD** use the
  `_HLS_msn` Delivery Directive (and `_HLS_part`, if the Playlist contains
  Partial Segments) to obtain Playlist updates in preference to the polling
  regime described in Section 6.3.4."
- "If up-to-date information on the next expected Media Sequence Number of a
  Rendition is not available, a Client **SHOULD** use a tune-in algorithm
  such as the one described in Appendix C to obtain a recent version of the
  Playlist."

### 2.3 Playlist Delta Updates — §6.2.5.1 (server, lines 3708-3756) and §6.3.7 (client, lines 4208-4234)

**Advertisement:** CAN-SKIP-UNTIL (Skip Boundary) on EXT-X-SERVER-CONTROL,
optionally with CAN-SKIP-DATERANGES=YES.

**What may be skipped:**

> The Playlist Delta Update is a version of the Playlist in which Media
> Segments that are further from the end of the last (Parent) Media Segment
> in the Playlist than the Skip Boundary (Section 4.4.3.8), as well as their
> associated tags, are replaced by an EXT-X-SKIP tag (Section 4.4.5.2).

With `_HLS_skip=v2`:

> the Playlist Delta Update additionally **MUST NOT** contain EXT-X-DATERANGE
> tags that were added to the Playlist more than CAN-SKIP-UNTIL seconds
> before the Playlist request. The RECENTLY-REMOVED-DATERANGES attribute of
> the EXT-X-SKIP tag **MUST** list the date ranges that were removed from the
> Playlist within CAN-SKIP-UNTIL seconds of the Playlist request.

**What must not be skipped:** "All tags that were not skipped **MUST** remain
in the Playlist Delta Update."

**Server request handling:**

- "When a Server receives a request for a Playlist containing the
  CAN-SKIP-UNTIL attribute but no EXT-X-ENDLIST tag, and the requested URI
  contains an `_HLS_skip` directive whose value is YES or v2, it **MUST**
  respond with a Playlist Delta Update."
- "A Server **MUST** ignore the `_HLS_skip` directive if the Playlist does
  not contain the CAN-SKIP-UNTIL attribute, or if it contains an
  EXT-X-ENDLIST tag."

**Client request form (§6.3.7):**

- "If a Media Playlist file contains an EXT-X-SERVER-CONTROL tag with a
  CAN-SKIP-UNTIL attribute and no EXT-X-ENDLIST tag, a Client **MAY** use the
  `_HLS_skip` Delivery Directive to request Playlist Delta Updates."
- "A Client **SHOULD NOT** request a Playlist Delta Update unless it already
  has a version of the Playlist that is no older than one-half of the Skip
  Boundary."
- Segments-only skip: `_HLS_skip=YES` on the Media Playlist URI.
- Segments + DATERANGE skip: `_HLS_skip=v2`, only valid if
  CAN-SKIP-DATERANGES=YES was advertised.
- "A Client **MUST** merge the contents of a Playlist Delta Update with its
  previous version of the Playlist to form an up-to-date version of the
  Playlist. If a Client receives a Playlist containing an EXT-X-SKIP tag and
  finds that it does not already have all of the information that was
  skipped, it **MUST** obtain a complete copy of the Playlist by reissuing
  its Playlist request without the `_HLS_skip` directive."

### 2.4 Preload Hints — §6.2.6 (lines 3817-3863) and §6.3.8 (client portion, lines 4261-4285)

**What may be hinted:** TYPE=PART (a Partial Segment) or TYPE=MAP (a Media
Initialization Section) — per the EXT-X-PRELOAD-HINT attribute table above.

**Server obligations once hinted:**

- "A hinted resource **MUST** be available for request when its
  EXT-X-PRELOAD-HINT tag is added to the Playlist."
- "When processing requests for a URI or a byte range of a URI that includes
  one or more Partial Segments that are not yet completely available to be
  sent — such as requests made in response to an EXT-X-PRELOAD-HINT tag —
  the server **MUST** refrain from transmitting any bytes belonging to a
  Partial Segment until all bytes of that Partial Segment can be transmitted
  at the full speed of the link to the client. If the requested range
  includes more than one Partial Segment then the server **MUST** enforce
  this delivery guarantee for each Partial Segment in turn. This enables the
  client to perform accurate Adaptive Bit Rate (ABR) measurements."
- "The Server **SHOULD NOT** hint a byte range that it does not expect its
  clients to require in the near term."
- "The server **SHOULD** respond with 'Not Found' (such as HTTP 404) to a
  request for a resource that it cannot find and that is not specified by an
  EXT-X-PRELOAD-HINT tag in an active Media Playlist."
- "A server **MAY** choose not to publish previously-hinted resources if the
  planned segmentation changes, such as the case of early return from an ad.
  The server **SHOULD** respond to client requests for those resources with
  'Not Found' (such as HTTP 404)."
- "If a Partial Segment is created as a sub-range of a larger resource and
  its length is not known at the time that its hint is added to the
  Playlist, the BYTERANGE-LENGTH attribute **SHOULD** be omitted. The
  BYTERANGE-OFFSET **SHOULD** indicate the Partial Segment's starting offset
  into the larger resource." (Note: the spec text here says
  "BYTERANGE-OFFSET"; the attribute defined in §4.4.5.3 is named
  `BYTERANGE-START` — transcribed exactly as it appears in both places,
  without reconciling the naming.)
- "The Server **SHOULD NOT** add more than one EXT-X-PRELOAD-HINT tag with
  the same TYPE to a Playlist."
- "A Playlist containing an EXT-X-ENDLIST tag **MUST NOT** contain an
  EXT-X-PRELOAD-HINT tag" (§4.4.5.3).

**Client obligations (§6.3.8):**

- "Clients **MUST** ignore EXT-X-PRELOAD-HINT tags with unrecognized TYPE
  attributes. Clients **SHOULD** ignore all but the first EXT-X-PRELOAD-HINT
  tag in a Playlist with a particular TYPE attribute."
- TYPE=PART: "a Client with sufficient space in its download pipeline that
  is not already loading the hinted resource **SHOULD** request it. This
  will typically happen at the same time as its blocking request for the
  next Playlist update."
- TYPE=MAP: "a Client with sufficient space in its download pipeline that
  has not already cached the hinted Media Initialization Section **SHOULD**
  request it."
- "A Client **SHOULD** cancel a request for a hinted resource if it is not
  present in a subsequent Playlist update, such as in an EXT-X-PRELOAD-HINT
  tag or as part of another tag such as EXT-X-PART. The client **SHOULD**
  ignore the results of such requests."
- "A Client **SHOULD** recognize when a Partial Segment indicated by an
  EXT-X-PART tag is a sub-range of a hint download and obtain the Partial
  Segment from the hint download. Clients **SHOULD** recognize contiguous
  ranges between existing Partial Segments and Partial Segment hints and
  avoid duplicate downloads."

### 2.5 Rendition Reports

**What must be reported (§4.4.5.4):** URI (relative to the containing
Playlist), LAST-MSN (Media Sequence Number of the last Media Segment in the
Rendition, or of the last Partial Segment if the Rendition has Partial
Segments), and LAST-PART (Part Index of the last Partial Segment, REQUIRED
only if the Rendition contains a Partial Segment). A server MAY omit any
attribute, mandatory or not, whose value is unchanged from the Rendition
Report of the Media Playlist the tag is being added to (§4.4.5.4, quoted
above in §1.6).

**When (Appendix B.1, lines 5618-5629, outside the assigned range — the
Section 4/6 tag and mechanism text does not itself state a per-Playlist
reporting cardinality, only the per-attribute contents already quoted
above; the "each Media Playlist reports every other Rendition" cardinality
is stated as a Server Configuration Profile requirement, not as an
unconditional MUST elsewhere in the document):**

> Each Media Playlist **MUST** contain one EXT-X-RENDITION-REPORT tag for
> each Media Playlist (Rendition) in the Multivariant Playlist, except for
> the Media Playlist to which the EXT-X-RENDITION-REPORT tag is being added,
> and Playlists that contain the EXT-X-I-FRAMES-ONLY tag.
>
> Rendition reports for Media Playlists containing the EXT-X-I-FRAMES-ONLY
> tag **SHOULD** be provided as well, in which case, each Media Playlist
> **MUST** additionally contain one EXT-X-RENDITION-REPORT tag for each
> EXT-X-I-FRAMES-ONLY Media Playlist in the Multivariant Playlist, except for
> the Media Playlist to which the EXT-X-RENDITION-REPORT tag is being added.

The rationale given for requiring this (same appendix, lines 5631-5635):

> Were an EXT-X-RENDITION-REPORT not available, a Client would need to use a
> tune-in algorithm such as the one described in Appendix C in order to
> guarantee that the Playlist it loads is up to date. To spare the Client the
> complexity and delay of performing tune-in, servers are required to
> provide Rendition Reports as described above.

This "servers are required" statement is scoped to the Low-Latency Server
Configuration Profile (Appendix B.1) — it is not phrased as a document-wide
MUST outside that profile. Sections 4.4.5.4, 6.2.5, and 6.2.6 (the assigned
ranges) are silent on when a Rendition Report must be added versus merely
defining its contents once present; that silence is preserved here rather
than filled in.

---

## 3. Numeric relationships between low-latency parameters

These are the exact stated relationships, gathered from §4.4.3.7 and
§4.4.3.8:

| Relationship | Direction | Stated requirement | Source |
|---|---|---|---|
| Skip Boundary (CAN-SKIP-UNTIL) vs. Target Duration | Skip Boundary ≥ 6 × Target Duration | MUST | §4.4.3.8 |
| HOLD-BACK vs. Target Duration | HOLD-BACK ≥ 3 × Target Duration | MUST | §4.4.3.8 |
| HOLD-BACK default (if attribute absent) | = 3 × Target Duration | (implied default) | §4.4.3.8 |
| PART-HOLD-BACK vs. Part Target Duration | PART-HOLD-BACK ≥ 2 × Part Target Duration | MUST | §4.4.3.8 |
| PART-HOLD-BACK vs. Part Target Duration (stronger) | PART-HOLD-BACK ≥ 3 × Part Target Duration | SHOULD | §4.4.3.8 |
| PART-HOLD-BACK with multiple Part Target Durations | PART-HOLD-BACK ≥ 3 × max(Part Target Duration across Renditions) | SHOULD | §4.4.3.8 |
| Partial Segment DURATION vs. Part Target Duration (upper bound) | DURATION ≤ Part Target Duration | MUST | §4.4.4.9 |
| Partial Segment DURATION vs. Part Target Duration (lower bound) | DURATION ≥ 85% × Part Target Duration, except INDEPENDENT=YES, GAP=YES, the segment immediately before a GAP=YES segment, and the final Partial Segment of any Parent Segment | MUST | §4.4.4.9 |
| Advance Part Limit (Blocking Reload) | 3 / Part Target Duration if Part Target Duration < 1s, else 3 | (definition, used in a SHOULD-return-400 rule) | §6.2.5.2 |
| `_HLS_msn` overshoot bound (Blocking Reload) | server SHOULD return 400 if requested MSN > last MSN in current Playlist + 2 | SHOULD | §6.2.5.2 |
| `_HLS_part` overshoot bound (Blocking Reload) | server SHOULD return 400 if requested Part Index exceeds the last Partial Segment by more than the Advance Part Limit | SHOULD | §6.2.5.2 |
| Blocking Reload maximum hold time | server SHOULD return 503 if it cannot provide the Playlist after blocking more than 3 × Target Duration | SHOULD | §6.2.5.2 |
| EXT-X-PART removal threshold | EXT-X-PART tags SHOULD be removed once more than 3 × Target Duration from the end of the Playlist | SHOULD | §6.2.2 |
| Post-removal Partial Segment availability | client MUST be able to download the Partial Segment for ≥ 3 × Target Duration after its EXT-X-PART tag is removed | MUST | §6.2.2 |
| Client Delta-Update eligibility | Client SHOULD NOT request a Delta Update unless its cached Playlist is no older than 0.5 × Skip Boundary | SHOULD NOT (i.e. negative form of a SHOULD) | §6.3.7 |
| New Partial Segment publication cadence | server MUST add a new Partial Segment within 1 × Part Target Duration of the previous one | MUST | §6.2.1 |

Notably, the spec states **no direct numeric relationship between PART-TARGET
(Part Target Duration) and TARGETDURATION** other than the indirect ones
above (PART-HOLD-BACK is bounded relative to Part Target Duration; EXT-X-PART
removal and Blocking Reload timeouts are bounded relative to Target Duration;
DURATION of a Partial Segment is bounded relative to Part Target Duration).
No sentence in any of the transcribed ranges states, e.g., "Part Target
Duration MUST be less than Target Duration" or gives a fixed ratio between
the two — that silence is as-transcribed, not filled in.

---

## 4. Cross-reference

- Version-9/10 findings above are consistent with, and duplicate for
  convenience, the EXT-X-SKIP rows already transcribed in
  `docs/version-compatibility.md` (source lines 4490-4607).
- Media Segment / Partial Segment format constraints (Section 3) are
  transcribed in full in `docs/media-segment-formats.md`.

## 5. Verification

This document was produced by reading the assigned line ranges directly from
`specs/ietf_draft_pantos_hls_rfc8216bis.txt`, plus the supporting ranges
listed in the header (Section 3.2, the Section 6.2.1/6.2.2 Partial-Segment
removal rules, and Appendix B.1), each explicitly marked where used. A
document-wide `grep -n "compatibility version"` was run to confirm no
low-latency tag other than EXT-X-SKIP is mentioned in any
"REQUIRES a compatibility version..." sentence, and Section 8 (lines
4490-4607) was read in full to confirm the same for the
"MUST indicate an EXT-X-VERSION of N or higher" form.
