//! [`SmoothOrigin`]: the live Manifest and fragment resolution over a `Trunk`.

use std::sync::{Arc, Mutex, MutexGuard};

use broadcast_common::{Timestamp, Unpackage};
use broadcast_smooth::{C, QualityLevel, SmoothManifest, StreamIndex, StreamType};
use media_plane::Trunk;
use media_plane::egress::{AwaitPolicy, CachePolicy, EgressResponse, ServedEgress};
use media_plane::ids::SegmentSeq;
use transmux::{
    Fmp4Demux, Media, SmoothCodec, SmoothPackager, SmoothStreamType, Track, TrackSpec,
    resolve_codec, to_smooth_ticks,
};

/// The prefix of this origin's fragment-URL template and of every fragment
/// request path: the `QualityLevels(N)` segment of MS-SSTR §2.2.3.
pub const QUALITY_LEVELS_PREFIX: &str = "QualityLevels(";

/// The segment of a fragment URL carrying its start time: `Fragments(N={start time})`
/// (MS-SSTR §2.2.3).
pub const FRAGMENTS_SEGMENT: &str = "Fragments(";

/// `StreamIndex@Subtype` for a text stream: `SUBT`, "Subtitles that are used for
/// foreign-language audio" (MS-SSTR §2.2.2.3). The IR does not say whether a
/// track is a caption service for the deaf (`CAPT`), so the subtitle category is
/// used.
pub const SUBTYPE_SUBT: &str = "SUBT";

/// `QualityLevel@AudioTag` for raw AAC (MS-SSTR §2.2.2.5).
const AUDIO_TAG_AAC: u32 = 255;
/// `QualityLevel@BitsPerSample` advertised for AAC audio.
const AAC_BITS_PER_SAMPLE: u16 = 16;
/// The quality-level count every stream this origin emits advertises: one
/// `QualityLevel` per track, whose fragments are served at that one rate.
const STREAM_QUALITY_LEVELS: u32 = 1;
/// The 1-based ordinal of the first stream of a `Type`; its `Name` is the bare
/// `Type`.
const FIRST_STREAM_ORDINAL: usize = 1;
/// `SmoothStreamingMedia@LookAheadFragmentCount`: this origin publishes a
/// fragment only once its segment has closed, so it never runs ahead of the
/// manifest it advertises.
const LOOK_AHEAD_FRAGMENT_COUNT: u32 = 0;
/// Target fragment duration above which a live segment is never cut: set far
/// above any real segment so one `Trunk` segment maps to exactly one Smooth
/// fragment. Whole seconds because [`SmoothPackager::target_duration_secs`] is
/// an integer.
const SINGLE_SEGMENT_FRAGMENT_TARGET_SECS: u32 = 3600;
/// Bits per second floor of the advertised `Bitrate` for an AAC track.
const MIN_AUDIO_BITRATE: u64 = 64_000;
/// Bits per second ceiling of the advertised `Bitrate` for an AAC track (320
/// kbps, stereo AAC).
const MAX_AUDIO_BITRATE: u64 = 320_000;
/// Bits per second floor of the advertised `Bitrate` for a video track.
const MIN_VIDEO_BITRATE: u64 = 500_000;
/// Bits per second ceiling of the advertised `Bitrate` for a video track.
const MAX_VIDEO_BITRATE: u64 = 20_000_000;
/// Bits per coded pixel per second in the video `Bitrate` estimate.
const VIDEO_BITS_PER_PIXEL: u64 = 4;
/// Width assumed for the video `Bitrate` estimate when the codec carries none.
const DEFAULT_VIDEO_WIDTH: u64 = 1280;
/// Height assumed for the video `Bitrate` estimate when the codec carries none.
const DEFAULT_VIDEO_HEIGHT: u64 = 720;
/// Sampling rate assumed for the audio `Bitrate` estimate when the codec carries
/// none.
const DEFAULT_SAMPLING_RATE: u64 = 48_000;
/// Channel count assumed for the audio `Bitrate` estimate when the codec carries
/// none.
const DEFAULT_CHANNELS: u64 = 2;
/// Bits per sample assumed for the audio `Bitrate` estimate.
const ESTIMATE_BITS_PER_SAMPLE: u64 = 16;

/// What identifies one advertised stream: its media kind and the 1-based
/// ordinal of that kind, `(kind, ordinal)`, exactly the pair the manifest's
/// `StreamIndex@Name`/`@Url` and the request grammar name. Two tracks of one
/// kind are therefore distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamKey {
    /// The `StreamIndex@Type` kind.
    pub kind: SmoothStreamType,
    /// 1-based position among the streams of `kind`.
    pub ordinal: u32,
}

impl StreamKey {
    /// The `StreamIndex@Name` this key is advertised under: the bare `Type` for
    /// the first stream of a kind, `TYPE_N` for the rest (MS-SSTR §2.2.2.3).
    pub fn name(&self) -> String {
        let kind = self.kind.name();
        match self.ordinal as usize {
            FIRST_STREAM_ORDINAL => kind.to_string(),
            ordinal => format!("{kind}_{ordinal}"),
        }
    }

    /// The `StreamIndex@Url` fragment-URL template whose `Fragments(N=...)`
    /// token carries this key's [`Self::name`], so each stream of a kind is
    /// addressed distinctly.
    pub fn url_template(&self) -> String {
        format!(
            "{QUALITY_LEVELS_PREFIX}{{bitrate}})/{FRAGMENTS_SEGMENT}{}={{start time}})",
            self.name()
        )
    }
}

/// One fragment a manifest advertises: its stream, absolute start time and
/// duration in [`SMOOTH_TIMESCALE`](transmux::SMOOTH_TIMESCALE) ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct FragmentLocation {
    /// The stream the fragment belongs to.
    pub stream: StreamKey,
    /// `c@t` — the fragment's absolute start time on its stream's timeline.
    pub start: u64,
    /// `c@d` — the fragment's duration.
    pub duration: u64,
}

/// What a caller asks a [`SmoothOrigin`] for: the client Manifest, or one
/// fragment of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SmoothRequest {
    /// `GET .../Manifest` — the live client Manifest.
    Manifest,
    /// `GET ...  /QualityLevels(BITRATE)/Fragments(KIND=START)`.
    Fragment {
        /// Which advertised stream (by name) the request names.
        stream: StreamKey,
        /// The `QualityLevel@Bitrate` the request's `QualityLevels(N)` carries.
        bitrate: u64,
        /// The requested `c@t`, in [`SMOOTH_TIMESCALE`](transmux::SMOOTH_TIMESCALE)
        /// ticks.
        start: u64,
    },
}

/// The body of a [`SmoothOrigin`] answer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SmoothBody {
    /// The client Manifest document.
    Manifest(String),
    /// One fragment's `styp`+`moof`(+`tfxd`)+`mdat` bytes.
    Fragment(Vec<u8>),
}

/// Why a [`SmoothOrigin`] cannot describe the trunk's media.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SmoothOriginError {
    /// A track's codec has no MS-SSTR representation (HEVC, Opus, a DVB
    /// subtitle, …).
    #[error("track {track} has no Smooth Streaming codec representation")]
    UnrepresentableCodec {
        /// The track's id.
        track: u32,
    },
    /// A track's codec is representable in principle but unusable in fact (an
    /// AAC track whose `esds` carries no DecoderSpecificInfo, an AVC track
    /// whose SPS does not decode) — the packager's own error, rendered.
    #[error("track {track} cannot be described: {reason}")]
    Codec {
        /// The track's id.
        track: u32,
        /// The codec resolver's error, rendered.
        reason: String,
    },
    /// The manifest model's writer refused the assembled document.
    #[error("manifest could not be rendered: {reason}")]
    Manifest {
        /// The writer's error, rendered.
        reason: String,
    },
}

/// One advertised stream's chunk timeline: per segment, the fragment's absolute
/// start and duration on the track's own timeline.
#[derive(Debug, Clone)]
struct ChunkTiming {
    sequence: SegmentSeq,
    start: u64,
    duration: u64,
}

/// One resolved track: its manifest codec parameters and chunk timeline.
#[derive(Debug, Clone)]
struct StreamPlan {
    key: StreamKey,
    codec: SmoothCodec,
    interleaved_ordinal: u32,
    chunks: Vec<ChunkTiming>,
}

/// An assembled manifest plus the query a [`SmoothRequest::Fragment`] needs.
#[derive(Debug, Clone)]
struct Plan {
    manifest: String,
    streams: Vec<StreamPlan>,
}

impl Plan {
    /// The manifest model `broadcast-smooth` serializes.
    fn manifest(&self) -> SmoothManifest {
        let duration = self
            .streams
            .iter()
            .filter_map(|stream| stream.chunks.last())
            .map(|chunk| chunk.start.saturating_add(chunk.duration))
            .max()
            .unwrap_or(0);
        let streams = self.streams.iter().map(stream_index).collect();
        SmoothManifest::new(streams)
            .with_duration(duration)
            .live(duration, LOOK_AHEAD_FRAGMENT_COUNT)
    }

    /// The chunk of `stream` starting at `start`.
    fn chunk(&self, stream: StreamKey, start: u64) -> Option<(StreamPlan, ChunkTiming)> {
        let plan = self.streams.iter().find(|plan| plan.key == stream)?.clone();
        let chunk = plan.chunks.iter().find(|chunk| chunk.start == start)?;
        let chunk = chunk.clone();
        Some((plan, chunk))
    }
}

/// The `StreamIndex` element for one resolved track: its `Name`/`Url` from the
/// stream key (so two streams of one kind stay distinct), the one `QualityLevel`
/// its fragments are served at, and one `c` per chunk.
fn stream_index(stream: &StreamPlan) -> StreamIndex {
    let codec = &stream.codec;
    let mut level = QualityLevel::new(
        codec.fourcc,
        advertised_bitrate(codec),
        codec.codec_private_data.clone(),
    );
    if let (Some(width), Some(height)) = (codec.width, codec.height) {
        level = level.with_size(width, height);
    }
    if codec.stream_type == SmoothStreamType::Audio
        && let (Some(rate), Some(channels)) = (codec.sampling_rate, codec.channels)
    {
        level = level.with_audio(rate, channels, AAC_BITS_PER_SAMPLE, 0, AUDIO_TAG_AAC);
    }
    level = level.without_packet_size();

    let chunks = stream
        .chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let ordinal = u32::try_from(i).unwrap_or(u32::MAX);
            C::new(chunk.duration).numbered(ordinal).at(chunk.start)
        })
        .collect();
    let count = u32::try_from(stream.chunks.len()).unwrap_or(u32::MAX);
    let mut index = StreamIndex::new(
        manifest_stream_type(stream.codec.stream_type),
        stream.key.url_template(),
        vec![level],
    )
    .with_counts(count, STREAM_QUALITY_LEVELS)
    .with_chunks(chunks);
    if stream.codec.stream_type == SmoothStreamType::Text {
        index = index.with_subtype(SUBTYPE_SUBT);
    }
    if stream.key.ordinal as usize > FIRST_STREAM_ORDINAL {
        index = index.with_name(stream.key.name());
    }
    index
}

/// The `StreamIndex@Type` token for a stream kind.
fn manifest_stream_type(kind: SmoothStreamType) -> StreamType {
    match kind {
        SmoothStreamType::Video => StreamType::Video,
        SmoothStreamType::Audio => StreamType::Audio,
        _ => StreamType::Text,
    }
}

/// The `QualityLevel@Bitrate` advertised for `codec`: a documented per-codec
/// estimate, because the live window carries no per-track byte count at
/// manifest-render time (Smooth's `Bitrate` is advisory ABR metadata that the
/// fragment URLs carry back).
fn advertised_bitrate(codec: &SmoothCodec) -> u64 {
    match codec.stream_type {
        SmoothStreamType::Audio => {
            let rate = u64::from(codec.sampling_rate.unwrap_or(DEFAULT_SAMPLING_RATE as u32));
            let channels = u64::from(codec.channels.unwrap_or(DEFAULT_CHANNELS as u16));
            rate.saturating_mul(channels)
                .saturating_mul(ESTIMATE_BITS_PER_SAMPLE)
                .clamp(MIN_AUDIO_BITRATE, MAX_AUDIO_BITRATE)
        }
        SmoothStreamType::Video => {
            let width = u64::from(codec.width.unwrap_or(DEFAULT_VIDEO_WIDTH as u32));
            let height = u64::from(codec.height.unwrap_or(DEFAULT_VIDEO_HEIGHT as u32));
            width
                .saturating_mul(height)
                .saturating_mul(VIDEO_BITS_PER_PIXEL)
                .clamp(MIN_VIDEO_BITRATE, MAX_VIDEO_BITRATE)
        }
        _ => MIN_AUDIO_BITRATE,
    }
}

/// The live Smooth origin over one `Trunk`.
pub struct SmoothOrigin {
    trunk: Arc<Trunk>,
    /// The init segment the trunk's segments were muxed with — the one thing
    /// no `Trunk` ring holds, and what a fragment request is re-demuxed
    /// against.
    init: Mutex<Option<Arc<[u8]>>>,
}

impl SmoothOrigin {
    /// An origin over `trunk`; it describes whatever segments the trunk
    /// currently holds, once an init segment is stored and at least one
    /// track is announced.
    pub fn new(trunk: Arc<Trunk>) -> Self {
        Self {
            trunk,
            init: Mutex::new(None),
        }
    }

    /// The `Trunk` this origin reads.
    pub fn trunk(&self) -> &Arc<Trunk> {
        &self.trunk
    }

    /// Store the fMP4 init segment the trunk's segments are muxed with — the
    /// one thing no `Trunk` ring holds, and what a fragment request is
    /// re-demuxed against.
    pub fn set_init(&self, bytes: impl Into<Arc<[u8]>>) {
        *locked(&self.init) = Some(bytes.into());
    }

    /// The init segment currently stored, if any.
    pub fn init(&self) -> Option<Arc<[u8]>> {
        locked(&self.init).clone()
    }

    /// The fragments currently advertised, in manifest order, or `None` while
    /// the trunk has nothing to advertise yet.
    pub fn fragments(&self) -> Result<Vec<FragmentLocation>, SmoothOriginError> {
        let plan = match self.plan() {
            Ok(plan) => plan,
            Err(Unavailable::Waiting { .. }) => return Ok(Vec::new()),
            Err(Unavailable::Unusable(error)) => return Err(error),
        };
        Ok(plan
            .streams
            .iter()
            .flat_map(|stream| {
                stream.chunks.iter().map(|chunk| FragmentLocation {
                    stream: stream.key,
                    start: chunk.start,
                    duration: chunk.duration,
                })
            })
            .collect())
    }

    /// Render the live client Manifest, or why it cannot be rendered yet
    /// ([`EgressResponse::Wait`]) or at all ([`EgressResponse::BadRequest`]).
    pub fn manifest(&self, now: Timestamp, await_policy: AwaitPolicy) -> EgressResponse<String> {
        match self.plan() {
            Ok(plan) => EgressResponse::Ready {
                body: plan.manifest,
                cache: CachePolicy::NoCache,
            },
            Err(Unavailable::Waiting {
                retry_no_earlier_than,
            }) => EgressResponse::pending(await_policy, now, retry_no_earlier_than),
            Err(Unavailable::Unusable(_)) => EgressResponse::BadRequest {
                reason: UNUSABLE_MANIFEST,
            },
        }
    }
}

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Why no plan could be built: the trunk has nothing to describe yet, or what
/// it has cannot be described.
enum Unavailable {
    Waiting { retry_no_earlier_than: Timestamp },
    Unusable(SmoothOriginError),
}

impl From<SmoothOriginError> for Unavailable {
    fn from(error: SmoothOriginError) -> Self {
        Unavailable::Unusable(error)
    }
}

impl SmoothOrigin {
    /// Describe the trunk's current media:
    /// resolve the trunk's media into a [`Plan`].
    fn plan(&self) -> Result<Plan, Unavailable> {
        let init = self.init().ok_or(Unavailable::Waiting {
            retry_no_earlier_than: Timestamp::from_nanos(0),
        })?;
        let specs = self.trunk.tracks();
        if specs.is_empty() {
            return Err(Unavailable::Waiting {
                retry_no_earlier_than: Timestamp::from_nanos(0),
            });
        }
        let segments: Vec<(SegmentSeq, Arc<[u8]>)> = self
            .trunk
            .segments()
            .into_iter()
            .map(|segment| (segment.sequence_number, Arc::from(segment.bytes.as_ref())))
            .collect();
        if segments.is_empty() {
            return Err(Unavailable::Waiting {
                retry_no_earlier_than: Timestamp::from_nanos(0),
            });
        }
        Ok(build_plan(specs.as_ref(), &init, &segments)?)
    }
}

/// Resolve `specs` and the window's `segments` into the live plan.
fn build_plan(
    specs: &[TrackSpec],
    init: &[u8],
    segments: &[(SegmentSeq, Arc<[u8]>)],
) -> Result<Plan, SmoothOriginError> {
    let mut ordinals: Vec<(SmoothStreamType, u32)> = Vec::new();
    let mut streams: Vec<StreamPlan> = Vec::new();
    for spec in specs {
        let codec = resolve_codec(&spec.config).map_err(|error| SmoothOriginError::Codec {
            track: spec.track_id.value(),
            reason: error.to_string(),
        })?;
        let key = StreamKey {
            kind: codec.stream_type,
            ordinal: match ordinals
                .iter_mut()
                .find(|(kind, _)| *kind == codec.stream_type)
            {
                Some((_, count)) => {
                    *count = count.saturating_add(1);
                    *count
                }
                None => {
                    let first = FIRST_STREAM_ORDINAL as u32;
                    ordinals.push((codec.stream_type, first));
                    first
                }
            },
        };
        let chunks = chunk_timeline(init, segments, spec, key)?;
        if chunks.is_empty() {
            continue;
        }
        streams.push(StreamPlan {
            key,
            codec,
            interleaved_ordinal: spec.track_id.value().saturating_sub(FIRST_TRACK_ID) + 1,
            chunks,
        });
    }
    if streams.is_empty() {
        return Err(SmoothOriginError::Codec {
            track: 0,
            reason: "the window carries none of the trunk's tracks".to_string(),
        });
    }
    let mut plan = Plan {
        manifest: String::new(),
        streams,
    };
    plan.manifest = plan.manifest().to_xml();
    Ok(plan)
}

/// Track ids are 1-based, so the first interleaved track is ordinal 1.
const FIRST_TRACK_ID: u32 = 1;

/// One track's chunk timeline over the window, in the track's own timeline:
/// each chunk starts where the previous one ended, and the first starts at its
/// segment's own absolute first-sample time, so the timeline is stable as the
/// window slides.
fn chunk_timeline(
    init: &[u8],
    segments: &[(SegmentSeq, Arc<[u8]>)],
    spec: &TrackSpec,
    key: StreamKey,
) -> Result<Vec<ChunkTiming>, SmoothOriginError> {
    let packager = SmoothPackager {
        target_duration_secs: SINGLE_SEGMENT_FRAGMENT_TARGET_SECS,
    };
    let mut chunks: Vec<ChunkTiming> = Vec::with_capacity(segments.len());
    for (sequence, bytes) in segments {
        let media = demux(init, bytes).map_err(|reason| SmoothOriginError::Codec {
            track: spec.track_id.value(),
            reason,
        })?;
        let Some(track) = find_track(&media, key) else {
            continue;
        };
        let Some(first) = track.samples.first() else {
            continue;
        };
        let timescale = track.spec.timescale.max(1);
        let absolute = u64::try_from(first.dts.unwrap_or_default().max(0)).unwrap_or(0);
        let start = chunks
            .last()
            .map(|chunk: &ChunkTiming| chunk.start.saturating_add(chunk.duration))
            .unwrap_or_else(|| to_smooth_ticks(absolute, timescale));
        let sequence_number = sequence.value();
        let duration = packager
            .package_track_fragment(track, sequence_number, start)
            .map(|fragment| fragment.duration)
            .unwrap_or_default();
        chunks.push(ChunkTiming {
            sequence: *sequence,
            start,
            duration,
        });
    }
    Ok(chunks)
}

/// Demux one init + media segment, rendering the demuxer's error.
fn demux(init: &[u8], segment: &[u8]) -> Result<Media, String> {
    let mut combined = Vec::with_capacity(init.len() + segment.len());
    combined.extend_from_slice(init);
    combined.extend_from_slice(segment);
    Fmp4Demux::new()
        .unpackage(combined.as_slice())
        .map_err(|error| error.to_string())
}

/// The demuxed track a [`StreamKey`] names: the `ordinal`-th track of its kind
/// in the segment's own interleaved track order.
fn find_track(media: &Media, key: StreamKey) -> Option<&Track> {
    media
        .tracks
        .iter()
        .filter(|track| stream_kind(&track.spec.config) == Some(key.kind))
        .nth(usize::try_from(key.ordinal.checked_sub(1)?).ok()?)
}

/// The stream kind a track's codec maps to, or `None` for a codec MS-SSTR (as
/// implemented here) cannot carry.
fn stream_kind(config: &transmux::CodecConfig) -> Option<SmoothStreamType> {
    match config {
        transmux::CodecConfig::Avc { .. } => Some(SmoothStreamType::Video),
        transmux::CodecConfig::Aac { .. } => Some(SmoothStreamType::Audio),
        transmux::CodecConfig::Subtitle {
            sample_entry: Some(_),
            ..
        } => Some(SmoothStreamType::Text),
        _ => None,
    }
}

impl ServedEgress for SmoothOrigin {
    type Request = SmoothRequest;
    type Body = SmoothBody;

    fn resolve(
        &self,
        request: SmoothRequest,
        now: Timestamp,
        await_policy: AwaitPolicy,
    ) -> EgressResponse<SmoothBody> {
        let plan = match self.plan() {
            Ok(plan) => plan,
            Err(Unavailable::Waiting {
                retry_no_earlier_than,
            }) => return EgressResponse::pending(await_policy, now, retry_no_earlier_than),
            Err(Unavailable::Unusable(_)) => {
                return EgressResponse::BadRequest {
                    reason: UNUSABLE_MANIFEST,
                };
            }
        };
        match request {
            SmoothRequest::Manifest => EgressResponse::Ready {
                body: SmoothBody::Manifest(plan.manifest),
                cache: CachePolicy::NoCache,
            },
            SmoothRequest::Fragment {
                stream,
                bitrate,
                start,
            } => match plan.chunk(stream, start) {
                Some((stream, chunk)) if bitrate == advertised_bitrate(&stream.codec) => {
                    match self.fragment(&stream, chunk) {
                        Ok(bytes) => EgressResponse::Ready {
                            body: SmoothBody::Fragment(bytes),
                            cache: CachePolicy::Immutable,
                        },
                        Err(_) => EgressResponse::BadRequest {
                            reason: UNUSABLE_FRAGMENT,
                        },
                    }
                }
                _ => EgressResponse::NotFound,
            },
        }
    }
}

impl SmoothOrigin {
    /// Build the fragment for `chunk` of `stream` from the window segment it
    /// came from: demux, take the track, and re-mux one fragment whose `tfxd`
    /// is the manifest's `c@t`.
    fn fragment(
        &self,
        stream: &StreamPlan,
        chunk: ChunkTiming,
    ) -> Result<Vec<u8>, SmoothOriginError> {
        let init = self.init().ok_or(SmoothOriginError::Codec {
            track: stream.interleaved_ordinal,
            reason: "no init segment is published".to_string(),
        })?;
        let bytes = self
            .trunk
            .segments()
            .into_iter()
            .find(|segment| segment.sequence_number == chunk.sequence)
            .map(|segment| segment.bytes)
            .ok_or(SmoothOriginError::Codec {
                track: stream.interleaved_ordinal,
                reason: "the fragment's segment has left the window".to_string(),
            })?;
        let media = demux(&init, &bytes).map_err(|reason| SmoothOriginError::Codec {
            track: stream.interleaved_ordinal,
            reason,
        })?;
        let track = find_track(&media, stream.key).ok_or(SmoothOriginError::Codec {
            track: stream.interleaved_ordinal,
            reason: "the segment does not carry the requested stream".to_string(),
        })?;
        let sequence_number = chunk.sequence.value();
        let fragment = SmoothPackager {
            target_duration_secs: SINGLE_SEGMENT_FRAGMENT_TARGET_SECS,
        }
        .package_track_fragment(track, sequence_number, chunk.start)
        .map_err(|error| SmoothOriginError::Codec {
            track: stream.interleaved_ordinal,
            reason: error.to_string(),
        })?;
        Ok(fragment.data)
    }
}

impl SmoothRequest {
    /// Parse a request from a fragment path or its file name
    /// (`QualityLevels(BITRATE)/Fragments(KIND=START)`, MS-SSTR §2.2.3), or a
    /// Manifest file name. `kinds` supplies the advertised stream ordinals the
    /// `KIND` token may name (`video`, `audio`, `text`, and `TYPE_N`). `None`
    /// for an unrecognised path.
    pub fn parse(path: &str, kinds: &[StreamKey]) -> Option<SmoothRequest> {
        let manifest_name = path.rsplit('/').find(|segment| !segment.is_empty())?;
        if manifest_name == MANIFEST_FILE_NAME {
            return Some(SmoothRequest::Manifest);
        }
        // The URL template has two path segments: `QualityLevels(N)` and
        // `Fragments(KIND=START)` (relative to the Manifest's own directory).
        let mut trailing = path
            .rsplit('/')
            .filter(|segment| !segment.is_empty())
            .take(2);
        let fragment = trailing.next()?;
        let quality = trailing.next()?.strip_prefix(QUALITY_LEVELS_PREFIX)?;
        let bitrate = quality.strip_suffix(')')?.parse().ok()?;
        let (kind, start) = fragment.strip_prefix(FRAGMENTS_SEGMENT)?.split_once('=')?;
        let start = start.strip_suffix(')')?.parse().ok()?;
        let stream = kinds
            .iter()
            .copied()
            .find(|key| key.name() == kind)
            .or_else(|| kinds.iter().copied().find(|key| key.kind.name() == kind))?;
        Some(SmoothRequest::Fragment {
            stream,
            bitrate,
            start,
        })
    }
}

/// The bad-request reason a manifest answers when a track it must describe has
/// no usable Smooth codec description.
const UNUSABLE_MANIFEST: &str =
    "a track has no usable Smooth Streaming codec description; the Manifest cannot be rendered";

/// The bad-request reason a fragment answers when its segment cannot be
/// re-muxed into the requested stream's fragment.
const UNUSABLE_FRAGMENT: &str = "the requested fragment cannot be built from the live window";

/// The file name of the client Manifest (MS-SSTR §2.2.1).
pub const MANIFEST_FILE_NAME: &str = "Manifest";

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::time::Duration;

    use broadcast_smooth::StreamType;
    use media_plane::TrunkConfig;
    use media_plane::trunk::SegmentEntry;
    use transmux::SegmentMeta;
    use transmux::avc_config_from_sprop;
    use transmux::ll_hls::LlHlsSegmenter;
    use transmux::pipeline::Sample;
    use transmux::smooth_fragment::fragment_tfxd;
    use transmux::{
        AVCConfigurationBox, AudioSpecificConfig, CodecConfig, DecoderConfigDescriptor,
        DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor, TrackId, TrackSpec,
    };

    use super::*;

    /// The SPS/PPS pair of a real H.264 stream, as an `sprop-parameter-sets`
    /// value: its SPS decodes to 640x480.
    const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
    /// The SPS's coded width (the `sprop-parameter-sets` stream is 640x360).
    const SPROP_WIDTH: u32 = 640;
    /// The SPS's coded height.
    const SPROP_HEIGHT: u32 = 480;
    /// Video timescale: the H.264 clock.
    const VIDEO_TIMESCALE: u32 = 90_000;
    /// A 30 fps frame duration in `VIDEO_TIMESCALE` ticks.
    const FRAME_DUR: u32 = VIDEO_TIMESCALE / 30;
    /// Frames per segment: two seconds of video.
    const FRAMES_PER_SEGMENT: u32 = 60;
    /// Segments published per test route.
    const SEGMENTS: u32 = 2;
    /// The AAC media timescale, in ticks per second.
    const AUDIO_TIMESCALE: u32 = 48_000;
    /// AAC-LC frame duration, in `AUDIO_TIMESCALE` ticks (1024 samples).
    const AAC_FRAME_DUR: u32 = 1024;
    /// Sample rate the AAC config reports.
    const SAMPLE_RATE: u32 = 48_000;
    /// Channel count the AAC config reports.
    const CHANNELS: u16 = 2;
    /// An AAC-LC `AudioSpecificConfig` for 48 kHz stereo (`0x11 0x90`).
    const ASC_48K_STEREO: [u8; 2] = [0x11, 0x90];
    /// `SLConfigDescriptor@predefined` 2: the MP4-storage form (ISO/IEC
    /// 14496-14 §3.1.2).
    const SL_CONFIG_PREDEFINED_MP4: u8 = 2;
    /// The `stpp` namespace a TTML track carries.
    const TTML_NAMESPACE: &str = "http://www.w3.org/ns/ttml";
    /// Segment duration published to the trunk.
    const SEGMENT_DURATION: Duration = Duration::from_secs(2);
    /// [`SEGMENT_DURATION`] in nanoseconds, for the trunk's timeline position.
    const SEGMENT_NANOS: u64 = 2_000_000_000;

    /// An `avcC` box from the real SPS/PPS pair above.
    fn avc_config() -> AVCConfigurationBox {
        avc_config_from_sprop(SPROP).expect("valid sprop")
    }

    /// A video track (id `1`) whose `CodecConfig` reports **no** geometry — the
    /// shape RTP/WHEP ingest builds before the SPS is known.
    fn video_spec() -> TrackSpec {
        TrackSpec::new(
            TrackId::new(1),
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config: avc_config(),
                width: 0,
                height: 0,
            },
        )
    }

    /// An `esds` carrying `dsi` as its DecoderSpecificInfo, or none when `dsi`
    /// is `None`.
    fn esds(dsi: Option<Vec<u8>>) -> EsdsBox {
        let decoder = DecoderConfigDescriptor::new(
            0x40,
            0x15,
            false,
            0,
            0,
            0,
            dsi.map(DecoderSpecificInfo::new),
        );
        EsdsBox::new(ESDescriptor::new(
            1,
            0,
            Some(decoder),
            Some(SLConfigDescriptor::new(vec![SL_CONFIG_PREDEFINED_MP4])),
        ))
    }

    /// An AAC track with the given id.
    fn audio_spec(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            TrackId::new(track_id),
            AUDIO_TIMESCALE,
            CodecConfig::Aac {
                esds: esds(Some(ASC_48K_STEREO.to_vec())),
                sample_rate: SAMPLE_RATE,
                channel_count: CHANNELS,
                sample_size: AAC_BITS_PER_SAMPLE,
            },
        )
    }

    /// A TTML (`stpp`) subtitle track with the given id.
    fn subtitle_spec(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            TrackId::new(track_id),
            1000,
            CodecConfig::Subtitle {
                format: transmux::ir::SubtitleFormat::Ttml,
                sample_entry: Some(transmux::pipeline::SubtitleSampleEntry::Ttml(
                    transmux::subtitle_entries::XmlSubtitleSampleEntry::new(TTML_NAMESPACE),
                )),
            },
        )
    }

    fn trunk() -> Arc<Trunk> {
        Trunk::new(TrunkConfig::new(
            NonZeroUsize::new(8).expect("non-zero"),
            NonZeroUsize::new(8).expect("non-zero"),
            NonZeroUsize::new(8).expect("non-zero"),
            NonZeroUsize::new(8).expect("non-zero"),
            NonZeroUsize::new(8).expect("non-zero"),
        ))
    }

    /// A trunk whose segment log holds every test segment.
    fn big_trunk() -> Arc<Trunk> {
        Trunk::new(TrunkConfig::new(
            NonZeroUsize::new(64).expect("non-zero"),
            NonZeroUsize::new(64).expect("non-zero"),
            NonZeroUsize::new(64).expect("non-zero"),
            NonZeroUsize::new(64).expect("non-zero"),
            NonZeroUsize::new(64).expect("non-zero"),
        ))
    }

    /// Everything a published route holds: the trunk and its init segment.
    struct Route {
        trunk: Arc<Trunk>,
        init: Vec<u8>,
    }

    /// Publish `specs` through a real LL-HLS segmenter: `SEGMENTS` two-second
    /// segments of video and/or audio, and one subtitle sample per video frame.
    fn publish(specs: Vec<TrackSpec>) -> Route {
        let trunk = big_trunk();
        let mut segmenter =
            LlHlsSegmenter::with_part_target(specs.clone(), transmux::VIDEO_CLOCK_RATE, 2.0, 500)
                .expect("segmenter builds");
        let truncated = trunk.writer().expect("fresh trunk");
        truncated.set_tracks(specs.clone());
        let writer = trunk.segment_writer().expect("fresh trunk");
        let init = segmenter.init_segment().expect("init segment");
        let mut published = 0u32;
        for index in 0..SEGMENTS * FRAMES_PER_SEGMENT {
            let dts = i64::from(index) * i64::from(FRAME_DUR);
            for spec in &specs {
                let sample = match spec.config {
                    CodecConfig::Avc { .. } => Sample::new(
                        vec![0xAA_u8.wrapping_add((index % 251) as u8); 64],
                        Some(dts),
                        Some(dts),
                        Some(FRAME_DUR),
                        index % FRAMES_PER_SEGMENT == 0,
                    ),
                    CodecConfig::Aac { .. } => {
                        let dts = i64::from(index) * i64::from(AAC_FRAME_DUR);
                        Sample::new(
                            vec![0x5A; 32],
                            Some(dts),
                            Some(dts),
                            Some(AAC_FRAME_DUR),
                            true,
                        )
                    }
                    _ => Sample::new(vec![0x54; 8], Some(dts), Some(dts), Some(FRAME_DUR), true),
                };
                segmenter.push(spec.track_id, sample).expect("push");
            }
            for segment in segmenter.take_ready_segments() {
                publish_segment(&writer, segment, &mut published);
            }
        }
        segmenter.flush().expect("flush");
        for segment in segmenter.take_ready_segments() {
            publish_segment(&writer, segment, &mut published);
        }
        Route { trunk, init }
    }

    /// Publish one closed segment to the trunk, numbered in publication order.
    fn publish_segment(
        writer: &media_plane::trunk::SegmentWriter,
        segment: transmux::ll_hls::SegmentInfo,
        published: &mut u32,
    ) {
        writer
            .publish_segment(SegmentEntry::new(
                segment.bytes,
                SegmentSeq::new(*published + 1),
                SEGMENT_DURATION,
                Timestamp::from_nanos(u64::from(*published) * SEGMENT_NANOS),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish segment");
        *published += 1;
    }

    /// The manifest a fresh origin renders for `route`.
    fn manifest(route: &Route) -> SmoothManifest {
        let origin = origin(route);
        let response = origin.manifest(
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(u64::MAX)),
        );
        match response {
            EgressResponse::Ready { body, .. } => {
                SmoothManifest::parse(&body).expect("manifest parses")
            }
            other => panic!("expected a rendered manifest, got {other:?}"),
        }
    }

    /// An origin over `route` with its init segment stored.
    fn origin(route: &Route) -> SmoothOrigin {
        let origin = SmoothOrigin::new(Arc::clone(&route.trunk));
        origin.set_init(route.init.clone());
        origin
    }

    /// The advertised streams' keys, in manifest order.
    fn keys(route: &Route) -> Vec<StreamKey> {
        let origin = origin(route);
        origin
            .fragments()
            .expect("fragments")
            .iter()
            .fold(Vec::new(), |mut acc, fragment| {
                if !acc.contains(&fragment.stream) {
                    acc.push(fragment.stream);
                }
                acc
            })
    }

    /// An empty trunk answers `Await` until the caller's policy expires, then
    /// `NotFound` — never a fabricated manifest.
    #[test]
    fn an_empty_origin_waits_then_gives_up() {
        let origin = SmoothOrigin::new(trunk());
        let policy = AwaitPolicy::new(Timestamp::from_nanos(10));
        assert!(matches!(
            origin.manifest(Timestamp::from_nanos(0), policy),
            EgressResponse::Await { .. }
        ));
        assert!(matches!(
            origin.manifest(Timestamp::from_nanos(10), policy),
            EgressResponse::NotFound
        ));
    }

    /// The manifest is the live model's own document: one `StreamIndex` per
    /// track, each with its own chunk timeline, and a duration covering the
    /// window.
    #[test]
    fn the_manifest_describes_every_track() {
        let route = publish(vec![video_spec(), audio_spec(2)]);
        let manifest = manifest(&route);
        assert!(manifest.is_live);
        assert_eq!(manifest.timescale, transmux::SMOOTH_TIMESCALE);
        let kinds: Vec<StreamType> = manifest.streams.iter().map(|s| s.stream_type).collect();
        assert_eq!(kinds, vec![StreamType::Video, StreamType::Audio]);
        for stream in &manifest.streams {
            let chunks = stream.enumerate_chunks().expect("chunks");
            assert_eq!(chunks.len(), SEGMENTS as usize);
            assert!(chunks.iter().all(|(_, d)| *d > 0));
        }
        let duration = manifest.duration.expect("duration");
        assert!(duration > 0, "{duration}");
        assert_eq!(manifest.dvr_window_length, Some(duration));
    }

    /// Two AAC tracks are two streams: distinct `Name` and `Url`, the second
    /// named `audio_2` (MS-SSTR §2.2.2.3).
    #[test]
    fn two_tracks_of_one_kind_get_distinct_names_and_urls() {
        let route = publish(vec![audio_spec(1), audio_spec(2)]);
        let manifest = manifest(&route);
        assert_eq!(
            manifest.streams.len(),
            2,
            "both audio tracks are advertised"
        );
        let first = &manifest.streams[0];
        let second = &manifest.streams[1];
        assert_eq!(first.name, None, "the first stream of a kind is unnamed");
        assert_eq!(second.name.as_deref(), Some("audio_2"));
        assert_ne!(first.url, second.url);
        assert!(second.url.contains("Fragments(audio_2={start time})"));
        for stream in &manifest.streams {
            assert_eq!(stream.quality_levels, Some(STREAM_QUALITY_LEVELS));
        }
    }

    /// A fragment request for the second audio stream resolves to that
    /// stream's own fragment, whose `tfxd` is the `c@t` the manifest gives it.
    #[test]
    fn the_second_stream_serves_its_own_fragments() {
        let route = publish(vec![audio_spec(1), audio_spec(2)]);
        let origin = SmoothOrigin::new(Arc::clone(&route.trunk));
        origin.set_init(route.init.clone());
        let manifest = manifest(&route);
        let second = &manifest.streams[1];
        let url = second.resolve_fragment_url(second.qualities[0].bitrate, 0);
        let path = format!("/cam/{url}");
        let request = SmoothRequest::parse(&path, &keys(&route))
            .unwrap_or_else(|| panic!("request parses: {path} from {:?}", keys(&route)));
        let response = origin.resolve(
            request,
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(u64::MAX)),
        );
        let EgressResponse::Ready {
            body: SmoothBody::Fragment(bytes),
            ..
        } = response
        else {
            panic!("expected fragment bytes from {path}, got {response:?}");
        };
        let tfxd = fragment_tfxd(&bytes).expect("fragment carries a tfxd");
        assert_eq!(tfxd.fragment_absolute_time, 0);
        assert!(tfxd.fragment_duration > 0);
    }

    /// An AAC track whose `esds` carries no DecoderSpecificInfo has no
    /// `CodecPrivateData` a client could initialise from, so the manifest is
    /// refused rather than advertising an empty value.
    #[test]
    fn aac_without_decoder_specific_info_is_refused() {
        let mut spec = audio_spec(2);
        spec.config = CodecConfig::Aac {
            esds: esds(None),
            sample_rate: SAMPLE_RATE,
            channel_count: CHANNELS,
            sample_size: AAC_BITS_PER_SAMPLE,
        };
        let route = publish(vec![video_spec(), spec]);
        let origin = origin(&route);
        let response = origin.manifest(
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(u64::MAX)),
        );
        assert!(
            matches!(response, EgressResponse::BadRequest { .. }),
            "expected a refusal, got {response:?}"
        );
    }

    /// Video geometry comes from the SPS, not from a `CodecConfig` that
    /// reports none (RTSP/WHEP ingest builds it with width and height `0`).
    #[test]
    fn video_geometry_comes_from_the_sps() {
        let route = publish(vec![video_spec()]);
        let manifest = manifest(&route);
        let level = &manifest.streams[0].qualities[0];
        assert_eq!(level.width, Some(SPROP_WIDTH));
        assert_eq!(level.height, Some(SPROP_HEIGHT));
        assert_eq!(level.four_cc, transmux::FOURCC_H264);
        assert!(!level.codec_private_data.is_empty());
    }

    /// AAC's `SamplingRate` is the AudioSpecificConfig's own effective rate.
    #[test]
    fn aac_advertises_its_audio_specific_config() {
        let route = publish(vec![audio_spec(1)]);
        let manifest = manifest(&route);
        let level = &manifest.streams[0].qualities[0];
        assert_eq!(level.sampling_rate, Some(SAMPLE_RATE));
        assert_eq!(level.channels, Some(CHANNELS));
        assert_eq!(level.audio_tag, Some(AUDIO_TAG_AAC));
        let asc = <AudioSpecificConfig as broadcast_common::Parse>::parse(&ASC_48K_STEREO)
            .expect("asc parses");
        assert_eq!(asc.effective_sampling_frequency(), Some(SAMPLE_RATE));
        assert_eq!(level.codec_private_data, ASC_48K_STEREO.to_vec());
    }

    /// A subtitle track is advertised as a `text` stream with the required
    /// `Subtype="SUBT"` and its sample-entry FourCC.
    #[test]
    fn a_subtitle_track_is_advertised() {
        let route = publish(vec![video_spec(), subtitle_spec(2)]);
        let manifest = manifest(&route);
        let text = manifest
            .streams
            .iter()
            .find(|s| s.stream_type == StreamType::Text)
            .expect("the subtitle track is advertised");
        assert_eq!(text.subtype.as_deref(), Some(SUBTYPE_SUBT));
        assert_eq!(text.qualities[0].four_cc, transmux::smooth::FOURCC_STPP);
        assert!(!text.enumerate_chunks().expect("chunks").is_empty());
    }

    /// The manifest round-trips through the parser byte-for-byte, so what a
    /// client reads is what `broadcast-smooth` wrote.
    #[test]
    fn the_rendered_manifest_round_trips() {
        let route = publish(vec![video_spec(), audio_spec(2)]);
        let origin = origin(&route);
        let EgressResponse::Ready { body, .. } = origin.manifest(
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(u64::MAX)),
        ) else {
            panic!("manifest renders");
        };
        let parsed = SmoothManifest::parse(&body).expect("manifest parses");
        assert_eq!(parsed.to_xml(), body);
    }

    /// A fragment path names its stream by the manifest's own `Name`
    /// (`TYPE` for the first, `TYPE_N` for the rest); an unknown one, a
    /// malformed one and a manifest name all parse as expected.
    #[test]
    fn requests_parse_from_their_paths() {
        let kinds = [
            StreamKey {
                kind: SmoothStreamType::Video,
                ordinal: 1,
            },
            StreamKey {
                kind: SmoothStreamType::Audio,
                ordinal: 2,
            },
        ];
        assert_eq!(
            SmoothRequest::parse("/cam/Manifest", &kinds),
            Some(SmoothRequest::Manifest)
        );
        assert_eq!(
            SmoothRequest::parse(
                "/cam/QualityLevels(128000)/Fragments(audio_2=21333333)",
                &kinds
            ),
            Some(SmoothRequest::Fragment {
                stream: kinds[1],
                bitrate: 128_000,
                start: 21_333_333,
            })
        );
        assert_eq!(
            SmoothRequest::parse("QualityLevels(2500000)/Fragments(video=0)", &kinds),
            Some(SmoothRequest::Fragment {
                stream: kinds[0],
                bitrate: 2_500_000,
                start: 0,
            })
        );
        assert_eq!(SmoothRequest::parse("/cam/nope", &kinds), None);
        assert_eq!(
            SmoothRequest::parse("/cam/QualityLevels(1)/Fragments(text=0)", &kinds),
            None,
            "a kind the manifest does not advertise is not a request"
        );
    }

    /// A stream key's name and URL template agree: the template's `Fragments`
    /// token carries exactly the name the `c` timeline is advertised under.
    #[test]
    fn a_stream_keys_url_carries_its_name() {
        let first = StreamKey {
            kind: SmoothStreamType::Video,
            ordinal: 1,
        };
        let second = StreamKey {
            kind: SmoothStreamType::Video,
            ordinal: 2,
        };
        assert_eq!(first.name(), "video");
        assert_eq!(second.name(), "video_2");
        assert!(
            second
                .url_template()
                .contains("Fragments(video_2={start time})")
        );
        assert_eq!(
            first.url_template(),
            "QualityLevels({bitrate})/Fragments(video={start time})"
        );
    }
}
