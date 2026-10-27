Source file: specs/ietf_rfc8216_hls.txt
RFC number: 8216
Licence: IETF Trust, BCP 78 / RFC 5378 allow reproduction with attribution.
Command: tools/spec_extract.py --source specs/ietf_rfc8216_hls.txt --sections 3.5 4.3.4.1 4.3.4.2 --output docs/specs/hls/rfc8216-webvtt-extract.md

## 3.5. WebVTT (specs/ietf_rfc8216_hls.txt:437-484)

```text
3.5.  WebVTT

   A WebVTT Segment is a section of a WebVTT [WebVTT] file.  WebVTT
   Segments carry subtitles.

   The Media Initialization Section of a WebVTT Segment is the WebVTT
   header.

   Each WebVTT Segment MUST contain all subtitle cues that are intended
   to be displayed during the period indicated by the segment EXTINF
   duration.  The start time offset and end time offset of each cue MUST
   indicate the total display time for that cue, even if part of the cue
   time range is outside the Segment period.  A WebVTT Segment MAY
   contain no cues; this indicates that no subtitles are to be displayed
   during that period.









   Each WebVTT Segment MUST either start with a WebVTT header or have an
   EXT-X-MAP tag applied to it.

   In order to synchronize timestamps between audio/video and subtitles,
   an X-TIMESTAMP-MAP metadata header SHOULD be added to each WebVTT
   header.  This header maps WebVTT cue timestamps to MPEG-2 (PES)
   timestamps in other Renditions of the Variant Stream.  Its format is:

   X-TIMESTAMP-MAP=LOCAL:<cue time>,MPEGTS:<MPEG-2 time>
   e.g., X-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:900000

   The cue timestamp in the LOCAL attribute MAY fall outside the range
   of time covered by the segment.

   If a WebVTT segment does not have the X-TIMESTAMP-MAP, the client
   MUST assume that the WebVTT cue time of 0 maps to an MPEG-2 timestamp
   of 0.

   When synchronizing WebVTT with PES timestamps, clients SHOULD account
   for cases where the 33-bit PES timestamps have wrapped and the WebVTT
   cue times have not.
```

## 4.3.4.1. EXT-X-MEDIA (specs/ietf_rfc8216_hls.txt:1391-1612)

```text
4.3.4.1.  EXT-X-MEDIA

   The EXT-X-MEDIA tag is used to relate Media Playlists that contain
   alternative Renditions (Section 4.3.4.2.1) of the same content.  For
   example, three EXT-X-MEDIA tags can be used to identify audio-only
   Media Playlists that contain English, French, and Spanish Renditions
   of the same presentation.  Or, two EXT-X-MEDIA tags can be used to
   identify video-only Media Playlists that show two different camera
   angles.

   Its format is:

   #EXT-X-MEDIA:<attribute-list>

   The following attributes are defined:

      TYPE

      The value is an enumerated-string; valid strings are AUDIO, VIDEO,
      SUBTITLES, and CLOSED-CAPTIONS.  This attribute is REQUIRED.

      Typically, closed-caption [CEA608] media is carried in the video
      stream.  Therefore, an EXT-X-MEDIA tag with TYPE of CLOSED-
      CAPTIONS does not specify a Rendition; the closed-caption media is
      present in the Media Segments of every video Rendition.

      URI

      The value is a quoted-string containing a URI that identifies the
      Media Playlist file.  This attribute is OPTIONAL; see
      Section 4.3.4.2.1.  If the TYPE is CLOSED-CAPTIONS, the URI
      attribute MUST NOT be present.







      GROUP-ID

      The value is a quoted-string that specifies the group to which the
      Rendition belongs.  See Section 4.3.4.1.1.  This attribute is
      REQUIRED.

      LANGUAGE

      The value is a quoted-string containing one of the standard Tags
      for Identifying Languages [RFC5646], which identifies the primary
      language used in the Rendition.  This attribute is OPTIONAL.

      ASSOC-LANGUAGE

      The value is a quoted-string containing a language tag [RFC5646]
      that identifies a language that is associated with the Rendition.
      An associated language is often used in a different role than the
      language specified by the LANGUAGE attribute (e.g., written versus
      spoken or a fallback dialect).  This attribute is OPTIONAL.

      The LANGUAGE and ASSOC-LANGUAGE attributes can be used, for
      example, to link Norwegian Renditions that use different spoken
      and written languages.

      NAME

      The value is a quoted-string containing a human-readable
      description of the Rendition.  If the LANGUAGE attribute is
      present, then this description SHOULD be in that language.  This
      attribute is REQUIRED.

      DEFAULT

      The value is an enumerated-string; valid strings are YES and NO.
      If the value is YES, then the client SHOULD play this Rendition of
      the content in the absence of information from the user indicating
      a different choice.  This attribute is OPTIONAL.  Its absence
      indicates an implicit value of NO.

      AUTOSELECT

      The value is an enumerated-string; valid strings are YES and NO.
      This attribute is OPTIONAL.  Its absence indicates an implicit
      value of NO.  If the value is YES, then the client MAY choose to
      play this Rendition in the absence of explicit user preference
      because it matches the current playback environment, such as
      chosen system language.








      If the AUTOSELECT attribute is present, its value MUST be YES if
      the value of the DEFAULT attribute is YES.

      FORCED

      The value is an enumerated-string; valid strings are YES and NO.
      This attribute is OPTIONAL.  Its absence indicates an implicit
      value of NO.  The FORCED attribute MUST NOT be present unless the
      TYPE is SUBTITLES.

      A value of YES indicates that the Rendition contains content that
      is considered essential to play.  When selecting a FORCED
      Rendition, a client SHOULD choose the one that best matches the
      current playback environment (e.g., language).

      A value of NO indicates that the Rendition contains content that
      is intended to be played in response to explicit user request.

      INSTREAM-ID

      The value is a quoted-string that specifies a Rendition within the
      segments in the Media Playlist.  This attribute is REQUIRED if the
      TYPE attribute is CLOSED-CAPTIONS, in which case it MUST have one
      of the values: "CC1", "CC2", "CC3", "CC4", or "SERVICEn" where n
      MUST be an integer between 1 and 63 (e.g., "SERVICE3" or
      "SERVICE42").

      The values "CC1", "CC2", "CC3", and "CC4" identify a Line 21 Data
      Services channel [CEA608].  The "SERVICE" values identify a
      Digital Television Closed Captioning [CEA708] service block
      number.

      For all other TYPE values, the INSTREAM-ID MUST NOT be specified.

      CHARACTERISTICS

      The value is a quoted-string containing one or more Uniform Type
      Identifiers [UTI] separated by comma (,) characters.  This
      attribute is OPTIONAL.  Each UTI indicates an individual
      characteristic of the Rendition.

      A SUBTITLES Rendition MAY include the following characteristics:
      "public.accessibility.transcribes-spoken-dialog",
      "public.accessibility.describes-music-and-sound", and
      "public.easy-to-read" (which indicates that the subtitles have
      been edited for ease of reading).









      An AUDIO Rendition MAY include the following characteristic:
      "public.accessibility.describes-video".

      The CHARACTERISTICS attribute MAY include private UTIs.

      CHANNELS

      The value is a quoted-string that specifies an ordered, backslash-
      separated ("/") list of parameters.  If the TYPE attribute is
      AUDIO, then the first parameter is a count of audio channels
      expressed as a decimal-integer, indicating the maximum number of
      independent, simultaneous audio channels present in any Media
      Segment in the Rendition.  For example, an AC-3 5.1 Rendition
      would have a CHANNELS="6" attribute.  No other CHANNELS parameters
      are currently defined.

      All audio EXT-X-MEDIA tags SHOULD have a CHANNELS attribute.  If a
      Master Playlist contains two Renditions encoded with the same
      codec but a different number of channels, then the CHANNELS
      attribute is REQUIRED; otherwise, it is OPTIONAL.

4.3.4.1.1.  Rendition Groups

   A set of one or more EXT-X-MEDIA tags with the same GROUP-ID value
   and the same TYPE value defines a Group of Renditions.  Each member
   of the Group MUST be an alternative Rendition of the same content;
   otherwise, playback errors can occur.

   All EXT-X-MEDIA tags in a Playlist MUST meet the following
   constraints:

   o  All EXT-X-MEDIA tags in the same Group MUST have different NAME
      attributes.

   o  A Group MUST NOT have more than one member with a DEFAULT
      attribute of YES.

   o  Each EXT-X-MEDIA tag with an AUTOSELECT=YES attribute SHOULD have
      a combination of LANGUAGE [RFC5646], ASSOC-LANGUAGE, FORCED, and
      CHARACTERISTICS attributes that is distinct from those of other
      AUTOSELECT=YES members of its Group.

   A Playlist MAY contain multiple Groups of the same TYPE in order to
   provide multiple encodings of that media type.  If it does so, each
   Group of the same TYPE MUST have the same set of members, and each
   corresponding member MUST have identical attributes with the
   exception of the URI and CHANNELS attributes.








   Each member in a Group of Renditions MAY have a different sample
   format.  For example, an English Rendition can be encoded with AC-3
   5.1 while a Spanish Rendition is encoded with AAC stereo.  However,
   any EXT-X-STREAM-INF tag (Section 4.3.4.2) or EXT-X-I-FRAME-STREAM-
   INF tag (Section 4.3.4.3) that references such a Group MUST have a
   CODECS attribute that lists every sample format present in any
   Rendition in the Group, or client playback failures can occur.  In
   the example above, the CODECS attribute would include
   "ac-3,mp4a.40.2".
```

## 4.3.4.2. EXT-X-STREAM-INF (specs/ietf_rfc8216_hls.txt:1613-1843)

```text
4.3.4.2.  EXT-X-STREAM-INF

   The EXT-X-STREAM-INF tag specifies a Variant Stream, which is a set
   of Renditions that can be combined to play the presentation.  The
   attributes of the tag provide information about the Variant Stream.

   The URI line that follows the EXT-X-STREAM-INF tag specifies a Media
   Playlist that carries a Rendition of the Variant Stream.  The URI
   line is REQUIRED.  Clients that do not support multiple video
   Renditions SHOULD play this Rendition.

   Its format is:

   #EXT-X-STREAM-INF:<attribute-list>
   <URI>

   The following attributes are defined:

      BANDWIDTH

      The value is a decimal-integer of bits per second.  It represents
      the peak segment bit rate of the Variant Stream.

      If all the Media Segments in a Variant Stream have already been
      created, the BANDWIDTH value MUST be the largest sum of peak
      segment bit rates that is produced by any playable combination of
      Renditions.  (For a Variant Stream with a single Media Playlist,
      this is just the peak segment bit rate of that Media Playlist.)
      An inaccurate value can cause playback stalls or prevent clients
      from playing the variant.

      If the Master Playlist is to be made available before all Media
      Segments in the presentation have been encoded, the BANDWIDTH
      value SHOULD be the BANDWIDTH value of a representative period of
      similar content, encoded using the same settings.










      Every EXT-X-STREAM-INF tag MUST include the BANDWIDTH attribute.

      AVERAGE-BANDWIDTH

      The value is a decimal-integer of bits per second.  It represents
      the average segment bit rate of the Variant Stream.

      If all the Media Segments in a Variant Stream have already been
      created, the AVERAGE-BANDWIDTH value MUST be the largest sum of
      average segment bit rates that is produced by any playable
      combination of Renditions.  (For a Variant Stream with a single
      Media Playlist, this is just the average segment bit rate of that
      Media Playlist.)  An inaccurate value can cause playback stalls or
      prevent clients from playing the variant.

      If the Master Playlist is to be made available before all Media
      Segments in the presentation have been encoded, the AVERAGE-
      BANDWIDTH value SHOULD be the AVERAGE-BANDWIDTH value of a
      representative period of similar content, encoded using the same
      settings.

      The AVERAGE-BANDWIDTH attribute is OPTIONAL.

      CODECS

      The value is a quoted-string containing a comma-separated list of
      formats, where each format specifies a media sample type that is
      present in one or more Renditions specified by the Variant Stream.
      Valid format identifiers are those in the ISO Base Media File
      Format Name Space defined by "The 'Codecs' and 'Profiles'
      Parameters for "Bucket" Media Types" [RFC6381].

      For example, a stream containing AAC low complexity (AAC-LC) audio
      and H.264 Main Profile Level 3.0 video would have a CODECS value
      of "mp4a.40.2,avc1.4d401e".

      Every EXT-X-STREAM-INF tag SHOULD include a CODECS attribute.

      RESOLUTION

      The value is a decimal-resolution describing the optimal pixel
      resolution at which to display all the video in the Variant
      Stream.

      The RESOLUTION attribute is OPTIONAL but is recommended if the
      Variant Stream includes video.









      FRAME-RATE

      The value is a decimal-floating-point describing the maximum frame
      rate for all the video in the Variant Stream, rounded to three
      decimal places.

      The FRAME-RATE attribute is OPTIONAL but is recommended if the
      Variant Stream includes video.  The FRAME-RATE attribute SHOULD be
      included if any video in a Variant Stream exceeds 30 frames per
      second.

      HDCP-LEVEL

      The value is an enumerated-string; valid strings are TYPE-0 and
      NONE.  This attribute is advisory; a value of TYPE-0 indicates
      that the Variant Stream could fail to play unless the output is
      protected by High-bandwidth Digital Content Protection (HDCP) Type
      0 [HDCP] or equivalent.  A value of NONE indicates that the
      content does not require output copy protection.

      Encrypted Variant Streams with different HDCP levels SHOULD use
      different media encryption keys.

      The HDCP-LEVEL attribute is OPTIONAL.  It SHOULD be present if any
      content in the Variant Stream will fail to play without HDCP.
      Clients without output copy protection SHOULD NOT load a Variant
      Stream with an HDCP-LEVEL attribute unless its value is NONE.

      AUDIO

      The value is a quoted-string.  It MUST match the value of the
      GROUP-ID attribute of an EXT-X-MEDIA tag elsewhere in the Master
      Playlist whose TYPE attribute is AUDIO.  It indicates the set of
      audio Renditions that SHOULD be used when playing the
      presentation.  See Section 4.3.4.2.1.

      The AUDIO attribute is OPTIONAL.

      VIDEO

      The value is a quoted-string.  It MUST match the value of the
      GROUP-ID attribute of an EXT-X-MEDIA tag elsewhere in the Master
      Playlist whose TYPE attribute is VIDEO.  It indicates the set of
      video Renditions that SHOULD be used when playing the
      presentation.  See Section 4.3.4.2.1.

      The VIDEO attribute is OPTIONAL.








      SUBTITLES

      The value is a quoted-string.  It MUST match the value of the
      GROUP-ID attribute of an EXT-X-MEDIA tag elsewhere in the Master
      Playlist whose TYPE attribute is SUBTITLES.  It indicates the set
      of subtitle Renditions that can be used when playing the
      presentation.  See Section 4.3.4.2.1.

      The SUBTITLES attribute is OPTIONAL.

      CLOSED-CAPTIONS

      The value can be either a quoted-string or an enumerated-string
      with the value NONE.  If the value is a quoted-string, it MUST
      match the value of the GROUP-ID attribute of an EXT-X-MEDIA tag
      elsewhere in the Playlist whose TYPE attribute is CLOSED-CAPTIONS,
      and it indicates the set of closed-caption Renditions that can be
      used when playing the presentation.  See Section 4.3.4.2.1.

      If the value is the enumerated-string value NONE, all EXT-X-
      STREAM-INF tags MUST have this attribute with a value of NONE,
      indicating that there are no closed captions in any Variant Stream
      in the Master Playlist.  Having closed captions in one Variant
      Stream but not another can trigger playback inconsistencies.

      The CLOSED-CAPTIONS attribute is OPTIONAL.

4.3.4.2.1.  Alternative Renditions

   When an EXT-X-STREAM-INF tag contains an AUDIO, VIDEO, SUBTITLES, or
   CLOSED-CAPTIONS attribute, it indicates that alternative Renditions
   of the content are available for playback of that Variant Stream.

   When defining alternative Renditions, the following constraints MUST
   be met to prevent client playback errors:

   o  All playable combinations of Renditions associated with an EXT-X-
      STREAM-INF tag MUST have an aggregate bandwidth less than or equal
      to the BANDWIDTH attribute of the EXT-X-STREAM-INF tag.

   o  If an EXT-X-STREAM-INF tag contains a RESOLUTION attribute and a
      VIDEO attribute, then every alternative video Rendition MUST have
      an optimal display resolution matching the value of the RESOLUTION
      attribute.

   o  Every alternative Rendition associated with an EXT-X-STREAM-INF
      tag MUST meet the constraints for a Variant Stream described in
      Section 6.2.4.







   The URI attribute of the EXT-X-MEDIA tag is REQUIRED if the media
   type is SUBTITLES, but OPTIONAL if the media type is VIDEO or AUDIO.
   If the media type is VIDEO or AUDIO, a missing URI attribute
   indicates that the media data for this Rendition is included in the
   Media Playlist of any EXT-X-STREAM-INF tag referencing this EXT-
   X-MEDIA tag.  If the media TYPE is AUDIO and the URI attribute is
   missing, clients MUST assume that the audio data for this Rendition
   is present in every video Rendition specified by the EXT-X-STREAM-INF
   tag.

   The URI attribute of the EXT-X-MEDIA tag MUST NOT be included if the
   media type is CLOSED-CAPTIONS.
```
