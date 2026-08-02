# Protocol Version Compatibility

Source: `draft-pantos-hls-rfc8216bis-22` (1 May 2026), Section 8, "Protocol
Version Compatibility" (source file
`specs/ietf_draft_pantos_hls_rfc8216bis.txt`, lines 4490-4607).

This is a transcription. Where the spec text is quoted verbatim it is marked
as a quote; the version table paraphrases only to the extent of turning the
spec's bulleted "MUST indicate an EXT-X-VERSION of N or higher if it
contains:" prose into rows, without altering the meaning of any triggering
condition.

## General compatibility rules (§8, opening paragraphs)

> Protocol compatibility is specified by the EXT-X-VERSION tag. A Playlist
> that contains tags or attributes that are not compatible with protocol
> version 1 MUST include an EXT-X-VERSION tag.

> A client MUST NOT attempt playback if it does not support the protocol
> version specified by the EXT-X-VERSION tag, or unintended behavior could
> occur.

The second sentence is the reason over-declaring a version is harmful, not
merely untidy: a client that supports version N but not the (falsely) declared
version M > N is required to refuse playback entirely, even though it could
have played the content correctly at its true, lower version requirement.

## Version-to-feature table (versions 1-13)

Each row states the exact playlist type (Media Playlist / Multivariant
Playlist / Playlist) the spec uses for that requirement — the type matters,
per the task instructions, and is preserved as written.

| Version | Playlist type required to declare it | Triggering condition (verbatim in meaning) |
|---|---|---|
| 1 | — | No stated requirement. Version 1 is the implicit baseline; the spec does not state a "MUST indicate EXT-X-VERSION of 1 or higher" condition anywhere in this section (the opening rule instead says a Playlist need not carry EXT-X-VERSION at all if it is fully compatible with version 1). |
| 2 | Media Playlist | Contains the IV attribute of the EXT-X-KEY tag. |
| 3 | Media Playlist | Contains floating-point EXTINF duration values. |
| 4 | Media Playlist | Contains the EXT-X-BYTERANGE tag, **or** the EXT-X-I-FRAMES-ONLY tag. |
| 5 | Media Playlist | Contains an EXT-X-KEY tag with a METHOD of SAMPLE-AES, **or** the KEYFORMAT and KEYFORMATVERSIONS attributes of the EXT-X-KEY tag, **or** the EXT-X-MAP tag. |
| 6 | Media Playlist | Contains the EXT-X-MAP tag *in a Media Playlist that does not contain EXT-X-I-FRAMES-ONLY* (i.e., EXT-X-MAP without EXT-X-I-FRAMES-ONLY needs version 6; EXT-X-MAP together with EXT-X-I-FRAMES-ONLY only needs version 5, per row 5 above). |
| 7 | Multivariant Playlist | Contains "SERVICE" values for the INSTREAM-ID attribute of the EXT-X-MEDIA tag. |
| 8 | Playlist | Contains variable substitution. |
| 9 | Playlist | Contains the EXT-X-SKIP tag. |
| 10 | Playlist | Contains an EXT-X-SKIP tag that replaces EXT-X-DATERANGE tags in a Playlist Delta Update. |
| 11 | Playlist | Contains an EXT-X-DEFINE tag with a QUERYPARAM attribute. |
| 12 | Playlist | Contains an attribute whose name starts with "REQ-". |
| 13 | Playlist | Contains an EXT-X-MEDIA tag with an INSTREAM-ID attribute for a non-CLOSED-CAPTIONS TYPE. |

Note on version 6 (stated immediately after the version-6 bullet in the
source, as a note rather than a new triggering condition):

> Note that in protocol version 6, the semantics of the EXT-X-TARGETDURATION
> tag changed slightly. In protocol version 5 and earlier it indicated the
> maximum segment duration; in protocol version 6 and later it indicates the
> maximum segment duration rounded to the nearest integer number of seconds.

This is a semantic change tied to the version number, not an independent
"MUST declare version 6" trigger — it is listed here for completeness since
it appears in the version-6 discussion, but the only stated MUST-trigger for
version 6 is the EXT-X-MAP-without-EXT-X-I-FRAMES-ONLY condition in the table.

## Backward-compatible EXT-X-MEDIA / AUDIO / VIDEO / SUBTITLES note

> The EXT-X-MEDIA tag and the AUDIO, VIDEO, and SUBTITLES attributes of the
> EXT-X-STREAM-INF tag are backward compatible to protocol version 1, but
> playback on older clients may not be desirable. A server MAY consider
> indicating an EXT-X-VERSION of 4 or higher in the Multivariant Playlist but
> is not required to do so.

This is a MAY, not a MUST: these tags/attributes do **not** by themselves
force a minimum EXT-X-VERSION under the rules above (they are compatible with
version 1), but a server is permitted to declare version 4 or higher anyway
if it wants to signal that older clients should not attempt playback.

## Removals

> The PROGRAM-ID attribute of the EXT-X-STREAM-INF and the EXT-X-I-FRAME-
> STREAM-INF tags was removed in protocol version 6.

> The EXT-X-ALLOW-CACHE tag was removed in protocol version 7.

These are the only two removals stated in this section. No other tag or
attribute removal is mentioned in §8.

## Verification

Re-read the transcribed table against source lines 4490-4607
(`specs/ietf_draft_pantos_hls_rfc8216bis.txt`) line by line, confirming:

- Every version 2-13 row matches its corresponding "A Media
  Playlist/Multivariant Playlist/Playlist MUST indicate an EXT-X-VERSION of N
  or higher if it contains:" paragraph and its bullet(s) exactly, including
  which playlist-type noun the spec uses per paragraph.
- Version 1 has no stated MUST-trigger anywhere in the section — confirmed
  absent from source, not omitted by oversight.
- The version-6 EXT-X-TARGETDURATION semantics note is a note attached to the
  version-6 paragraph, not a separate version trigger — confirmed by its
  position directly after the version-6 bullet and before the version-7
  paragraph.
- Both removal statements (PROGRAM-ID at v6, EXT-X-ALLOW-CACHE at v7) are
  transcribed verbatim, and no third removal is present in lines 4490-4607.
- The general compatibility rule and the client MUST-NOT-attempt-playback
  rule are quoted verbatim from the section's opening paragraphs.
- The EXT-X-MEDIA/AUDIO/VIDEO/SUBTITLES backward-compatibility paragraph
  (with its "MAY... but is not required" language) is quoted verbatim.

This verification pass was performed; the table above matches the source line
by line.
