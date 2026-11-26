# OxideAV review: facts before an adapter

Reviewed 2026-11-26 on lane bo1, against the crates crates.io served that
day. Every row says where it was read. "Not established" means it was not
established and why.

## 1. The crates

Versions from `python3 tools/crates-io.py version <crate> ...` (the
workspace's only supported crates.io lookup):

| crate | live version |
| --- | --- |
| `oxideav-core` | 0.1.43 |
| `oxideav-codec` | 0.1.3 |
| `oxideav-aac` | 0.1.7 |
| `oxideav-mp3` | 0.1.4 |
| `oxideav-opus` | 0.0.15 |
| `oxideav-basic` | 0.0.12 |
| `oxideav-flac` | 0.0.13 |
| `oxideav-vorbis` | 0.0.13 |

The manifests cargo checked out (`~/.cargo/registry/src/.../<crate>/Cargo.toml`)
state:

| crate | edition | `rust-version` | licence | repository |
| --- | --- | --- | --- | --- |
| `oxideav-core` 0.1.43 | 2024 | 1.95 | MIT | `https://github.com/OxideAV/oxideav-core` |
| `oxideav-codec` 0.1.3 | 2021 | 1.80 | MIT | `https://github.com/OxideAV/oxideav-codec` |
| `oxideav-aac` 0.1.7 | 2021 | 1.80 | MIT | `https://github.com/OxideAV/oxideav-aac` |
| `oxideav-mp3` 0.1.4 | 2024 | 1.95 | MIT | `https://github.com/OxideAV/oxideav-mp3` |
| `oxideav-opus` 0.0.15 | 2024 | 1.95 | MIT | `https://github.com/OxideAV/oxideav-opus` |
| `oxideav-basic` 0.0.12 | 2024 | 1.95 | MIT | `https://github.com/OxideAV/oxideav-basic` |

Build on the workspace toolchain (1.95.0): **yes**. `cargo check --locked`
in `spikes/codec-oxideav`, whose manifest depends on all six crates,
finished successfully on rustc 1.95.0.

## 2. Licences in the dependency tree

`cargo metadata --locked` in `spikes/codec-oxideav`, every resolved
package's manifest `license` field:

| package | version | licence | in `deny.toml` allow |
| --- | --- | --- | --- |
| `oxideav-core` | 0.1.43 | MIT | yes |
| `oxideav-codec` | 0.1.3 | MIT | yes |
| `oxideav-aac` | 0.1.7 | MIT | yes |
| `oxideav-mp3` | 0.1.4 | MIT | yes |
| `oxideav-opus` | 0.0.15 | MIT | yes |
| `oxideav-basic` | 0.0.12 | MIT | yes |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | yes (`Zlib`, `Apache-2.0`, `MIT`) |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | yes |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 | yes |
| `itoa` | 1.0.19 | MIT OR Apache-2.0 | yes |
| `memchr` | 2.8.3 | Unlicense OR MIT | yes (`Unlicense`, `MIT`) |
| `zmij` | 1.0.23 | MIT | yes |
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 | yes |
| `thiserror-impl` | 2.0.21 | MIT OR Apache-2.0 | yes |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 | yes |
| `quote` | 1.0.47 | MIT OR Apache-2.0 | yes |
| `syn` | 3.0.10 | MIT OR Apache-2.0 | yes |
| `unicode-ident` | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | yes (`MIT`, `Apache-2.0`, `Unicode-3.0`) |

The runtime tree the adapter would pull in is MIT throughout. The
`deny.toml` `[licenses] allow` list is at `deny.toml:16-30`; every
licence above appears in it.

The spike's **dev-dependencies** also resolve `symphonia` 0.6.1 and its
five sibling crates, all MPL-2.0 (manifests read the same way). MPL-2.0
is deliberately not in `deny.toml`'s allow list; it is the reason
`spikes/codec-symphonia` lives in its own workspace (the same reason the
existing spike gives). It is test-only, reaches no published adapter,
and is not part of the adapter's runtime tree. T2 requires it, so this
is not a blocker for the adapter itself; it would block publishing a
crate that depended on it.

## 3. `-sys`, `cc`, `bindgen`, `cmake`, `links`

`cargo metadata --locked` and `cargo tree --locked` in
`spikes/codec-oxideav`:

- **No `links` key** on any resolved package.
- **No `cc`, `bindgen` or `cmake` build dependency.** The only build
  dependencies in the whole graph are `version_check` (for
  `generic-array`) and `autocfg` (for `num-traits`), both Rust build
  scripts with no C toolchain. Neither is in the adapter's runtime
  subtree; both arrive through `test-fixtures`.
- **No `-sys` crate in the adapter's graph.** `windows-sys` 0.61.2 is
  in the lock file only as a Windows-target dependency of `mio` /
  `socket2` / `tokio`, which arrive through the `test-fixtures`
  dev-dependency. `cargo tree --locked -i windows-sys` (native target)
  prints nothing.

The oxideav manifests also set `build = false`; none of the six has a
build script.

## 4. Traits as published

`oxideav-codec` 0.1.3 is a re-export shim: its `lib.rs` re-exports
`Decoder`, `Encoder`, `CodecRegistry`, `CodecInfo`,
`CodecImplementation`, `DecoderFactory`, `EncoderFactory` from
`oxideav_core`. The real definitions are in `oxideav-core` 0.1.43
`registry/codec.rs` (traits), `packet.rs` (`Packet`),
`frame.rs` (`Frame`, `AudioFrame`, `VideoFrame`), `time.rs`
(`TimeBase`, `Timestamp`), `error.rs` (`Error`), `stream.rs`
(`CodecParameters`).

### Decoder

```rust
pub trait Decoder: Send {
    fn codec_id(&self) -> &CodecId;
    fn send_packet(&mut self, packet: &Packet) -> Result<()>;
    fn receive_frame(&mut self) -> Result<Frame>;
    fn flush(&mut self) -> Result<()>;
    fn reset(&mut self) -> Result<()> { /* flush + drain */ }
    fn set_execution_context(&mut self, _ctx: &ExecutionContext) {}
    fn output_pixel_format(&self) -> Option<PixelFormat> { None }
    fn output_params(&self) -> Option<CodecParameters> { /* default */ }
    fn output_params_generation(&self) -> u64 { 0 }
}
```

(`oxideav-core` 0.1.43 `registry/codec.rs:47-158`, 160-320.)

End of stream: after `flush`, `receive_frame` drains queued frames and
then returns `Error::Eof`. "Need more input" is `Error::NeedMore`
(`error.rs`, module docs and `Error::is_need_more`).

Audio shape lives on the stream, not the frame: `AudioFrame` carries
`samples: u32`, `pts: Option<i64>` and `data: Vec<Vec<u8>>`; sample
format, sample rate and channel layout come from
`Decoder::output_params()` (`frame.rs`, `stream.rs`).

### Encoder

```rust
pub trait Encoder: Send {
    fn codec_id(&self) -> &CodecId;
    fn output_params(&self) -> &CodecParameters;
    fn send_frame(&mut self, frame: &Frame) -> Result<()>;
    fn receive_packet(&mut self) -> Result<Packet>;
    fn flush(&mut self) -> Result<()>;
    fn set_execution_context(&mut self, _ctx: &ExecutionContext) {}
}
```

(`registry/codec.rs:415-443`.)

### The mapping

| OxideAV | `broadcast-codec` | Verdict |
| --- | --- | --- |
| `Decoder::send_packet(&Packet)` | `Decoder::send(Packet<'_>)` | fits; the adapter copies the borrowed bytes into OxideAV's owned `Vec<u8>` |
| `Decoder::receive_frame() -> Result<Frame>` | `Decoder::receive() -> Result<Option<Frame>>` | needs adapting; `NeedMore` and `Eof` both become `Ok(None)`, a media frame becomes `Ok(Some(_))`, other errors map to the seam's typed error |
| `Decoder::flush()` | `Decoder::finish()` | fits; both mean "no more input, drain" |
| `Decoder::reset()` | `Decoder::reset()` | fits |
| `Decoder::codec_id()` | `Decoder::parameters()` | needs adapting; the seam exposes the whole `CodecParameters`, which the adapter stores |
| `Decoder::output_params()` | per-frame `AudioFrame` fields | needs adapting; the seam repeats rate/channels/layout on every frame, OxideAV states them once per stream |
| `Packet { stream_index, time_base, pts, dts, duration, flags, data: Vec<u8> }` | `Packet { data: Cow<[u8]>, pts, dts, key }` | fits; `time_base` is dropped because the seam's `CodecParameters` carries one `Timescale`, `duration` and the extra flags have no seam counterpart |
| `AudioFrame { samples, pts, data: Vec<Vec<u8>> }` | `AudioFrame { sample_rate, channels, layout, samples: AudioSamples, pts }` | needs adapting; bytes must be decoded to the seam's typed sample buffer and planar planes concatenated (or interleaved kept) |
| `Frame::Video(VideoFrame)` | `Frame::Video(VideoFrame)` | not exercised by this audio spike; a plane/stride conversion would be needed and is **not established** here |
| `Frame::StreamParams` | no counterpart | needs adapting; the adapter re-reads `output_params()` on every frame and skips the event, so a mid-stream rate change reaches the seam through the per-frame `AudioFrame` fields |
| `Error::Eof` / `Error::NeedMore` | `Ok(None)` | fits, via the mapping above |
| `Error::Unsupported` | `CodecError::Unsupported` | fits |
| `Error::InvalidData` | `CodecError::Refused` | needs adapting; the seam distinguishes "refused input, still usable" from "failed, unusable", and OxideAV has no exact split |
| `TimeBase` + `pts: i64` | `Timescale` + `pts: i64` | fits unchanged; the adapter passes the caller's PTS through, and the test checks timestamps are unchanged |
| `Encoder::send_frame(&Frame)` | `Encoder::send(Frame)` | fits |
| `Encoder::receive_packet()` | `Encoder::receive()` | needs adapting, as `receive_frame` above |
| `Encoder::flush()` | `Encoder::finish()` | fits |

The published traits are `Send` but not `Sync`; the seam's traits have
no auto-trait bound, so object safety is unaffected.

## 5. Codecs published, per direction

### The codecs this spike wraps

| crate | version | decode | encode | the crate's own documentation on completeness |
| --- | --- | --- | --- | --- |
| `oxideav-aac` | 0.1.7 | AAC-LC, HE-AAC v1 (SBR), HE-AAC v2 (PS), AAC Main, SSR, ER AAC LC/LTP/LD, ER AAC scalable; ADTS and LOAS/LATM carriage | AAC-LC and HE-AAC v1 (SBR) | `README.md` "Status": "full AAC-LC decode chain end to end"; HE v1 "99.98% sample-exact"; HE v2 "5e-5 per-channel error-to-signal RMS"; Main predictor, LTP, SSR gain control, ER LC/LTP/LD and scalable wired in; BSAC front half only ("reconstructed PCM does not yet match the reference waveforms"); ER AAC LD TNS-bearing AUs (~6% of the `er_ad1103*` family) decode wrong PCM |
| `oxideav-mp3` | 0.1.4 | MPEG-1/2/2.5 Audio Layer III; `.mp3`/`.mp2`/`.mp1` demuxing (Layers I and II framed but not decoded) | Layer III CBR/VBR | `README.md` "Status": "clean-room rebuild in progress"; "What is verified against outside tools": 970 foreign-stream cells, 969 equal to both black-box decoders (one where those two disagree), RMS 0.25-0.29 LSB against float output; "Not yet supported": VBRI seek headers, gapless figures on the registry path, Layers I/II, ISO/IEC 13818-3 multichannel extension |
| `oxideav-opus` | 0.0.15 | Opus SILK, CELT and hybrid packets, in-band FEC | CELT, hybrid and SILK encoders, VBR | `README.md` "Status": "Clean-room rebuild in progress (orphan scaffold)"; SILK decode "bit-exact against the reference listing's decoder"; CELT fixtures at "~88-108 dB SNR"; hybrid "end-to-end"; the crate is honest that it is a rebuild in progress |
| `oxideav-basic` | 0.0.12 | linear PCM (`pcm_u8`, `pcm_s8`, `pcm_{s,u}{16,24,32}{le,be}`, `pcm_f{32,64}{le,be}`, `slin*` aliases), WAV/RF64/BW64 demux | the same PCM ids, WAV/RF64/BW64 mux | `README.md`: "Bit-exact both ways against black-box `ffmpeg -f <fmt>` conversions"; 64-bit integer PCM not registered; G.711 lives in `oxideav-g711` |

Each row's direction was confirmed in source, not only prose: `impl
Decoder` / `impl Encoder` exist in `oxideav-aac`'s `codec_decoder.rs` and
`codec_encoder.rs`, `oxideav-mp3`'s `codec_decoder.rs` and
`codec_encoder.rs`, `oxideav-opus`'s `registry.rs`, and
`oxideav-basic`'s `pcm.rs`.

### Other published audio codecs (not wrapped: no committed workspace fixture)

Versions are `cargo search oxideav-<name>` on 2026-11-26; direction and
completeness are that registry description, not a code read (marked
"registry description only").

| crate | version | direction, per registry description | completeness, per registry description |
| --- | --- | --- | --- |
| `oxideav-flac` | 0.0.13 | decoder, encoder, demuxer, muxer | "FLAC lossless audio codec and native container" (registry description only) |
| `oxideav-vorbis` | 0.0.13 | decoder and encoder | "sample-exact streaming decoder and a psychoacoustic whole-str..." (registry description only) |
| `oxideav-wavpack` | 0.0.5 | decoder | "lossless audio decoder" (registry description only) |
| `oxideav-tta` | 0.0.4 | decoder | "lossless audio decoder" (registry description only) |
| `oxideav-ape` | 0.0.5 | codec (direction not stated in the description) | "byte-exact whole-file dec..." (registry description only) |
| `oxideav-g711` | 0.0.8 | decoder + encoder | "ITU-T G.711 μ-law and A-law (PSTN) codec" (registry description only) |
| `oxideav-ac3` | 0.0.12 | decoder | "AC-3 (Dolby Digital) audio decoder" (registry description only) |
| `oxideav-dts` | 0.0.3 | decoder and Core encoder | "DTS audio decoder and Core encoder (Core profile, with EXSS/XCH/XXCH/X96/XLL extensions plan..." (registry description only) |
| `oxideav-gsm` | 0.0.11 | codec (direction not stated in the description) | "clean-room rebuild in progress" (registry description only) |
| `oxideav-g7231` | 0.0.9 | codec (direction not stated in the description) | "ITU-T G.723.1 dual-rate ... speech codec" (registry description only) |
| `oxideav-ilbc` | 0.0.8 | decoder | "iLBC ... audio decoder" (registry description only) |
| `oxideav-speex` | 0.0.12 | decoder + encoder | "NB/WB/UWB decoder + encoder with Ogg integration" (registry description only) |
| `oxideav-mod` | 0.0.11 | codec (direction not stated in the description) | "Amiga ProTracker / SoundTracker module (MOD) codec" (registry description only) |
| `oxideav-s3m` | 0.0.9 | codec (direction not stated in the description) | "Scream Tracker 3 Module (S3M) container + codec" (registry description only) |
| `oxideav-audiotoolbox` | 0.0.4 | hardware decode/encode bridge | "macOS AudioToolbox ... runtime-loaded" (registry description only) |
| `oxideav-adpcm` | 0.0.8 | decoder + encoder | "ADPCM audio decoders + encoders (Microsoft, IMA WAV/QuickTime, Yamaha A/B, OKI/Dialogic VO..." (registry description only) |
| `oxideav-wma` | 0.0.5 | decoder | "Windows Media Audio (WMA v1 + v2 baseline; later Pro + Lossless) decoder" (registry description only) |
| `oxideav-musepack` | 0.0.4 | decoder | "Musepack (SV7 + SV8) audio decoder" (registry description only) |
| `oxideav-shorten` | 0.0.4 | decoder + DIFFn encoder | "Shorten (.shn) lossless audio codec" (registry description only) |
| `oxideav-celt` | 0.1.16 | codec (direction not stated in the description) | "orphan-rebuild scaffold pending clean-room re-implementation"; the CELT layer of Opus (registry description only) |

The workspace has committed fixtures for AAC, MP3, Opus and linear PCM
only (`fixtures/audio/aac.adts`, `fixtures/audio/audio.mp3`,
`fixtures/audio/opus.ogg`, `fixtures/audio/pcm-s16le.wav`). The other
published audio codecs are therefore listed and not wrapped: adding a
decoder without a fixture would be an unverified claim.

## 6. Registry and profiles

**Registry: yes.** `oxideav-core` 0.1.43 has `CodecRegistry`
(`registry/codec.rs`): `register(CodecInfo)`, `has_decoder`,
`has_encoder`, `first_decoder`, `first_encoder`, `decoder_by_impl`,
`decoder_ids` (an iterator over registered ids), tag and payload-magic
resolution. Each codec crate exposes a `register` / `register_codecs`
function that populates it (`oxideav-aac`'s `codec_decoder.rs:478`,
`oxideav-mp3`'s `codec_decoder.rs:691`,
`oxideav-opus`'s `lib.rs:1156`, `oxideav-basic`'s `lib.rs:16`).
`oxideav-codec` re-exports the same registry.

**Profiles: no.** `CodecInfo` (`registry/codec.rs:536-599`) and
`CodecCapabilities` (`capabilities.rs`) carry no profile field, and
`grep -rn profile` over `registry/codec.rs`, `capabilities.rs` and
`stream.rs` finds only the unrelated ICC-profile metadata field. A
codec cannot state which profile of its format it handles; the only
granularity is the codec id and its free-form capability flags. The
adapter's profile lists therefore come from each crate's prose (the
`README.md` rows above), not from a machine-readable answer.

## Step 0 outcome

Every runtime dependency is MIT (or MIT-compatible) and every licence is
in `deny.toml`'s allow list. No `-sys`, `cc`, `bindgen`, `cmake` or
`links` in the adapter's graph. The top-level crates declare MIT. Step 0
passes; the adapter spike is allowed to proceed.

## Spike results

Measured on 2026-11-26 with `cargo test --locked` in
`spikes/codec-oxideav`, all four decoders obtained through a
`broadcast_codec::Registry` as `Box<dyn Decoder>`.

| test | result |
| --- | --- |
| T1 AAC loudness | **-21.087 LUFS over 49 152 samples**, against the pinned -21.1 +/- 0.1 LU; passes |
| T2 cross-check | both decoders 49 152 samples, priming offset **0** (equal lengths compared at offset 0), largest absolute difference 9.353e-4, RMS 3.125e-5; selectors `dec:aac=oxideav` and `dec:aac=symphonia` each served by the named provider (factory-call counters) |
| T3 MP3 | 50 688 samples decoded vs 48 000 from ffmpeg; aligned offset **2 257** samples, overlap 48 000, largest 1.524e-5, RMS **8.633e-6** |
| T3 Opus | 48 648 samples decoded vs 48 000 from ffmpeg; aligned offset **0**, overlap 48 000, largest 1.527e-5, RMS **8.844e-6** |
| T3 PCM | 48 000 samples, offset 0, largest 0, RMS **0** (bit-exact) |
| T4 inventory | four rows, all `available`, licence `MIT`; AAC states `lc/he/hev2` |
| T5 mutation | dropping every tenth input packet reduced AAC to 45 056 samples and failed the sample-count assertion; reverted |

Two findings come out of the adapter work:

1. **The AAC and Opus decoders do not override `Decoder::output_params`.**
   `oxideav-aac` has an inherent `AacDecoder::output_params`
   (`codec_decoder.rs:224`) but its `impl Decoder` does not forward it;
   `oxideav-opus`'s stream decoder has neither. Through
   `Box<dyn Decoder>` the trait answer is therefore `None`. The adapter
   falls back to the construction-time sample rate and channel count and
   to `S16`, which is correct for this fixture. It cannot follow an
   HE-AAC or Parametric Stereo output-shape change, because the frame
   carries no rate or channel count. MP3 and PCM do implement the trait
   method.
2. **MP3 and Opus sample counts include the container's lead-in/tail.**
   ffmpeg applies the LAME gapless trim to the MP3 fixture and the Ogg
   granule end trim to the Opus fixture; the registry path does not
   (`oxideav-mp3`'s README lists "gapless figures on the registry path"
   as not yet supported). The decoded audio matches ffmpeg over the whole
   trimmed span (RMS 8.6e-6 and 8.8e-6), so this is metadata handling,
   not a decode error. The strict sample-count tests are `#[ignore]`d
   with these measurements; the aligned-content tests stay green. An
   adapter cannot fix this from a bare packet: the end position is
   container state the seam does not carry.

The AAC decoder is the strongest result: its output is within 0.013 LU
of the pinned ffmpeg measurement and differs from symphonia's independent
decode by about one 16-bit LSB RMS.
