# HLS Interstitials — Appendix D (+ Appendix F) Reference

Transcribed from `draft-pantos-hls-rfc8216bis-22` (1 May 2026), **Appendix D
"Interstitials"** (§D.1–§D.7) in its entirety, plus the closely-related
**Appendix F "Preloading HLS Date Range Resources"** (the `CLASS
"com.apple.hls.preload"` mechanism that Appendix D's Preload EXT-X-DATERANGE
text points at).
Source: `specs/ietf_draft_pantos_hls_rfc8216bis.txt`, lines 5786-6307
(Appendix D) and lines 6486-6556 (Appendix F). This is the **same vendored
file** `broadcast-hls/docs/playlist-tags.md` transcribes §4.4 from — verified
byte-identical to `draft-pantos-hls-rfc8216bis-22` fetched fresh from
`https://www.ietf.org/archive/id/draft-pantos-hls-rfc8216bis-22.txt` (0-line
diff) at transcription time.

This is the **base `EXT-X-DATERANGE` tag** (`ID`, `CLASS`, `START-DATE`,
`CUE`, `END-DATE`, `DURATION`, `PLANNED-DURATION`, `X-<extension-attribute>`,
`SCTE35-CMD`/`SCTE35-OUT`/`SCTE35-IN`, `END-ON-NEXT`) that Interstitials layer
on top of via `CLASS="com.apple.hls.interstitial"` — the base tag is already
transcribed in `broadcast-hls/docs/playlist-tags.md` §4.4.5.1 and is NOT
repeated here; only the interstitial-specific `CLASS` schema and its `X-`
attributes are new material.

As with `playlist-tags.md`, this is a literal transcription for
implementation reference: prose is quoted or paraphrased only where the spec
itself is prose (constraints, MUST/SHOULD rules); attribute tables are
transcribed field-by-field from the spec text.

---

## D.1. Overview (lines 5792-5840)

> "Content producers can insert separate interstitial content into their
> primary presentations in order to display advertising, branding, or other
> information to viewers."
>
> "Servers can schedule interstitials by placing EXT-X-DATERANGE tags into
> the Media Playlists of the primary asset."
>
> "Interstitials themselves are self-contained media assets. They can be
> scheduled anywhere on the timeline of a primary media asset. While
> interstitials MUST be VOD assets, they can be scheduled against either VOD
> or live primary content (including Low-Latency HLS streams (Appendix
> B.1))."
>
> "Interstitials are specified by URI. The client SHOULD load the resource
> specified by the URI after it buffers the primary asset to the scheduled
> interstitial playback time. This allows for late binding to interstitial
> content. Since an interstitial is described by a single URI, a server can
> respond to it with a limited number of HTTP redirects without a major
> impact on performance."
>
> "An interstitial request can either be for a single interstitial asset or
> a list of assets. In the second case, the composition of the list MAY be
> determined when the server responds to the interstitial request."
>
> "Each interstitial asset is a Playlist, usually a Multivariant Playlist.
> The primary media asset and the interstitial asset both provide sets of
> Variant Streams. The two sets MAY have different characteristics. The bit
> rates of the Variant Streams of the interstitial asset may not exactly
> match those of the primary asset, but they SHOULD allow for effective bit
> rate adaptation in similar network conditions. It is RECOMMENDED that the
> set of languages in Rendition Groups of a specific type within the
> interstitial asset match those used in the primary media asset.
> Interstitials MAY use the different codecs from the primary content,
> although using different codecs will cause transition delays on certain
> devices."
>
> "Devices that do not implement HLS interstitial support SHOULD ignore
> server-generated interstitial events when playing a primary asset."
>
> "Interstitials scheduled inside other interstitials MUST be ignored by
> clients."

---

## D.2. EXT-X-DATERANGE Schema for Interstitials (lines 5841-6108)

> "The server MAY insert EXT-X-DATERANGE tags according to the rules in this
> section to tell the client to schedule interstitial playback."
>
> "An Interstitial EXT-X-DATERANGE tag MUST have a CLASS attribute whose
> value is `"com.apple.hls.interstitial"`. This class defines the following
> attributes:"

| Attribute | Value type | Required/Optional | Meaning |
|-----------|------------|--------------------|---------|
| `X-ASSET-URI` | quoted-string, absolute URI | one of `X-ASSET-URI`/`X-ASSET-LIST` REQUIRED, mutually exclusive | Identifies a single interstitial asset. An Interstitial `EXT-X-DATERANGE` tag **MUST** have either `X-ASSET-URI` or `X-ASSET-LIST`; it **MUST NOT** have both. |
| `X-ASSET-LIST` | quoted-string URI to a JSON object; MAY be absolute or relative to the Playlist URI | one of `X-ASSET-URI`/`X-ASSET-LIST` REQUIRED, mutually exclusive | Points at a JSON object (schema below). Since every Playlist with Date Ranges **MUST** have identical `EXT-X-DATERANGE` tags (§6.2.4), a relative URI **MUST** resolve successfully against every Playlist URI. |
| `X-RESUME-OFFSET` | signed-decimal-floating-point (seconds) | OPTIONAL | Time offset, from where the interstitial playback was scheduled on the primary player timeline, at which primary playback is to resume following the interstitial. Typical value is zero. If absent, its value is considered to be the duration of the interstitial (appropriate for live content kept at a constant delay from the live edge, or VOD playback where the interstitial exactly replaces primary content). |
| `X-PLAYOUT-LIMIT` | decimal-floating-point (seconds) | OPTIONAL | Limit for the playout time of the entire interstitial. If present, the client **SHOULD** end the interstitial on reaching that offset from its start. Otherwise the interstitial **MUST** end upon reaching the end of the interstitial asset(s). |
| `X-SNAP` | enumerated-string-list of Snap Identifiers: `OUT`, `IN` | OPTIONAL | See snap semantics below. |
| `X-RESTRICT` | enumerated-string-list of Navigation Restriction Identifiers: `SKIP`, `JUMP` | OPTIONAL | Enforced at the player UI level; see restriction semantics below. |
| `X-CONTENT-MAY-VARY` | quoted-string, valid values `"YES"`/`"NO"` | OPTIONAL, default (if missing) treated as `"YES"` | `"NO"` = all players get the same interstitial content (clients MAY use this as a signal that playback can be coordinated across multiple players); `"YES"` = content may vary between clients. |
| `X-TIMELINE-OCCUPIES` | quoted-string, valid values `"POINT"`/`"RANGE"` | OPTIONAL, default (if missing) treated as `"POINT"` | Whether the interstitial should be presented in a timeline UI as a single point or a range. If the interstitial has a positive non-zero resumption offset, the client MAY instead use `"RANGE"`. |
| `X-TIMELINE-STYLE` | quoted-string, valid values `"HIGHLIGHT"`/`"PRIMARY"` | OPTIONAL, default (if missing) treated as `"HIGHLIGHT"` | Whether the interstitial is intended to be presented in a timeline UI as distinct from the content (`"HIGHLIGHT"`) or not differentiated (`"PRIMARY"`). |
| `X-<extension-attribute>` | quoted-string, hexadecimal-sequence, or signed-decimal-floating-point | OPTIONAL | Extension attributes are allowed; see the base `EXT-X-DATERANGE` discussion (§4.4.5.1, `broadcast-hls/docs/playlist-tags.md`). |

> "For X-CONTENT-MAY-VARY, X-TIMELINE-OCCUPIES, and X-TIMELINE-STYLE an
> unrecognized value should be treated as if the value were missing."

### X-ASSET-LIST JSON schema (lines 5878-5905)

> "The JSON object MUST contain a key/value pair whose key is `"ASSETS"` and
> whose value is a JSON array of Asset-Description JSON objects. The JSON
> object MAY also contain a key/value pair whose key is `"SKIP-CONTROL"` and
> whose value is a Skip-Control JSON object. (Note that keys in a JSON
> object are case-sensitive.)"
>
> "Each Asset-Description JSON object MUST have a `"URI"` member whose value
> is a quoted-string absolute URI for a single interstitial asset, and a
> `"DURATION"` member whose value is a decimal-floating-point indicating the
> duration of the interstitial asset in seconds."
>
> "The keys for the Skip-Control object are described in Appendix D.3. …"
>
> "The client SHOULD play the interstitial assets back-to-back in the order
> that they appear in the ASSETS array."

| JSON key | Type | Required/Optional | Meaning |
|---|---|---|---|
| `ASSETS` | array of Asset-Description objects | REQUIRED | Interstitial assets, played back-to-back in array order. |
| `ASSETS[].URI` | quoted-string, absolute URI | REQUIRED | Single interstitial asset location. |
| `ASSETS[].DURATION` | decimal-floating-point (seconds) | REQUIRED | Duration of that asset. |
| `SKIP-CONTROL` | Skip-Control object (§D.3) | OPTIONAL | Overrides `X-SKIP-CONTROL-*` attributes. |

### X-SNAP semantics (lines 5920-5936)

> "If the list contains OUT then the client SHOULD locate the segment
> boundary closest to the START-DATE of the interstitial in the Media
> Playlist of the primary content and transition to the interstitial at that
> boundary. If more than one Media Playlist is contributing to playback
> (audio plus video for example), the client SHOULD transition at the
> earliest segment boundary."
>
> "If the list contains IN then the client SHOULD locate the segment
> boundary closest to the scheduled resumption point from the interstitial
> in the Media Playlist of the primary content and resume playback of
> primary content at that boundary. If more than one Media Playlist is
> contributing to playback, the client SHOULD transition at the latest
> segment boundary."

### X-RESTRICT semantics (lines 5938-5952)

> "If the list contains SKIP then while the interstitial is being played,
> the client MUST NOT allow the user to seek forward from the current
> playhead position or set the rate to greater than the regular playback
> rate until playback reaches the end of the interstitial."
>
> "If the list contains JUMP then the client MUST NOT allow the user to seek
> from a position in the primary asset earlier than the START-DATE attribute
> to a position after it without first playing the interstitial asset, even
> if the interstitial at START-DATE was played through earlier. If the user
> attempts to seek across more than one interstitial, the client SHOULD
> choose at least one interstitial to play before allowing the seek to
> complete."
>
> "A client with specific knowledge of the presentation rules for an asset
> MAY override restrictions specified by the X-RESTRICT attribute if such an
> action is consistent with those rules."

### CUE / scheduling / ordering rules (lines 5954-5985, prose transcribed in full)

> "For an Interstitial EXT-X-DATERANGE tag, the action whose trigger time is
> controlled by the CUE attribute is the playback of the interstitial."
>
> "An Interstitial whose EXT-X-DATERANGE tag does not contain a CUE
> attribute SHOULD be scheduled for playback at the start of the Date
> Range."
>
> "Multiple interstitials that are scheduled for the same time SHOULD be
> played in the order that their EXT-X-DATERANGE tags appear in the
> Playlist. In that case, X-RESUME-OFFSET values are cumulative. Multiple
> interstitials that are scheduled for the same time MUST occur in the same
> order in all Media Playlists of a Multivariant Playlist."
>
> "If the resumption point for an interstitial (or group of interstitials
> scheduled for the same time) precedes the start of the interstitial(s)
> then that interstitial or group of interstitials SHOULD NOT be played
> again unless the user explicitly moves the playhead back to a position
> prior to the start time of the interstitial(s)."
>
> "If present, the duration (or planned duration) of the Date Range SHOULD
> be the duration of the interstitial asset(s), even if a CUE attribute
> allows the interstitial to start at some time other than the START-DATE."

### Preload EXT-X-DATERANGE cross-reference (lines 5987-6003)

> "A Server MAY add a Preload EXT-X-DATERANGE tag to indicate that the
> client SHOULD preload the URI specified by the X-ASSET-URI or
> X-ASSET-LIST attribute of an existing or upcoming Interstitial
> EXT-X-DATERANGE tag. See Appendix F for more information."

The Preload `EXT-X-DATERANGE` tag itself is `CLASS="com.apple.hls.preload"`,
a **separate** DATERANGE class defined in Appendix F — transcribed below in
its own section, not part of the Interstitial class.

> "The server SHOULD specify a preload range that ends at or shortly before
> the interstitial is expected to be scheduled. The duration of the period
> SHOULD be as long as the preload could be considered valid before that
> point (even if practically the client will not be able to issue it that
> far in advance)."
>
> "The client SHOULD choose a random point inside the Preload Date Range and
> preload the URI when the current playhead position passes that point. A
> client SHOULD NOT load an Interstitial any later than it would have in the
> absence of the Preload EXT-X-DATERANGE tag."
>
> "After obtaining the preload date range but before selecting a random
> point within it, the client SHOULD:
>
> A. Ensure that the effective start of the preload range is greater than or
>    equal to the current playhead position, to avoid selecting a preload
>    time in the past.
>
> B. Ensure that the effective end of the preload range occurs before the
>    client expects to normally resolve the interstitial.
>
>    1. If the START-DATE of the interstitial is already known, the client
>       SHOULD predict how long it would wait before resolving the
>       interstitial if it were not preloaded, and ensure that the end of
>       the preload range is no greater than the current playhead position
>       plus that wait time.
>
>    2. In the case where the START-DATE of the interstitial is not yet
>       known, the client SHOULD assume that it will appear the next time
>       that the Playlist is reloaded, with the same date as the end of the
>       preload range, and then proceed as in the first case."
>
> "Clients SHOULD ignore Preload EXT-X-DATERANGE tags if the Playlist
> contains an EXT-X-ENDLIST tag."
>
> "If the X-RESUME-OFFSET is not present and X-PLAYOUT-LIMIT specifies a
> value less than the total duration of the Interstitial, then the value of
> the resumption offset will be the playout limit."

---

## D.3. Skip button control for an Interstitial (lines 6109-6145)

> "Content producers can allow clients to skip an Interstitial. This section
> describes how to configure a Skip button. The following attributes are
> defined to help control the skip button behavior."

| Attribute | Value type | Meaning |
|---|---|---|
| `X-SKIP-CONTROL-OFFSET` | decimal-integer (seconds) | Seconds of interstitial content played before a skip button is displayed. `0` = display the skip button immediately upon entering the interstitial. |
| `X-SKIP-CONTROL-DURATION` | decimal-integer (seconds) | Seconds the skip button should be displayed for. Absent = displayed for the entire duration of the interstitial. |
| `X-SKIP-CONTROL-LABEL-ID` | quoted-string; characters restricted to `[a-z]`, `[A-Z]`, `-`, `_` | Key a client application uses to render a localized label for the skip button. Absent = client applies a default label. |

> "One or more attributes defined in this section MAY be overridden by
> supplying a Skip-Control object in the X-ASSET-LIST JSON object. The keys
> of the Skip-Control object are the attribute names above with
> `"X-SKIP-CONTROL-"` removed. That is, `"OFFSET"`, `"DURATION"`, and
> `"LABEL-ID"`."

---

## D.4. Interstitial query parameters (lines 6146-6187)

> "Packagers producing Interstitial EXT-X-DATERANGE tags SHOULD ensure that
> X-ASSET-URI and X-ASSET-LIST requests contain an `_HLS_interstitial_id`
> query parameter whose value is the ID attribute value of the
> EXT-X-DATERANGE tag with the quotation marks removed. This supports
> interoperability between content producers and decisioning servers. To
> prevent conflicts with future versions of this specification packagers
> SHOULD NOT define query parameters that begin with the string `"_HLS_"`."
>
> "Certain clients support setting the X-PLAYBACK-SESSION-ID request header
> with a common, globally-unique value on every HTTP request associated with
> a particular playback session. Such clients SHOULD add an
> `_HLS_primary_id` query parameter to interstitial requests whose value
> matches the X-PLAYBACK-SESSION-ID of the primary playback session. This
> provides useful context for decisioning servers."
>
> "Clients that cannot set the X-PLAYBACK-SESSION-ID request header SHOULD
> create a globally-unique value for every primary playback session, and
> provide this value as an `_HLS_primary_id` query parameter on both the
> request for the primary asset and interstitial requests made on behalf of
> that asset."
>
> "Interstitial requests are X-ASSET-URI requests, X-ASSET-LIST requests,
> and requests for a URI from an X-ASSET-LIST Asset-Description JSON
> object."
>
> "Clients starting playback of a live stream in an interstitial SHOULD
> ensure that X-ASSET-LIST requests contain an `_HLS_start_offset` query
> parameter whose value is the offset in seconds of the playback start point
> from the beginning of the interstitial. This allows servers to customize
> interstitial content based on the starting offset."

| Query parameter | Added by | Meaning |
|---|---|---|
| `_HLS_interstitial_id` | Packager, on `X-ASSET-URI`/`X-ASSET-LIST` requests | ID attribute value of the `EXT-X-DATERANGE` tag (quotes removed). |
| `_HLS_primary_id` | Client, on interstitial requests | Globally-unique value identifying the primary playback session (mirrors `X-PLAYBACK-SESSION-ID` request header where supported). |
| `_HLS_start_offset` | Client, on `X-ASSET-LIST` requests when starting live playback mid-interstitial | Offset in seconds of the playback start point from the beginning of the interstitial. |

Packagers **SHOULD NOT** define custom query parameters beginning with
`"_HLS_"` (namespace reserved by the spec).

---

## D.5. Client Behavior (lines 6188-6225)

> "If an interstitial specifies a non-zero resume offset and the user tries
> to seek to a time between the start of the interstitial and its resumption
> point on the primary asset timeline, the client SHOULD begin playback from
> the start of the interstitial."
>
> "If a request for either an interstitial asset URI or an asset list URI
> returns an error, the client SHOULD cancel playback of the interstitial
> with a resume offset of 0."
>
> "If a request for the URI of a single asset within an asset list returns
> an error, the client SHOULD skip playback of that asset. When
> X-RESUME-OFFSET is determined by the duration of the interstitial you MAY
> ignore the duration of the skipped asset(s) in computing the duration of
> the interstitial."
>
> "If the JSON object returned by the asset list URI has an empty array as
> the value of the `"ASSETS"` key, the client SHOULD apply the resume offset
> without playing any interstitial content. A resume offset of 0 SHOULD be
> used when no Interstitials are played, unless a value was specified with
> X-RESUME-OFFSET."
>
> "Clients SHOULD allow a generous amount of time (up to a minute) for a
> server to respond to requests for interstitial assets or asset lists, to
> enable the server to perform back-end decisioning. Servers MUST respond
> quickly enough to avoid playback disruptions on the client."

---

## D.6. Example: Interstitial EXT-X-DATERANGE (lines 6226-6245)

> "In this playlist an EXT-X-DATERANGE tag schedules a 15-second ad to play
> four seconds into a six-second primary asset. The client will play the
> interstitial and then resume playback of the primary asset where it left
> off. Seeking and scanning forward will be disabled during interstitial
> playback. The EXT-X-DATERANGE tag includes a vendor-defined beacon
> attribute that can be processed by the client."

```
#EXTM3U
#EXT-X-TARGETDURATION:6
#EXT-X-PROGRAM-DATE-TIME:2020-01-02T21:55:40.000Z
#EXTINF:6,
main1.0.ts
#EXT-X-ENDLIST
#EXT-X-DATERANGE:ID="ad1",CLASS="com.apple.hls.interstitial",
    START-DATE="2020-01-02T21:55:44.000Z",DURATION=15.0,
    X-ASSET-URI="http://example.com/ad1.m3u8",X-RESUME-OFFSET=0,
    X-RESTRICT="SKIP,JUMP",X-COM-EXAMPLE-BEACON=123
```

---

## D.7. Example: Skip button control for an Interstitial (lines 6246-6307)

> "In this playlist an EXT-X-DATERANGE tag schedules two 15-second ads to
> play four seconds into a six-second primary asset. The client will play
> these two interstitial assets one after the other and then resume playback
> of the primary asset where it left off. A Skip-Control overrides values of
> X-SKIP-CONTROL-OFFSET and X-SKIP-CONTROL-DURATION. A Skip button will show
> after 5 seconds into the interstitial and will be displayed for 20
> seconds, or 10 seconds into the second asset, before being taken down.
> "Exit-Label" is the key an application will use to render localized label
> for the skip button."

```
#EXTM3U
#EXT-X-TARGETDURATION:6
#EXT-X-PROGRAM-DATE-TIME:2020-01-02T21:55:40.000Z
#EXTINF:6,
main1.mp4
#EXT-X-ENDLIST
#EXT-X-DATERANGE:ID="ad1",CLASS="com.apple.hls.interstitial",
    START-DATE="2020-01-02T21:55:44.000Z",DURATION=30.0,
    X-ASSET-LIST="http://example.com/adv.json",
    X-RESUME-OFFSET=0,X-SKIP-CONTROL-OFFSET=2,X-SKIP-CONTROL-DURATION=5,
    X-SKIP-CONTROL-LABEL-ID="Exit-Label"
```

Assume `X-ASSET-LIST` JSON contains this:

```json
{
    "ASSETS": [
        {
            "URI" : "http://example.com/ad1.m3u8",
            "DURATION" : 15
        },
        {
            "URI" : "http://example.com/ad2.m3u8",
            "DURATION" : 15
        }
    ],
    "SKIP-CONTROL": {
        "OFFSET": 5,
        "DURATION": 20
    }
}
```

---

## Appendix F. Preloading HLS Date Range Resources (lines 6486-6556)

Included here because Appendix D §D.2 explicitly cross-references it for the
"Preload EXT-X-DATERANGE tag" used to preload upcoming interstitial content
ahead of its scheduled `START-DATE`.

> "An EXT-X-DATERANGE tag with the CLASS `"com.apple.hls.preload"` MAY be
> used to advise clients to preload a URI for a separate EXT-X-DATERANGE tag,
> even before that separate tag is added to a Playlist."
>
> "This class defines the following attributes:"

| Attribute | Value type | Required/Optional | Meaning |
|---|---|---|---|
| `X-URI` | quoted-string | REQUIRED | URI identifying the resource to be preloaded. **SHOULD** include the same query parameters as the URI in the target Date Range. If it does not match the URL(s) to be preloaded in the target `EXT-X-DATERANGE` tag (as specified by its `CLASS`), the client **SHOULD** discard the preload result. |
| `X-TARGET-ID` | quoted-string | REQUIRED | ID of the Date Range for which the resource should be preloaded. |
| `X-TARGET-CLASS` | quoted-string | REQUIRED | `CLASS` of the Date Range with the specified `X-TARGET-ID`. |

> "A Preload EXT-X-DATERANGE tag MUST contain a DURATION or END-DATE
> attribute. A Preload EXT-X-DATERANGE tag MUST NOT contain a CUE attribute
> or an END-ON-NEXT attribute."
>
> "The semantics of when a client should preload the resource specified by a
> Preload EXT-X-DATERANGE tag is determined by the target class."
>
> "A client MAY ignore a Preload EXT-X-DATERANGE tag, unless otherwise
> specified by the target class."
>
> "If the X-TARGET-ID of a Preload EXT-X-DATERANGE tag matches the ID of an
> EXT-X-DATERANGE tag but the X-TARGET-CLASS does not match its CLASS, the
> X-URI SHOULD NOT be preloaded, or if preloaded, SHOULD NOT be used."
>
> "If a Preload EXT-X-DATERANGE tag is removed from a Playlist, a client
> SHOULD discard any resource preloaded for that tag."

---

## Provenance note

`draft-pantos-hls-rfc8216bis` is an IETF Internet-Draft (Informational,
"obsoletes RFC 8216 if approved") — freely redistributable, no paywall. The
vendored copy at `specs/ietf_draft_pantos_hls_rfc8216bis.txt` was verified
byte-identical to `draft-pantos-hls-rfc8216bis-22` fetched directly from
`https://www.ietf.org/archive/id/draft-pantos-hls-rfc8216bis-22.txt` during
this transcription (2026-08-09). Because it is a numbered `-22` draft (not
RFC 8216bis itself, which has not been published as an RFC at time of
writing), attribute definitions here are subject to change in later draft
revisions or a future published RFC — re-verify against the current draft
number before treating this as a permanently frozen wire format.

No values in this document were invented: every attribute name, value type,
and default was read from the quoted source text above. Nothing in this
transcription is marked UNVERIFIED because Appendix D and Appendix F are
both fully present, self-contained prose+table sections in the freely
available source with no gaps requiring a paid spec.
