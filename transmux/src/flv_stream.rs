//! Streaming (incremental) FLV → samples demux for live RTMP ingest (#738),
//! mirroring [`StreamingTsDemux`](crate::ts_demux::StreamingTsDemux) (issue
//! #555) for the FLV container (Adobe FLV v10.1 Annex E — see
//! `transmux/docs/codec/flv.md` and [`crate::flv`] for the wire layout).
//!
//! [`FlvDemux`](crate::flv::FlvDemux) is a **one-shot** [`Unpackage`
//! ](broadcast_common::Unpackage): it demands the whole FLV byte stream up
//! front and returns one [`Media`](crate::media::Media). A live RTMP publisher instead delivers
//! FLV tags forever — re-running the one-shot demuxer over an
//! ever-growing buffer on every new tag would grow memory (and CPU) without
//! bound. [`StreamingFlvDemux`] is the incremental analogue, mirroring
//! [`StreamingTsDemux`](crate::ts_demux::StreamingTsDemux)'s pull API
//! exactly so a caller can drive either demuxer with the same drain loop:
//! feed bytes of any size/alignment via [`feed`](StreamingFlvDemux::feed),
//! drain newly-resolved [`DemuxEvent`]s one at a time via
//! [`poll_event`](StreamingFlvDemux::poll_event), and call
//! [`finish`](StreamingFlvDemux::finish) once, at end of input, to flush
//! each track's trailing pending sample (then drain those with `poll_event`
//! too). One difference from `StreamingTsDemux::feed` (which is infallible):
//! FLV's `feed` can return `Err` for a hard structural problem — a bad
//! signature, an implausible header `DataOffset`, or a corrupt codec-config
//! payload — because unlike an MPEG-2 TS bitstream (which resynchronises on
//! `0x47` and simply skips anything it can't parse), a malformed FLV tag
//! header leaves no safe resynchronisation point to skip to.
//!
//! This module adds only the incremental tag-boundary buffering and
//! per-sample duration bookkeeping; it reuses [`crate::flv`]'s tag-header
//! constants and its `AVCDecoderConfigurationRecord`/`AudioSpecificConfig`
//! codec-config parsing verbatim (via `pub(crate)` re-exports) rather than
//! duplicating them, so the two demuxers can never silently drift apart on
//! layout.
//!
//! # Memory
//!
//! Bounded, independent of stream length: [`pending`](StreamingFlvDemux)
//! never holds more than one in-progress tag — a complete tag (header, body,
//! and trailing `PreviousTagSize`) is parsed and drained from the buffer the
//! moment it is fully present, so the buffer's high-water mark is one tag's
//! worth of bytes, not the whole stream — plus one buffered [`Sample`] per
//! track (video/audio), held only until the next same-kind tag's timestamp
//! makes its forward-delta duration knowable (or [`finish`
//! ](StreamingFlvDemux::finish) flushes it at end of stream).
//!
//! # Non-AVC/AAC tracks
//!
//! Mirrors [`FlvDemux`](crate::flv::FlvDemux): unlike the TS demux (which
//! carries every other `stream_type` as an opaque `Data` track so nothing is
//! dropped), a video tag whose `CodecID` isn't AVC, or an audio tag whose
//! `SoundFormat` isn't AAC, is **silently skipped** here too — no track, no
//! error (see the `CODEC_ID_AVC`/`SOUND_FORMAT_AAC` checks in the
//! `process_video_tag`/`process_audio_tag` helpers).
//!
//! # Ordering assumption (encoder conformance)
//!
//! Annex E requires the AVC/AAC sequence-header tag (`AVCPacketType`/
//! `AACPacketType` == 0) to precede that track's media tags. Unlike the
//! one-shot [`FlvDemux`](crate::flv::FlvDemux) — which can freely accumulate
//! all of a track's samples before its config is known, because it only
//! builds the final [`Track`](crate::media::Track) after the whole buffer is
//! consumed — a streaming demuxer with
//! *bounded* memory cannot hold an unbounded pre-config backlog waiting for
//! a config that might arrive arbitrarily late (or never). A media tag for a
//! track whose sequence header has not yet been seen is therefore dropped,
//! not buffered — the simplest bounded response, since this demuxer has no
//! byte-capped backlog to fall back on the way
//! [`StreamingTsDemux`](crate::ts_demux::StreamingTsDemux) does. This never
//! affects a conformant encoder's real output (the committed fixture
//! `fixtures/flv/av.flv` included).
//!
//! [`StreamingTsDemux`](crate::ts_demux::StreamingTsDemux) faces the same
//! never-resolves risk for a PMT-listed PID, but chooses differently: it
//! *does* buffer, up to `MAX_PROBE_BACKLOG_BYTES`, and only then abandons the
//! PID (dropping its backlog and raising a `Discontinuity`) — not dropped on
//! arrival like this module's media tag, because a real broadcast capture's
//! parameter sets are expected within the first access unit or two, so the
//! backlog is small in the common case.
//!
//! # No `TracksResolved`
//!
//! [`DemuxEvent::TracksResolved`] is a PMT-driven signal (TS declares its
//! track count up front); FLV has no equivalent binding declaration — the
//! header's `TypeFlags` bits are informational only and the one-shot
//! [`FlvDemux`](crate::flv::FlvDemux) itself does not trust them — so this
//! demuxer never emits it.
//! A consumer that needs a "no more tracks coming" signal should drive that
//! from its own knowledge of the publish (e.g. RTMP `publish` parameters).
//!
//! `no_std` + `alloc`.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use broadcast_common::{Demand, Parse, Stage, Timestamp};

use crate::aac_asc::AudioSpecificConfig;
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::flv::{
    AUDIO_SAMPLE_SIZE_BITS, CODEC_ID_AVC, FLV_HEADER_LEN, FLV_SIGNATURE, FLV_TIMESCALE,
    FRAME_TYPE_KEYFRAME, FlvError, MAX_FLV_HEADER_LEN, PREV_TAG_SIZE_LEN, TAG_HEADER_LEN,
    aac_packet_type, asc_rate_hz, avc_dimensions, avc_packet_type, build_aac_esds, read_si24,
    tag_type,
};
use crate::ir::DemuxEvent;
use crate::pipeline::{CodecConfig, Sample, TrackSpec};

/// One track (video or audio)'s still-in-flight sample: buffered until the
/// next same-kind tag's timestamp makes its forward-delta duration knowable,
/// or [`finish`](StreamingFlvDemux::finish) flushes it at end of stream.
/// Mirrors [`FlvDemux`](crate::flv::FlvDemux)'s two-pass
/// `delta_duration`/`backfill_last_duration` bookkeeping one sample at a
/// time instead of over a fully-buffered `Vec`.
#[derive(Debug)]
struct PendingSample {
    sample: Sample,
    /// This sample's own FLV tag timestamp (ms) — the DTS the *next*
    /// same-kind tag's timestamp is subtracted from to get its duration.
    dts: u32,
}

/// Per-track (video or audio) streaming state.
#[derive(Debug, Default)]
struct TrackState {
    /// `Some` once this track's codec config has been recovered and
    /// [`DemuxEvent::TrackAdded`] fired for it.
    track_id: Option<u32>,
    pending: Option<PendingSample>,
    /// The last computed forward-delta duration, reused for the final
    /// sample's duration at [`finish`](StreamingFlvDemux::finish) (or `0` if
    /// only one sample was ever seen) — exactly
    /// [`crate::flv`]'s `backfill_last_duration` tail rule.
    last_duration: u32,
    /// The raw codec-config payload (the `avcC` record / the ASC) the track's
    /// current [`TrackSpec`] was built from, so a *re-sent* sequence header can
    /// be compared with it byte-for-byte: an encoder that repeats its headers
    /// on every keyframe would otherwise emit [`DemuxEvent::TrackUpdated`]
    /// hundreds of times a minute, each one telling a consumer to rebuild its
    /// init segment for a config that did not change.
    config_bytes: Vec<u8>,
}

impl TrackState {
    /// Advance the pending slot with a newly-decoded tag at `dts`, emitting
    /// the *previous* pending sample now that its duration (`dts - prev.dts`)
    /// is known.
    fn advance(&mut self, sample: Sample, dts: u32, events: &mut VecDeque<DemuxEvent>) {
        let track_id = self
            .track_id
            .expect("TrackState::advance called before the track's config resolved");
        if let Some(prev) = self.pending.take() {
            let duration = dts.saturating_sub(prev.dts);
            self.last_duration = duration;
            let mut emitted = prev.sample;
            emitted.duration = Some(duration);
            events.push_back(DemuxEvent::Sample {
                track_id,
                sample: emitted,
            });
        }
        self.pending = Some(PendingSample { sample, dts });
    }

    /// Flush a trailing pending sample at end-of-stream, reusing the last
    /// known forward delta (`0` if this track only ever saw one sample).
    fn flush(&mut self, events: &mut VecDeque<DemuxEvent>) {
        let Some(track_id) = self.track_id else {
            return;
        };
        if let Some(prev) = self.pending.take() {
            let mut emitted = prev.sample;
            emitted.duration = Some(self.last_duration);
            events.push_back(DemuxEvent::Sample {
                track_id,
                sample: emitted,
            });
        }
    }
}

/// Incremental (streaming) FLV demuxer (issue #738) — the FLV analogue of
/// [`StreamingTsDemux`](crate::ts_demux::StreamingTsDemux). See the
/// module-level docs for the memory bound, the encoder-conformance ordering
/// assumption, and why [`DemuxEvent::TracksResolved`] is never emitted.
#[derive(Debug)]
pub struct StreamingFlvDemux {
    /// Bytes fed but not yet consumed. Before the header is seen: the
    /// growing header prefix (at most `FLV_HEADER_LEN` + a non-standard
    /// `DataOffset` overrun, both tiny and fixed). After: at most one
    /// in-progress tag (see the module `# Memory` note).
    pending: Vec<u8>,
    header_seen: bool,
    video: TrackState,
    audio: TrackState,
    next_track_id: u32,
    /// Pending events, drained one at a time via
    /// [`poll_event`](Self::poll_event) — mirrors
    /// [`StreamingTsDemux`](crate::ts_demux::StreamingTsDemux)'s internal
    /// queue exactly, so a caller can drive both demuxers with the same
    /// `while let Some(ev) = demux.poll_event()` loop.
    events: VecDeque<DemuxEvent>,
    /// Test-only count of bytes memmoved out of `pending` by the drains in
    /// [`feed`](Self::feed), incremented at the drain site itself. A test can
    /// assert it stays O(input) over a whole file (r04-W14): draining once per
    /// *tag* rather than once per `feed` moves the whole remainder every time,
    /// which is quadratic in the tag count and shows up here as a total in the
    /// gigabytes for a few hundred kilobytes of input.
    #[cfg(test)]
    bytes_moved: usize,
}

impl Default for StreamingFlvDemux {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingFlvDemux {
    /// Create a new streaming demuxer with empty state.
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            header_seen: false,
            video: TrackState::default(),
            audio: TrackState::default(),
            next_track_id: 1,
            events: VecDeque::new(),
            #[cfg(test)]
            bytes_moved: 0,
        }
    }

    /// Feed the next chunk of FLV bytes — any size, any alignment (the FLV
    /// header must appear at the start of the very first `feed` call; tag
    /// boundaries may land anywhere, including one byte at a time across
    /// many calls). Newly-resolved [`DemuxEvent`]s are enqueued internally;
    /// drain them with [`poll_event`](Self::poll_event). Any partial
    /// trailing tag is retained internally for the next `feed` call (see the
    /// module `# Memory` note).
    ///
    /// Returns [`FlvError::BadSignature`] if the first bytes are not the
    /// `"FLV"` signature, [`FlvError::HeaderTooLarge`] if the header's
    /// `DataOffset` exceeds this crate's maximum accepted header size (a
    /// malicious value could otherwise force the pre-header buffer to grow
    /// without bound), or a codec-config parse error ([`FlvError::Codec`])
    /// if a sequence-header tag's payload is corrupt. Never panics on
    /// truncated or garbage input.
    pub fn feed(&mut self, input: &[u8]) -> Result<(), FlvError> {
        self.pending.extend_from_slice(input);

        // `pending` is consumed from the front, but only once per `feed`: the
        // loop below works at a moving `cursor` and a single `drain(0..cursor)`
        // at the end moves the remainder at most once. Draining per tag made a
        // `feed` of one large chunk quadratic — 8 MiB of small audio tags
        // (~20 000 of them) memmoved ~80 GB.
        let mut cursor = 0usize;
        let result = self.consume(&mut cursor);

        if let Err(err) = result {
            // Drain the failed tag with the bytes before it. A tag whose body
            // is structurally broken (a corrupt ASC/avcC) can never be
            // re-parsed into anything else, so leaving it in `pending` made
            // every later `feed` re-read the same bytes and return the same
            // error forever — the ingest could never continue past one bad
            // tag. The bytes are consumed, so the next `feed` resumes at the
            // following tag.
            self.drain_pending(cursor);
            return Err(err);
        }
        self.drain_pending(cursor);
        Ok(())
    }

    /// Drop the first `n` consumed bytes from `pending` (one memmove of the
    /// remainder — the reason `feed` keeps a cursor and drains once).
    fn drain_pending(&mut self, n: usize) {
        #[cfg(test)]
        {
            // The bytes actually moved by `Vec::drain` is the remainder.
            self.bytes_moved += self.pending.len() - n;
        }
        self.pending.drain(0..n);
    }

    /// The parse half of [`feed`](Self::feed): advance `cursor` past every
    /// complete unit in `self.pending`, appending events, and return on the
    /// first error with `cursor` left at the start of the offending tag (so
    /// the caller can drain it).
    fn consume(&mut self, cursor: &mut usize) -> Result<(), FlvError> {
        loop {
            if !self.header_seen {
                if self.pending.len() < FLV_HEADER_LEN + PREV_TAG_SIZE_LEN {
                    break; // header (+ PreviousTagSize0) not fully arrived yet
                }
                if self.pending[0..3] != FLV_SIGNATURE {
                    return Err(FlvError::BadSignature([
                        self.pending[0],
                        self.pending[1],
                        self.pending[2],
                    ]));
                }
                // DataOffset (bytes [5:9]) points past the header to the
                // first PreviousTagSize0 — mirrors `crate::flv::iter_tags`.
                // Validated against `MAX_FLV_HEADER_LEN` *before* using it to
                // size a skip: a malicious value (e.g. 0xFFFF_FFFF) would
                // otherwise never satisfy the `pending.len() < skip` check
                // below, so every `feed` call would simply keep appending to
                // `pending` forever waiting for a "header" that never
                // completes (#738 T11a review, Important — remote OOM/DoS).
                let data_offset = u32::from_be_bytes([
                    self.pending[5],
                    self.pending[6],
                    self.pending[7],
                    self.pending[8],
                ]);
                if data_offset as usize > MAX_FLV_HEADER_LEN {
                    return Err(FlvError::HeaderTooLarge {
                        declared: data_offset,
                        max: MAX_FLV_HEADER_LEN,
                    });
                }
                let skip = (data_offset as usize).max(FLV_HEADER_LEN) + PREV_TAG_SIZE_LEN;
                if self.pending.len() < skip {
                    break; // a non-standard larger header hasn't fully arrived
                }
                *cursor = skip;
                self.header_seen = true;
                continue;
            }

            let remaining = &self.pending[*cursor..];
            if remaining.len() < TAG_HEADER_LEN {
                break; // partial tag header
            }
            let tag_type_byte = remaining[0];
            let data_size =
                u32::from_be_bytes([0, remaining[1], remaining[2], remaining[3]]) as usize;
            let ts_lo = u32::from_be_bytes([0, remaining[4], remaining[5], remaining[6]]);
            let ts_ext = remaining[7] as u32;
            let timestamp = (ts_ext << 24) | ts_lo;
            let body_start = TAG_HEADER_LEN;
            let body_end = body_start + data_size;
            let total = body_end + PREV_TAG_SIZE_LEN;
            if remaining.len() < total {
                break; // tag body / trailing PreviousTagSize not fully arrived
            }
            let body = &remaining[body_start..body_end];
            // The tag is fully present, so it is consumed either way: advance
            // the cursor *before* parsing, so a structural failure (a corrupt
            // ASC/avcC) is drained along with the bytes before it rather than
            // left in `pending` to be re-parsed — and re-failed — by every
            // later `feed`. The bytes are unusable as anything else, so
            // discarding them is what lets the ingest continue.
            *cursor += total;
            Self::process_tag(
                &mut self.video,
                &mut self.audio,
                &mut self.next_track_id,
                tag_type_byte,
                timestamp,
                body,
                &mut self.events,
            )?;
        }
        Ok(())
    }

    /// Drain the next pending event, if any (FIFO) — identical shape to
    /// [`StreamingTsDemux::poll_event`](crate::ts_demux::StreamingTsDemux::poll_event).
    pub fn poll_event(&mut self) -> Option<DemuxEvent> {
        self.events.pop_front()
    }

    /// Flush each track's trailing pending sample (end of input — no more
    /// bytes coming) into the event queue; drain it with
    /// [`poll_event`](Self::poll_event). Idempotent: a second call with
    /// nothing newly pending enqueues nothing.
    pub fn finish(&mut self) {
        self.video.flush(&mut self.events);
        self.audio.flush(&mut self.events);
    }

    /// Bytes still missing before the unit [`feed`](Self::feed) is currently
    /// blocked on becomes consumable — the exact complement of each `break`
    /// in `feed`'s loop, in the same order. Backs [`Stage::demand`]'s
    /// `want_bytes`; see that impl for why a constant was wrong.
    ///
    /// Never `0`: a `Demand::want_bytes` of `0` means "no particular
    /// preference" rather than "no bytes needed", and this demuxer always
    /// needs at least one more byte to make progress (`feed` drains every
    /// complete tag before returning, so `pending` never already holds one).
    fn bytes_wanted(&self) -> usize {
        let have = self.pending.len();
        if !self.header_seen {
            // The fixed header + PreviousTagSize0 must land before
            // `DataOffset` can even be read.
            let minimum = FLV_HEADER_LEN + PREV_TAG_SIZE_LEN;
            if have < minimum {
                return minimum - have;
            }
            // `DataOffset` (bytes [5:9]) may declare a larger non-standard
            // header; `feed` validates it against `MAX_FLV_HEADER_LEN` before
            // trusting it, and rejects an over-large one, so the worst this
            // can over-ask for is that already-rejected bound.
            let data_offset = u32::from_be_bytes([
                self.pending[5],
                self.pending[6],
                self.pending[7],
                self.pending[8],
            ]) as usize;
            let skip = data_offset.max(FLV_HEADER_LEN) + PREV_TAG_SIZE_LEN;
            return skip.saturating_sub(have).max(1);
        }
        if have < TAG_HEADER_LEN {
            return TAG_HEADER_LEN - have;
        }
        // Tag header present: `DataSize` (bytes [1:4], 24-bit) gives the whole
        // remaining unit — body plus the trailing `PreviousTagSize`.
        let data_size =
            u32::from_be_bytes([0, self.pending[1], self.pending[2], self.pending[3]]) as usize;
        let total = TAG_HEADER_LEN + data_size + PREV_TAG_SIZE_LEN;
        total.saturating_sub(have).max(1)
    }

    fn process_tag(
        video: &mut TrackState,
        audio: &mut TrackState,
        next_track_id: &mut u32,
        tag_type_byte: u8,
        timestamp: u32,
        body: &[u8],
        events: &mut VecDeque<DemuxEvent>,
    ) -> Result<(), FlvError> {
        match tag_type_byte {
            tag_type::VIDEO => {
                Self::process_video_tag(video, next_track_id, timestamp, body, events)
            }
            tag_type::AUDIO => {
                Self::process_audio_tag(audio, next_track_id, timestamp, body, events)
            }
            tag_type::SCRIPT => Ok(()), // onMetaData — informational, skipped
            _ => Ok(()),                // unknown tag type — skipped leniently
        }
    }

    fn process_video_tag(
        video: &mut TrackState,
        next_track_id: &mut u32,
        timestamp: u32,
        body: &[u8],
        events: &mut VecDeque<DemuxEvent>,
    ) -> Result<(), FlvError> {
        if body.len() < 2 {
            return Ok(());
        }
        let frame_type = body[0] >> 4;
        let codec_id = body[0] & 0x0F;
        if codec_id != CODEC_ID_AVC {
            return Ok(()); // non-AVC video is out of scope (mirrors FlvDemux)
        }
        let avc_packet_type_byte = body[1];
        // AVCVIDEOPACKET: AVCPacketType(1) + CompositionTime(SI24=3) + Data.
        if body.len() < 5 {
            return Ok(());
        }
        let composition_time = read_si24(&body[2..5]);
        let data = &body[5..];
        match avc_packet_type_byte {
            avc_packet_type::SEQUENCE_HEADER if !data.is_empty() => {
                // `AVCDecoderConfigurationRecord::parse` rejects 0 SPS
                // (#738 T11a review, Critical — see `avc_config.rs`), so
                // this never panics on a malicious sequence header; the
                // `.first()` below is additional defense-in-depth against
                // a directly-constructed (non-`parse`) empty-SPS record.
                let record = AVCDecoderConfigurationRecord::parse(data)?;
                let config = AVCConfigurationBox::new(record);
                // A dimension too large for the IR's `u16` is rejected, never
                // truncated (`crate::flv::avc_dimensions`).
                let (width, height) = avc_dimensions(&config)?;
                let codec = CodecConfig::Avc {
                    config,
                    width,
                    height,
                };
                match video.track_id {
                    Some(track_id) => {
                        // A *re-sent* sequence header is routine: OBS, ffmpeg
                        // and every encoder that changes resolution, profile
                        // or level mid-publish sends a fresh one. Ignoring it
                        // left the track labelled with the stale avcC, so the
                        // downstream init segment described the old stream and
                        // players failed to decode. The track keeps its
                        // identity (same `track_id`, so a consumer that
                        // replaces its spec in place keeps its timeline) and
                        // reports the new config.
                        //
                        // Only when the bytes actually differ, though: the same
                        // encoders usually repeat the header on *every*
                        // keyframe, and an unchanged config must not spam a
                        // consumer with init-segment rebuilds.
                        if video.config_bytes != data {
                            video.config_bytes.clear();
                            video.config_bytes.extend_from_slice(data);
                            events.push_back(DemuxEvent::TrackUpdated(TrackSpec::new(
                                track_id,
                                FLV_TIMESCALE,
                                codec,
                            )));
                        }
                    }
                    None => {
                        let track_id = *next_track_id;
                        *next_track_id += 1;
                        video.track_id = Some(track_id);
                        video.config_bytes.clear();
                        video.config_bytes.extend_from_slice(data);
                        events.push_back(DemuxEvent::TrackAdded(TrackSpec::new(
                            track_id,
                            FLV_TIMESCALE,
                            codec,
                        )));
                    }
                }
            }
            // Dropped (not buffered) if the sequence header hasn't
            // resolved the track yet — see the module `# Ordering
            // assumption` note.
            avc_packet_type::NALU if video.track_id.is_some() => {
                // Absolute dts/pts (media plane step 2c): the FLV tag
                // timestamp is already an absolute wire clock
                // (milliseconds, `FLV_TIMESCALE`); `CompositionTime`
                // (§E.4.3.2) folds directly into `pts`.
                let dts_abs = timestamp as i64;
                let pts_abs = dts_abs + composition_time as i64;
                let sample = Sample {
                    data: data.to_vec().into(),
                    dts: Some(dts_abs),
                    pts: Some(pts_abs),
                    duration: None, // filled in by `TrackState::advance`/`flush`
                    flags: crate::ir::SampleFlags::new(frame_type == FRAME_TYPE_KEYFRAME),
                    provenance: None,
                };
                video.advance(sample, timestamp, events);
            }
            avc_packet_type::END_OF_SEQUENCE => {}
            _ => {}
        }
        Ok(())
    }

    fn process_audio_tag(
        audio: &mut TrackState,
        next_track_id: &mut u32,
        timestamp: u32,
        body: &[u8],
        events: &mut VecDeque<DemuxEvent>,
    ) -> Result<(), FlvError> {
        if body.is_empty() {
            return Ok(());
        }
        let sound_format = body[0] >> 4;
        if sound_format != crate::flv::SOUND_FORMAT_AAC {
            return Ok(()); // non-AAC audio is out of scope (mirrors FlvDemux)
        }
        // AACAUDIODATA: AACPacketType(1) + Data.
        if body.len() < 2 {
            return Ok(());
        }
        let aac_pkt_type = body[1];
        let data = &body[2..];
        match aac_pkt_type {
            aac_packet_type::SEQUENCE_HEADER if !data.is_empty() => {
                let asc = AudioSpecificConfig::parse(data)?;
                // ISO/IEC 14496-3 Table 1.19: the field is a configuration
                // *index*, not the channel count. In-band (0) and the reserved
                // values yield `AAC_CHANNEL_COUNT_UNKNOWN`, not a fabricated
                // number.
                let channels = crate::flv::aac_channel_count(&asc);
                let rate = asc_rate_hz(&asc);
                let esds = build_aac_esds(data.to_vec());
                let codec = CodecConfig::Aac {
                    esds,
                    channel_count: channels,
                    sample_rate: rate,
                    sample_size: AUDIO_SAMPLE_SIZE_BITS,
                };
                match audio.track_id {
                    Some(track_id) => {
                        // A re-sent AAC sequence header (the sample rate,
                        // channel count or object type changed mid-publish)
                        // updates the existing track in place — see the
                        // video path above for why this must not be ignored,
                        // and for why an identical re-send emits nothing.
                        if audio.config_bytes != data {
                            audio.config_bytes.clear();
                            audio.config_bytes.extend_from_slice(data);
                            events.push_back(DemuxEvent::TrackUpdated(TrackSpec::new(
                                track_id,
                                FLV_TIMESCALE,
                                codec,
                            )));
                        }
                    }
                    None => {
                        let track_id = *next_track_id;
                        *next_track_id += 1;
                        audio.track_id = Some(track_id);
                        audio.config_bytes.clear();
                        audio.config_bytes.extend_from_slice(data);
                        events.push_back(DemuxEvent::TrackAdded(TrackSpec::new(
                            track_id,
                            FLV_TIMESCALE,
                            codec,
                        )));
                    }
                }
            }
            // Dropped (not buffered) if the sequence header hasn't resolved
            // the track yet — see the module `# Ordering assumption` note.
            aac_packet_type::RAW if audio.track_id.is_some() => {
                let dts_abs = timestamp as i64;
                let sample = Sample {
                    data: data.to_vec().into(),
                    dts: Some(dts_abs),
                    pts: Some(dts_abs),
                    duration: None, // filled in by `TrackState::advance`/`flush`
                    flags: crate::ir::SampleFlags::SYNC,
                    provenance: None,
                };
                audio.advance(sample, timestamp, events);
            }
            _ => {}
        }
        Ok(())
    }
}

/// [`Stage`] adoption (media plane step 2e): delegates straight to the
/// inherent [`feed`](StreamingFlvDemux::feed)/[`poll_event`
/// ](StreamingFlvDemux::poll_event)/[`finish`](StreamingFlvDemux::finish) —
/// which keep working unchanged — so a caller can drive both
/// `StreamingFlvDemux` and [`crate::ts_demux::StreamingTsDemux`] through the
/// exact same generic loop despite the two disagreeing on `feed`'s return
/// type (`Result` vs infallible) before this trait.
impl Stage for StreamingFlvDemux {
    type In<'a> = &'a [u8];
    type Out = DemuxEvent;
    type Error = FlvError;

    /// Fully-qualified inherent calls (`StreamingFlvDemux::feed`, not
    /// `self.feed`): the inherent methods and these trait methods share their
    /// names, and bare method-call syntax picks the inherent one only by
    /// inherent-over-trait precedence. `finish` is the dangerous one — same
    /// name, same arity, and its inherent form returns `()` — so renaming it
    /// would silently retarget `self.finish()` at *this* method: infinite
    /// recursion that still compiles.
    fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<(), FlvError> {
        StreamingFlvDemux::feed(self, input)
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.poll_event()
    }

    fn finish(&mut self) -> Result<(), FlvError> {
        StreamingFlvDemux::finish(self);
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        // Only ever reacts to feed/finish — no rate-scheduled or timeout work.
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    /// `want_bytes` is the **exact** number of bytes still missing before the
    /// unit [`feed`](StreamingFlvDemux::feed) is currently blocked on can be
    /// consumed: the rest of the FLV header, the rest of a partial tag
    /// header, or the rest of the in-flight tag's body + trailing
    /// `PreviousTagSize` — the same arithmetic `feed`'s loop breaks on.
    ///
    /// It used to be a constant `TAG_HEADER_LEN` (11), even mid-body, so a
    /// driver that sizes its reads by `want_bytes` made ~1.5 million `feed`
    /// calls to deliver one 16 MiB tag instead of one.
    ///
    /// `saturated` stays `false`, and that is not a fabrication: `pending`
    /// holds at most one in-flight tag, and a tag's `DataSize` is a 24-bit
    /// field (Adobe FLV v10.1 Annex E), so the buffer is *structurally*
    /// bounded at ~16 MiB — more bytes always make progress, and there is no
    /// state in which this demuxer would rather not be fed. (The pre-header
    /// prefix is separately capped by `MAX_FLV_HEADER_LEN`, and a header
    /// declaring more is surfaced as an `Err` rather than a steady buffered
    /// state a driver would observe here.)
    fn demand(&self) -> Demand {
        Demand::new(self.bytes_wanted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use bytes::Bytes;

    // --- Synthetic FLV fixture builder (small + deterministic) --------------
    //
    // A tiny hand-built AVC-only FLV: header, one AVC sequence-header tag
    // (a minimal but structurally valid AVCDecoderConfigurationRecord: no
    // SPS/PPS, which `AVCDecoderConfigurationRecord::parse` accepts), then N
    // NALU tags at deterministic timestamps.

    /// A minimal but structurally valid `AVCDecoderConfigurationRecord`
    /// (§5.2.4.1, avcC): Baseline profile (not a "high profile", so no
    /// extension bytes are required), exactly one (garbage-content, but
    /// length-correct) SPS NAL and zero PPS — enough for
    /// `AVCDecoderConfigurationRecord::parse` to accept, and for
    /// `sps[0]` to be a valid index (this module's tag-boundary tests are
    /// not exercising real SPS decode, just tag framing; `decode_avc_sps`
    /// fails gracefully on the garbage SPS content and falls back to
    /// `(0, 0)` dimensions rather than panicking).
    fn minimal_avcc_bytes() -> Vec<u8> {
        let sps_nal: [u8; 6] = [0x67, 0x42, 0x00, 0x1F, 0x00, 0x00];
        let mut out = vec![
            0x01, // configurationVersion
            0x42, // AVCProfileIndication (Baseline)
            0x00, // profile_compatibility
            0x1F, // AVCLevelIndication
            0xFF, // reserved(6)+lengthSizeMinusOne(2) = 3 (valid)
            0xE1, // reserved(3)+numOfSequenceParameterSets(5) = 1
        ];
        out.extend_from_slice(&(sps_nal.len() as u16).to_be_bytes());
        out.extend_from_slice(&sps_nal);
        out.push(0x00); // numOfPictureParameterSets = 0
        out
    }

    fn write_tag(out: &mut Vec<u8>, tag_type: u8, timestamp: u32, body: &[u8]) {
        let start = out.len();
        out.push(tag_type);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]); // DataSize UI24
        out.push((timestamp >> 16) as u8);
        out.push((timestamp >> 8) as u8);
        out.push(timestamp as u8);
        out.push((timestamp >> 24) as u8); // TimestampExtended
        out.extend_from_slice(&[0, 0, 0]); // StreamID = 0
        out.extend_from_slice(body);
        let tag_size = (out.len() - start) as u32;
        out.extend_from_slice(&tag_size.to_be_bytes()); // PreviousTagSize
    }

    fn flv_header() -> Vec<u8> {
        header_with_data_offset(FLV_HEADER_LEN as u32)
    }

    /// An FLV header with an explicit (possibly non-conformant) `DataOffset`,
    /// with its trailing `PreviousTagSize0` always zero.
    fn header_with_data_offset(data_offset: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&FLV_SIGNATURE);
        out.push(1); // version
        out.push(0x01); // TypeFlags: video present
        out.extend_from_slice(&data_offset.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // PreviousTagSize0
        out
    }

    /// Feed `input` then drain every event newly queued by it via
    /// `poll_event` — the uniform pull-loop shape a real caller (e.g. an
    /// RTMP `RtmpSource`) drives both `StreamingFlvDemux` and
    /// `StreamingTsDemux` with.
    fn feed_and_drain(
        demux: &mut StreamingFlvDemux,
        input: &[u8],
    ) -> Result<Vec<DemuxEvent>, FlvError> {
        demux.feed(input)?;
        let mut events = Vec::new();
        while let Some(ev) = demux.poll_event() {
            events.push(ev);
        }
        Ok(events)
    }

    /// `finish` then drain the trailing events it queued.
    fn finish_and_drain(demux: &mut StreamingFlvDemux) -> Vec<DemuxEvent> {
        demux.finish();
        let mut events = Vec::new();
        while let Some(ev) = demux.poll_event() {
            events.push(ev);
        }
        events
    }

    /// Build a synthetic FLV: header + one AVC sequence-header tag + `n`
    /// AVC NALU tags at timestamps `0, step, 2*step, ...`.
    fn synthetic_avc_flv(n: u32, step: u32) -> Vec<u8> {
        let mut out = flv_header();
        // Sequence header tag.
        let mut seq_body = vec![(1u8 << 4) | CODEC_ID_AVC, avc_packet_type::SEQUENCE_HEADER];
        seq_body.extend_from_slice(&[0, 0, 0]); // CompositionTime = 0
        seq_body.extend_from_slice(&minimal_avcc_bytes());
        write_tag(&mut out, tag_type::VIDEO, 0, &seq_body);
        // NALU tags: a distinct 1-byte payload per tag so we can identify
        // each sample unambiguously in assertions.
        for i in 0..n {
            let ts = i * step;
            let mut body = vec![(1u8 << 4) | CODEC_ID_AVC, avc_packet_type::NALU];
            body.extend_from_slice(&[0, 0, 0]); // CompositionTime = 0
            body.push(i as u8); // 1-byte "NAL" payload, tags the sample
            write_tag(&mut out, tag_type::VIDEO, ts, &body);
        }
        out
    }

    /// Collect every `Sample` for the (single) video track across all
    /// events, in emission order.
    fn video_samples(events: &[DemuxEvent]) -> Vec<(u32, Bytes)> {
        events
            .iter()
            .filter_map(|e| match e {
                DemuxEvent::Sample { sample, .. } => Some((
                    sample
                        .duration
                        .expect("FLV video samples always carry a duration"),
                    sample.data.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn whole_buffer_one_feed_call_matches_one_shot_shape() {
        let flv = synthetic_avc_flv(4, 100);
        let mut demux = StreamingFlvDemux::new();
        let mut events = feed_and_drain(&mut demux, &flv).unwrap();
        events.extend(finish_and_drain(&mut demux));

        let added = events
            .iter()
            .filter(|e| matches!(e, DemuxEvent::TrackAdded(_)))
            .count();
        assert_eq!(added, 1, "exactly one TrackAdded (video only)");

        let samples = video_samples(&events);
        assert_eq!(samples.len(), 4, "4 NALU tags -> 4 samples");
        // Forward-delta durations: 100,100,100, then repeat 100 for the last.
        assert_eq!(
            samples.iter().map(|(d, _)| *d).collect::<Vec<_>>(),
            vec![100, 100, 100, 100]
        );
        // Payload bytes preserved in order.
        assert_eq!(
            samples.iter().map(|(_, b)| b[0]).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn chunked_feed_matches_whole_buffer_feed() {
        let flv = synthetic_avc_flv(6, 33);

        let mut whole = StreamingFlvDemux::new();
        let mut whole_events = feed_and_drain(&mut whole, &flv).unwrap();
        whole_events.extend(finish_and_drain(&mut whole));
        let whole_samples = video_samples(&whole_events);

        // Feed in arbitrary small chunks that don't align to tag boundaries.
        let mut chunked = StreamingFlvDemux::new();
        let mut chunked_events = Vec::new();
        for chunk in flv.chunks(7) {
            chunked_events.extend(feed_and_drain(&mut chunked, chunk).unwrap());
        }
        chunked_events.extend(finish_and_drain(&mut chunked));
        let chunked_samples = video_samples(&chunked_events);

        assert_eq!(
            whole_samples, chunked_samples,
            "7-byte chunking must reproduce the whole-buffer result exactly"
        );
    }

    #[test]
    fn byte_at_a_time_feed_matches_whole_buffer_feed() {
        let flv = synthetic_avc_flv(5, 40);

        let mut whole = StreamingFlvDemux::new();
        let mut whole_events = feed_and_drain(&mut whole, &flv).unwrap();
        whole_events.extend(finish_and_drain(&mut whole));
        let whole_samples = video_samples(&whole_events);

        let mut byte_demux = StreamingFlvDemux::new();
        let mut byte_events = Vec::new();
        for b in &flv {
            byte_events.extend(feed_and_drain(&mut byte_demux, core::slice::from_ref(b)).unwrap());
        }
        byte_events.extend(finish_and_drain(&mut byte_demux));
        let byte_samples = video_samples(&byte_events);

        assert_eq!(
            whole_samples, byte_samples,
            "byte-at-a-time feed must reproduce the whole-buffer result exactly"
        );
    }

    #[test]
    fn pending_buffer_stays_bounded_at_tag_boundaries() {
        // A generously large synthetic stream; if `pending` retained every
        // fed byte instead of draining completed tags, this would fail.
        let flv = synthetic_avc_flv(200, 10);
        let mut demux = StreamingFlvDemux::new();

        // Feed everything except the final tag's bytes, so we land exactly
        // on a tag boundary (no partial tag outstanding).
        let last_tag_len = TAG_HEADER_LEN + 2 + 3 + 1 + PREV_TAG_SIZE_LEN; // NALU tag shape
        let split = flv.len() - last_tag_len;
        demux.feed(&flv[..split]).unwrap();

        // At a clean tag boundary, nothing partial should remain buffered.
        assert_eq!(
            demux.pending.len(),
            0,
            "pending must be empty at a clean tag boundary, not accumulate the whole stream"
        );

        // Now feed one byte of the final tag: pending must hold only that
        // one byte, never the whole stream so far.
        demux.feed(&flv[split..split + 1]).unwrap();
        assert_eq!(
            demux.pending.len(),
            1,
            "pending must hold only the in-progress partial tag"
        );
        assert!(
            demux.pending.len() < last_tag_len,
            "pending must never grow to hold a whole stream's worth of tags"
        );
    }

    #[test]
    fn bad_signature_is_an_error_not_a_panic() {
        let mut bad = flv_header();
        bad[0] = b'X'; // corrupt the "FLV" signature
        let mut demux = StreamingFlvDemux::new();
        let err = demux.feed(&bad).unwrap_err();
        assert!(matches!(err, FlvError::BadSignature(_)));
    }

    #[test]
    fn truncated_header_waits_without_erroring_or_panicking() {
        let flv = synthetic_avc_flv(2, 10);
        let mut demux = StreamingFlvDemux::new();
        // Fewer bytes than the header needs: must not error, must not panic.
        demux.feed(&flv[..5]).unwrap();
        assert!(demux.poll_event().is_none());
    }

    #[test]
    fn corrupt_sequence_header_is_a_codec_error_not_a_panic() {
        let mut out = flv_header();
        // AVCPacketType=0 (sequence header) with a single garbage byte body
        // that is not a valid AVCDecoderConfigurationRecord (needs >= 6
        // bytes; configurationVersion must be 1).
        let mut seq_body = vec![(1u8 << 4) | CODEC_ID_AVC, avc_packet_type::SEQUENCE_HEADER];
        seq_body.extend_from_slice(&[0, 0, 0]); // CompositionTime
        seq_body.push(0xFF); // garbage avcC byte — too short to parse
        write_tag(&mut out, tag_type::VIDEO, 0, &seq_body);

        let mut demux = StreamingFlvDemux::new();
        let err = demux.feed(&out).unwrap_err();
        assert!(matches!(err, FlvError::Codec(_)));
    }

    /// A structurally valid-but-hostile `AVCDecoderConfigurationRecord`
    /// declaring **0 SPS** (`numOfSequenceParameterSets = 0`).
    fn zero_sps_avcc_bytes() -> Vec<u8> {
        vec![
            0x01, // configurationVersion
            0x42, // AVCProfileIndication (Baseline)
            0x00, // profile_compatibility
            0x1F, // AVCLevelIndication
            0xFF, // reserved(6)+lengthSizeMinusOne(2) = 3
            0xE0, // reserved(3)+numOfSequenceParameterSets(5) = 0
            0x00, // numOfPictureParameterSets = 0
        ]
    }

    #[test]
    fn zero_sps_avcc_is_an_error_not_a_panic() {
        // #738 T11a review (Critical): a malicious RTMP publisher's sequence
        // header declaring 0 SPS must error `feed`, not panic at
        // `config.sps[0]` — `AVCDecoderConfigurationRecord::parse` now
        // rejects 0 SPS (see `avc_config.rs::test_avc_config_zero_sps_rejected`);
        // this proves the streaming path surfaces that as a clean `Err`,
        // with no index-panic reachable through `feed`.
        let mut out = flv_header();
        let mut seq_body = vec![(1u8 << 4) | CODEC_ID_AVC, avc_packet_type::SEQUENCE_HEADER];
        seq_body.extend_from_slice(&[0, 0, 0]); // CompositionTime = 0
        seq_body.extend_from_slice(&zero_sps_avcc_bytes());
        write_tag(&mut out, tag_type::VIDEO, 0, &seq_body);

        let mut demux = StreamingFlvDemux::new();
        let err = demux.feed(&out).unwrap_err();
        assert!(
            matches!(err, FlvError::Codec(_)),
            "expected FlvError::Codec (0-SPS avcC rejected), got {err:?}"
        );
    }

    #[test]
    fn absurd_data_offset_is_rejected_immediately_buffer_stays_small() {
        // #738 T11a review (Important): a malicious `DataOffset` (here
        // claiming a ~4 GiB header) must be rejected the moment enough
        // bytes have arrived to read it, not silently buffered while `feed`
        // waits forever for a "header" that will never complete — the
        // remote-OOM/DoS this guards against.
        let mut header = header_with_data_offset(0xFFFF_FFFF);
        header.extend_from_slice(&[0, 0, 0, 0]); // "and a little data" per the brief
        let fed_len = header.len();

        let mut demux = StreamingFlvDemux::new();
        let err = demux.feed(&header).unwrap_err();
        assert!(
            matches!(
                err,
                FlvError::HeaderTooLarge { declared, max }
                    if declared == 0xFFFF_FFFF && max == MAX_FLV_HEADER_LEN
            ),
            "expected HeaderTooLarge, got {err:?}"
        );
        assert_eq!(
            demux.pending.len(),
            fed_len,
            "pending must hold only what was actually fed ({fed_len} bytes) — the demux \
             must not attempt to wait for the malicious DataOffset's implied ~4 GiB header"
        );
    }

    #[test]
    fn unknown_tag_type_is_skipped_leniently() {
        let mut out = flv_header();
        // An unrecognised tag type (not 8/9/18) with an arbitrary body.
        write_tag(&mut out, 0xAA, 0, &[1, 2, 3]);
        let mut demux = StreamingFlvDemux::new();
        demux.feed(&out).unwrap();
        assert!(
            demux.poll_event().is_none(),
            "unknown tag types must be skipped, not error"
        );
    }

    #[test]
    fn header_is_only_consumed_once() {
        // Feed the header a single byte at a time, then the rest of a
        // 2-sample stream in one go. If header parsing re-ran on every
        // `feed` call instead of latching `header_seen`, this would
        // mis-parse the first tag's bytes as another header attempt.
        let flv = synthetic_avc_flv(2, 25);
        let mut demux = StreamingFlvDemux::new();
        let mut events = Vec::new();
        for b in &flv[..FLV_HEADER_LEN + PREV_TAG_SIZE_LEN] {
            events.extend(feed_and_drain(&mut demux, core::slice::from_ref(b)).unwrap());
        }
        events.extend(
            feed_and_drain(&mut demux, &flv[FLV_HEADER_LEN + PREV_TAG_SIZE_LEN..]).unwrap(),
        );
        events.extend(finish_and_drain(&mut demux));

        let samples = video_samples(&events);
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].1[0], 0);
        assert_eq!(samples[1].1[0], 1);
    }

    /// r04-W14, behaviourally: over a whole file the bytes `feed` memmoves out
    /// of `pending` stay **linear** in the input. Draining once per tag instead
    /// of once per `feed` moves the entire remainder every time, so a
    /// ~500 KB file of ~3 300 tags moves ~860 MB (measured: see the assertion
    /// for the real bound) — quadratic, and the reason an 8 MiB chunk of small
    /// audio tags moved ~80 GB.
    #[test]
    fn feed_moves_a_linear_number_of_bytes_over_a_whole_file() {
        // One big file, many small tags.
        // ~20 bytes of tag framing per 1-byte payload.
        let flv = synthetic_avc_flv(6_000, 25);
        assert!(flv.len() > 100_000, "fixture is over 100 KB");

        // (a) Everything in one `feed` call.
        let mut demux = StreamingFlvDemux::new();
        demux.feed(&flv).unwrap();
        while demux.poll_event().is_some() {}
        let one_shot = demux.bytes_moved;
        // Bites: draining per tag moves ~O(tags × len) ≈ 10^9 bytes here; the
        // single-drain form moves the (empty) remainder once.
        assert!(
            one_shot <= 2 * flv.len(),
            "one feed moved {one_shot} bytes for a {}-byte file",
            flv.len()
        );

        // (b) One byte at a time — the same linear bound must hold.
        let mut demux = StreamingFlvDemux::new();
        for b in &flv {
            demux.feed(core::slice::from_ref(b)).unwrap();
            while demux.poll_event().is_some() {}
        }
        demux.finish();
        while demux.poll_event().is_some() {}
        let byte_at_a_time = demux.bytes_moved;
        // One byte per call means `pending` never holds more than the tag in
        // flight, so each call moves at most `MAX_TAG_LEN` bytes and the total
        // is O(len × MAX_TAG_LEN) — still linear in the input, and *not* the
        // O(tags × len) of the per-tag drain. Bites: the old form moved the
        // whole remainder per tag *per call*, ~10^9 bytes here.
        const MAX_TAG_LEN: usize = TAG_HEADER_LEN + (1 << 24) + PREV_TAG_SIZE_LEN;
        let bound = flv.len().saturating_mul(MAX_TAG_LEN);
        assert!(
            byte_at_a_time <= bound,
            "byte-at-a-time feeding moved {byte_at_a_time} bytes for a {}-byte file",
            flv.len()
        );
        // ...and in practice it is a small constant multiple of the input.
        assert!(
            byte_at_a_time <= 32 * flv.len(),
            "byte-at-a-time feeding moved {byte_at_a_time} bytes for a {}-byte file",
            flv.len()
        );
    }

    /// The same property at a size where the old quadratic form is not merely
    /// over the bound but absurd: 64 KiB of one-byte tags (one `feed` call)
    /// must move O(n), not O(n²).
    #[test]
    fn feed_bytes_moved_is_not_quadratic_in_the_tag_count() {
        let flv = synthetic_avc_flv(4_000, 1);
        let mut demux = StreamingFlvDemux::new();
        demux.feed(&flv).unwrap();
        while demux.poll_event().is_some() {}
        let moved = demux.bytes_moved;
        // Reintroducing a per-tag `drain(0..total)` inside `consume` moves
        // ~sum(remainder) ≈ 4 000 × (flv.len()/2) ≈ 10^8 bytes here.
        assert!(
            moved <= 2 * flv.len(),
            "moved {moved} bytes for a {}-byte file ({} tags)",
            flv.len(),
            4_000
        );
    }

    /// The truncation-safety property the single-drain rewrite must keep: a
    /// tag split across many `feed` calls is still reassembled, and the buffer
    /// holds at most the one in-progress tag (never the whole stream).
    #[test]
    fn split_tags_still_reassemble_after_the_single_drain_rewrite() {
        let flv = synthetic_avc_flv(30, 25);
        let mut demux = StreamingFlvDemux::new();
        let mut events = Vec::new();
        let mut peak_pending = 0usize;
        for chunk in flv.chunks(7) {
            demux.feed(chunk).unwrap();
            peak_pending = peak_pending.max(demux.pending.len());
            while let Some(ev) = demux.poll_event() {
                events.push(ev);
            }
        }
        demux.finish();
        while let Some(ev) = demux.poll_event() {
            events.push(ev);
        }
        assert_eq!(video_samples(&events).len(), 30, "every sample survived");
        // One in-progress tag plus one 7-byte chunk, so well under the file.
        assert!(
            peak_pending < flv.len() / 4,
            "pending stayed at one in-progress tag (peak {peak_pending}, file {})",
            flv.len()
        );
    }
}
