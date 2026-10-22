//! DASH-IF Live Media Ingest (Interface-1) — sans-IO ingest session (increment P2a).
//!
//! Mirrors `crate::source::dash_pull::DashIngestSession` for per-Representation
//! init bytes + the `local_to_global` track-id remap in fixed MPD order, but
//! driven by pushed objects (`IngestObject`) instead of a pull plan. One
//! `Manifest` announces the presentation; `Header`s register tracks; `Segment`s
//! demux against `header ++ segment` and emit `SessionEvent::Sample`s.
//!
//! Cited against `private/specs-md/dashif_live_media_ingest_v1.3.md`
//! §6.2 items 1-3, 10, 11, 18, 19 and §5.3 items 2-3.
//!
//! ## Buffering rule (segments arriving before `NewProgram`)
//!
//! A segment whose Representation already has a header but whose presentation
//! has not yet announced `NewProgram` (some other Representation is still
//! missing its header) is **not** dropped and **not** emitted: its samples are
//! buffered per-Representation (bounded at 64 samples per Representation; an
//! overflow is an error) and flushed in MPD order immediately after
//! `NewProgram` is queued. A segment for a Representation with no header at
//! all returns `IngestError::SegmentWithoutHeader` (412) regardless of the
//! presentation state. Nothing is buffered when the presentation is already
//! live.

use std::collections::{HashMap, VecDeque};

use broadcast_common::{Demand, Stage, Timestamp, Unpackage};
use media_plane::ingress::{IngestSession, ProgramId, SessionEvent};
use media_plane::trunk::RetentionClass;
use transmux::TrackId;
use transmux::media::Fmp4Demux;
use transmux::pipeline::TrackSpec;

use crate::dashif_ingest::{IngestError, IngestState, ManifestGrouping, ObjectKind};
use crate::source::MAX_TS_READ;

use tracing::{debug, warn};

/// One pushed ingest object: its classified kind (from [`crate::dashif_ingest::classify`])
/// plus the raw body bytes. The session never re-parses paths.
#[derive(Debug)]
pub struct IngestObject<'a> {
    /// Classified object kind.
    pub kind: ObjectKind,
    /// Raw body bytes.
    pub bytes: &'a [u8],
}

/// Error returned by [`DashifIngestSession`]'s [`Stage`] implementation.
///
/// Exposes the underlying [`IngestError`] (with its §5.3 HTTP status) for the
/// P2b listener, plus a stringly-typed demux failure for everything else.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// An ingest-protocol error carrying its §5.3 status.
    #[error("ingest: {0}")]
    Ingest(#[from] IngestError),
    /// A CMAF demux failure (init or segment).
    #[error("demux: {0}")]
    Demux(String),
}

#[derive(Default)]
struct RepData {
    header_bytes: Option<Vec<u8>>,
    local_tracks: Option<Vec<TrackSpec>>,
    init_bytes: Option<Vec<u8>>,
    local_to_global: Option<HashMap<TrackId, TrackId>>,
    buffered: Vec<(TrackId, transmux::ir::Sample)>,
}

/// Handle for a streaming (chunked) segment being fed via
/// [`DashifIngestSession::begin_segment`] / `push_segment_bytes` /
/// `finish_segment`.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SegmentHandle {
    rep_id: String,
    number: Option<u64>,
    time: Option<u64>,
    skip: bool,
    buf: Vec<u8>,
}

/// Sans-IO ingest session for one DASH-IF Interface-1 publishing point.
pub struct DashifIngestSession {
    grouping: Option<ManifestGrouping>,
    rep_order: Vec<String>,
    reps: HashMap<String, RepData>,
    ingest_state: IngestState,
    established_emitted: bool,
    new_program_emitted: bool,
    pending_events: VecDeque<SessionEvent>,
}

impl DashifIngestSession {
    /// The manifest grouping learned from a successful Manifest, if any.
    pub fn grouping(&self) -> Option<ManifestGrouping> {
        self.grouping.clone()
    }

    /// Construct an empty session. No manifest has been received yet.
    pub fn new() -> Self {
        Self {
            grouping: None,
            rep_order: Vec::new(),
            reps: HashMap::new(),
            ingest_state: IngestState::new(),
            established_emitted: false,
            new_program_emitted: false,
            pending_events: VecDeque::new(),
        }
    }

    fn on_header(&mut self, rep_id: &str, bytes: &[u8]) -> Result<(), SessionError> {
        if self.grouping.is_none() {
            return Err(SessionError::Ingest(IngestError::NoManifest));
        }
        if !self.reps.contains_key(rep_id) {
            return Err(SessionError::Ingest(IngestError::BadRequest));
        }

        // Identical repeat header: idempotent restart (§6.2.19).
        let is_identical = self
            .reps
            .get(rep_id)
            .and_then(|r| r.header_bytes.as_ref())
            .map(|prev| prev == bytes)
            .unwrap_or(false);
        if is_identical {
            debug!("dashif-ingest: identical header repeat ignored for {rep_id}");
            self.ingest_state.on_header(rep_id);
            return Ok(());
        }

        // Different header for an already-live Representation is a new generation.
        if self.new_program_emitted
            && self
                .reps
                .get(rep_id)
                .is_some_and(|r| r.header_bytes.is_some())
        {
            return Err(SessionError::Ingest(IngestError::HeaderChanged));
        }

        // Demux init: must carry at least one track.
        let media = Fmp4Demux::new()
            .unpackage(bytes)
            .map_err(|e| SessionError::Demux(e.to_string()))?;
        if media.tracks.is_empty() {
            return Err(SessionError::Ingest(IngestError::BadRequest));
        }
        let local_tracks: Vec<TrackSpec> = media.tracks.into_iter().map(|t| t.spec).collect();

        let rep = self
            .reps
            .get_mut(rep_id)
            .ok_or(SessionError::Ingest(IngestError::Internal))?;
        rep.header_bytes = Some(bytes.to_vec());
        rep.init_bytes = Some(bytes.to_vec());
        rep.local_tracks = Some(local_tracks);

        self.ingest_state.on_header(rep_id);

        // Once every Representation has a header, announce the program.
        if !self.new_program_emitted
            && self.rep_order.iter().all(|id| {
                self.reps
                    .get(id)
                    .map(|r| r.header_bytes.is_some())
                    .unwrap_or(false)
            })
        {
            self.emit_new_program();
            self.flush_buffered();
        }

        Ok(())
    }

    fn emit_new_program(&mut self) {
        let mut next_track_id = TrackId::new(1);
        let mut specs = Vec::new();
        for rep_id in self.rep_order.clone() {
            let Some(rep) = self.reps.get_mut(&rep_id) else {
                continue;
            };
            let local_tracks = rep.local_tracks.clone().unwrap_or_default();
            let mut l2g = HashMap::new();
            for spec in &local_tracks {
                let global_id = next_track_id;
                next_track_id = next_track_id.next();
                l2g.insert(spec.track_id, global_id);
                let mut remapped = spec.clone();
                remapped.track_id = global_id;
                specs.push(remapped);
            }
            rep.local_to_global = Some(l2g);
        }
        self.pending_events.push_back(SessionEvent::NewProgram {
            program: ProgramId(0),
            tracks: specs,
        });
        self.new_program_emitted = true;
    }

    fn flush_buffered(&mut self) {
        for rep_id in self.rep_order.clone() {
            let Some(rep) = self.reps.get_mut(&rep_id) else {
                continue;
            };
            let l2g = match &rep.local_to_global {
                Some(m) => m.clone(),
                None => continue,
            };
            let buffered = std::mem::take(&mut rep.buffered);
            for (local_id, sample) in buffered {
                if let Some(&global_id) = l2g.get(&local_id) {
                    self.pending_events.push_back(SessionEvent::Sample {
                        program: ProgramId(0),
                        track_id: global_id,
                        retention: RetentionClass::Timed,
                        sample,
                    });
                }
            }
        }
    }

    fn on_segment(
        &mut self,
        rep_id: &str,
        number: Option<u64>,
        time: Option<u64>,
        bytes: &[u8],
    ) -> Result<(), SessionError> {
        let mut handle = self.begin_segment(rep_id, number, time)?;
        self.push_segment_bytes(&mut handle, bytes)?;
        self.finish_segment(handle)
    }

    pub fn begin_segment(
        &mut self,
        rep: impl AsRef<str>,
        number: Option<u64>,
        time: Option<u64>,
    ) -> Result<SegmentHandle, SessionError> {
        let rep_id = rep.as_ref();
        if self.grouping.is_none() {
            return Err(SessionError::Ingest(IngestError::NoManifest));
        }
        if !self.reps.contains_key(rep_id) {
            return Err(SessionError::Ingest(IngestError::BadRequest));
        }
        if !self.ingest_state.has_header(rep_id) {
            return Err(SessionError::Ingest(IngestError::SegmentWithoutHeader));
        }

        let disp = self
            .ingest_state
            .on_segment(rep_id, number, time)
            .map_err(SessionError::Ingest)?;

        if disp == crate::dashif_ingest::SegmentDisposition::Regression {
            warn!("dashif-ingest: segment regression for {rep_id}");
        }

        Ok(SegmentHandle {
            rep_id: rep_id.to_string(),
            number,
            time,
            skip: disp == crate::dashif_ingest::SegmentDisposition::Duplicate,
            buf: Vec::new(),
        })
    }

    pub fn push_segment_bytes(
        &mut self,
        handle: &mut SegmentHandle,
        bytes: &[u8],
    ) -> Result<(), SessionError> {
        handle.buf.extend_from_slice(bytes);

        if handle.skip {
            // Swallow complete box groups (detect lmsg on a leading styp) but
            // emit nothing.
            loop {
                let mut i = 0usize;
                let mut end = None;
                while i < handle.buf.len() {
                    match transmux::parse_box(&handle.buf[i..]) {
                        Ok((bx, consumed)) => {
                            i += consumed;
                            if bx.header.box_type.is(b"mdat") {
                                end = Some(i);
                                break;
                            }
                            if consumed == 0 {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                match end {
                    Some(e) => {
                        self.detect_styp_lmsg(&handle.rep_id, &handle.buf[..e]);
                        handle.buf.drain(..e);
                    }
                    None => break,
                }
            }
            return Ok(());
        }

        loop {
            let mut i = 0usize;
            let mut end = None;
            while i < handle.buf.len() {
                match transmux::parse_box(&handle.buf[i..]) {
                    Ok((bx, consumed)) => {
                        i += consumed;
                        if bx.header.box_type.is(b"mdat") {
                            end = Some(i);
                            break;
                        }
                        if consumed == 0 {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let Some(e) = end else {
                break;
            };

            let group = handle.buf[..e].to_vec();
            self.detect_styp_lmsg(&handle.rep_id, &group);

            // Demux init ++ chunk.
            let init = self
                .reps
                .get(&handle.rep_id)
                .and_then(|r| r.init_bytes.clone())
                .ok_or(SessionError::Ingest(IngestError::SegmentWithoutHeader))?;
            let mut combined = Vec::with_capacity(init.len() + group.len());
            combined.extend_from_slice(&init);
            combined.extend_from_slice(&group);
            let media = Fmp4Demux::new()
                .unpackage(&combined)
                .map_err(|e| SessionError::Demux(e.to_string()))?;

            if self.new_program_emitted {
                let l2g = self
                    .reps
                    .get(&handle.rep_id)
                    .and_then(|r| r.local_to_global.clone())
                    .unwrap_or_default();
                for track in media.tracks {
                    if let Some(&global_id) = l2g.get(&track.spec.track_id) {
                        for sample in track.samples {
                            self.pending_events.push_back(SessionEvent::Sample {
                                program: ProgramId(0),
                                track_id: global_id,
                                retention: RetentionClass::Timed,
                                sample,
                            });
                        }
                    }
                }
            } else {
                let new_count: usize = media.tracks.iter().map(|t| t.samples.len()).sum();
                let rep = self
                    .reps
                    .get_mut(&handle.rep_id)
                    .ok_or(SessionError::Ingest(IngestError::Internal))?;
                if rep.buffered.len() + new_count > 64 {
                    return Err(SessionError::Ingest(IngestError::Internal));
                }
                for track in media.tracks {
                    for sample in track.samples {
                        rep.buffered.push((track.spec.track_id, sample));
                    }
                }
            }

            handle.buf.drain(..e);
        }

        Ok(())
    }

    pub fn finish_segment(&mut self, mut handle: SegmentHandle) -> Result<(), SessionError> {
        // Process any remaining complete groups first.
        self.push_segment_bytes(&mut handle, &[])?;

        if handle.skip {
            return Ok(());
        }

        if !handle.buf.is_empty() {
            return Err(SessionError::Ingest(IngestError::BadRequest));
        }
        Ok(())
    }

    fn detect_styp_lmsg(&mut self, rep_id: &str, bytes: &[u8]) {
        if let Ok((boxref, _)) = transmux::parse_box(bytes)
            && boxref.header.box_type.is(b"styp")
        {
            let body = boxref.body;
            if body.len() >= 8 {
                let mut i = 8usize;
                let mut slat_count = 0usize;
                while i + 4 <= body.len() {
                    let brand = &body[i..i + 4];
                    if brand == b"lmsg" {
                        self.ingest_state.mark_last(rep_id);
                    } else if brand == b"slat" {
                        slat_count += 1;
                    }
                    i += 4;
                }
                if slat_count > 0 {
                    debug!("dashif-ingest: slat brand count {slat_count} for {rep_id}");
                }
            }
        }
    }
}

impl Default for DashifIngestSession {
    fn default() -> Self {
        Self::new()
    }
}

impl Stage for DashifIngestSession {
    type In<'a> = IngestObject<'a>;
    type Out = SessionEvent;
    type Error = SessionError;

    fn feed(&mut self, input: Self::In<'_>, _now: Timestamp) -> Result<(), Self::Error> {
        match input.kind {
            ObjectKind::Probe => Ok(()),
            ObjectKind::Manifest(crate::dashif_ingest::ManifestFormat::Dash) => {
                if self.grouping.is_some() {
                    debug!("dashif-ingest: re-sent manifest ignored");
                    return Ok(());
                }
                let g = ManifestGrouping::from_mpd(input.bytes).map_err(SessionError::Ingest)?;
                self.rep_order = g.switching_sets.iter().flatten().cloned().collect();
                for id in &self.rep_order {
                    self.reps.entry(id.clone()).or_default();
                }
                self.grouping = Some(g);
                if !self.established_emitted {
                    self.pending_events.push_back(SessionEvent::Established);
                    self.established_emitted = true;
                }
                Ok(())
            }
            ObjectKind::Manifest(crate::dashif_ingest::ManifestFormat::Hls) => {
                debug!("dashif-ingest: HLS manifest ignored (§6.2.17e)");
                Ok(())
            }
            ObjectKind::Header { representation_id } => {
                self.on_header(&representation_id, input.bytes)
            }
            ObjectKind::Segment {
                representation_id,
                number,
                time,
            } => self.on_segment(&representation_id, number, time, input.bytes),
        }
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.pending_events.pop_front()
    }

    fn finish(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(MAX_TS_READ)
    }
}

impl IngestSession for DashifIngestSession {
    type Request = std::convert::Infallible;

    fn poll_transmit(&mut self) -> Option<Self::Request> {
        None
    }
}

// end of file
