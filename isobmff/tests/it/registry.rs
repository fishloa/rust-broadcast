//! The private-box registry end to end: a sample entry and a `udta` atom this
//! crate does not type come back typed through [`BoxRegistry`], come back
//! opaque without one, and the file round-trips byte-identically either way.

use broadcast_common::timescale::Timescale;
use broadcast_common::{Parse, Serialize};

use isobmff::box_support::{ChildOrder, OpaqueBox};
use isobmff::box_types::{BOX_HEADER_MIN_SIZE, write_box_header, write_full_box_header};
use isobmff::init_segment::MovieHeaderBox;
use isobmff::plan::{MovieShell, PlanBuilder, PlanSample};
use isobmff::registry::{BoxDef, BoxRegistry, RegisteredEntry};
use isobmff::writer::{
    FileType, Interleave, Layout, MovieHeader, SampleEntry, Sink, TrackSetup, Writer,
};
use isobmff::{
    Error, Limits, Reader, ReaderConfig, SampleOrder, Step, TrackId, TrackInfo, parse_box,
};

const ACME_SAMPLE_ENTRY: [u8; 4] = *b"actm";
const ACME_CALIBRATION: [u8; 4] = *b"acal";
const ACME_HANDLER: [u8; 4] = *b"meta";
const ENTRY_PREFIX: usize = 8;
const TIMESCALE: u32 = 1_000;
const SAMPLE_COUNT: u32 = 4;
const SAMPLE_BYTES: u32 = 32;
const MOVIE_DURATION: u64 = 4_000;
const IDENTITY_MATRIX: [i32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
const MVHD_RATE_UNITY: u32 = 0x0001_0000;
const MVHD_VOLUME_UNITY: u16 = 0x0100;
const CALIBRATION_HEADER: usize = 4 + 4 + 4 + 1;

/// A private FullBox: `version`/`flags`, two fixed-point values and a counted
/// label.
#[derive(Debug, Clone, PartialEq)]
struct AcmeCalibrationBox {
    gain: i32,
    shift: i32,
    label: Vec<u8>,
}

impl<'a> Parse<'a> for AcmeCalibrationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let body = parse_box(bytes)?.0.body;
        let rest = body
            .get(4..)
            .ok_or(Error::InvalidInput("short acal body"))?;
        let field = |at: usize| -> Result<i32, Error> {
            let slice = rest
                .get(at..at + 4)
                .ok_or(Error::InvalidInput("short acal field"))?;
            let array: [u8; 4] = slice
                .try_into()
                .map_err(|_| Error::InvalidInput("acal field width"))?;
            Ok(i32::from_be_bytes(array))
        };
        let gain = field(0)?;
        let shift = field(4)?;
        let len = usize::from(*rest.get(8).ok_or(Error::InvalidInput("acal label len"))?);
        let label = rest
            .get(9..9 + len)
            .ok_or(Error::InvalidInput("short acal label"))?
            .to_vec();
        Ok(Self { gain, shift, label })
    }
}

impl Serialize for AcmeCalibrationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + CALIBRATION_HEADER + self.label.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let need = self.serialized_len();
        let mut written = write_full_box_header(buf, &ACME_CALIBRATION, need, 0, 0, false)?;
        buf[written..written + 4].copy_from_slice(&self.gain.to_be_bytes());
        written += 4;
        buf[written..written + 4].copy_from_slice(&self.shift.to_be_bytes());
        written += 4;
        buf[written] = broadcast_common::len::fit_u8(self.label.len(), "acal label")?;
        written += 1;
        buf[written..written + self.label.len()].copy_from_slice(&self.label);
        written += self.label.len();
        Ok(written)
    }
}

impl BoxDef for AcmeCalibrationBox {
    const FOURCC: [u8; 4] = ACME_CALIBRATION;
    const PARENT: Option<[u8; 4]> = Some(*b"udta");
}

/// A private sample entry: its reserved/`data_reference_index` prefix and one
/// payload word.
#[derive(Debug, Clone, PartialEq)]
struct AcmeTelemetryEntry {
    serial: u32,
}

impl<'a> Parse<'a> for AcmeTelemetryEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        let body = parse_box(bytes)?.0.body;
        let slice = body
            .get(ENTRY_PREFIX..ENTRY_PREFIX + 4)
            .ok_or(Error::InvalidInput("short actm body"))?;
        let array: [u8; 4] = slice
            .try_into()
            .map_err(|_| Error::InvalidInput("actm field width"))?;
        Ok(Self {
            serial: u32::from_be_bytes(array),
        })
    }
}

impl Serialize for AcmeTelemetryEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + ENTRY_PREFIX + 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let need = self.serialized_len();
        let mut written = write_box_header(buf, &ACME_SAMPLE_ENTRY, need)?;
        buf[written..written + ENTRY_PREFIX].fill(0);
        buf[written + 1] = 1;
        written += ENTRY_PREFIX;
        buf[written..written + 4].copy_from_slice(&self.serial.to_be_bytes());
        written += 4;
        Ok(written)
    }
}

impl BoxDef for AcmeTelemetryEntry {
    const FOURCC: [u8; 4] = ACME_SAMPLE_ENTRY;
    const PARENT: Option<[u8; 4]> = Some(*b"stsd");
}

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

/// The registered parse of one hand-built box, as the writer takes it.
fn registered<T: BoxDef>(value: &T) -> RegisteredEntry {
    let bytes = value.try_to_bytes().expect("box bytes");
    let mut registry = BoxRegistry::new();
    registry.register::<T>().expect("register");
    registry
        .parse(T::PARENT, &bytes)
        .expect("parse")
        .expect("registered")
}

/// The movie shell carrying one `udta`/`acal` box, in the conventional
/// `mvhd`, `trak`, `udta` order.
fn shell() -> MovieShell {
    let calibration = AcmeCalibrationBox {
        gain: 0x0001_8000,
        shift: -2,
        label: b"acme".to_vec(),
    };
    let calibration = calibration.try_to_bytes().expect("acal");
    let mut order = ChildOrder::default();
    order.push(*b"mvhd");
    order.push(*b"trak");
    order.push_opaque(*b"udta");
    MovieShell {
        mvhd: MovieHeaderBox {
            version: 0,
            flags: 0,
            creation_time: 0,
            modification_time: 0,
            timescale: timescale(TIMESCALE),
            duration: MOVIE_DURATION,
            rate: MVHD_RATE_UNITY,
            volume: MVHD_VOLUME_UNITY,
            matrix: IDENTITY_MATRIX,
            next_track_id: TrackId::new(2),
        },
        mvex: None,
        opaque: vec![OpaqueBox::new(*b"udta", calibration)],
        order,
        top_level: Vec::new(),
    }
}

/// One reader run: its tracks, its samples in file order, and its plan.
fn read_all(
    bytes: &[u8],
    registry: Option<BoxRegistry>,
) -> (
    Vec<TrackInfo>,
    Vec<isobmff::SampleRef>,
    Option<isobmff::plan::OwnedPlan>,
) {
    let mut config = ReaderConfig::default()
        .with_total_len(Some(u64::try_from(bytes.len()).expect("length")))
        .with_order(SampleOrder::File);
    if let Some(registry) = registry {
        config = config.with_registry(registry);
    }
    let mut reader = Reader::new(config);
    let mut tracks = Vec::new();
    let mut samples = Vec::new();
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end").min(bytes.len());
                reader
                    .supply(range.offset, &bytes[start..end])
                    .expect("supply");
            }
            Step::Tracks(step_tracks) => tracks = step_tracks.to_vec(),
            Step::Sample(sample) => samples.push(sample),
            Step::Done => break,
            _ => {}
        }
    }
    let plan = reader.plan().and_then(|plan| plan.to_owned_plan().ok());
    (tracks, samples, plan)
}

fn place(out: &mut [u8], write: &isobmff::writer::Write<'_>) {
    let start = usize::try_from(write.offset).expect("offset");
    let end = start + write.bytes.len();
    out[start..end].copy_from_slice(&write.bytes);
}

/// Build and write the whole file, with one private track and one `udta` atom.
fn build_file(entry: SampleEntry) -> Vec<u8> {
    let setup = TrackSetup::new(
        TrackId::new(1),
        ACME_HANDLER,
        b"acme telemetry".to_vec(),
        timescale(TIMESCALE),
        entry,
    );
    let mut builder = PlanBuilder::new(vec![setup], Limits::default());
    for _ in 0..SAMPLE_COUNT {
        builder
            .push(TrackId::new(1), PlanSample::new(TIMESCALE, SAMPLE_BYTES))
            .expect("push");
    }
    builder
        .set_durations(TrackId::new(1), MOVIE_DURATION, MOVIE_DURATION)
        .expect("durations");
    let plan = builder.finish().expect("plan").with_shell(shell());
    let layout = Layout::new(true, Interleave::ByDuration { millis: 500 }, Sink::Seekable);
    let mut writer = Writer::progressive(
        plan,
        layout,
        FileType::default(),
        MovieHeader::with_times(timescale(TIMESCALE), 0, 0),
        Limits::default(),
    )
    .expect("writer");
    let mut out = vec![0u8; usize::try_from(writer.file_len()).expect("length")];
    for write in writer.header() {
        place(&mut out, &write);
    }
    let data = vec![0xABu8; usize::try_from(SAMPLE_BYTES).expect("sample size")];
    for index in 0..SAMPLE_COUNT {
        for write in writer.push(TrackId::new(1), index, &data).expect("push") {
            place(&mut out, &write);
        }
    }
    for write in writer.finish().expect("finish") {
        place(&mut out, &write);
    }
    out
}

/// The `udta` payload inside the file's `moov`.
fn udta_payload(bytes: &[u8]) -> Vec<u8> {
    let moov = isobmff::box_types::find_box(bytes, b"moov").expect("moov");
    let body = parse_box(moov).expect("moov box").0.body;
    isobmff::box_types::find_box(body, b"udta")
        .map(|udta| parse_box(udta).expect("udta box").0.payload().to_vec())
        .expect("udta")
}

fn acme_entry(serial: u32) -> AcmeTelemetryEntry {
    AcmeTelemetryEntry { serial }
}

fn acme_registry() -> BoxRegistry {
    let mut registry = BoxRegistry::new();
    registry
        .register::<AcmeTelemetryEntry>()
        .expect("register telemetry");
    registry
        .register::<AcmeCalibrationBox>()
        .expect("register calibration");
    registry
}

#[test]
fn a_private_sample_entry_comes_back_typed_through_the_registry() {
    let entry = registered(&acme_entry(7));
    let sample_entry = SampleEntry::from_registered(entry.as_registered()).expect("entry");
    assert!(matches!(sample_entry, SampleEntry::Raw { fourcc, .. } if fourcc == ACME_SAMPLE_ENTRY));
    let file = build_file(sample_entry);

    let (tracks, samples, _) = read_all(&file, Some(acme_registry()));
    assert_eq!(tracks.len(), 1, "one track");
    assert_eq!(samples.len(), usize::try_from(SAMPLE_COUNT).expect("count"));
    let entry = tracks[0].entries.first().expect("a sample entry");
    assert_eq!(entry.fourcc, ACME_SAMPLE_ENTRY);
    assert!(entry.typed.is_none(), "not a four-CC this crate types");
    let parsed = entry
        .registered
        .as_ref()
        .expect("registered parse")
        .downcast_ref::<AcmeTelemetryEntry>()
        .expect("concrete type");
    assert_eq!(parsed, &acme_entry(7));

    let udta = udta_payload(&file);
    let calibration: Vec<_> = acme_registry()
        .parse_children(*b"udta", &udta)
        .collect::<Result<Vec<_>, Error>>()
        .expect("walk udta");
    assert_eq!(calibration.len(), 1);
    let parsed = calibration[0]
        .registered
        .as_ref()
        .expect("registered parse")
        .downcast_ref::<AcmeCalibrationBox>()
        .expect("concrete type");
    assert_eq!(
        parsed,
        &AcmeCalibrationBox {
            gain: 0x0001_8000,
            shift: -2,
            label: b"acme".to_vec(),
        }
    );
}

#[test]
fn without_a_registry_the_entry_and_the_atom_stay_opaque() {
    let entry = registered(&acme_entry(9));
    let sample_entry = SampleEntry::from_registered(entry.as_registered()).expect("entry");
    let file = build_file(sample_entry);

    let (tracks, samples, _) = read_all(&file, None);
    assert_eq!(tracks.len(), 1);
    assert_eq!(samples.len(), usize::try_from(SAMPLE_COUNT).expect("count"));
    let entry = tracks[0].entries.first().expect("a sample entry");
    assert_eq!(entry.fourcc, ACME_SAMPLE_ENTRY);
    assert!(entry.typed.is_none());
    assert!(entry.registered.is_none());

    let udta = udta_payload(&file);
    let children: Vec<_> = acme_registry()
        .parse_children(*b"udta", &udta)
        .collect::<Result<Vec<_>, Error>>()
        .expect("walk udta");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].fourcc, ACME_CALIBRATION);
    assert_eq!(children[0].bytes, udta.as_slice(), "the atom's bytes");
}

#[test]
fn the_registry_does_not_change_what_the_writer_emits() {
    let entry = registered(&acme_entry(11));
    let sample_entry = SampleEntry::from_registered(entry.as_registered()).expect("entry");
    let file = build_file(sample_entry);

    let (tracks_with, samples, plan) = read_all(&file, Some(acme_registry()));
    let (tracks_without, _, _) = read_all(&file, None);
    let without = |tracks: &mut Vec<TrackInfo>| {
        for track in tracks {
            for entry in &mut track.entries {
                entry.registered = None;
            }
        }
    };
    let mut observed = tracks_with;
    without(&mut observed);
    assert_eq!(observed, tracks_without, "the registry adds no field");

    let plan = plan.expect("plan");
    let layout = Layout::new(true, Interleave::ByDuration { millis: 500 }, Sink::Seekable);
    let mut writer = Writer::progressive(
        plan,
        layout,
        FileType::default(),
        MovieHeader::with_times(timescale(TIMESCALE), 0, 0),
        Limits::default(),
    )
    .expect("writer");
    let mut out = vec![0u8; usize::try_from(writer.file_len()).expect("length")];
    for write in writer.header() {
        place(&mut out, &write);
    }
    for sample in &samples {
        let start = usize::try_from(sample.offset).expect("offset");
        let end = start + usize::try_from(sample.size).expect("size");
        for write in writer
            .push(sample.track, sample.index, &file[start..end])
            .expect("push")
        {
            place(&mut out, &write);
        }
    }
    for write in writer.finish().expect("finish") {
        place(&mut out, &write);
    }
    assert_eq!(out, file, "byte-identical round trip");
}
