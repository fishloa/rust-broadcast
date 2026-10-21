# Multivariant playlist attribute rules (Apple authoring 9.x + RFC 8216bis)

Sources (text, not PDF; verbatim):
- Apple, *HLS Authoring Specification for Apple Devices*, "Multivariant Playlist" section (items 9.1-9.20). Fetched 2026-10-21 via developer.apple.com/tutorials/data JSON.
- IETF draft-pantos-hls-rfc8216bis, section 4.4.6.2 (`specs/ietf_draft_pantos_hls_rfc8216bis.txt` lines 2601-2620 and 2716-2725).

## Apple 9.x (verbatim)

- 9.1. Your `EXT-X-STREAM-INF` tag MUST always provide the `CODECS` attribute.
- 9.2. Your `EXT-X-STREAM-INF` tag MUST always provide the `RESOLUTION` attribute if the rendition includes video.
- 9.3. Your `EXT-X-I-FRAME-STREAM-INF` tag MUST always provide the `CODECS` attribute.
- 9.4. Your `EXT-X-I-FRAME-STREAM-INF` tag MUST always provide the `RESOLUTION` attribute.
- 9.5. You SHOULD deliver video and audio as separate streams.
- 9.6. If you have multichannel audio, you MUST use separate audio streams.
- 9.7. If you have alternative audio content (languages/commentary/DVS), you MUST use separate audio streams.
- 9.8. If you have different video angles, you MUST use separate audio streams.
- 9.9. You MUST provide multiple bit rates of video (that is, variants).
- 9.10. For HD content, there SHOULD be at least two variants encoded at the highest-available resolution.
- 9.11. If your video segments start with an IDR, you SHOULD use the `EXT-X-INDEPENDENT-SEGMENTS` tag in the Multivariant Playlist. (See item 7.4.)
- 9.12. If your video segments start with an IDR and the `EXT-X-INDEPENDENT-SEGMENTS` tag isn’t in the Multivariant Playlist, you MUST use the `EXT-X-INDEPENDENT-SEGMENTS` tag in all video Media Playlists. (See item 7.4.)
- 9.13. The `BANDWIDTH` attribute MUST be the largest sum of peak segment bit rates that’s produced by any playable combination of renditions.
- 9.14. You MUST include the `AVERAGE-BANDWIDTH` attribute.
- 9.15. Each `EXT-X-STREAM-INF` tag that includes video content MUST have a `FRAME-RATE` attribute.
- 9.16. The `VIDEO-RANGE` attribute MUST be specified unless all variants and renditions are SDR.
- 9.17. Within a group of Renditions (`EXT-X-MEDIA` tags having the same `TYPE` and `GROUP-ID`), those tags having the same `LANGUAGE` value should be ordered from most general to most specific. (Because `CHARACTERISTICS` are open-ended, the matching algorithm needs the ordering since it can’t interpret all the semantics.)
- 9.18. An `EXT-X-CONTENT-STEERING` tag SHOULD always have a `PATHWAY-ID` attribute.
- 9.19. The `SCORE` attribute (if present) MUST be on every variant. Otherwise, the `SCORE` attribute will be ignored.
- 9.20. For APAC, the APAC profile and level MUST be provided as a part of Codec string. See Apple-Positional-Audio-Codec.pdf for more information about APAC profile and level.

## RFC 8216bis AVERAGE-BANDWIDTH (verbatim)

```
4.4.6.2.  EXT-X-STREAM-INF

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



```

## RFC 8216bis FRAME-RATE (verbatim)

```
      SUPPLEMENTAL-CODECS

      The SUPPLEMENTAL-CODECS attribute describes media samples with
      both a backward-compatible base layer and a newer enhancement
      layer.  The base layers are specified in the CODECS attribute and
      the enhancement layers are specified by the SUPPLEMENTAL-CODECS
      attribute.

      The value is a quoted-string containing a comma-separated list of
      elements, where each element specifies an enhancement layer media
```

## Status in this workspace (2026-10-21)

| Rule | State |
|---|---|
| 9.1 CODECS | emitted (hls-runtime `set_track_specs`) |
| 9.2 RESOLUTION | emitted (Q8; square-pixel presentation size) |
| 9.14 AVERAGE-BANDWIDTH | `Variant::average_bandwidth` parses/renders it; hls-runtime emits it (mean of drained segments, clamped to BANDWIDTH) |
| 9.15 FRAME-RATE | NOT emitted - only source is optional H.264/HEVC VUI timing_info (field vs frame ambiguity); needs a design decision |
| 9.16 VIDEO-RANGE | satisfied: emitted for HLG/PQ, omitted when all SDR |
