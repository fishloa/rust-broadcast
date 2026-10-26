# transmux implementation notes

The page-verified transcriptions of the standards this crate implements (ISO/IEC,
ITU-T, ETSI, ATSC, Adobe, Microsoft, Apple) live in the private repository, checked
out as the `private/` submodule under `private/specs-verified/mpeg/` (the
`transmux/` and `isobmff/` subtrees). They are copyrighted by their publishers and
are not part of the public tree. This file is the public, own-words summary: what the
code reads, what it derives, and the judgment calls. Source comments that cite
`transmux/docs/codec/<name>.md` (or `docs/drm/`, `docs/rtmp/`, `docs/smooth/`,
`docs/klv/`) refer to those private transcriptions. Spec clauses are cited by number
only. Everything here is checkable against `transmux/src`; where I could not confirm
a point in the source it is marked "unverified".

## 0. Ground rules the whole crate follows

- transmux is a container hub. It parses codec *configuration* headers and a few
  in-band facts, and never encodes or decodes media. Coded samples are opaque byte
  strings (`Sample::data`); video samples are carried length-prefixed (4-byte
  big-endian lengths, `annexb` module) once inside the IR.
- Every box or record has a parser and a serializer that recomputes lengths from the
  typed fields. Round-trip (parse, serialize, byte-identical) is the acceptance test.
- Container children keep the wire order they were parsed in (`ChildOrder` in
  `init_segment.rs`); hand-built containers fall back to a conventional order.
- Where the source standard is not owned by the project (CENC, for instance), reference
  implementations (Bento4 `mp4decrypt`, Shaka Packager, ffmpeg) were used as oracles and
  the module doc says so.

## 1. Video

### 1.1 H.264 / AVC

Modules: `avc_config.rs` (record), `sps.rs` (SPS decode), `pipeline.rs`
(frame-rate and presentation size), `ir/display.rs` + `display_track.rs` (display
geometry), `interlace.rs`, `slice_header.rs`, `annexb.rs`, `nal.rs`.

Read from `avcC` (ISO/IEC 14496-15 §5.3.3): the configuration version, profile
indication, profile compatibility, level indication, the NAL length size, the SPS and
PPS arrays, and, only for the high profiles (100, 110, 122, 244), the chroma format,
the two bit depths and the SPS-extension array. The condition uses 244, not the 144
that early drafts printed; this is pinned by a real remux of a High 4:4:4 stream.

Read from the SPS (`decode_avc_sps`): profile_idc, the constraint-flag byte, level_idc,
chroma_format_idc (inferred 1 when the profile has no such field),
`separate_colour_plane_flag`, bit depths (inferred 8), `frame_mbs_only_flag`, the picture
size in macroblocks, the four frame-crop offsets, and the VUI: aspect ratio,
video-signal-type and colour description, and timing info.

Derived:

- Coded size = macroblocks times 16; height is doubled when `frame_mbs_only_flag` is 0
  (map units become field pairs).
- Cropped output size = coded size minus crop offsets, each multiplied by a crop unit.
  The horizontal unit is the chroma subsampling factor (1 for monochrome or separate
  planes, 2 for 4:2:0 and 4:2:2, 1 for 4:4:4); the vertical unit is the vertical
  subsampling factor times (2 minus `frame_mbs_only_flag`). All arithmetic saturates;
  an oversize crop yields an empty rectangle rather than a panic.
- `fps` (legacy float) = `time_scale / (2 * num_units_in_tick)`.
- Nominal frame rate (`pipeline::nominal_frame_rate`) in millihertz =
  `round_half_up(1000 * time_scale / (2 * num_units_in_tick))`, in integer math. The
  returned basis is `Normative` when `fixed_frame_rate_flag` is 1 and `Convention`
  otherwise; both tiers produce the same number (owner decision). Zero
  `num_units_in_tick` yields none. Unit test: 60000/1001 gives 29970.
- Sample aspect ratio: `aspect_ratio_idc` is kept as the wire byte (`AspectRatioIdc`).
  Index 0, a reserved index (17 to 254), or an extended SAR with a zero dimension means
  "unspecified" and surfaces as `None`, never as 0:0. Two different codings of the
  same picture shape (for example 16:11 and 64:45 at 720x576) are kept as signalled and
  never normalised to each other.
- Interlace facts (`interlace.rs`): per track from the SPS, per sample from the slice
  header's field/frame bits and the picture-timing SEI `pic_struct`.
- Parameter-set activation (`display_track.rs`): a display change is attributed to the
  access unit whose slices first reference the changed SPS (slice header, PPS id, SPS
  id), not to the access unit that merely carries the new SPS. A slice that names a PPS
  not yet received keeps the previous state and the dangling reference is remembered.

Not parsed: slice data and SEI other than caption/AFD/recovery-point/pic-timing. Whether
the SPS reader keeps or only skips scaling lists and H.264 HRD parameters is unverified.

Muxer side: `avc1` entry with `avcC`, then optional `pasp` (only when the SAR is
specified and not 1:1) and `colr` (`nclx`, only when a colour description was signalled),
in that order after the config box. The `tkhd` presentation size is the cropped output
height and the cropped output width rescaled horizontally by SAR, both 16.16 fixed
point, width rounded half up.

### 1.2 H.265 / HEVC

Modules: `hevc_config.rs` (`hvcC`, ISO/IEC 14496-15 §8.3.3), `sps.rs`, `pipeline.rs`.

Read from `hvcC`: profile space, tier, profile idc, 32 compatibility flags, 48
constraint-indicator bits, level, minimum spatial segmentation, parallelism type,
chroma format, bit depths, the stored average frame rate and constant-frame-rate
field, temporal layer count and nesting flag, NAL length size, and the NAL arrays
(array completeness, NAL type, units). The arrays are located by NAL type (VPS 32,
SPS 33 are the ones the frame-rate code looks up).

Read from the SPS (`decode_hevc_sps`): profile/tier/level block, chroma format, bit
depths, picture size, the conformance window, VUI aspect ratio, colour description,
default display window, `vui_timing_info` (`vui_num_units_in_tick`, `vui_time_scale`),
`field_seq_flag`, `frame_field_info_present_flag` (inferred from the progressive and
interlaced source flags when the VUI is absent or truncated), and the HRD fixed-picture-
rate fields for the highest temporal layer.

Derived:

- Output size = coded luma size minus the conformance window; horizontal unit 2 for
  4:2:0 and 4:2:2, else 1; vertical unit 2 for 4:2:0, else 1.
- Frame rate, `hevc_frame_rate_from_timing`:
  - HRD fixed rate present (normative tier): interval = clock tick times
    `elemental_duration_in_tc_minus1 + 1` ticks, so rate in millihertz =
    `round_half_up(1000 * time_scale / (num_units * elemental_duration))`; when
    `field_seq_flag` is set the pictures are fields and the denominator doubles
    (assumes complementary field pairs). If `field_seq_flag` is 0 and
    `frame_field_info_present_flag` is 1, the cadence lives in the per-picture
    `pic_struct`, so no value is produced.
  - No HRD fixed rate (convention tier, implementation-defined): only when both
    interlace flags are 0, `round_half_up(1000 * time_scale / num_units)`.
  - Otherwise none.
- VPS fallback (`hevc_timing_with_vps_fallback`, reads `vps_timing_info` via
  `decode_hevc_vps_timing`): used only when the SPS has no VUI timing. The VPS
  contributes just `vps_num_units_in_tick` and `vps_time_scale`; VPS HRD is not used, so
  the result is at best convention tier, with the SPS's interlace facts applied. A zero in
  either VPS value disables the fallback. Applying the fallback when the SPS has no VUI
  at all is an inference (the clause phrases inheritance in terms of the VUI being
  absent), documented as such in the code. When SPS and VPS timing both exist the SPS
  wins.

Not parsed: slice segment headers beyond what interlace and display tracking need, SEI
other than caption/AFD/pic-timing, VPS extensions, multi-layer syntax.

### 1.3 H.266 / VVC (brief)

`vvc_config.rs` (`vvcC`, ISO/IEC 14496-15 §11, a FullBox, two-byte NAL header) and
`decode_vvc_sps` in `sps.rs`; `rfc6381_vvc1` builds the codec string. The caption-SEI
path deliberately returns nothing for VVC. Frame-rate derivation is not implemented for
VVC (`nominal_frame_rate` returns `None` for it).

### 1.4 MPEG-2 video (H.262)

Modules: `h262.rs`, `mpeg_legacy.rs`, `mpeg2_display.rs`, `user_data.rs`; carried as
`CodecConfig::Mpeg2Video` (an `mp4v` entry with `esds`, object type 0x60 to 0x65).

Read: the `sequence_header()` (start code 0x000001B3): 12-bit width and height,
aspect_ratio_information; `sequence_extension()` size extensions (coded size is the
extension shifted left 12 or-ed with the base value); `sequence_display_extension()`:
video_format, optional colour primaries/transfer/matrix triple, and the intended display
size. `mpeg_legacy::Mpeg2SeqHeader` only reads the two base size fields; `h262` is the
fuller walk.

Derived: sample aspect ratio from the signalled display-aspect code and the coded size,
or the display-extension size when that extension is present (square code gives 1:1);
codes with no defined ratio, or a result outside 16 bits, give "unspecified". H.262 has
no crop syntax, so the output rectangle is the whole coded picture; the display size is
described as an intended rectangle only. MPEG-2 has no full-range flag (always false);
the inferred BT.709-style colour default is applied through `DisplayGeometry::is_mpeg2`.

AFD and bar data (ETSI TS 101 154 Annex B): picture-level `user_data()` after the
picture header, tracked by `Mpeg2DisplayTracker`. Persistence rules implemented: AFD and
bar data stay in force until the next sequence header or a new AFD; after a sequence
header the area of interest reverts to the sequence's own aspect (`afd = None`, which is
distinct from "same as coded frame"); a new AFD without bar data clears bar data. AFD
changes on non-random-access pictures still raise a display change.

Not parsed: GOP header, picture coding extension, quantiser matrices, slices.

### 1.5 AV1, VP9, VP8, AVS2/AVS3, others

- AV1 (`av1.rs`): `av1C` fields read are version, seq_profile, seq_level_idx_0,
  seq_tier_0, high_bitdepth, twelve_bit, monochrome, both chroma subsampling bits,
  chroma sample position, optional initial presentation delay, and the trailing config
  OBUs (kept opaque so the box round-trips). Bit depth is derived: 12 if twelve_bit, else
  10 if high_bitdepth, else 8. Codec string `av01.P.LLT.DD` (tier M or H). The sequence
  header OBU is not decoded.
- VP9 (`vp9.rs`): `vpcC` version 1 only (version 0 has no published syntax, so no layout is
  guessed): profile, level, bit depth, chroma subsampling, full-range flag, the three
  colour codes, codec initialization data. Codec string `vp09.PP.LL.DD`.
- VP8 and Vorbis arrive through the WebM demuxer; the WebM demuxer reads VP8 key-frame
  dimensions and the Vorbis identification header (private-transcription topics).
- AVS2/AVS3 (`avs2.rs`, `avs3.rs`, `avs3_audio.rs`): configuration records and sample
  entries with the embedded sequence header preserved verbatim; sample groups `telg`,
  `lrap`, `a3lg`.

## 2. Audio

### 2.1 AAC

Modules: `aac_asc.rs`, `mp4esds.rs`, `ts_demux.rs` (ADTS to config), `sps.rs`
(`rfc6381_mp4a`).

`esds`: a self-describing descriptor chain (tag byte, then a 1-to-4-byte 7-bit-per-byte
size). Tags handled: ES descriptor 0x03, decoder config 0x04, decoder specific info 0x05,
SL config 0x06; unknown tags are skipped by size. The serializer recomputes sizes and keeps
the parsed width so real files round-trip byte-exact (tested against an ffmpeg-written box).

`AudioSpecificConfig`: audio object type (typed enum with a reserved arm), sampling
frequency index (with the escape value 15 followed by an explicit 24-bit rate),
channel configuration, and the remaining bits kept as `trailing`. `heaac_signaling`
recovers explicit SBR/PS signalling from both encodings (hierarchical object types 5 and
29, and the backward-compatible sync extensions 0x2B7 for SBR and 0x548 for PS); the
effective object type feeds the RFC 6381 string `mp4a.40.<n>`.
`effective_sampling_frequency` prefers the explicit rate over the table rate.

ADTS: `parse_adts_header` reads the protection-absent bit (a CRC-protected frame has a
9-byte header, not 7), profile, sampling index, channel configuration, 13-bit frame
length, buffer fullness and raw block count. The ASC to ADTS relation implemented is
`ADTS.profile = audioObjectType - 1`, with the other two fields copied; objects whose
core type is outside 1 to 4 are rejected because the profile field is two bits wide.

Not parsed: any AAC payload, SBR/PS bitstream, program config elements.

### 2.2 AC-3 and E-AC-3

Module `ac3.rs` (boxes `dac3`, `dec3`; ETSI TS 102 366 Annex F and the syncframe BSI).

AC-3 syncframe: find the 0x0B77 syncword, then read fscod, frmsizecod, bsid, bsmod,
acmod, skip the conditional mix-level and surround fields (cmixlev when there are three
front channels, surmixlev when there is a surround channel, dsurmod in 2/0 mode), and read
lfeon. Derived: sample rate from fscod; frame length in bytes = words per syncframe
(table keyed by fscod and frmsizecod) times 2; `dac3.bit_rate_code = frmsizecod >> 1`;
channel count from acmod, plus one for LFE in `channels_with_lfe`.

E-AC-3 syncframe: stream type, substream id, frmsiz, fscod, number of blocks, acmod, lfeon,
bsid, optional custom channel map of dependent substreams, bsmod. Samples per frame =
number of blocks times the per-block sample count. `dec3` is built from the syncframes of
the first access unit: `data_rate` (13 bits), independent substream count, and per
substream fscod/bsid/asvc/bsmod/acmod/lfeon/dependent-count/chan_loc. Extra channel
locations declared by dependent substreams add to the channel count. Bytes after the last
substream are retained verbatim in `reserved_tail`; that is where a Dolby Atmos extension
flag lives, so dropping them would silently lose Atmos signalling.

Splitting: `split_ac3_syncframes` and `split_eac3_syncframes` cut a concatenated PES payload
into access units by each frame's own length; an E-AC-3 access unit is an independent
frame plus the dependent frames that follow. A bad sync word or truncated tail stops the
split without losing earlier frames.

Not parsed: audio blocks, mixing metadata beyond the BSI fields above.

### 2.3 Opus, FLAC, MPEG audio, DTS, AC-4, MPEG-H

- Opus (`opus.rs`): `dOps` fields (version, output channel count, pre-skip, input sample
  rate, output gain, channel mapping family and optional stream/coupled counts plus the
  mapping table). Unlike the Ogg header they are big-endian with no magic. Codec string is
  the literal `Opus`; the sample entry rate is 48000.
- FLAC (`flac.rs`): `dfLa` is a FullBox holding metadata blocks; the first must be
  STREAMINFO (type 0), exposed as raw block data. Codec string `fLaC`.
- MPEG-1/2 audio (`mpeg_legacy.rs`): 11-bit syncword, version, layer, bitrate index,
  sampling frequency, padding, channel mode. Derived: sample rate (MPEG-2 halves the base
  table, MPEG-2.5 quarters it), bitrate from the table (free-format and forbidden indices
  rejected), channels (1 for single-channel mode else 2), frame length, samples per frame
  (384 / 1152 / 1152, or 576 for MPEG-2 Layer III). Carried via `esds` object type 0x69 or
  0x6B.
- DTS (`dts.rs`): the core-substream frame header (sync 0x7FFE8001) is parsed into a
  core-only `ddts`; the four entry FourCCs `dtsc`/`dtsh`/`dtsl`/`dtse` are supported.
- AC-4 (`ac4.rs`) and MPEG-H (`mpegh.rs`): the config body (`dac4`, `mhaC` carrying
  `mpegh3daConfig`) is caller-supplied and opaque; no AC-4 TOC or MPEG-H config parse.

## 3. Captions, ancillary data, metadata

- CEA-608/708 in AVC/HEVC SEI (`nal.rs::caption_cc_data`): walks every NAL, every
  `sei_message()` (payloadType and payloadSize varints, emulation prevention removed),
  keeps `user_data_registered_itu_t_t35` (payloadType 4) messages whose T.35 country code
  is 0xB5 (United States) or 0x26 (China), provider 0x0031, user identifier "GA94"
  (0x47413934), user_data_type_code 0x03. It appends the embedded `cc_data()` (2-byte
  header, `cc_count` three-byte triplets where `cc_count` is the low 5 bits of the first
  byte, one trailing marker byte) in AU order. Anything else, including a declared count
  that overruns the payload, is skipped silently; an AU with no captions returns an empty
  vector. The output is meant for the `cc-data` crate. H.264/H.265 SEI framing of this
  payload follows industry practice (ffmpeg convention); the base standard only defines the
  MPEG-2 `user_data()` embedding. VVC always returns empty. 608/708 decoding is not done
  here.
- AFD and bar data (`user_data.rs`): `afd_data()` (user identifier 0x44544731, active_format
  flag in bit 6, 4-bit code) and `bar_data()` (GA94 identifier, type code 0x06, 14-bit bar
  values with two marker bits) from H.264/H.265 T.35 SEI and MPEG-2 picture user data.
- SMPTE ST 2038, DVB subtitles, teletext, SCTE-35, DSM-CC, ID3 and other PMT streams: not
  decoded. The TS demuxer carries them losslessly as `CodecConfig::Data` tracks with the
  raw PMT ES_info loop and a flag saying whether the payload is PES data or whole PSI
  sections (private sections 0x05, DSM-CC 0x0A to 0x0D and 0x14, and 0x86 are sections).
  Such tracks cannot be muxed into ISOBMFF; `build_init_segment` rejects them with a named
  error rather than dropping them.
- KLV (`klv.rs`): BER length (short form below 128, long form with a count of big-endian
  length bytes), generic 16-byte-key items with computed lengths, BER-OID tags, and the
  MISB ST 0601 UAS Datalink Local Set: its 16-byte universal label, precision timestamp
  (tag 2, 64-bit microseconds since the epoch, first) and checksum (tag 1, last). The
  checksum is a running big-endian 16-bit sum over the whole packet, not a CRC. Item
  values other than those two tags are opaque.

## 4. Content protection

- Scheme identity: `CencScheme` lives in `broadcast-common` and has two members, `cenc`
  (AES-128 counter mode) and `cbcs` (AES-128 CBC with a crypt:skip block pattern). The
  `cens` and `cbc1` schemes are not implemented; their four-CCs map to `None`.
- Boxes (`cenc.rs`): `tenc` (version, default crypt and skip byte blocks, is-protected,
  per-sample IV size, default KID, optional constant IV), `senc`, `pssh`, `saiz`, `saio`,
  `sinf`/`frma`/`schm`/`schi`. The `schm` version written is 1.0.
- Cipher core (`cenc_crypto.rs`, shared by encrypt and decrypt): `cenc` is CTR with a
  counter advancing once per 16-byte block across the protected bytes only (clear subsample
  ranges are skipped, not counted); the per-sample IV is 8 or 16 bytes, left-justified.
  `cbcs` encrypts `crypt` blocks then passes `skip` blocks clear, repeating; a trailing
  partial block stays clear; with both counts 0 the whole range is one 1:0 run. The chain
  is seeded by the sample's IV (per-sample IV, or the `tenc` constant IV when the
  per-sample size is 0) at the start of every subsample's protected range, and skipped
  bytes never enter the chain. The reset at each subsample boundary was established against
  Bento4 and Shaka Packager, because an earlier version carried the chain across
  subsamples and diverged on every later subsample.
- Decrypt (`cenc_decrypt.rs`): handles progressive files (sample layout from `stsz`/
  `stsc`/`stco`, one `senc` in `stbl`) and fragmented CMAF (crypto defaults from the
  `moov`, one `senc` and `trun` per `traf`); fragment addressing goes through
  `frag_offsets::sample_ranges` so decrypt and the demuxer cannot disagree.
- Encrypt (`cenc_encrypt.rs`): for NAL-carried video each length-prefixed NAL contributes one
  subsample whose clear part is the 4-byte length plus the NAL header (1 byte AVC, 2 bytes
  HEVC/VVC); other tracks or the whole-sample policy use an empty subsample map. CTR IVs must
  be unique per key forever, so `CencEncryptor` is stateful and its counter continues across
  calls; the caller must reuse one instance per key.
- `pssh` payload builders (`drm.rs`):
  - PlayReady: WRMHEADER v4.2.0.0 XML (namespace of the 2007/03 PlayReady header schema)
    with one `KID` per key (`ALGID="AESCTR"`, `VALUE` the base64 of the key id in
    little-endian GUID layout: first 4 bytes reversed, next two pairs each reversed, last 8
    unchanged), optional `LA_URL`, no checksum. Wrapped in a PlayReady Object: 32-bit LE
    total length, 16-bit LE record count (1), then record type 0x0001, 16-bit LE value
    length, UTF-16LE XML. A value of 64 KiB or more is an error.
  - Widevine: a hand-encoded protobuf with key ids (field 2), provider (3), content id
    (4, constant defined but not populated) and protection scheme (9, as the four-CC
    integer), in ascending field order.
  - FairPlay: the UTF-8 `skd://` URI as the whole payload. This is a packager convention,
    not a published format.
  - System ID UUIDs for Widevine, PlayReady, FairPlay and the common system are constants.
- HLS (`sample_aes.rs`, feature `sample-aes`): `METHOD=AES-128` encrypts the whole segment
  with CBC and PKCS#7 padding. `SAMPLE-AES`: H.264 slice NALs (types 1 and 5) of at least 48
  bytes keep the first 32 bytes clear, then a 16-byte encrypted block is followed by up to
  144 clear bytes, repeating; the CBC chain runs across one NAL's blocks and resets per
  NAL. AAC keeps the ADTS header plus a 16-byte leader clear; AC-3 and E-AC-3 keep a 16-byte
  leader clear; trailing bytes under 16 stay clear; the IV resets per audio frame (per
  syncframe for E-AC-3). The implicit IV is the media sequence number as a 128-bit big-endian
  value. `ExtXKey::to_tag` validates and escapes attribute values (the caller-supplied URI
  could otherwise inject playlist lines) and orders METHOD, URI, IV, KEYFORMAT,
  KEYFORMATVERSIONS. The E-AC-3 independent-plus-dependent IV-carry corner is unverified
  against a real encoder (flagged in the module).
- AES itself comes from the `aes` and `cbc` crates; nothing is rolled by hand.

## 5. Transport and delivery formats

- FLV (`flv.rs`, `flv_stream.rs`): 9-byte header (signature, version, type-flag byte with
  audio 0x04 and video 0x01, data offset), tags with an 11-byte header (type, 24-bit size,
  24-bit timestamp plus 8-bit extension, stream id), 32-bit previous-tag size. Video tag:
  frame type and codec id nibbles; for AVC an `AVCPacketType` byte (0 sequence header
  carrying `avcC`, 1 NALU, 2 end of sequence) and a signed 24-bit composition time, with
  PTS = DTS + composition time. Audio tag: format, rate, size, type; for AAC an
  `AACPacketType` byte (0 carries the ASC, 1 raw frame). Only AVC video and AAC audio are
  mapped; any other codec id is skipped silently, by design, unlike the TS demuxer which
  keeps unknown streams as opaque data. Script data (`onMetaData`) is skipped on read and
  emitted minimally on write. Note a doc comment in `flv.rs` names the type flags
  backwards; the constants in the code are authoritative.
- RTMP (`rtmp.rs`): handshake C0/C1/C2 and S0/S1/S2, chunk basic and message headers,
  protocol control messages, AMF0 command messages, default chunk size 128. Audio (type 8)
  and video (type 9) message bodies are FLV tag bodies, so the spoke rebuilds an FLV stream
  and reuses `FlvDemux`/`FlvMux` rather than a second parser. The message stream id is
  little-endian; everything else is big-endian.
- Smooth Streaming (`smooth.rs`, `smooth_parse.rs`): client manifest with 10 000 000 Hz
  timescale, `StreamIndex` per kind, `QualityLevel` (index, bitrate, FourCC,
  `CodecPrivateData`), `c` timeline elements. Fragments are `moof` + `mdat` with a `tfxd`
  `uuid` box (absolute time and duration in manifest ticks) inside the `traf`; `tfrf`
  look-ahead is live-only and not emitted. FourCCs: `H264` and the `AVC1` alias, `AACL`,
  `AACH`, and `stpp`/`wvtt` for text (the standard reserves no text FourCC; the ISOBMFF
  codes are reused). `CodecPrivateData` is the hex of start-code-framed SPS plus PPS for
  video and of the ASC for audio. The parser bounds untrusted manifests: `c@r` expansion
  is capped at 100 000 chunks and `CodecPrivateData` hex at 65 536 characters, both returning
  named errors.
- DASH and LL-DASH (`dash.rs`, `dash_parse.rs`, `ll_dash.rs`), LL-HLS (`ll_hls.rs`),
  TS-HLS (`ts_hls.rs`): MPD and playlist text is produced with `quick-xml` and
  `broadcast-hls`. LL-DASH sets `availabilityTimeComplete="false"` and an
  `availabilityTimeOffset` of segment duration minus chunk duration, a latency target, and a
  caller-supplied UTC timing source. `ProducerReferenceTime` is not emitted because the IR
  has no wall-clock capture time.
- RTP/SDP (`rtp.rs`, `rtp_sdp.rs`): H.264 `sprop-parameter-sets` (base64 SPS/PPS) to `avcC`
  and AAC `config` (hex ASC) to `esds`, with an anchored fmtp key/value parser.

## 6. ISOBMFF box ordering and placement, as implemented

Citations are ISO/IEC 14496-12 clause numbers; CMAF is ISO/IEC 23000-19.

File level
1. `ftyp` is the first box of an init segment or progressive file (§4.3); the validator
   reports a missing `ftyp` and a box before it. Segments written by `build_init_segment` use
   major brand `iso5`, minor version 512, compatible `iso5`/`iso6`/`mp41`.
2. A media segment starts with `styp` (§8.16.2; brands `msdh`/`msix` are written). Any `emsg`
   boxes follow `styp` and precede `moof`, in the order given.
3. A `moof` is followed by its `mdat`; an `mdat` with no preceding `moof` is an error.
4. Progressive files hold one `moov` and one `mdat`. With faststart `moov` precedes `mdat`;
   without it `mdat` comes first. Chunk offsets are absolute, so faststart runs two passes
   (lay out `moov`, compute offsets, re-serialise). `stco` becomes 64-bit `co64` if any offset
   exceeds `u32::MAX`; the decision is re-measured once because `co64` changes the `moov` size.

Movie and track nesting
5. `moov` holds `mvhd`, then the `trak` boxes, then `mvex` when the movie is fragmented.
   `mvhd` carries the movie timescale and `next_track_id` (largest track id plus one). A
   `pssh` found while parsing keeps its parsed position; the typed model alone would
   otherwise move it after the tracks.
6. `trak` holds `tkhd`, optional `edts`, then `mdia`. `tkhd` flags are enabled, in-movie,
   in-preview (0x7).
7. `mdia` holds `mdhd`, `hdlr`, `minf`. `minf` holds the media header first (`vmhd` for
   video, `smhd` for audio; subtitle tracks put `sthd`/`nmhd` first, which strict readers
   require), then `dinf`, then `stbl`.
8. `stbl` order for progressive output: `stsd`, `stts`, `ctts` (when any composition
   offset is non-zero), `stsc`, `stsz`, `stco` or `co64`, `stss` (omitted when every sample is
   a sync sample).
9. A fragmented init `moov` has the same tables but empty, and `mvex` holds one `trex` per
   track with sample description index 1 and zero defaults. A track that cannot be placed in
   a `trak` (opaque data, or a subtitle with no entry) makes `build_init_segment` fail,
   naming the track, instead of omitting it.
10. A sampling rate above 65535 Hz cannot fit the 16.16 field of an audio entry, so the
    version-1 entry form is used: `samplerate` field 1.0, the real rate in an `srat` box, and
    `stsd` version 1.

Fragments
11. `moof` holds `mfhd` (sequence number) then one `traf` per track; CMAF prefers one `traf`
    per fragment (a warning, not an error).
12. Each `traf` holds `tfhd`, `tfdt` (always version 1 here) and at least one `trun`.
13. `tfhd` is written with only the default-base-is-moof flag; no base data offset, no
    defaults. So per-sample duration, size and flags always live in the `trun`.
14. `trun` flags: data offset, sample duration, size and flags present; sample composition
    offset present only if some sample has a non-zero offset, in which case the `trun` is
    version 1 (signed offsets). Non-sync samples use the dependency-bearing flag word
    0x01010000, sync samples the sync constant.
15. `data_offset` is measured from the first byte of the `moof`. The writer serializes the
    `moof` once to learn its size (the size does not depend on offset values), then sets
    each track's offset to moof size plus the 8-byte `mdat` header plus the bytes of the
    tracks before it. Tracks share one `mdat` in track order.
16. Offset resolution on read (`frag_offsets.rs`): the base is `tfhd.base_data_offset` when its
    flag is set; else the `moof` start when default-base-is-moof is set; else the `moof` start
    for the first `traf` and the end of the previous fragment's data for later ones. A
    `trun.data_offset` is relative to that base; without one a run starts where the previous
    run ended. Every resolved range must lie inside an `mdat` payload, and the total handed out
    is capped at the file length, so a hostile `trun` cannot expose `moov` bytes as samples or
    amplify work.
17. LL-DASH and LL-HLS chunks are the same structure: the first chunk of a segment carries
    `styp`, later chunks are bare `moof` + `mdat`; `mfhd` numbers increase across all chunks
    and `tfdt` is the first sample's decode time in that chunk.

Timing, edits and indexes
18. `stts` run-length codes each sample's own delta to the next sample's `dts`, so a gap
    survives. `ctts` is run-length coded. Version 0 is unsigned and version 1 signed.
19. `elst` (§8.6.6): each track is placed on the movie timeline relative to the earliest
    track. A late start becomes an empty edit (media time -1) of the gap duration; a
    composition lead-in becomes the media time of the main edit. No edit list is written when
    both are zero. Version 1 (64-bit fields) is chosen only when a value overflows 32 bits.
    `tkhd` duration includes the leading empty edit.
20. `sidx` (§8.16.3) is implemented as a typed parse/serialize pair with real-box coverage
    (`timing.rs`). I found no packager that emits one; treat sidx emission as not done.
21. Sample-table consistency: progressive output writes one `stsc` run per distinct
    samples-per-chunk, `stsz` with the per-sample list and sample size 0, and checks every
    count and length through checked narrowing helpers (no silent wrap).

Validator (`validate.rs`, never decodes media)
22. Init checks: `ftyp` present and first, `moov`, `mvhd`, at least one `trak`, each `trak`
    complete (`tkhd`, `mdia` with `mdhd`, `hdlr`, `minf` with `stbl` with `stsd`), `mvex`
    present (warning only).
23. Media checks: `styp` and its brand set (warnings), `moof`, `mfhd`, `traf`, `tfhd`, `tfdt`,
    `trun`, matching `mdat`, `trun.data_offset` plus the sum of sizes inside the `mdat`
    payload, no zero-duration sample.
24. Cross-segment: the next `tfdt` must equal the previous `tfdt` plus the sum of its sample
    durations (any gap or overlap is an error); `mfhd` sequence numbers should strictly
    increase (warning).

## 7. Judgment calls

- Segment cuts happen on the anchor track (first video, else first anchor-capable track) at a
  sync sample once the target duration is reached, so every video segment starts at a random
  access point. Elapsed time uses sample durations, falling back to the `dts` delta when a
  duration is 0 (routine on live input, and a frozen clock would stop all cutting). A stream
  with no second sync sample is never cut mid-GOP; instead the per-track pending-sample cap
  makes `push` return a named error.
- Frame rate reports its basis (normative or convention) rather than hiding the difference;
  H.264 gives the same value for both, H.265 differs by tier.
- SAR is stored as signalled and never normalised, and an unspecified ratio is `None`, never a
  0:0 sentinel. Reserved `aspect_ratio_idc` values 17 to 254 are treated as unspecified for
  both H.264 and H.265 (the H.265 rule, applied to both).
- Demuxers are lenient but loud: a non-AVC/AAC FLV tag is skipped silently (a documented
  exception), a malformed optional box costs that box not the file, and parse failures keep
  the raw bytes so the real error is not hidden behind "no entry".
- Counts and lengths are never narrowed with `as`; checked helpers return typed errors, and
  bit-depth and similar SPS fields are range-checked before narrowing.
- XML (DASH, Smooth, PlayReady) goes through `quick-xml` only; protobuf for Widevine is
  hand-encoded (four fields), which is deliberate to avoid a protobuf dependency.
- Codec config for AC-4, MPEG-H and DTS extension substreams is caller-supplied: transmux
  serialises it but does not parse the underlying stream.
- Public tests use committed fixtures through `test_fixtures`; reference-tool outputs
  (ffmpeg, Bento4, Shaka) are used as oracles where the standard is not owned.

## 8. Known gaps

- `cens` and `cbc1` protection schemes; any DRM licence-server interaction (the crate only
  builds `pssh`/header payloads).
- `sidx` is typed but not emitted by any packager; Smooth `tfrf` and DASH
  `ProducerReferenceTime` are not emitted.
- HEVC/VVC: no slice-level parsing beyond display and interlace needs; VVC has no frame-rate
  derivation and no caption-SEI extraction.
- CEA-608/708 and ST 2038 are carried or extracted but not decoded here; DVB subtitles,
  teletext, SCTE-35 and DSM-CC survive only as opaque data tracks and cannot be muxed to
  ISOBMFF.
- FLV: only AVC and AAC are mapped; other codec ids are dropped without a signal.
- E-AC-3 HLS Sample-AES with independent plus dependent syncframes: the IV-carry behaviour is
  unverified against a real encoder.
- AC-3/E-AC-3 and AAC are recovered from the first access unit only for config purposes; the
  TS demuxer re-probes on a changed AAC ADTS header or AVC/HEVC parameter sets and emits
  `TrackUpdated`, but other mid-stream audio parameter changes are not tracked (unverified
  beyond what `ts_demux.rs` documents).
