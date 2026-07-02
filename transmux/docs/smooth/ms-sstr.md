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
summing `d`), `r` = repeat count (optional).

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
