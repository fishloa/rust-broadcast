//! Shared helpers for the reader and writer differential tests: drive a file
//! through either blocking driver, reconstruct WebM's per-frame timeline, and
//! compare against the recorded `origin/main` goldens.

use std::fs::File;

use broadcast_matroska::Limits;
use broadcast_matroska::driver::{DriverStep, FileReader, ForwardReader};
use broadcast_matroska::reader::{BlockStep, Mode, Reader, SegmentInfo, Source, Step, TrackInfo};

/// Matroska's default `TimestampScale` in nanoseconds.
pub const IR_NS: u64 = 1_000_000;

/// One fixture's recorded differential values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Golden {
    /// Per-track lines, in track order.
    pub tracks: Vec<String>,
    /// The muxed output's length and FNV line.
    pub mkv: String,
}

/// Read one golden file.
pub fn golden(fixture: &str) -> Golden {
    let name = fixture.rsplit('/').next().unwrap_or(fixture);
    let text = test_fixtures::read_to_string(&format!("mkv/golden/{name}.txt"));
    let mut tracks = Vec::new();
    let mut mkv = String::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("mkv ") {
            mkv = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("track ") {
            tracks.push(rest.to_string());
        }
    }
    Golden { tracks, mkv }
}

/// FNV-1a 64-bit.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

/// One track's owned `TrackEntry` facts.
#[derive(Debug, Clone)]
pub struct TrackMeta {
    pub number: u64,
    pub codec_id: String,
    pub codec_private: Vec<u8>,
    pub track_type: u64,
    pub default_duration_ns: Option<u64>,
    pub pixel_width: Option<u64>,
    pub pixel_height: Option<u64>,
    pub channels: Option<u64>,
    pub sampling_frequency: Option<f64>,
}

impl TrackMeta {
    fn from_reader(info: &TrackInfo) -> Self {
        Self {
            number: info.track_number(),
            codec_id: info.codec_id().to_string(),
            codec_private: info.codec_private().to_vec(),
            track_type: info.track_type_value(),
            default_duration_ns: info.default_duration_ns(),
            pixel_width: info.pixel_width(),
            pixel_height: info.pixel_height(),
            channels: info.channels(),
            sampling_frequency: info.sampling_frequency(),
        }
    }

    /// The `CodecID` the golden line carries.
    pub fn codec_id(&self) -> &str {
        &self.codec_id
    }
}

/// One Block frame as the reader emitted it, with its bytes.
#[derive(Debug, Clone)]
pub struct RawFrame {
    pub track: u64,
    pub block_pts: i64,
    pub lace_index: u32,
    pub lace_count: u32,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

/// Everything one read of a file produced.
#[derive(Debug, Clone)]
pub struct Collected {
    pub scale: u64,
    pub info: Option<SegmentInfo>,
    pub tracks: Vec<TrackMeta>,
    pub frames: Vec<RawFrame>,
    pub cues: usize,
}

/// Which driver to read through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    Seekable,
    Forward,
}

/// Read `path` end to end through one driver, collecting every frame's bytes.
pub fn collect(path: &str, driver: Driver) -> Collected {
    match driver {
        Driver::Seekable => {
            let file = File::open(fixture_path(path)).expect("fixture opens");
            let mut reader = FileReader::new(file, Mode::Strict, Limits::new()).expect("driver");
            drive_collect(&mut reader).expect("read")
        }
        Driver::Forward => {
            let file = File::open(fixture_path(path)).expect("fixture opens");
            let mut reader = ForwardReader::new(file, Mode::Strict, Limits::new());
            drive_collect(&mut reader).expect("read")
        }
    }
}

fn fixture_path(fixture: &str) -> std::path::PathBuf {
    test_fixtures::path(fixture)
}

fn drive_collect<D>(reader: &mut D) -> Result<Collected, Box<dyn std::error::Error>>
where
    D: Drive,
{
    let mut scale = IR_NS;
    let mut info = None;
    let mut tracks = Vec::new();
    let mut frames = Vec::new();
    let mut cues = 0usize;
    loop {
        let step = reader.next_step()?;
        match step {
            OwnedStep::Info(segment) => {
                scale = segment.timestamp_scale_ns;
                info = Some(segment);
            }
            OwnedStep::Tracks(table) => {
                tracks = table;
            }
            OwnedStep::Block(block) => {
                let data = reader.read_frame(&block)?;
                frames.push(RawFrame {
                    track: block.track,
                    block_pts: ir(&block, scale),
                    lace_index: block.lace_index,
                    lace_count: block.lace_count,
                    keyframe: block.keyframe,
                    data,
                });
            }
            OwnedStep::Cues(len) => cues = len,
            OwnedStep::Other => {}
            OwnedStep::Done => break,
        }
    }
    Ok(Collected {
        scale,
        info,
        tracks,
        frames,
        cues,
    })
}

/// The material steps both drivers produce, owned so a caller need not hold a
/// borrow.
#[derive(Debug)]
pub enum OwnedStep {
    Info(SegmentInfo),
    Tracks(Vec<TrackMeta>),
    Block(BlockStep),
    Cues(usize),
    Other,
    Done,
}

/// A blocking driver over a fixture.
pub trait Drive {
    fn next_step(&mut self) -> Result<OwnedStep, Box<dyn std::error::Error>>;
    fn read_frame(&mut self, block: &BlockStep) -> Result<Vec<u8>, Box<dyn std::error::Error>>;
}

impl<R: std::io::Read + std::io::Seek> Drive for FileReader<R> {
    fn next_step(&mut self) -> Result<OwnedStep, Box<dyn std::error::Error>> {
        Ok(match self.step()? {
            DriverStep::Info { info, .. } => OwnedStep::Info(info),
            DriverStep::Tracks { tracks, .. } => {
                OwnedStep::Tracks(tracks.iter().map(TrackMeta::from_reader).collect())
            }
            DriverStep::Block(block) => OwnedStep::Block(block),
            DriverStep::Cues(cues) => OwnedStep::Cues(cues.len()),
            DriverStep::Done => OwnedStep::Done,
            _ => OwnedStep::Other,
        })
    }

    fn read_frame(&mut self, block: &BlockStep) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut out = Vec::new();
        self.read_frame(block, &mut out)?;
        Ok(out)
    }
}

impl<R: std::io::Read> Drive for ForwardReader<R> {
    fn next_step(&mut self) -> Result<OwnedStep, Box<dyn std::error::Error>> {
        Ok(match self.step()? {
            DriverStep::Info { info, .. } => OwnedStep::Info(info),
            DriverStep::Tracks { tracks, .. } => {
                OwnedStep::Tracks(tracks.iter().map(TrackMeta::from_reader).collect())
            }
            DriverStep::Block(block) => OwnedStep::Block(block),
            DriverStep::Cues(cues) => OwnedStep::Cues(cues.len()),
            DriverStep::Done => OwnedStep::Done,
            _ => OwnedStep::Other,
        })
    }

    fn read_frame(&mut self, block: &BlockStep) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut out = Vec::new();
        self.read_frame(block, &mut out)?;
        Ok(out)
    }
}

/// One laid-out sample: its per-track presentation time, in decode order.
#[derive(Debug, Clone)]
pub struct LaidOut {
    /// Per track, in `tracks` order, the frames in file order with their
    /// laid-out presentation times.
    pub per_track: Vec<Vec<(i64, RawFrame)>>,
}

impl LaidOut {
    /// The frames merged across tracks by decode time, ties by track order —
    /// what `transmux::mux_stage::drive_media` feeds a muxer.
    pub fn merged(&self) -> Vec<(i64, RawFrame)> {
        let mut cursors = vec![0usize; self.per_track.len()];
        let mut out = Vec::new();
        loop {
            let mut best: Option<usize> = None;
            for (track, list) in self.per_track.iter().enumerate() {
                let Some((pts, _)) = list.get(cursors[track]) else {
                    continue;
                };
                best = match best {
                    None => Some(track),
                    Some(previous) => {
                        let other = self.per_track[previous][cursors[previous]].0;
                        if (*pts, track) < (other, previous) {
                            Some(track)
                        } else {
                            Some(previous)
                        }
                    }
                };
            }
            let Some(track) = best else {
                break;
            };
            out.push(self.per_track[track][cursors[track]].clone());
            cursors[track] += 1;
        }
        out
    }
}

/// Reconstruct WebM's per-frame timeline: `DefaultDuration` when the track
/// states one, else the first positive block gap divided by its frame count.
pub fn lay_out(collected: &Collected) -> LaidOut {
    let mut per_track = Vec::with_capacity(collected.tracks.len());
    for track in &collected.tracks {
        let blocks: Vec<(i64, u32)> = collected
            .frames
            .iter()
            .filter(|frame| frame.track == track.number && frame.lace_index == 0)
            .map(|frame| (frame.block_pts, frame.lace_count))
            .collect();
        let nominal = match track.default_duration_ns {
            Some(ns) => (ns + IR_NS / 2) / IR_NS,
            None => {
                let mut nominal = 0u64;
                for pair in blocks.windows(2) {
                    let gap = pair[1].0.saturating_sub(pair[0].0);
                    if gap > 0 && pair[1].1 > 0 {
                        nominal = (gap as u64) / u64::from(pair[1].1);
                        break;
                    }
                }
                nominal
            }
        };
        let nominal = if nominal == 0 { 1 } else { nominal };
        let mut list = Vec::with_capacity(collected.frames.len());
        for frame in collected.frames.iter().filter(|f| f.track == track.number) {
            let pts = frame.block_pts + i64::from(frame.lace_index) * nominal as i64;
            list.push((pts, frame.clone()));
        }
        per_track.push(list);
    }
    LaidOut { per_track }
}

/// The golden `track` line for one laid-out track.
pub fn summary_line(track: &TrackMeta, frames: &[(i64, RawFrame)]) -> String {
    let mut all = Vec::new();
    for (_, frame) in frames {
        all.extend_from_slice(&frame.data);
    }
    let first = frames.first().map(|(pts, _)| *pts).unwrap_or(0);
    let last = frames.last().map(|(pts, _)| *pts).unwrap_or(0);
    format!(
        "{} {} samples={} first={} last={} fnv={:016x}",
        track.number,
        track.codec_id(),
        frames.len(),
        first,
        last,
        fnv1a(&all)
    )
}

/// The per-track golden lines for a collection.
pub fn summary_lines(collected: &Collected) -> Vec<String> {
    let laid = lay_out(collected);
    collected
        .tracks
        .iter()
        .zip(&laid.per_track)
        .map(|(track, frames)| summary_line(track, frames))
        .collect()
}

fn ir(block: &BlockStep, scale: u64) -> i64 {
    let ns = i128::from(block.timestamp) * i128::from(scale);
    i64::try_from(ns / i128::from(IR_NS)).unwrap_or(i64::MIN)
}

/// A reader that hands out at most `CHUNK` bytes per `read`, so a driver's own
/// loop is exercised at that granularity.
#[derive(Debug)]
pub struct Chunked<R> {
    inner: R,
    chunk: usize,
}

impl<R> Chunked<R> {
    /// Wrap `inner`, returning at most `chunk` bytes per read.
    pub const fn new(inner: R, chunk: usize) -> Self {
        Self { inner, chunk }
    }
}

impl<R: std::io::Read> std::io::Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let len = buf.len().min(self.chunk);
        self.inner.read(&mut buf[..len])
    }
}

impl<R: std::io::Seek> std::io::Seek for Chunked<R> {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}

/// Read a byte buffer through the seekable driver with `chunk`-bounded reads.
pub fn collect_bytes_seekable(bytes: &[u8], chunk: usize) -> Collected {
    let cursor = std::io::Cursor::new(bytes.to_vec());
    let mut reader = FileReader::new(
        Chunked::new(cursor, chunk.max(1)),
        Mode::Strict,
        Limits::new(),
    )
    .expect("driver");
    drive_collect(&mut reader).expect("read")
}

/// Read a byte buffer through the forward-only driver with `chunk`-bounded
/// reads.
pub fn collect_bytes_forward(bytes: &[u8], chunk: usize) -> Collected {
    let cursor = std::io::Cursor::new(bytes.to_vec());
    let mut reader = ForwardReader::new(
        Chunked::new(cursor, chunk.max(1)),
        Mode::Strict,
        Limits::new(),
    );
    drive_collect(&mut reader).expect("read")
}

/// A [`Reader`] over one whole byte slice, answering its own demands.
pub struct Slice<'a> {
    /// The underlying reader, for `seek_to` and its tables.
    pub reader: Reader,
    bytes: &'a [u8],
}

impl<'a> Slice<'a> {
    /// A seekable reader over `bytes`.
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            reader: Reader::new(
                Source::Seekable {
                    len: bytes.len() as u64,
                },
                Mode::Strict,
                Limits::new(),
            ),
            bytes,
        }
    }

    /// A forward-only reader over `bytes`.
    #[must_use]
    pub fn forward(bytes: &'a [u8]) -> Self {
        Self {
            reader: Reader::new(Source::ForwardOnly, Mode::Strict, Limits::new()),
            bytes,
        }
    }

    /// The next Block, supplying every demand from `bytes`; `None` at the end.
    pub fn advance_block(&mut self) -> Result<Option<BlockStep>, Box<dyn std::error::Error>> {
        loop {
            match self.reader.step()? {
                Step::Need(range) => {
                    let start = usize::try_from(range.offset)?;
                    let length = usize::try_from(range.len)?;
                    let end = start.saturating_add(length).min(self.bytes.len());
                    let supplied = self.bytes.get(start..end).unwrap_or(&[]);
                    self.reader.supply(range.offset, supplied)?;
                }
                Step::Block(block) => return Ok(Some(block)),
                Step::Done => return Ok(None),
                _ => {}
            }
        }
    }

    /// Step to the end.
    pub fn walk(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        while self.advance_block()?.is_some() {}
        Ok(())
    }

    /// The next Block's timestamp and keyframe flag.
    pub fn next_block(&mut self) -> (i64, bool) {
        self.advance_block()
            .expect("step")
            .map(|block| (block.timestamp, block.keyframe))
            .expect("no block after seek")
    }
}
