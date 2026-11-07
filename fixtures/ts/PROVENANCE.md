# Provenance

## dvb-tools test fixtures

## `sdt-nit-two-ts-shared-service-id.ts`

Synthetic capture built with TSDuck 3.44 to exercise the case where two
different transport streams each carry a service numbered `service_id =
0x0301` (ETSI EN 300 468 §5.2.2/§5.2.3 only guarantees `service_id`
uniqueness within one `(original_network_id, transport_stream_id)`
namespace) — audit finding W-DT-2 / issue #1100.

Contents: SDT actual for TS `(onid=0x0001, tsid=0x0006)` naming service
`0x0301` "Service A", SDT other for TS `(onid=0x0001, tsid=0x0007)` naming
the same numeric `service_id` "Service B", and a NIT actual listing both
transport streams, each with an `eacem_logical_channel_number_descriptor`
(tag 0x83, preceded by its `private_data_specifier_descriptor` = EACEM)
assigning LCN 101 to the first and LCN 202 to the second.

Generated from synthetic input only (no real broadcast captured); released
under the workspace licence.

```sh
cat > sdt_only.xml <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<tsduck>
  <SDT transport_stream_id="0x0006" original_network_id="0x0001" actual="true">
    <service service_id="0x0301" EIT_schedule="false" EIT_present_following="false" running_status="running" CA_mode="false">
      <service_descriptor service_type="0x01" service_provider_name="Synthetic" service_name="Service A"/>
    </service>
  </SDT>
  <SDT transport_stream_id="0x0007" original_network_id="0x0001" actual="false">
    <service service_id="0x0301" EIT_schedule="false" EIT_present_following="false" running_status="running" CA_mode="false">
      <service_descriptor service_type="0x01" service_provider_name="Synthetic" service_name="Service B"/>
    </service>
  </SDT>
</tsduck>
XML

cat > nit_only.xml <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<tsduck>
  <NIT network_id="0x0001" actual="true">
    <transport_stream transport_stream_id="0x0006" original_network_id="0x0001">
      <private_data_specifier_descriptor private_data_specifier="eacem"/>
      <eacem_logical_channel_number_descriptor>
        <service service_id="0x0301" logical_channel_number="101" visible_service="true"/>
      </eacem_logical_channel_number_descriptor>
    </transport_stream>
    <transport_stream transport_stream_id="0x0007" original_network_id="0x0001">
      <private_data_specifier_descriptor private_data_specifier="eacem"/>
      <eacem_logical_channel_number_descriptor>
        <service service_id="0x0301" logical_channel_number="202" visible_service="true"/>
      </eacem_logical_channel_number_descriptor>
    </transport_stream>
  </NIT>
</tsduck>
XML

tstabcomp --compile sdt_only.xml -o sdt_only.bin
tstabcomp --compile nit_only.xml -o nit_only.bin
tsp -I null 1000 \
    -P inject sdt_only.bin --pid 0x11 --inter-packet 50 \
    -P inject nit_only.bin --pid 0x10 --inter-packet 50 \
    -O file sdt-nit-two-ts-shared-service-id.ts
```

(TSDuck 3.44-4676, `tsp`/`tstabcomp`.)

## `ts/france-tnt-pcr.ts` (workspace root `fixtures/`, referenced by
`pids_bitrate_oracle.rs`)

Pre-existing real broadcast capture (see workspace root `fixtures/` for its
own provenance); reused here as the independent oracle for `dvb-tools pids`
bitrate estimation (audit finding W-DT-1 / issue #1100) because it carries
five independent PCR PIDs (0x0078, 0x00DC, 0x026C, 0x0208, 0x02D0), one per
service.

## `france-tnt-pcr-wrap.ts`

Derived from `ts/france-tnt-pcr.ts` to exercise the PCR-wrap follow-up to
W-DT-1 (issue #1100): PID 0x00DC isolated alone (so no *other* PCR PID can
mask the bug by giving the PID-selection logic a still-valid fallback
candidate), then its PCR values shifted with TSDuck's `pcredit --add-pcr`
(which performs correct modular PCR arithmetic on the 27 MHz clock) so they
cross the wrap partway through the file. `--add-pcr`'s value is `(2^33 *
300) - 1_500_000 - <first PCR of PID 0x00DC in ts/france-tnt-pcr.ts>` — i.e.
the shift that lands the first sample 1,500,000 ticks before the wrap.

```sh
# Isolate PID 0x00DC (its own independent PCR clock, ISO/IEC 13818-1
# §2.4.4.9) from the original capture.
tsp -I file ts/france-tnt-pcr.ts -P filter --pid 0x00DC -O file single_pid.ts

# First PCR on that PID (from `tsp -P pcrextract --pcr -p 0x00DC`):
# 920154241112. Shift so it lands near the top of the 2^33*300 modulus:
tsp -I file single_pid.ts \
    -P pcredit --add-pcr 1656824636488 \
    -O file france-tnt-pcr-wrap.ts
```

(TSDuck 3.44-4676, `tsp`.) Oracle for this fixture (before the shift, since
`pcredit --add-pcr` only rewrites PCR fields, not packet count/size/timing):
`tsanalyze single_pid.ts` reports `Selected reference bitrate: 4,113,084
b/s` (188 bytes/pkt).


---

# Merged from fixtures/ts/PROVENANCE.md

# `fixtures/ts/`

## `h264-two-resolutions.ts`

The mid-stream codec-config-change fixture for the `ts_demux` tests
(issue #1080, audit r04-W51): H.264 320x240 for 2 s immediately followed by
H.264 640x480, concatenated at the transport-stream level so a single video
PID carries two different SPS/PPS sets. Produced with ffmpeg 8.1.2
(libx264 0.165.3222):

```bash
ffmpeg -y -f lavfi -i "testsrc2=size=320x240:rate=25:duration=2" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -x264-params keyint=25 \
  -f mpegts lo.ts
ffmpeg -y -f lavfi -i "testsrc2=size=640x480:rate=25:duration=2" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -x264-params keyint=25 \
  -f mpegts hi.ts
printf "file 'lo.ts'\nfile 'hi.ts'\n" > list.txt
ffmpeg -y -f concat -safe 0 -i list.txt -c copy h264-two-resolutions.ts
```

Oracle values from ffprobe 8.1.2 and a raw byte scan:

- first SPS (in `00 00 00 01 67`) codes 320x240, second codes 640x480;
  `ffprobe -select_streams v -show_entries packet=flags` reports 4 keyframes
  (2 clips x `keyint=25`), and each keyframe carries one SPS/PPS pair
- `ffprobe -select_streams v -show_entries stream=width,height` reports the
  first (320x240), since the container-level description is the first one



## Folded note: CODEC-ORACLE.md

# Codec-config fixture oracle

Real fixtures (ffmpeg-encoded real bitstreams) for the SPS/PPS/VPS-decode + RFC 6381
work (#425) and the AC-3/E-AC-3 fMP4 work (#426). Oracle values are the authoritative
SPS/VPS fields dumped by `ffmpeg -bsf:v trace_headers` (H.264/H.265) and `ffprobe`
(dims/pixfmt/Dolby). Every value a decoder produces must match its row here.

Regenerate: see `fixtures/ts/PROVENANCE.md`.

## H.264 profile matrix (`fixtures/ts/h264/`, all 320×240 unless noted)

The high-profile branch (profile_idc ∈ {100,110,122,244,…}) carries extra SPS syntax
(`chroma_format_idc`, `bit_depth_*_minus8`, `seq_scaling_matrix_present_flag`) that
Baseline/Main omit — each fixture exercises a distinct decode path.

| fixture | profile | profile_idc | constraint set0/1 | level_idc | chroma_format_idc | bit_depth luma/chroma (−8) | width_mbs−1 | height_map−1 | frame_mbs_only | crop | coded W×H | rfc6381 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `baseline.ts` | Constrained Baseline | 66 | 1 / 1 | 13 | (n/a → 1) | (n/a → 0/0) | 19 | 14 | 1 | — | 320×240 | `avc1.42C00D` |
| `main.ts` | Main | 77 | 0 / 1 | 13 | (n/a → 1) | (n/a → 0/0) | 19 | 14 | 1 | — | 320×240 | `avc1.4D400D` |
| `high.ts` | High | 100 | 0 / 0 | 13 | 1 | 0 / 0 | 19 | 14 | 1 | — | 320×240 | `avc1.64000D` |
| `high10.ts` | High 10 | 110 | 0 / 0 | 13 | 1 | 2 / 2 | 19 | 14 | 1 | — | 320×240 | `avc1.6E000D` |
| `high422.ts` | High 4:2:2 | 122 | 0 / 0 | 13 | 2 | 0 / 0 | 19 | 14 | 1 | — | 320×240 | `avc1.7A000D` |
| `high444.ts` | High 4:4:4 Predictive | 244 | 0 / 0 | 13 | 3 | 0 / 0 | 19 | 14 | 1 | — | 320×240 | `avc1.F4000D` |
| `interlaced.ts` | High | 100 | 0 / 0 | 30 | 1 | 0 / 0 | 44 | 17 | **0** (MBAFF) | — | 720×576 | `avc1.64001E` |
| `high-1080-cropped.ts` | High | 100 | 0 / 0 | — | 1 | 0 / 0 | 119 | 67 | 1 | bottom=4 | **1920×1080** | — |

Coded dimensions:
- Width  = `(pic_width_in_mbs_minus1 + 1) × 16` − chroma-scaled `(crop_left+crop_right)`.
- Height = `(2 − frame_mbs_only_flag) × (pic_height_in_map_units_minus1 + 1) × 16`
  − chroma-scaled `(crop_top+crop_bottom)`.
  - `interlaced.ts`: `(2−0) × (17+1) × 16 = 576`.
  - `high-1080-cropped.ts`: `1088 − CropUnitY(=2 for 4:2:0, frame_mbs_only=1) × 4 = 1080`.

The `seq_scaling_matrix_present_flag=1` branch (SPS-embedded scaling lists) is **not**
emitted by common encoders (x264 puts custom matrices in the PPS), so it has no real
fixture — cover it with a spec-vector unit test that forces the flag and asserts the
decoder skips the scaling lists and still reaches the correct dimensions.

`h264-aac.ts` (repo root, already committed): Main profile, level 1.3, 320×240,
progressive, 4:2:0 8-bit → `avc1.4D000D`; AAC-LC 44100 Hz 1ch.

## HEVC (`fixtures/ts/hevc/`, 320×240)

| fixture | profile | pix_fmt | bit depth | notes |
|---|---|---|---|---|
| `main.ts` | Main | yuv420p | 8 | general_profile_idc=1 |
| `main10.ts` | Main 10 | yuv420p10le | 10 | general_profile_idc=2 |

VPS/SPS/`profile_tier_level` fields (general_profile_space/tier_flag/profile_idc,
compatibility flags, constraint bytes, level_idc, chroma_format_idc, bit-depths,
conformance-window-cropped dims) → dump with `trace_headers` when implementing the
HEVC path; the `hvc1.…` RFC 6381 string is built from them.

## Dolby (`fixtures/ts/dolby/`) — for #426

| fixture | codec | sample_rate | channels | bitrate |
|---|---|---|---|---|
| `ac3.ts` | AC-3 | 44100 | 1 | 192 kbps |
| `eac3.ts` | E-AC-3 | 44100 | 1 | ~192 kbps |

`dac3`/`dec3` field oracle (fscod/bsid/bsmod/acmod/lfeon; E-AC-3 substream layout)
comes from ETSI TS 102 366 Annex F when implementing #426.



## Folded note: GENERATE.md

# Regenerating the codec-config fixtures

All fixtures are real ffmpeg-encoded bitstreams (real SPS/VPS/PTL, real syncframes),
kept tiny (≤16 frames, 320×240 unless the branch needs otherwise). Requires `ffmpeg`
with `libx264` + `libx265`, and the `ac3`/`eac3` encoders.

## H.264 profile matrix (`fixtures/ts/h264/`)

```bash
GEN() {  # profile pixfmt extra outfile
  ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=size=320x240:rate=25" \
    -frames:v 15 -c:v libx264 -profile:v "$1" -pix_fmt "$2" \
    -x264-params "keyint=15:scenecut=0$3" -an -f mpegts "$4"
}
GEN baseline yuv420p     "" fixtures/ts/h264/baseline.ts
GEN main     yuv420p     "" fixtures/ts/h264/main.ts
GEN high     yuv420p     "" fixtures/ts/h264/high.ts
GEN high10   yuv420p10le "" fixtures/ts/h264/high10.ts
GEN high422  yuv422p     "" fixtures/ts/h264/high422.ts
GEN high444  yuv444p     "" fixtures/ts/h264/high444.ts

# interlaced (frame_mbs_only_flag=0, MBAFF) — needs a broadcast size
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=size=720x576:rate=25" \
  -frames:v 16 -c:v libx264 -profile:v high -pix_fmt yuv420p \
  -flags +ilme+ildct -x264-params "interlaced=1:tff=1:keyint=16:scenecut=0" \
  -an -f mpegts fixtures/ts/h264/interlaced.ts

# frame_cropping_flag=1 (1088 coded → 1080 displayed)
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=size=1920x1080:rate=25" \
  -frames:v 12 -c:v libx264 -profile:v high -pix_fmt yuv420p \
  -x264-params "keyint=12:scenecut=0" -an -f mpegts fixtures/ts/h264/high-1080-cropped.ts
```

## HEVC (`fixtures/ts/hevc/`)

```bash
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=size=320x240:rate=25" \
  -frames:v 15 -c:v libx265 -profile:v main -pix_fmt yuv420p \
  -x265-params "keyint=15:scenecut=0" -an -f mpegts fixtures/ts/hevc/main.ts
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "testsrc2=size=320x240:rate=25" \
  -frames:v 15 -c:v libx265 -profile:v main10 -pix_fmt yuv420p10le \
  -x265-params "keyint=15:scenecut=0" -an -f mpegts fixtures/ts/hevc/main10.ts
```

## Dolby (`fixtures/ts/dolby/`)

```bash
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "sine=frequency=440:duration=2" \
  -c:a ac3  -b:a 192k -f mpegts fixtures/ts/dolby/ac3.ts
ffmpeg -y -hide_banner -loglevel error -f lavfi -i "sine=frequency=440:duration=2" \
  -c:a eac3 -b:a 192k -f mpegts fixtures/ts/dolby/eac3.ts
```

## HE-AAC in TS (`fixtures/ts/heaac/heaac-v{1,2}-adts.ts`, issue #1428)

Real HE-AAC (ADTS in TS) from the macOS AudioToolbox encoder, audio of this
workspace's own `fixtures/ts/h264-aac.ts`, video stream-copied. ffmpeg 8.1.2
(`aac_at`) rejects the profile names `aac_he`/`aac_he_v2`; the numeric
AudioToolbox profile values `4` (HE-AAC) and `28` (HE-AAC v2) work (same values
as the `heaac-v{1,2}.mp4` oracles). The source audio is mono; both profiles are
asked for stereo with `-ac 2` (v1 given mono produces parametric stereo).

```bash
ffmpeg -y -i fixtures/ts/h264-aac.ts -map 0:v:0 -map 0:a:0 -c:v copy \
  -c:a aac_at -profile:a 4 -ac 2 -b:a 48k -f mpegts fixtures/ts/heaac/heaac-v1-adts.ts
ffmpeg -y -i fixtures/ts/h264-aac.ts -map 0:v:0 -map 0:a:0 -c:v copy \
  -c:a aac_at -profile:a 28 -ac 2 -b:a 32k -f mpegts fixtures/ts/heaac/heaac-v2-adts.ts
```

Verified with `ffprobe -v error -analyzeduration 10M -probesize 10M -select_streams a:0
-show_entries stream=codec_name,profile,sample_rate,channels,channel_layout -of default=nw=1`:

| File | codec_name | profile | sample_rate | channels | ADTS header |
|---|---|---|---|---|---|
| `heaac-v1-adts.ts` | aac | HE-AAC | 44100 | 2 (stereo) | `fff1 5c80`: profile LC, sfi 7 (22050 core), channel_configuration 2 |
| `heaac-v2-adts.ts` | aac | HE-AACv2 | 44100 | 2 (stereo) | `fff1 5c40`: profile LC, sfi 7 (22050 core), channel_configuration 1 |

The ADTS header of an HE-AAC stream states only the core rate (22050 Hz) and
core channels; the SBR/PS signalling is inside the raw_data_block, which
transmux never decodes. These files carry no DVB `AAC_descriptor`, so they
exercise the per-route override path.

## Re-derive the oracle

```bash
# H.264/H.265 SPS/VPS fields (authoritative):
ffmpeg -hide_banner -loglevel trace -i <fixture> -c copy -bsf:v trace_headers -f null - 2>&1 \
  | grep -E 'profile_idc|constraint_set|level_idc|chroma_format_idc|bit_depth|pic_(width|height)|frame_mbs_only|frame_crop'
# RFC 6381 avc1 bytes (profile.constraint.level):
ffmpeg -loglevel error -i <fixture> -c:v copy -bsf:v filter_units=pass_types=7 -f h264 - | xxd | head
```

## ts-fix test fixtures

- `m6-single.ts` — copy of the workspace fixture `fixtures/ts/m6-single.ts`.
- `pat-with-nit.ts` — the workspace fixture `fixtures/ts/h264-aac.ts` with one PAT
  entry added (`program_number` 0 → `network_PID` 0x0010) by TSDuck 3.44-4676:
  `tsp -I file fixtures/ts/h264-aac.ts -P pat --nit 16 -O file fixtures/ts/pat-with-nit.ts`.
  Used as the input for the TSDuck oracle tests in `ts-fix/tests/it/tsduck_oracle.rs`. Same licence as
  its source fixture (the workspace licence).
