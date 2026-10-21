# I-frame playlists — spec transcriptions (RFC 8216bis + Apple HLS authoring, trick play)

Sources and their limits (written 2026-10-21, BEFORE any implementation, issue #1413):

* **IETF draft-pantos-hls-rfc8216bis (May 2026)**, `specs/ietf_draft_pantos_hls_rfc8216bis.txt`. Verbatim
  blocks; line numbers count `\n`-separated lines (NOT Python `splitlines`, which also splits on form
  feeds and shifts the numbers).
* **Apple, *HLS Authoring Specification for Apple Devices*** (web documentation, no PDF exists):
  items 6.x and the platform amendments are the full-text capture taken from the developer.apple.com
  documentation data (the same capture as `broadcast-hls/docs/multivariant-attribute-rules.md`'s 9.x
  rows). The *I-frame bit rates versus normal bit rates* appendix (the explanation behind item 6.5) comes
  from the Appendixes page (`.../hls-authoring-specification-for-apple-devices-appendixes`), extracted
  twice independently; the two extractions were identical. A web page cannot be page-verified like a PDF —
  this is the strongest check available for it.

## 1. RFC 8216bis §4.4.3.6 `EXT-X-I-FRAMES-ONLY` (lines 1181-1211)

```
4.4.3.6.  EXT-X-I-FRAMES-ONLY

   The EXT-X-I-FRAMES-ONLY tag indicates that each Media Segment in the
   Playlist describes a single I-frame.  I-frames are encoded video
   frames whose decoding does not depend on any other frame.  I-frame
   Playlists can be used for trick play, such as fast forward, rapid
   reverse, and scrubbing.

   The EXT-X-I-FRAMES-ONLY tag applies to the entire Playlist.  Its
   format is:

   #EXT-X-I-FRAMES-ONLY

   In a Playlist with the EXT-X-I-FRAMES-ONLY tag, the Media Segment
   duration (EXTINF tag value) is the time between the presentation time
   of the I-frame in the Media Segment and the presentation time of the
   next I-frame in the Playlist, or the end of the presentation if it is
   the last I-frame in the Playlist.

   Media resources containing I-frame segments MUST begin with either a
   Media Initialization Section (Section 3) or be accompanied by an EXT-
   X-MAP tag indicating the Media Initialization Section so that clients
   can load and decode I-frame segments in any order.  The byte range of
   an I-frame segment with an EXT-X-BYTERANGE tag applied to it
   (Section 4.4.4.2) MUST NOT include its Media Initialization Section;
   clients can assume that the Media Initialization Section is defined
   by the EXT-X-MAP tag, or is located between the start of the resource
   and the offset of the first I-frame segment in that resource.

   Use of the EXT-X-I-FRAMES-ONLY REQUIRES a compatibility version
   number of 4 or greater.
```

## 2. RFC 8216bis §4.4.6.3 `EXT-X-I-FRAME-STREAM-INF` (lines 3032-3061)

```
4.4.6.3.  EXT-X-I-FRAME-STREAM-INF

   The EXT-X-I-FRAME-STREAM-INF tag identifies a Media Playlist file
   containing the I-frames of a multimedia presentation.  It stands
   alone, in that it does not apply to a particular URI in the
   Multivariant Playlist.  Its format is:

   #EXT-X-I-FRAME-STREAM-INF:<attribute-list>

   All attributes defined for the EXT-X-STREAM-INF tag (Section 4.4.6.2)
   are also defined for the EXT-X-I-FRAME-STREAM-INF tag, except for the
   FRAME-RATE, AUDIO, SUBTITLES, and CLOSED-CAPTIONS attributes.  In
   addition, the following attribute is defined:

      URI

      The value is a quoted-string containing a URI that identifies the
      I-frame Media Playlist file.  That Playlist file MUST contain an
      EXT-X-I-FRAMES-ONLY tag.

   Every EXT-X-I-FRAME-STREAM-INF tag MUST include a BANDWIDTH attribute
   and a URI attribute.

   The provisions in Section 4.4.6.2.1 also apply to EXT-X-I-FRAME-
   STREAM-INF tags with a VIDEO attribute.

   A Multivariant Playlist that specifies alternative VIDEO Renditions
   and I-frame Playlists SHOULD include an alternative I-frame VIDEO
   Rendition for each regular VIDEO Rendition, with the same NAME and
   LANGUAGE attributes.
```

## 3. RFC 8216bis §4.4.4.2 `EXT-X-BYTERANGE` (lines 1369-1404)

```
4.4.4.2.  EXT-X-BYTERANGE

   The EXT-X-BYTERANGE tag indicates that a Media Segment is a sub-range
   of the resource identified by its URI.  It applies only to the next
   URI line that follows it in the Playlist.  Its format is:

   #EXT-X-BYTERANGE:<n>[@<o>]

   where n is a decimal-integer indicating the length of the sub-range
   in bytes.  If present, o is a decimal-integer indicating the start of
   the sub-range, as a byte offset from the beginning of the resource.
   If o is not present, the sub-range begins at the next byte following
   the sub-range of the previous Media Segment.

   If o is not present, a previous Media Segment MUST appear in the
   Playlist file and MUST be a sub-range of the same media resource, or
   the Media Segment is undefined and the client MUST fail to parse the
   Playlist.

   A Media Segment without an EXT-X-BYTERANGE tag consists of the entire
   resource identified by its URI.

   Use of the EXT-X-BYTERANGE tag REQUIRES a compatibility version
   number of 4 or greater.







Pantos                   Expires 2 November 2026               [Page 25]

Internet-Draft       HTTP Live Streaming 2nd Edition            May 2026
```

## 4. Related rules (RFC 8216bis)

fMP4 segment in an I-frames-only playlist (§3.1.2 Fragmented MPEG-4, lines 466-469):

```

   An fMP4 Segment in a Playlist containing the EXT-X-I-FRAMES-ONLY tag
   (Section 4.4.3.6) MAY omit the portion of the Media Data Box
   following the intra-coded frame (I-frame) sample data.
```

`EXT-X-MAP` in an I-frames-only playlist (§4.4.4.5, lines 1603-1613):

```
   An EXT-X-MAP tag SHOULD be supplied for Media Segments in Playlists
   with the EXT-X-I-FRAMES-ONLY tag when the first Media Segment (i.e.,
   I-frame) in the Playlist (or the first segment following an EXT-
   X-DISCONTINUITY tag) does not immediately follow the Media
   Initialization Section at the beginning of its resource.

   Use of the EXT-X-MAP tag in a Media Playlist that contains the EXT-
   X-I-FRAMES-ONLY tag REQUIRES a compatibility version number of 5 or
   greater.  Use of the EXT-X-MAP tag in a Media Playlist that DOES NOT
   contain the EXT-X-I-FRAMES-ONLY tag REQUIRES a compatibility version
   number of 6 or greater.
```

Partial segments (§4.4.4.9, lines 1755-1756):

```
   Playlists that contain the EXT-X-I-FRAMES-ONLY tag SHOULD NOT use
   Partial Segments.
```

Blocking playlist reload (§6.2.5.2, lines 3813-3815):

```
   Playlists that contain the EXT-X-I-FRAMES-ONLY tag SHOULD support
   Blocking Playlist Reload using the _HLS_msn directive if other
   Renditions in the presentation contain CAN-BLOCK-RELOAD.
```

Rendition reports — the only mention of I-frames-only playlists is in Appendix A, "Changes from RFC 8216" (lines 5618-5628); the rule is not restated in the §4.4.5.4 text:

```
      Each Media Playlist MUST contain one EXT-X-RENDITION-REPORT tag
      for each Media Playlist (Rendition) in the Multivariant Playlist,
      except for the Media Playlist to which the EXT-X-RENDITION-REPORT
      tag is being added, and Playlists that contain the EXT-X-I-FRAMES-
      ONLY tag.

      Rendition reports for Media Playlists containing the EXT-X-I-
      FRAMES-ONLY tag SHOULD be provided as well, in which case, each
      Media Playlist MUST additionally contain one EXT-X-RENDITION-
      REPORT tag for each EXT-X-I-FRAMES-ONLY Media Playlist in the
      Multivariant Playlist, except for the Media Playlist to which the
```

Example 9.5 (lines 4684-4700):

```
9.5.  Multivariant Playlist with I-Frames

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

## 5. Apple authoring — Trick Play (items 6.1-6.17; capture lines 436-486)

```
## Trick Play
6. Trick play requirements
6.1. I-frame playlists (`EXT-X-I-FRAME-STREAM-INF`) MUST be provided to support scrubbing and scanning UI.
6.2. You SHOULD have one frame per second dense I-frame renditions. These are dedicated renditions that only contain I-frames.
6.3. Alternatively, you MAY use the I-frames from your normal content, but trick play performance is improved with a higher density of I-frames.
6.4. If you provide multiple bit rates at the same spatial resolution for your regular video, you SHOULD create the I-frame playlist for that resolution from the same source used for the lowest bit rate in that group.
6.5. The bit rate of I-frame playlists SHOULD be the bit rate of a normal playlist of the same resolution times the fps of the I-frame playlist divided by eight. (See hls-authoring-specification-for-apple-devices-appendixes#I-frame-bit-rates-versus-normal-bit-rates.)
6.6. * You SHOULD provide multiple I-frame Media Playlists with different bit rates.
6.7. As with normal video, there are many possible choices of bit rates for dense I-frame variants. The following tables provide one possible set of variants. (See hls-authoring-specification-for-apple-devices-appendixes#On-bit-rates-for-variants.)
16:9 aspect ratio
H.264/AVC
640 x 360
45
768 x 432
90
960 x 540
250
1280 x 720
375
1920 x 1080
580
16:9 aspect ratio
HEVC/H.265
HDR (HEVC)
768 x 432
40
55
960 x 540
75
94
960 x 540
200
238
1280 x 720
300
360
1920 x 1080
525
650
These bit rates are based on the assumption of a 1 fps dense I-frame track. The values should be modified based on the actual frame rate of the I-frame rendition. (See item 6.5.)
The above tables are average bit rates. Earlier versions of this specification used peak bit rates.
6.8. I-frame playlists MUST contain the `EXT-X-I-FRAMES-ONLY` tag.
6.9. The peak segment bit rate MUST be calculated according to the HLS specification.
6.10. If using fMP4, I-frame segments MUST include the ‘moof’ header associated with the I-frame.
6.11. For live (linear) content, target durations for I-frame playlists MUST be identical to other media.
6.12. For VOD content, target durations of I-frame playlists MAY be different from the other media.
6.13. * I-frame playlists MAY use a different video codec than the normal video segments.
6.14. * For backward compatibility, some trick play content SHOULD be encoded with H.264.
6.15. If HDR trick play streams are provided, they SHOULD be provided at all resolutions.
6.16. * For backward compatibility, SDR trick play streams MUST be provided.
6.17. I-frame content MAY be encoded using the fMP4 ‘mjpg’ codec. (See hls-authoring-specification-for-apple-devices-appendixes#I-frame-image-sequences.)
```

Platform amendments: tvOS (capture line 800-802), AirPlay 2-enabled TVs (capture lines 833-835):

```
## Trick Play
6. Trick play requirements
6.16. SDR trick play streams MUST be provided.
```

```
## Trick Play
6. Trick play requirements
6.13. If a normal video variant uses a video codec, one or more I-frame variants MUST exist with the same video codec or the `mjpg` codec.
```

Multivariant items that bear on I-frame variants (items 9.3, 9.4; capture lines 533-534):

```
9.4. Your `EXT-X-I-FRAME-STREAM-INF` tag MUST always provide the `RESOLUTION` attribute.
9.5. You SHOULD deliver video and audio as separate streams.
```

## 6. Apple appendix — *I-frame bit rates versus normal bit rates* (explains item 6.5)

```
The formula in item 6.5 (for I-frame bit rates) in this document requires some explanation.

During trick play, the player tries to deliver eight frames per second. If the playback rate implies a higher frame rate, frames will be dropped to keep the frame rate at eight frames per second. If fewer frames are available, it will play at a lower frame rate.

The I-frame bit rate is the bit rate of the rendition if it were played at normal speed. The I-frame rendition may be 2 fps, 1 fps, 0.5 fps, 0.2 fps, or something else. Let F be the frame rate of the I-frame rendition. Then, 8/F is the multiplier, that is, how much the bit rate is increased by playing the rendition at eight frames per second. The bit rate doesn't increase by more than this factor because the player can't play more than eight frames per second.

The I-frame rendition should have an effective bit rate similar to the normal content. So the normal bit rate divided by the multiplier should be the bit rate for the I-frame. Dividing by a fraction is the same as multiplying by its inverse, so the result is 6.5 — the bit rate of a normal playlist of the same resolution times the fps of the I-frame playlist divided by eight.
```

## 7. Facts this gives (derived, each tied to a quoted block above)

* An I-frame playlist is a Media Playlist with `#EXT-X-I-FRAMES-ONLY` (§1, Apple 6.8); each "segment" is ONE
  I-frame; its `EXTINF` is the presentation-time distance to the next I-frame in the playlist (§1).
* With `EXT-X-BYTERANGE` the segment points into an existing resource; the range MUST NOT include the
  Media Initialization Section, which comes from `EXT-X-MAP` (§1, §3). `EXT-X-MAP` in an I-frames-only
  playlist needs version >= 5 (§4); `EXT-X-I-FRAMES-ONLY` needs >= 4 (§1).
* The master tag `EXT-X-I-FRAME-STREAM-INF` needs `BANDWIDTH` and `URI`, takes every `EXT-X-STREAM-INF`
  attribute EXCEPT `FRAME-RATE`, `AUDIO`, `SUBTITLES`, `CLOSED-CAPTIONS` (§2); Apple also requires
  `CODECS` and `RESOLUTION` on it (9.3, 9.4). So `FRAME-RATE` must NOT be emitted on I-frame variants.
* Using the I-frames of the normal content is allowed (Apple 6.3); dense dedicated renditions are
  recommended (6.2) but need a separate encode.
* fMP4 I-frame segments must include the `moof` header associated with the I-frame (Apple 6.10); an fMP4
  segment in an I-frames-only playlist MAY omit the `mdat` portion after the I-frame sample data (§4).
* Live: the I-frame playlist's target duration must equal the other media's (Apple 6.11); I-frame
  playlists SHOULD NOT use partial segments and SHOULD support blocking reload when other renditions do
  (§4); rendition reports for I-frame playlists are optional (§4).
* `BANDWIDTH` of an I-frame variant = peak segment bit rate per the HLS spec (Apple 6.9); the 6.5 estimate
  (normal bit rate x I-frame fps / 8) is a SHOULD for authoring, explained in §6.
