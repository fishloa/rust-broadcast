# Microsoft Smooth Streaming ([MS-SSTR]) — for the Smooth spoke (#473)

Source: **[MS-SSTR]** Smooth Streaming Protocol (Microsoft Open Specifications).
Smooth = a client **Manifest** (XML) describing streams/qualities/fragment
timeline + a set of **fragment responses**, each a fragmented-MP4 `moof`+`mdat`
carrying the Smooth-specific `tfxd` (and optionally `tfrf`) `uuid` boxes.

## Client Manifest (§2.2.2 Manifest Response)

```
<SmoothStreamingMedia MajorVersion="2" MinorVersion="0"
                      Duration="<total, in TimeScale ticks>"
                      TimeScale="10000000">        <!-- default 10 MHz (100 ns ticks) -->
  <StreamIndex Type="video" Subtype="" Chunks="<n>" QualityLevels="<q>"
               Url="QualityLevels({bitrate})/Fragments(video={start time})"
               MaxWidth=".." MaxHeight="..">
    <QualityLevel Index="0" Bitrate="<bps>" FourCC="H264"
                  MaxWidth=".." MaxHeight=".." CodecPrivateData="<hex>"/>
    <c n="0" d="<dur ticks>" t="<abs time, optional on first>"/>   <!-- one per fragment; r="rep" optional -->
    <c d="<dur>"/>
    ...
  </StreamIndex>
  <StreamIndex Type="audio" ... Url="QualityLevels({bitrate})/Fragments(audio={start time})">
    <QualityLevel Index="0" Bitrate=".." FourCC="AACL" SamplingRate="44100"
                  Channels="1" BitsPerSample="16" PacketSize=".." AudioTag="255"
                  CodecPrivateData="<hex ASC>"/>
    <c d=".."/> ...
  </StreamIndex>
</SmoothStreamingMedia>
```

**`c` (StreamFragmentElement, §2.2.2.6):** `n` = fragment number (ordinal),
`d` = duration (TimeScale ticks), `t` = absolute time (optional; derivable by
summing `d`), `r` = repeat count (optional, "repeat the previous chunk `r`
more times" — `r+1` total chunks of duration `d`, `t` accumulating across the
run — same shape as a DASH `SegmentTimeline` `<S t= d= r=>` run, §5.3.9.6).

**Live manifest attributes (§2.2.2.1, `SmoothStreamingMedia` root element)** —
present on a live/DVR presentation, absent on VOD:
- `IsLive` — `"TRUE"`/`"FALSE"` (default `"FALSE"`); the presentation is still
  being appended to.
- `LookAheadFragmentCount` — the number of fragments ahead of the live edge
  the server signals via `tfrf` (§2.2.4.5) look-ahead.
- `DVRWindowLength` — the sliding DVR window length, in the manifest
  `TimeScale` ticks; `0`/absent means an unbounded (full) DVR window.

**Client-manifest (`.ism/Manifest`) parsing/consumption** — added for the
Smooth-pull ingest client (issue #759, T1): [`crate::smooth_parse::SmoothManifest`]
parses this document (the inverse of [`crate::smooth::SmoothPackager`]'s
writer); [`crate::smooth_parse::StreamIndex::enumerate_chunks`] expands a
`StreamIndex`'s `c` timeline (bounded, see below); a `StreamIndex@Url`
fragment-URL template is resolved by literal substitution of the `{bitrate}`
and `{start time}` tokens (§2.2.4.1 fragment addressing) via
[`crate::smooth_parse::StreamIndex::resolve_fragment_url`].

Because a client manifest is fetched from an untrusted remote server, the
parser bounds both of the two places a hostile manifest could otherwise drive
unbounded allocation: a `c@r` repeat count (mirrors the DASH `SegmentTimeline`
`<S r="...">` cap, `MAX_CHUNK_RUN` = 100,000) and a `QualityLevel`'s
`CodecPrivateData` hex length (`MAX_CODEC_PRIVATE_DATA_HEX_LEN`) before it is
hex-decoded.

**Init-segment synthesis (no Smooth init segment)** — unlike DASH/CMAF/HLS,
Smooth has no bootstrapping init segment: a `QualityLevel`'s
`CodecPrivateData` (§2.2.2.5) IS the codec config, and the client must
synthesise an ISOBMFF init segment (`moov`) from it before
[`crate::media::Fmp4Demux`] (which hard-requires a `moov`) can absorb the
fragment stream. [`crate::smooth_parse::track_spec_from_quality_level`] builds
the `TrackSpec` transmux's `build_init_segment` needs: for `FourCC="H264"` it
splits the Annex-B `CodecPrivateData` into SPS/PPS NAL units (start-code
delimited, per the Video `CodecPrivateData` shape documented above) and builds
an `avcC`; for `FourCC="AACL"` the `CodecPrivateData` bytes ARE the
`AudioSpecificConfig` and are carried straight into an `esds`.

**FourCC / CodecPrivateData (§2.2.2.5 TrackElement):**
- Video `FourCC="H264"` (a.k.a. AVC1): `CodecPrivateData` = the hex of the
  SPS+PPS as start-code-prefixed NAL units (`00000001 <sps> 00000001 <pps>`).
- Audio `FourCC="AACL"` (AAC-LC): `CodecPrivateData` = the hex of the
  AudioSpecificConfig; `AudioTag="255"` (raw AAC), plus SamplingRate/Channels/
  BitsPerSample.

## Fragment Response (§2.2.4) — fragmented MP4

`FragmentResponse = MoofBox MdatBox` where the `moof` is:
`moof( mfhd(sequence_number) traf( tfhd trun tfxd [tfrf] ) )`, `mdat` = samples.

**TfxdBox (§2.2.4.4)** — a `uuid` box, extended-type UUID
`6d1d9b05-42d5-44e6-80e2-141daff757b2`, FullBox(version,flags). Body:
`FragmentAbsoluteTime` (u64) + `FragmentDuration` (u64), both in the manifest
TimeScale. (version 1 = 64-bit fields.)

**TfrfBox (§2.2.4.5)** — a `uuid` box, UUID
`d4807ef2-ca39-4695-8e54-26cb9e46a79f`, lists the absolute time+duration of the
*next* fragment(s) (live look-ahead). Optional for VOD — omit and document.

Standard `tfhd`/`trun`/`mfhd`/`mdat` are the same as CMAF fMP4 (ISO/IEC 14496-12)
— REUSE the crate's existing `movie_fragment`/`build_media_segment` machinery and
inject the `tfxd` `uuid` box into the `traf`.

## Mapping to transmux

- Reuse `TsDemux`/`Fmp4Demux` for input IR; reuse the fMP4 fragment builder.
- Per track → a `StreamIndex` + `QualityLevel` (FourCC H264/AACL, CodecPrivateData
  from the SPS/PPS or ASC already available via `avc_config`/`aac_asc`).
- Per segment → a Smooth fragment (`moof`+`tfxd`+`mdat`) + a `c` manifest entry.
- TimeScale 10_000_000 (Smooth default); convert IR timestamps accordingly.
