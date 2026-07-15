# transmux Streaming RTP Demux + SDP→CodecConfig Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a streaming, timing- and config-aware RTP depayloader to `transmux` (RFC 6184 H.264 / RFC 3640 AAC), plus SDP-fmtp→`CodecConfig` helpers, so #663 multimux can wrap it. (Issue #700.)

**Architecture:** The wire reassembly (FU-A/STAP-A/AAC AU-header) already exists and is tested in `transmux/src/rtp.rs`. We (1) refactor that reassembly to also return per-AU RTP timestamp + sync flag, (2) add fmtp→`CodecConfig` builders reusing the crate's existing `base64_decode`/`hex_decode`, and (3) add a stateful `RtpStreamDepacketizer` that consumes RTP packets incrementally and emits fully-timed, real-config `Sample`s. All additive — the existing batch `RtpDepacketizer` stays.

**Tech Stack:** Rust, `no_std`+`alloc` (transmux default), `rtp-packet` crate for the RTP fixed header, `broadcast_common` Parse/Serialize traits, `thiserror`-style `crate::error::Error`.

## Global Constraints

- MSRV **1.86**, edition 2024; build/test with `--locked`. After any dep touch, restore the MSRV-pinned `Cargo.lock` (no new deps are expected in this plan — do NOT add `base64`, a hand-rolled decoder already exists).
- transmux is `#![cfg_attr(not(feature = "std"), no_std)]` + `alloc`; **everything here must build `--no-default-features`** (use `alloc::{vec::Vec, string::String}`, not `std`).
- **Additive only** → transmux minor bump. Do not change or remove existing public API (`RtpDepacketizer`, `RtpInput`, etc.); the existing `transmux/tests/rtp.rs` suite must stay green unmodified.
- **No magic numbers** outside `#[cfg(test)]`: every constant named (reuse the existing `rtp.rs` consts; add named consts for new ones like NAL type 5 = IDR).
- **Spec citation** in every new module's `//!` doc (RFC 6184 §5.x / RFC 3640 §3.x), and a transcription committed to `transmux/docs/`.
- **Round-trip / symmetry discipline** and **biting tests** (real fixture, asserts that fail if the feature regresses — never happy-path-only).
- Any new public enum gets `name()` + `dvb_common::impl_spec_display!` per the #204 label convention, or is added to transmux's `label_coverage` SKIP list with reason.
- Errors use `crate::error::Error` variants (`BufferTooShort{need,have,what}`, `InvalidValue{field,value,reason}`, `InvalidInput(&'static str)`).

---

## File Structure

- **Modify** `transmux/src/rtp.rs` — extract timing-returning reassembly (`reassemble_video`, `reassemble_audio`) from the existing `depacketize_video`/`depacketize_audio`; the old fns delegate (discard timing) for back-compat. Add `NAL_TYPE_IDR` const + `au_is_sync` helper.
- **Create** `transmux/src/rtp_sdp.rs` — P2: `avc_config_from_sprop`, `aac_config_from_fmtp` (fmtp string → codec config). Reuses `rtp::{base64_decode, hex_decode}`.
- **Create** `transmux/src/rtp_stream.rs` — P1: `RtpStreamDepacketizer` + `RtpStreamTrack`, stateful per-track timing recovery.
- **Modify** `transmux/src/lib.rs` — `mod rtp_sdp; mod rtp_stream;` + re-exports.
- **Create** `transmux/docs/rtp-depacketization.md` — RFC 6184/3640 syntax transcription (freely redistributable RFC text).
- **Create** `transmux/tests/rtp_stream.rs` — TS-round-trip timing/config gate + SDP round-trip gate.
- **Modify** `transmux/CHANGELOG.md` — `[Unreleased]` additive entry.

---

### Task 1: Timing-returning reassembly core

Extract the AU reassembly so it returns per-AU RTP timestamp + sync flag, shared by the existing batch path and the new streaming path.

**Files:**
- Modify: `transmux/src/rtp.rs` (functions `depacketize_video` ~739-838, `depacketize_audio` ~852-899)
- Test: `transmux/src/rtp.rs` (in-module `#[cfg(test)]`) + existing `transmux/tests/rtp.rs` must still pass.

**Interfaces:**
- Produces:
  - `pub(crate) struct ReassembledAu { pub timestamp: u32, pub is_sync: bool, pub data: Vec<u8> }`
  - `pub(crate) fn reassemble_video(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>>`
  - `pub(crate) fn reassemble_audio(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>>`
  - `pub(crate) const NAL_TYPE_IDR: u8 = 5;`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `transmux/src/rtp.rs`:

```rust
#[test]
fn reassemble_video_reports_timestamp_and_sync() {
    // Two single-NAL AUs at different RTP timestamps; first is an IDR (type 5),
    // second a non-IDR slice (type 1). Marker bit ends each AU.
    // RTP fixed header: V=2 (0x80), PT=96; seq; timestamp; ssrc=0.
    fn pkt(seq: u16, ts: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
        let mut p = alloc::vec![0x80u8, if marker { 0x80 | 96 } else { 96 }];
        p.extend_from_slice(&seq.to_be_bytes());
        p.extend_from_slice(&ts.to_be_bytes());
        p.extend_from_slice(&[0, 0, 0, 0]); // ssrc
        p.extend_from_slice(nal);
        p
    }
    let idr = [0x65u8, 0xAA]; // nal_ref_idc=3, type=5 (IDR)
    let non = [0x41u8, 0xBB]; // nal_ref_idc=2, type=1 (non-IDR)
    let packets = alloc::vec![pkt(1, 1000, true, &idr), pkt(2, 4000, true, &non)];
    let aus = reassemble_video(&packets).unwrap();
    assert_eq!(aus.len(), 2);
    assert_eq!(aus[0].timestamp, 1000);
    assert!(aus[0].is_sync, "IDR AU must be sync");
    assert_eq!(aus[1].timestamp, 4000);
    assert!(!aus[1].is_sync, "non-IDR AU must not be sync");
    // data is length-prefixed NAL (4-byte length + NAL)
    assert_eq!(&aus[0].data[..4], &[0, 0, 0, 2]);
    assert_eq!(&aus[0].data[4..], &idr);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p transmux --lib rtp::tests::reassemble_video_reports_timestamp_and_sync`
Expected: FAIL — `cannot find function reassemble_video`.

- [ ] **Step 3: Implement the reassembly refactor**

In `transmux/src/rtp.rs`, add near the other consts:

```rust
/// RFC 6184 — H.264 IDR NAL unit type (nal_unit_type == 5).
pub(crate) const NAL_TYPE_IDR: u8 = 5;
```

Add the shared struct + functions (place above `depacketize_video`):

```rust
/// A reassembled access unit with its RTP presentation timestamp and a
/// random-access (sync) flag. RFC 6184 §5.7 (video) / RFC 3640 §3.2 (audio).
pub(crate) struct ReassembledAu {
    pub timestamp: u32,
    pub is_sync: bool,
    pub data: Vec<u8>,
}

/// H.264 FU-A/STAP-A/single-NAL reassembly (RFC 6184 §5.7/§5.8), preserving
/// the RTP timestamp and marking IDR access units as sync points.
pub(crate) fn reassemble_video(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>> {
    let mut aus: Vec<ReassembledAu> = Vec::new();
    let mut cur_nals: Vec<Vec<u8>> = Vec::new();
    let mut cur_ts: Option<u32> = None;
    let mut fu_buf: Vec<u8> = Vec::new();
    let mut fu_active = false;

    fn flush_au(aus: &mut Vec<ReassembledAu>, nals: &mut Vec<Vec<u8>>, ts: u32) {
        if nals.is_empty() {
            return;
        }
        let is_sync = nals
            .iter()
            .any(|n| !n.is_empty() && (n[0] & NAL_TYPE_MASK) == NAL_TYPE_IDR);
        aus.push(ReassembledAu {
            timestamp: ts,
            is_sync,
            data: length_prefix_nals(nals),
        });
        nals.clear();
    }

    for pkt in packets {
        let hdr = parse_rtp_header(pkt)?;
        let payload = hdr.payload;
        if payload.is_empty() {
            continue;
        }
        if let Some(ts) = cur_ts {
            if ts != hdr.timestamp && !cur_nals.is_empty() {
                flush_au(&mut aus, &mut cur_nals, ts);
            }
        }
        cur_ts = Some(hdr.timestamp);

        let nal_type = payload[0] & NAL_TYPE_MASK;
        match nal_type {
            NAL_TYPE_STAP_A => {
                let mut off = 1usize;
                while off < payload.len() {
                    if off + STAP_A_SIZE_LEN > payload.len() {
                        return Err(Error::BufferTooShort {
                            need: off + STAP_A_SIZE_LEN,
                            have: payload.len(),
                            what: "STAP-A size prefix",
                        });
                    }
                    let size =
                        u16::from_be_bytes([payload[off], payload[off + 1]]) as usize;
                    off += STAP_A_SIZE_LEN;
                    let end = off + size;
                    if end > payload.len() {
                        return Err(Error::BufferTooShort {
                            need: end,
                            have: payload.len(),
                            what: "STAP-A NAL",
                        });
                    }
                    cur_nals.push(payload[off..end].to_vec());
                    off = end;
                }
            }
            NAL_TYPE_FU_A => {
                if payload.len() < 2 {
                    return Err(Error::BufferTooShort {
                        need: 2,
                        have: payload.len(),
                        what: "FU-A header",
                    });
                }
                let fu_indicator = payload[0];
                let fu_header = payload[1];
                let is_start = fu_header & FU_START_MASK != 0;
                let is_end = fu_header & FU_END_MASK != 0;
                let orig_type = fu_header & NAL_TYPE_MASK;
                let fnri = fu_indicator & NAL_FNRI_MASK;
                if is_start {
                    fu_buf.clear();
                    fu_buf.push(fnri | orig_type);
                    fu_active = true;
                }
                if !fu_active {
                    return Err(Error::InvalidInput("FU-A fragment before start"));
                }
                fu_buf.extend_from_slice(&payload[2..]);
                if is_end {
                    cur_nals.push(core::mem::take(&mut fu_buf));
                    fu_active = false;
                }
            }
            _ => cur_nals.push(payload.to_vec()),
        }

        if hdr.marker && !cur_nals.is_empty() && !fu_active {
            let ts = hdr.timestamp;
            flush_au(&mut aus, &mut cur_nals, ts);
            cur_ts = None;
        }
    }
    if let Some(ts) = cur_ts {
        flush_au(&mut aus, &mut cur_nals, ts);
    }
    Ok(aus)
}

/// RFC 3640 AAC-hbr AU-header reassembly, preserving the RTP timestamp.
/// Audio AUs are always sync points.
pub(crate) fn reassemble_audio(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>> {
    let mut aus = Vec::new();
    for pkt in packets {
        let hdr = parse_rtp_header(pkt)?;
        let payload = hdr.payload;
        if payload.len() < AAC_AU_HEADERS_LENGTH_LEN {
            return Err(Error::BufferTooShort {
                need: AAC_AU_HEADERS_LENGTH_LEN,
                have: payload.len(),
                what: "AAC AU-headers-length",
            });
        }
        let au_headers_len_bits =
            u16::from_be_bytes([payload[0], payload[1]]) as usize;
        let header_bytes = au_headers_len_bits.div_ceil(8);
        let num_headers = au_headers_len_bits / (AAC_AU_HEADER_LEN * 8);
        let mut off = AAC_AU_HEADERS_LENGTH_LEN;
        if off + header_bytes > payload.len() {
            return Err(Error::BufferTooShort {
                need: off + header_bytes,
                have: payload.len(),
                what: "AAC AU headers",
            });
        }
        let mut sizes = Vec::with_capacity(num_headers);
        for h in 0..num_headers {
            let hoff = off + h * AAC_AU_HEADER_LEN;
            let ah = u16::from_be_bytes([payload[hoff], payload[hoff + 1]]);
            sizes.push((ah >> AAC_INDEX_LENGTH) as usize);
        }
        off += header_bytes;
        for size in sizes {
            let end = off + size;
            if end > payload.len() {
                return Err(Error::BufferTooShort {
                    need: end,
                    have: payload.len(),
                    what: "AAC AU payload",
                });
            }
            aus.push(ReassembledAu {
                timestamp: hdr.timestamp,
                is_sync: true,
                data: payload[off..end].to_vec(),
            });
            off = end;
        }
    }
    Ok(aus)
}
```

Then make the existing batch fns delegate (replace their bodies), preserving their `Vec<Vec<u8>>` return so the old path is byte-for-byte unchanged:

```rust
fn depacketize_video(packets: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
    Ok(reassemble_video(packets)?.into_iter().map(|au| au.data).collect())
}

fn depacketize_audio(packets: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
    Ok(reassemble_audio(packets)?.into_iter().map(|au| au.data).collect())
}
```

(If `length_prefix_nals`, `STAP_A_SIZE_LEN`, `NAL_FNRI_MASK`, `AAC_*` consts are not already visible at this scope, they are defined in this same file — no new consts needed beyond `NAL_TYPE_IDR`.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p transmux --lib rtp:: && cargo test -p transmux --test rtp`
Expected: PASS — new test green AND all existing `transmux/tests/rtp.rs` tests (`video_round_trip_byte_identical`, `audio_round_trip_byte_identical`, `fu_a_fragmentation_happens`, `valid_rtp_headers_and_marker_semantics`, `sdp_matches_demuxed_config`) still pass.

- [ ] **Step 5: Commit**

```bash
git add transmux/src/rtp.rs
git commit -m "refactor(transmux): timing-returning RTP reassembly core (#700)"
```

---

### Task 2: SDP sprop → AVCConfigurationBox (P2, H.264)

**Files:**
- Create: `transmux/src/rtp_sdp.rs`
- Modify: `transmux/src/lib.rs` (add `pub mod rtp_sdp;` + re-export)
- Test: in-module `#[cfg(test)]`

**Interfaces:**
- Consumes: `rtp::base64_decode` (pub, `rtp.rs:1080`), `avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord}`, `nalu_types::{AvcSps, AvcPps}`, `crate::error::{Error, Result}`.
- Produces: `pub fn avc_config_from_sprop(sprop_parameter_sets: &str) -> Result<AVCConfigurationBox>`

- [ ] **Step 1: Write the failing test**

Create `transmux/src/rtp_sdp.rs` with only the module doc + test first:

```rust
//! SDP fmtp → transmux `CodecConfig` (RFC 6184 §8.1 / RFC 3640 §4.1).
//!
//! Turns the media-format parameters carried in an RTSP DESCRIBE SDP into the
//! codec configuration transmux muxers need: H.264 `sprop-parameter-sets`
//! (base64 SPS/PPS) → `avcC`, AAC `config` (hex AudioSpecificConfig) → `esds`.
//! The caller (e.g. multimux) extracts the raw fmtp attribute strings via an
//! SDP parser; this module owns only the codec-config construction, because
//! transmux owns `AVCConfigurationBox`/`EsdsBox`.

use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::nalu_types::{AvcPps, AvcSps};
use crate::rtp::base64_decode;
use alloc::vec::Vec;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtp::base64_encode;

    #[test]
    fn sprop_round_trips_sps_pps_and_profile() {
        // A minimal but real SPS (type 7) and PPS (type 8).
        // SPS bytes after the NAL header byte: profile_idc, constraints, level_idc.
        let sps = alloc::vec![0x67u8, 0x42, 0xC0, 0x1E, 0xAB]; // profile 0x42, level 0x1E
        let pps = alloc::vec![0x68u8, 0xCE, 0x3C, 0x80];
        let sprop = alloc::format!("{},{}", base64_encode(&sps), base64_encode(&pps));

        let boxed = avc_config_from_sprop(&sprop).unwrap();
        let r = &boxed.config;
        assert_eq!(r.sps.len(), 1);
        assert_eq!(r.pps.len(), 1);
        assert_eq!(r.sps[0].0, sps);
        assert_eq!(r.pps[0].0, pps);
        assert_eq!(r.profile_indication, 0x42);
        assert_eq!(r.profile_compatibility, 0xC0);
        assert_eq!(r.level_indication, 0x1E);
        assert_eq!(r.length_size_minus_one, 3);
    }

    #[test]
    fn sprop_rejects_when_no_sps() {
        // Only a PPS (type 8) present → no SPS → error.
        let pps = alloc::vec![0x68u8, 0xCE];
        let sprop = base64_encode(&pps);
        assert!(avc_config_from_sprop(&sprop).is_err());
    }
}
```

Add to `transmux/src/lib.rs` (near the other `mod` lines): `pub mod rtp_sdp;`

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p transmux --lib rtp_sdp`
Expected: FAIL — `cannot find function avc_config_from_sprop`.

- [ ] **Step 3: Implement `avc_config_from_sprop`**

Add to `transmux/src/rtp_sdp.rs` (after the `use` lines):

```rust
/// H.264 NAL unit type mask (RFC 6184) and the SPS/PPS type values.
const NAL_TYPE_MASK: u8 = 0x1F;
const NAL_TYPE_SPS: u8 = 7;
const NAL_TYPE_PPS: u8 = 8;
/// Length prefix size transmux uses for coded NALs (4-byte).
const NAL_LENGTH_SIZE_MINUS_ONE: u8 = 3;

/// Parse an SDP `sprop-parameter-sets` value (RFC 6184 §8.1: comma-separated
/// base64 parameter-set NAL units) into an `avcC` configuration box.
///
/// SPS units (nal_unit_type 7) supply `profile_indication` /
/// `profile_compatibility` / `level_indication` (SPS bytes `[1..4]` after the
/// NAL header). At least one SPS is required.
pub fn avc_config_from_sprop(sprop_parameter_sets: &str) -> Result<AVCConfigurationBox> {
    let mut sps: Vec<AvcSps> = Vec::new();
    let mut pps: Vec<AvcPps> = Vec::new();
    for token in sprop_parameter_sets.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let nal = base64_decode(token)?;
        if nal.is_empty() {
            return Err(Error::InvalidInput("empty sprop parameter set"));
        }
        match nal[0] & NAL_TYPE_MASK {
            NAL_TYPE_SPS => sps.push(AvcSps(nal)),
            NAL_TYPE_PPS => pps.push(AvcPps(nal)),
            _ => return Err(Error::InvalidInput("sprop NAL is neither SPS nor PPS")),
        }
    }
    let first_sps = sps.first().ok_or(Error::InvalidInput(
        "sprop-parameter-sets contained no SPS",
    ))?;
    if first_sps.0.len() < 4 {
        return Err(Error::BufferTooShort {
            need: 4,
            have: first_sps.0.len(),
            what: "SPS profile/level bytes",
        });
    }
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: first_sps.0[1],
        profile_compatibility: first_sps.0[2],
        level_indication: first_sps.0[3],
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        sps,
        pps,
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: Vec::new(),
    };
    Ok(AVCConfigurationBox::new(record))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p transmux --lib rtp_sdp`
Expected: PASS — both tests green.

- [ ] **Step 5: Commit**

```bash
git add transmux/src/rtp_sdp.rs transmux/src/lib.rs
git commit -m "feat(transmux): SDP sprop-parameter-sets -> avcC (#700)"
```

---

### Task 3: SDP config → CodecConfig::Aac (P2, AAC)

**Files:**
- Modify: `transmux/src/rtp_sdp.rs`
- Test: in-module `#[cfg(test)]`

**Interfaces:**
- Consumes: `rtp::hex_decode` (pub, `rtp.rs:1123`), `aac_asc::AudioSpecificConfig` (`Parse`), `mp4esds::{EsdsBox, ESDescriptor, DecoderConfigDescriptor, DecoderSpecificInfo, ObjectTypeIndication, StreamType, SLConfigDescriptor}`, `pipeline::CodecConfig`.
- Produces: `pub fn aac_config_from_fmtp(config_hex: &str) -> Result<CodecConfig>`

> **Step 0 (read before coding):** Open `transmux/src/mp4esds.rs` and confirm the exact constructors/field names for `ESDescriptor`, `DecoderConfigDescriptor`, `DecoderSpecificInfo`, `ObjectTypeIndication`, `StreamType`, `SLConfigDescriptor`, and open `transmux/src/aac_asc.rs` for how `sampling_frequency`/`SamplingFrequencyIndex::raw()`/`ChannelConfiguration::raw()` expose rate + channels. Use whatever the real constructors are (the report gave field-literal forms; the crate may expose `::new` helpers — prefer those). The test below pins the *observable* result (sample_rate, channels, round-tripped ASC bytes), so any correct construction satisfies it.

- [ ] **Step 1: Write the failing test**

Add to `transmux/src/rtp_sdp.rs` `mod tests`:

```rust
#[test]
fn aac_fmtp_config_recovers_rate_channels_and_asc() {
    // AudioSpecificConfig for AAC-LC, 44100 Hz (freq index 4), stereo (2ch):
    // audioObjectType=2 (5 bits), samplingFreqIndex=4 (4 bits),
    // channelConfig=2 (4 bits) => bits: 00010 0100 0010 000 = 0x12 0x10
    let config_hex = "1210";
    let cfg = aac_config_from_fmtp(config_hex).unwrap();
    match cfg {
        crate::pipeline::CodecConfig::Aac {
            sample_rate,
            channel_count,
            esds,
            ..
        } => {
            assert_eq!(sample_rate, 44100);
            assert_eq!(channel_count, 2);
            // The ASC bytes must survive into the esds decoder-specific info.
            let dsi = esds
                .es_descriptor
                .decoder_config
                .as_ref()
                .unwrap()
                .decoder_specific_info
                .as_ref()
                .unwrap();
            assert_eq!(dsi.data, alloc::vec![0x12u8, 0x10]);
        }
        _ => panic!("expected CodecConfig::Aac"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p transmux --lib rtp_sdp::tests::aac_fmtp_config_recovers`
Expected: FAIL — `cannot find function aac_config_from_fmtp`.

- [ ] **Step 3: Implement `aac_config_from_fmtp`**

Add to `transmux/src/rtp_sdp.rs`. Use the real `mp4esds` constructors confirmed in Step 0; the skeleton below shows the required shape (AAC object-type-indication `0x40`, audio stream-type `5`, ASC bytes as decoder-specific info):

```rust
use crate::aac_asc::AudioSpecificConfig;
use crate::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox,
    ObjectTypeIndication, SLConfigDescriptor, StreamType,
};
use crate::pipeline::CodecConfig;
use crate::rtp::hex_decode;
use broadcast_common::Parse;

/// MPEG-4 Audio object-type indication and audio stream-type (ISO/IEC 14496-1).
const OTI_AUDIO_ISO14496_3: u8 = 0x40;
const STREAM_TYPE_AUDIO: u8 = 5;
const AAC_SAMPLE_SIZE_BITS: u16 = 16;

/// Parse an SDP AAC `config` fmtp value (RFC 3640 §4.1: hex-encoded
/// AudioSpecificConfig) into `CodecConfig::Aac`, recovering sample rate and
/// channel count from the ASC and carrying the ASC bytes in the `esds`.
pub fn aac_config_from_fmtp(config_hex: &str) -> Result<CodecConfig> {
    let asc_bytes = hex_decode(config_hex)?;
    let asc = AudioSpecificConfig::parse(&asc_bytes)?;
    let sample_rate = asc_sample_rate(&asc)?;
    let channel_count = u16::from(asc.channel_configuration.raw());

    let esds = EsdsBox::new(ESDescriptor {
        es_id: 0,
        stream_dependence_flag: false,
        url_flag: false,
        ocr_stream_flag: false,
        stream_priority: 0,
        depends_on_es_id: None,
        url: None,
        ocr_es_id: None,
        decoder_config: Some(DecoderConfigDescriptor {
            object_type_indication: ObjectTypeIndication(OTI_AUDIO_ISO14496_3),
            stream_type: StreamType(STREAM_TYPE_AUDIO),
            up_stream: false,
            buffer_size_db: 0,
            max_bitrate: 0,
            avg_bitrate: 0,
            decoder_specific_info: Some(DecoderSpecificInfo { data: asc_bytes }),
        }),
        sl_config: Some(SLConfigDescriptor { body: alloc::vec![0x02] }),
    });

    Ok(CodecConfig::Aac {
        esds,
        channel_count,
        sample_rate,
        sample_size: AAC_SAMPLE_SIZE_BITS,
    })
}

/// Sample rate from the ASC: the explicit escape value if present, otherwise
/// the ISO/IEC 14496-3 Table 1.10 frequency for the index.
fn asc_sample_rate(asc: &AudioSpecificConfig) -> Result<u32> {
    if let Some(freq) = asc.sampling_frequency {
        return Ok(freq);
    }
    asc.sampling_frequency_index
        .frequency_hz()
        .ok_or(Error::InvalidValue {
            field: "sampling_frequency_index",
            value: u64::from(asc.sampling_frequency_index.raw()),
            reason: "no frequency for index",
        })
}
```

> If `SamplingFrequencyIndex` has no `frequency_hz()` method, add a small `match` over the index → Hz per ISO/IEC 14496-3 Table 1.10 as a private const-backed helper in this module (named constants, no bare literals), and cite the table. If `ChannelConfiguration` has no `raw()`, use its documented accessor. Confirm both in Step 0.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p transmux --lib rtp_sdp`
Expected: PASS — all `rtp_sdp` tests green.

- [ ] **Step 5: Commit**

```bash
git add transmux/src/rtp_sdp.rs
git commit -m "feat(transmux): SDP AAC config= -> CodecConfig::Aac (#700)"
```

---

### Task 4: Streaming RTP depayloader (P1)

**Files:**
- Create: `transmux/src/rtp_stream.rs`
- Modify: `transmux/src/lib.rs` (add `pub mod rtp_stream;` + re-exports)
- Test: in-module `#[cfg(test)]`

**Interfaces:**
- Consumes: `rtp::{reassemble_video, reassemble_audio, ReassembledAu, RtpMediaKind, parse_rtp_header}`, `pipeline::{Sample, TrackSpec, CodecConfig}`, `crate::error::{Error, Result}`.
- Produces:
  - `pub struct RtpStreamTrack { pub track_id: u32, pub kind: RtpMediaKind, pub config: CodecConfig, pub clock_rate: u32 }`
  - `pub struct RtpStreamDepacketizer { .. }`
  - `pub fn new(tracks: Vec<RtpStreamTrack>) -> Self`
  - `pub fn track_specs(&self) -> Vec<TrackSpec>` (track_id, timescale = clock_rate, config)
  - `pub fn push(&mut self, track_id: u32, rtp_packet: &[u8]) -> Result<Vec<Sample>>`
  - `pub fn flush(&mut self, track_id: u32) -> Result<Vec<Sample>>`

> Requires `RtpMediaKind`, `parse_rtp_header`, `reassemble_video`, `reassemble_audio`, `ReassembledAu` reachable from `rtp_stream`. `RtpMediaKind` is already `pub`. Mark `parse_rtp_header`, `reassemble_video`, `reassemble_audio`, `ReassembledAu` as `pub(crate)` (Task 1 already made the reassembly fns `pub(crate)`; ensure `parse_rtp_header` is at least `pub(crate)`).

**Timing model (documented in the module doc):**
- Per track, timescale = `clock_rate` (video 90 000; AAC = sample rate). Sample `duration` = RTP-timestamp delta to the *next* AU (so a sample is emitted only once the following AU's timestamp is known — one-AU latency). 32-bit RTP timestamps are unwrapped to `u64`.
- `is_sync` from `ReassembledAu` (IDR for video; always true for audio).
- `composition_offset = 0` — **v1 assumes low-delay / no B-frame reorder** (RTP carries presentation time only; DTS reconstruction with B-frames is future work). Document this limit.
- Each track rebases its first unwrapped timestamp to `start_decode_time = 0`; cross-track A/V alignment via RTCP SR/NTP is out of v1 scope (document).

- [ ] **Step 1: Write the failing test**

Create `transmux/src/rtp_stream.rs`:

```rust
//! Streaming RTP depayloader — RFC 6184 (H.264) / RFC 3640 (AAC).
//!
//! Stateful counterpart to [`crate::rtp::RtpDepacketizer`]: fed RTP packets
//! incrementally via [`RtpStreamDepacketizer::push`], it emits fully-timed
//! [`Sample`]s (real per-AU `duration` from RTP-timestamp deltas, `is_sync`
//! from IDR detection) carrying the real [`CodecConfig`] supplied at
//! construction (e.g. from [`crate::rtp_sdp`]). v1 assumes low-delay H.264
//! (no B-frame reorder → `composition_offset = 0`); cross-track A/V sync via
//! RTCP SR is future work.

use crate::error::Result;
use crate::pipeline::{CodecConfig, Sample, TrackSpec};
use crate::rtp::{reassemble_audio, reassemble_video, RtpMediaKind};
use alloc::vec::Vec;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};

    fn dummy_avc() -> CodecConfig {
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 0x42,
                profile_compatibility: 0,
                level_indication: 0x1E,
                length_size_minus_one: 3,
                sps: alloc::vec![],
                pps: alloc::vec![],
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: alloc::vec![],
            }),
            width: 0,
            height: 0,
        }
    }

    fn vpkt(seq: u16, ts: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
        let mut p = alloc::vec![0x80u8, if marker { 0x80 | 96 } else { 96 }];
        p.extend_from_slice(&seq.to_be_bytes());
        p.extend_from_slice(&ts.to_be_bytes());
        p.extend_from_slice(&[0, 0, 0, 0]);
        p.extend_from_slice(nal);
        p
    }

    #[test]
    fn video_stream_recovers_durations_and_sync() {
        let mut d = RtpStreamDepacketizer::new(alloc::vec![RtpStreamTrack {
            track_id: 1,
            kind: RtpMediaKind::H264,
            config: dummy_avc(),
            clock_rate: 90_000,
        }]);

        // AU0 @1000 (IDR), AU1 @4000 (non-IDR), AU2 @7000 (non-IDR). 3000-tick spacing.
        let idr = [0x65u8, 0xAA];
        let non = [0x41u8, 0xBB];
        // AU0: emits nothing yet (duration needs AU1).
        assert!(d.push(1, &vpkt(1, 1000, true, &idr)).unwrap().is_empty());
        // AU1 arrives → AU0 emitted with duration 3000, is_sync=true.
        let s0 = d.push(1, &vpkt(2, 4000, true, &non)).unwrap();
        assert_eq!(s0.len(), 1);
        assert_eq!(s0[0].duration, 3000);
        assert!(s0[0].is_sync);
        assert_eq!(s0[0].composition_offset, 0);
        // AU2 arrives → AU1 emitted, duration 3000, is_sync=false.
        let s1 = d.push(1, &vpkt(3, 7000, true, &non)).unwrap();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].duration, 3000);
        assert!(!s1[0].is_sync);
        // flush → AU2 emitted with the last-known duration (3000).
        let s2 = d.flush(1).unwrap();
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].duration, 3000);
    }

    #[test]
    fn track_specs_use_clock_rate_as_timescale() {
        let d = RtpStreamDepacketizer::new(alloc::vec![RtpStreamTrack {
            track_id: 7,
            kind: RtpMediaKind::H264,
            config: dummy_avc(),
            clock_rate: 90_000,
        }]);
        let specs = d.track_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].track_id, 7);
        assert_eq!(specs[0].timescale, 90_000);
    }
}
```

Add to `transmux/src/lib.rs`: `pub mod rtp_stream;` and (near existing re-exports) `pub use rtp_stream::{RtpStreamDepacketizer, RtpStreamTrack};`

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p transmux --lib rtp_stream`
Expected: FAIL — types/functions not found.

- [ ] **Step 3: Implement the streaming depacketizer**

Add to `transmux/src/rtp_stream.rs` (after the `use` lines, before `#[cfg(test)]`):

```rust
/// One track's decode config for [`RtpStreamDepacketizer`].
pub struct RtpStreamTrack {
    pub track_id: u32,
    pub kind: RtpMediaKind,
    /// Real codec config (e.g. from [`crate::rtp_sdp`]).
    pub config: CodecConfig,
    /// RTP clock rate (Hz) — also used as the IR track timescale.
    pub clock_rate: u32,
}

struct TrackState {
    kind: RtpMediaKind,
    config: CodecConfig,
    clock_rate: u32,
    /// Packets accumulated for the current (not-yet-complete) RTP timestamp.
    cur_ts: Option<u32>,
    cur_pkts: Vec<Vec<u8>>,
    /// Unwrapped 64-bit form of the most recent RTP timestamp seen.
    last_unwrapped: Option<u64>,
    /// AU awaiting its duration (filled when the next AU's timestamp arrives).
    pending: Option<PendingAu>,
    /// Last computed duration, used for the final flushed AU.
    last_duration: u32,
    emitted_first: bool,
}

struct PendingAu {
    unwrapped_ts: u64,
    is_sync: bool,
    data: Vec<u8>,
}

/// Stateful, timing- and config-aware RTP depayloader (see module docs).
pub struct RtpStreamDepacketizer {
    tracks: Vec<(u32, TrackState)>,
}

/// RTP timestamps are 32-bit and wrap; unwrap relative to the previous value.
fn unwrap_ts(prev: Option<u64>, ts: u32) -> u64 {
    match prev {
        None => u64::from(ts),
        Some(prev) => {
            let prev_low = prev & 0xFFFF_FFFF;
            let hi = prev & !0xFFFF_FFFF;
            let cur = u64::from(ts);
            if cur.wrapping_sub(prev_low) as i64 >= 0 || prev_low < 0x8000_0000 {
                // forward within the same epoch (or small backward jitter)
                if cur >= prev_low {
                    hi + cur
                } else if prev_low - cur > 0x8000_0000 {
                    hi + (1 << 32) + cur // wrapped forward
                } else {
                    hi + cur // minor reorder; keep epoch
                }
            } else {
                hi + cur
            }
        }
    }
}

impl RtpStreamDepacketizer {
    pub fn new(tracks: Vec<RtpStreamTrack>) -> Self {
        let tracks = tracks
            .into_iter()
            .map(|t| {
                (
                    t.track_id,
                    TrackState {
                        kind: t.kind,
                        config: t.config,
                        clock_rate: t.clock_rate,
                        cur_ts: None,
                        cur_pkts: Vec::new(),
                        last_unwrapped: None,
                        pending: None,
                        last_duration: 0,
                        emitted_first: false,
                    },
                )
            })
            .collect();
        Self { tracks }
    }

    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.tracks
            .iter()
            .map(|(id, st)| TrackSpec::new(*id, st.clock_rate, st.config.clone()))
            .collect()
    }

    fn state(&mut self, track_id: u32) -> Option<&mut TrackState> {
        self.tracks
            .iter_mut()
            .find(|(id, _)| *id == track_id)
            .map(|(_, st)| st)
    }

    pub fn push(&mut self, track_id: u32, rtp_packet: &[u8]) -> Result<Vec<Sample>> {
        let st = match self.state(track_id) {
            Some(st) => st,
            None => return Ok(Vec::new()),
        };
        let hdr = crate::rtp::parse_rtp_header(rtp_packet)?;
        let ts = hdr.timestamp;
        let mut out = Vec::new();

        // Timestamp change → the previous timestamp's packets form complete AU(s).
        if let Some(cur) = st.cur_ts {
            if cur != ts && !st.cur_pkts.is_empty() {
                Self::drain_complete(st, &mut out)?;
            }
        }
        st.cur_ts = Some(ts);
        st.cur_pkts.push(rtp_packet.to_vec());

        // Video marker bit ends an AU immediately.
        if matches!(st.kind, RtpMediaKind::H264) && hdr.marker {
            Self::drain_complete(st, &mut out)?;
            st.cur_ts = None;
        }
        Ok(out)
    }

    pub fn flush(&mut self, track_id: u32) -> Result<Vec<Sample>> {
        let st = match self.state(track_id) {
            Some(st) => st,
            None => return Ok(Vec::new()),
        };
        let mut out = Vec::new();
        if !st.cur_pkts.is_empty() {
            Self::drain_complete(st, &mut out)?;
            st.cur_ts = None;
        }
        // Emit the final pending AU with the last-known duration.
        if let Some(p) = st.pending.take() {
            out.push(Sample::new(p.data, st.last_duration, p.is_sync, 0));
        }
        Ok(out)
    }

    /// Reassemble the buffered packets into AUs, then for each AU: unwrap its
    /// timestamp, and emit the previously-pending AU with duration = delta.
    fn drain_complete(st: &mut TrackState, out: &mut Vec<Sample>) -> Result<()> {
        let pkts = core::mem::take(&mut st.cur_pkts);
        let aus = match st.kind {
            RtpMediaKind::H264 => reassemble_video(&pkts)?,
            RtpMediaKind::Aac => reassemble_audio(&pkts)?,
        };
        for au in aus {
            let unwrapped = unwrap_ts(st.last_unwrapped, au.timestamp);
            st.last_unwrapped = Some(unwrapped);
            if let Some(prev) = st.pending.take() {
                let delta = unwrapped.saturating_sub(prev.unwrapped_ts);
                let duration = u32::try_from(delta).unwrap_or(u32::MAX);
                st.last_duration = duration;
                out.push(Sample::new(prev.data, duration, prev.is_sync, 0));
                st.emitted_first = true;
            }
            st.pending = Some(PendingAu {
                unwrapped_ts: unwrapped,
                is_sync: au.is_sync,
                data: au.data,
            });
        }
        Ok(())
    }
}
```

> Note: `CodecConfig` must be `Clone` for `track_specs`/`state.config.clone()`. Confirm it derives `Clone` (it is used by value across the crate; if not, store the config in an `alloc::rc`-free way — but per pipeline.rs it derives `Clone`). If `parse_rtp_header`'s return type is private, add a `pub(crate)` accessor or make its `RtpHeader` fields `pub(crate)`; the only fields used here are `timestamp` and `marker`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p transmux --lib rtp_stream`
Expected: PASS — both tests green.

- [ ] **Step 5: Commit**

```bash
git add transmux/src/rtp_stream.rs transmux/src/lib.rs
git commit -m "feat(transmux): streaming timing+config-aware RTP depayloader (#700)"
```

---

### Task 5: Real-fixture gate — TS round-trip + SDP round-trip

Proves the whole feature on a real broadcast fixture: demux `h264_aac.ts` (real timing + config) → packetize to RTP → stream-depacketize → the recovered samples must match in config, sync points, and total duration, and build a valid fMP4; and the generated SDP must parse back (via P2) to the same config.

**Files:**
- Create: `transmux/tests/rtp_stream.rs`
- Uses fixture: `transmux/tests/fixtures/ts/h264_aac.ts` (already committed)

**Interfaces:**
- Consumes: the crate's existing TS demux entry point and `RtpPacketizer::package` (as used by `transmux/tests/rtp.rs` — reuse the exact same calls that file uses to obtain a `Media` and an `RtpOutput`), plus `RtpStreamDepacketizer`, `avc_config_from_sprop`, `aac_config_from_fmtp`.

> **Step 0 (read before coding):** Open `transmux/tests/rtp.rs` and copy verbatim the helper(s) it uses to (a) demux `h264_aac.ts` into a `Media` and (b) call `packetize(&media) -> RtpOutput { streams, sdp }`. Reuse those exact calls so this test can't drift from the established fixture path. Note the `RtpStream`/`RtpOutput` field names it uses to get each stream's ordered packet list + payload type.

- [ ] **Step 1: Write the failing test**

Create `transmux/tests/rtp_stream.rs`. Fill the demux/packetize calls from Step 0 where marked:

```rust
//! Real-fixture gate for the streaming RTP depayloader (#700).
#![cfg(feature = "std")]

use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;
use transmux::rtp_sdp::{aac_config_from_fmtp, avc_config_from_sprop};
use transmux::{RtpStreamDepacketizer, RtpStreamTrack};

// Pull one fmtp attribute value ("sprop-parameter-sets=" / "config=") out of an
// SDP string. Test-only crude extraction (no sdp-types dependency in transmux).
fn fmtp_value<'a>(sdp: &'a str, key: &str) -> Option<&'a str> {
    for line in sdp.lines() {
        if let Some(idx) = line.find(key) {
            let rest = &line[idx + key.len()..];
            let end = rest.find([';', ' ', '\r', '\n']).unwrap_or(rest.len());
            return Some(&rest[..end]);
        }
    }
    None
}

#[test]
fn ts_round_trip_recovers_timing_config_and_builds_fmp4() {
    // --- Step 0: obtain `media` (demux h264_aac.ts) and `out` (packetize) ---
    // let media = /* demux transmux/tests/fixtures/ts/h264_aac.ts, per rtp.rs */;
    // let out = /* packetize(&media), per rtp.rs → RtpOutput { streams, sdp } */;

    // Original per-track truth from the demuxed Media.
    let orig_video = media.tracks.iter().find(|t| matches!(
        t.spec.config, CodecConfig::Avc { .. })).expect("video track");
    let orig_video_syncs = orig_video.samples.iter().filter(|s| s.is_sync).count();
    let orig_video_total: u64 =
        orig_video.samples.iter().map(|s| u64::from(s.duration)).sum();

    // Build codec config from the generated SDP (exercises P2).
    let sprop = fmtp_value(&out.sdp, "sprop-parameter-sets=").expect("sprop");
    let avc = avc_config_from_sprop(sprop).expect("avc from sprop");
    // SPS/PPS bytes recovered from SDP must equal the fixture's.
    if let CodecConfig::Avc { config, .. } = &orig_video.spec.config {
        assert_eq!(avc.config.sps.len(), config.sps.len());
        assert_eq!(avc.config.sps[0].0, config.sps[0].0, "SPS bytes round-trip");
        assert_eq!(avc.config.pps[0].0, config.pps[0].0, "PPS bytes round-trip");
    }

    // Feed the packetized RTP for the video stream through the streaming depayloader.
    // `out.streams` entries carry an ordered `Vec<Vec<u8>>` of RTP packets and a
    // kind/payload-type; use the field names confirmed in Step 0.
    let video_stream = /* the H264 RtpStream from out.streams */;
    let mut d = RtpStreamDepacketizer::new(vec![RtpStreamTrack {
        track_id: 1,
        kind: RtpMediaKind::H264,
        config: CodecConfig::Avc {
            config: avc.config.clone_box_or_rebuild(), // use avc (see note)
            width: 0,
            height: 0,
        },
        clock_rate: 90_000,
    }]);
    let mut recovered = Vec::new();
    for pkt in /* video_stream packets iter */ {
        recovered.extend(d.push(1, pkt).unwrap());
    }
    recovered.extend(d.flush(1).unwrap());

    // Recovered sample count within 1 of the original (last-AU flush edge).
    assert!(
        (recovered.len() as i64 - orig_video.samples.len() as i64).abs() <= 1,
        "recovered {} vs original {}", recovered.len(), orig_video.samples.len()
    );
    // Sync points preserved.
    let rec_syncs = recovered.iter().filter(|s| s.is_sync).count();
    assert_eq!(rec_syncs, orig_video_syncs, "keyframe count preserved");
    // Total duration within one frame of the original (one-AU flush tolerance).
    let rec_total: u64 = recovered.iter().map(|s| u64::from(s.duration)).sum();
    let frame = orig_video.samples.first().map(|s| u64::from(s.duration)).unwrap_or(3000);
    assert!(
        rec_total.abs_diff(orig_video_total) <= frame,
        "total duration {} vs {}", rec_total, orig_video_total
    );

    // AAC: SDP config= → CodecConfig::Aac, rate/channels sane.
    if let Some(cfg_hex) = fmtp_value(&out.sdp, "config=") {
        let aac = aac_config_from_fmtp(cfg_hex).expect("aac from config");
        match aac {
            CodecConfig::Aac { sample_rate, channel_count, .. } => {
                assert!(sample_rate >= 8_000 && sample_rate <= 96_000);
                assert!(channel_count >= 1 && channel_count <= 8);
            }
            _ => panic!("expected AAC"),
        }
    }

    // Recovered video samples build a valid fMP4 init+segment.
    let specs = d.track_specs();
    // Build init + one media segment from `specs` + `recovered` using the crate's
    // build_init_segment / build_media_segment (per Step 0 signatures) and assert
    // both parse via transmux's fMP4 validator (reuse the validator call other
    // transmux tests use). Non-empty + parses = pass.
}
```

> **`avc.config.clone_box_or_rebuild()` is a placeholder for a real call** — do not write that method. Simplest: call `avc_config_from_sprop(sprop)` a second time to get a fresh `AVCConfigurationBox` for the depacketizer (cheap), or `.clone()` if `AVCConfigurationBox: Clone`. Use whichever compiles; the intent is "give the depacketizer the P2-derived config."
>
> Fill the three Step-0 placeholders (demux, packetize, per-stream packet iteration) and the fMP4 build+validate tail with the exact calls the existing `transmux/tests/rtp.rs` and CMAF/segment tests use. The assertions above are the binding gate; the plumbing must match the real API.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p transmux --test rtp_stream --features std`
Expected: FAIL initially (unfilled placeholders / compile), then once plumbed, must PASS.

- [ ] **Step 3: Fill the plumbing until it passes**

Wire the Step-0 demux/packetize/validate calls. No new production code should be needed — if the test reveals a real bug in Tasks 1–4, fix it there (and note it), don't paper over it in the test.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p transmux --test rtp_stream --features std`
Expected: PASS — timing/config/sync recovered, fMP4 validates.

- [ ] **Step 5: Commit**

```bash
git add transmux/tests/rtp_stream.rs
git commit -m "test(transmux): real-fixture RTP-stream timing+config+SDP round-trip gate (#700)"
```

---

### Task 6: Docs, spec transcription, no_std + full gate, CHANGELOG

**Files:**
- Create: `transmux/docs/rtp-depacketization.md`
- Modify: `transmux/src/rtp_stream.rs`, `transmux/src/rtp_sdp.rs` (ensure `//!` cite the transcription), `transmux/CHANGELOG.md`

- [ ] **Step 1: Write the RFC transcription**

Create `transmux/docs/rtp-depacketization.md` transcribing the payload-format syntax used: RFC 6184 §5.2 (NAL/STAP-A/FU-A packet structures + the FU header S/E/R bits + nal_unit_type table incl. 5=IDR,7=SPS,8=PPS,24=STAP-A,28=FU-A), §8.1 (`sprop-parameter-sets`), and RFC 3640 §3.2–3.3 (AU-headers-length + AU-header size/index fields, AAC-hbr) + §4.1 (`config` fmtp). RFC text is freely redistributable — quote the relevant field tables. End with a short "transmux mapping" note (which struct/field each maps to).

- [ ] **Step 2: Verify module docs cite it**

Ensure `rtp_stream.rs` and `rtp_sdp.rs` `//!` blocks name the RFC + section and reference `transmux/docs/rtp-depacketization.md`. (Bit-range notation in doc comments must be backticked, e.g. `` `[7:4]` ``.)

- [ ] **Step 3: Update CHANGELOG**

Add under `[Unreleased]` in `transmux/CHANGELOG.md`:

```markdown
### Added
- Streaming, timing- and config-aware RTP depayloader `RtpStreamDepacketizer`
  (RFC 6184 H.264 / RFC 3640 AAC): incremental `push`/`flush`, real per-sample
  duration from RTP-timestamp deltas, `is_sync` from IDR detection. v1 assumes
  low-delay H.264 (no B-frame reorder; `composition_offset = 0`).
- SDP fmtp → `CodecConfig` helpers `rtp_sdp::{avc_config_from_sprop,
  aac_config_from_fmtp}` (RFC 6184 §8.1 / RFC 3640 §4.1).
```

- [ ] **Step 4: Run the full CI gate suite**

Run each; all must pass:
```bash
cargo build -p transmux --all-features --locked
cargo build -p transmux --no-default-features --locked
cargo test  -p transmux --all-features --locked
cargo clippy -p transmux --all-features --all-targets --locked -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc -p transmux --all-features --no-deps --locked
```
Expected: all green. The `--no-default-features` build is the critical one (no_std) — ensure no `std::` leaked into the new modules.

- [ ] **Step 5: Commit**

```bash
git add transmux/docs/rtp-depacketization.md transmux/src/rtp_stream.rs transmux/src/rtp_sdp.rs transmux/CHANGELOG.md
git commit -m "docs(transmux): RTP depacketization RFC transcription + CHANGELOG (#700)"
```

---

## Self-Review

**Spec coverage (against `docs/superpowers/specs/2026-07-14-multimux-design.md` Upstream Prerequisites):**
- P1 streaming RTP depayloader, timing + config aware → Tasks 1, 4. ✓
- P1 real timing (duration from RTP-ts delta; is_sync from IDR; composition_offset=0 low-delay limit) → Task 4 + documented. ✓
- P1 RFC 6184 FU-A/STAP-A + RFC 3640 AAC reassembly → reused via Task 1. ✓
- P2 SDP sprop → avcC → Task 2. ✓
- P2 SDP AAC config= → esds/CodecConfig → Task 3. ✓
- R1 (B-frame DTS) + R2 (RTP-ts→timescale) → addressed/scoped in Task 4 timing model + docs. ✓
- Hard gate: RFC transcription in `transmux/docs/` + cited (Task 6); real-fixture parse→timing→round-trip→validate (Task 5); no_std build (Task 6); biting tests (Tasks 1–5). ✓
- Additive → minor bump; old batch `RtpDepacketizer` untouched (Task 1 delegation keeps it). ✓

**Type consistency:** `ReassembledAu{timestamp,is_sync,data}`, `RtpStreamTrack{track_id,kind,config,clock_rate}`, `RtpStreamDepacketizer::{new,track_specs,push,flush}`, `avc_config_from_sprop(&str)->Result<AVCConfigurationBox>`, `aac_config_from_fmtp(&str)->Result<CodecConfig>` — used consistently across tasks and the fixture test. `Sample::new(data,duration,is_sync,composition_offset)`, `TrackSpec::new(track_id,timescale,config)` match the confirmed signatures.

**Open verification points flagged for the implementer (Step 0 reads):** exact `mp4esds` constructors (Task 3), `SamplingFrequencyIndex`/`ChannelConfiguration` accessors (Task 3), the existing demux+packetize helpers in `transmux/tests/rtp.rs` (Task 5), `CodecConfig: Clone` + `parse_rtp_header` visibility (Task 4). These are stable local facts pinned to file paths; the tests assert observable results so any correct construction passes.
