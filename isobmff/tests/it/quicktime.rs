//! QuickTime `.mov` fixtures read completely: version 1/2 sound descriptions,
//! `wave`, `gmhd`, timecode, apertures and the `qt  ` brand.
//!
//! Fixtures live in `private/fixtures/mp4/quicktime/` and `camera/`; they are
//! loaded only through `test_fixtures`.

use broadcast_core::{Parse, Serialize};
use isobmff::MovieBox;
use isobmff::init_segment::{SampleEntryVariant, TrackBox};
use isobmff::metadata::{ItemKey, MovieMetadata};
use isobmff::quicktime::{
    GmhdAtom, GminAtom, TcmiAtom, TimecodeEntry, TrackApertureModeDimensionsBox, WaveAtom,
    timecode_from_sample,
};
use isobmff::reader::{ByteRange, Reader, ReaderConfig, SampleOrder, Step, TrackInfo, Warning};
use isobmff::sample_table::SampleRef;
use isobmff::segments::FileTypeBox;

struct Drive {
    tracks: Vec<TrackInfo>,
    samples: Vec<SampleRef>,
    metadata: Option<MovieMetadata>,
    brand: Option<FileTypeBox>,
    warnings: Vec<Warning>,
}

fn drive(data: &[u8], config: ReaderConfig) -> isobmff::Result<Drive> {
    let mut reader = Reader::new(config);
    let mut drive = Drive {
        tracks: Vec::new(),
        samples: Vec::new(),
        metadata: None,
        brand: None,
        warnings: Vec::new(),
    };
    loop {
        match reader.step()? {
            Step::Need(ByteRange { offset, len }) => {
                let start = usize::try_from(offset).expect("offset");
                let end = usize::try_from(offset + len).expect("end").min(data.len());
                reader.supply(offset, &data[start..end])?;
            }
            Step::Tracks(tracks) => drive.tracks = tracks.to_vec(),
            Step::Metadata(metadata) => drive.metadata = Some(metadata.clone()),
            Step::Sample(sample) => drive.samples.push(sample),
            Step::Fragment(_) => {}
            Step::Warning(warning) => drive.warnings.push(warning),
            Step::Done => break,
            _ => {}
        }
    }
    drive.brand = reader.brand().cloned();
    Ok(drive)
}

fn fixture(rel: &str) -> Vec<u8> {
    let path = test_fixtures::require(&format!("mp4/{rel}"), "quicktime").expect("fixture");
    std::fs::read(&path).expect("fixture bytes")
}

fn entry<'a>(track: &'a TrackInfo, fourcc: &[u8; 4]) -> &'a SampleEntryVariant {
    track
        .entries
        .iter()
        .find(|entry| &entry.fourcc == fourcc)
        .and_then(|entry| entry.typed.as_ref())
        .unwrap_or_else(|| panic!("{fourcc:?} entry"))
}

fn moov_tracks(data: &[u8]) -> Vec<TrackBox> {
    let at = find_top(data, b"moov").expect("moov");
    MovieBox::parse(&data[at..]).expect("moov parse").tracks
}

fn find_top(data: &[u8], fourcc: &[u8; 4]) -> Option<usize> {
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size = usize::try_from(u32::from_be_bytes([
            data[at],
            data[at + 1],
            data[at + 2],
            data[at + 3],
        ]))
        .ok()?;
        if size < 8 {
            return None;
        }
        if &data[at + 4..at + 8] == fourcc {
            return Some(at);
        }
        at = at.checked_add(size)?;
    }
    None
}

/// `ffprobe -v error -select_streams a:0 -show_entries stream=extradata
/// -show_data /private/fixtures/mp4/quicktime/lavf-aac-tmcd.mov` prints
/// `1188 56e5 00`.
const LAVF_AAC_ASC: [u8; 5] = [0x11, 0x88, 0x56, 0xe5, 0x00];
/// `ffprobe ... -show_entries stream=nb_frames` on the PCM fixture.
const LAVF_PCM_SAMPLES: u32 = 96000;

#[test]
fn a_lavf_quicktime_aac_track_is_a_version_1_wave_description() {
    let data = fixture("quicktime/lavf-aac-tmcd.mov");
    let drive = drive(&data, ReaderConfig::default().with_order(SampleOrder::File)).expect("read");
    assert_eq!(drive.tracks.len(), 3);
    assert_eq!(drive.warnings, Vec::new());
    assert_eq!(drive.brand.expect("ftyp").major_brand, *b"qt  ");

    let audio = drive
        .tracks
        .iter()
        .find(|track| track.handler == *b"soun")
        .expect("audio track");
    let SampleEntryVariant::Mp4a(mp4a) = entry(audio, b"mp4a") else {
        panic!("mp4a entry");
    };
    assert_eq!(
        mp4a.entry_version, 1,
        "QuickTime version 1 sound description"
    );
    let Some(isobmff::quicktime::QuickTimeSound::V1(v1)) = mp4a.quicktime else {
        panic!("version 1 extension");
    };
    assert_eq!(v1.samples_per_packet, 1024);
    assert_eq!(v1.bytes_per_sample, 2);
    let wave = mp4a.wave().expect("wave atom");
    assert_eq!(wave.format, *b"mp4a");
    let esds = mp4a.esds().expect("esds inside wave");
    assert_eq!(
        esds.decoder_specific_info_data(),
        Some(LAVF_AAC_ASC.as_slice())
    );

    let timecode = drive
        .tracks
        .iter()
        .find(|track| track.handler == *b"tmcd")
        .expect("timecode track");
    let SampleEntryVariant::Tmcd(tmcd) = entry(timecode, b"tmcd") else {
        panic!("tmcd entry");
    };
    assert_timecode_entry(tmcd);
    let sample = drive
        .samples
        .iter()
        .find(|sample| sample.track == timecode.track_id)
        .expect("timecode sample");
    let start = usize::try_from(sample.offset).expect("offset");
    let end = start + usize::try_from(sample.size).expect("size");
    let timecode = timecode_from_sample(tmcd, &data[start..end]).expect("decode");
    assert_eq!(
        (
            timecode.hours,
            timecode.minutes,
            timecode.seconds,
            timecode.frames
        ),
        (1, 0, 0, 0)
    );
    assert!(!timecode.drop_frame);
}

fn assert_timecode_entry(tmcd: &TimecodeEntry) {
    assert_eq!(tmcd.flags, 0);
    assert_eq!(tmcd.timescale, 12800);
    assert_eq!(tmcd.frame_duration, 512);
    assert_eq!(tmcd.number_of_frames, 25);
}

#[test]
fn a_quicktime_pcm24_track_is_a_version_1_in24_entry() {
    let data = fixture("quicktime/lavf-pcm24.mov");
    let drive = drive(&data, ReaderConfig::default()).expect("read");
    assert_eq!(drive.tracks.len(), 1);
    let SampleEntryVariant::QuickTimeSound(sound) = entry(&drive.tracks[0], b"in24") else {
        panic!("in24 entry");
    };
    assert_eq!(sound.entry_version, 1);
    let Some(isobmff::quicktime::QuickTimeSound::V1(v1)) = sound.quicktime else {
        panic!("version 1 extension");
    };
    assert_eq!(v1.bytes_per_packet, 3);
    let wave = sound.wave().expect("wave atom");
    assert_eq!(wave.format, *b"in24");
    assert!(
        wave.others.iter().any(|box_| box_.box_type == *b"enda"),
        "the wave children are typed or kept opaque"
    );
    assert_eq!(
        u32::try_from(drive.samples.len()).expect("sample count"),
        LAVF_PCM_SAMPLES
    );
}

#[test]
fn an_iphone_live_photo_reads_every_track() {
    let data = fixture("quicktime/iphone-x-live-photo.mov");
    let drive = drive(&data, ReaderConfig::default()).expect("read");
    assert_eq!(drive.brand.expect("ftyp").major_brand, *b"qt  ");
    let fourccs: Vec<[u8; 4]> = drive
        .tracks
        .iter()
        .map(|track| track.entries[0].fourcc)
        .collect();
    assert_eq!(
        fourccs,
        vec![*b"hvc1", *b"lpcm", *b"mebx", *b"mebx"],
        "every track is surfaced"
    );

    let video = &drive.tracks[0];
    let tapt = video.tapt.as_ref().expect("tapt");
    assert_ne!(tapt.clef.width, 0);
    assert_ne!(tapt.clef.height, 0);

    let SampleEntryVariant::QuickTimeSound(sound) = entry(&drive.tracks[1], b"lpcm") else {
        panic!("lpcm entry");
    };
    assert_eq!(sound.entry_version, 2);
    let Some(isobmff::quicktime::QuickTimeSound::V2(v2)) = sound.quicktime else {
        panic!("version 2 extension");
    };
    assert_eq!(v2.audio_sample_rate(), 44100.0);
    assert_eq!(v2.num_audio_channels, 1);

    let tracks = moov_tracks(&data);
    assert_eq!(tracks.len(), 4);
    for track in &tracks[2..] {
        let minf = track.mdia.as_ref().and_then(|mdia| mdia.minf.as_ref());
        assert!(minf.and_then(|minf| minf.gmhd()).is_some(), "gmhd typed");
    }

    let metadata = drive.metadata.expect("movie metadata");
    let location = ItemKey::Mdta(ItemKey::APPLE_LOCATION_ISO6709.to_string());
    assert!(
        metadata.movie.text(&location).is_some(),
        "the mdta location key reads"
    );

    for track in &drive.tracks[2..] {
        let keys = keys_of(&track.entries[0].body).expect("mebx keys table");
        assert!(!keys.entries.is_empty());
        assert!(keys.entries.iter().all(|entry| !entry.value.is_empty()));
    }
}

/// Parse the key table itself. Apple also stores per-key data-type boxes after
/// the table inside the same `keys` box, which are not table entries.
fn keys_table(payload: &[u8]) -> Option<isobmff::metadata::keys::Keys> {
    let count = usize::try_from(broadcast_core::wire::u32_at(payload, 4)).ok()?;
    let mut end = 8usize;
    for _ in 0..count {
        let size = usize::try_from(broadcast_core::wire::u32_at(payload, end)).ok()?;
        end = end.checked_add(size)?;
    }
    isobmff::metadata::keys::Keys::parse_body(payload.get(..end)?).ok()
}

fn keys_of(entry: &[u8]) -> Option<isobmff::metadata::keys::Keys> {
    let start = isobmff::box_types::BoxHeader::parse(entry)
        .ok()?
        .header_size()
        + 8;
    for item in isobmff::BoxIter::new(entry.get(start..)?) {
        let item = item.ok()?;
        if item.four_cc() == *b"keys" {
            return keys_table(item.payload());
        }
    }
    None
}

#[test]
fn a_camera_timecode_track_and_its_gmhd_are_typed() {
    let data = fixture("camera/gopro-karma.mp4");
    let drive = drive(&data, ReaderConfig::default()).expect("read");
    let timecode = drive
        .tracks
        .iter()
        .find(|track| track.handler == *b"tmcd")
        .expect("tmcd track");
    let SampleEntryVariant::Tmcd(tmcd) = entry(timecode, b"tmcd") else {
        panic!("tmcd entry");
    };
    assert_eq!(tmcd.flags, 2);
    assert_eq!(tmcd.timescale, 90000);
    assert_eq!(tmcd.frame_duration, 3003);
    assert_eq!(tmcd.number_of_frames, 29);

    let tracks = moov_tracks(&data);
    let track = tracks
        .iter()
        .find(|track| track.tkhd.track_id == timecode.track_id)
        .expect("tmcd trak");
    let gmhd = track
        .mdia
        .as_ref()
        .and_then(|mdia| mdia.minf.as_ref())
        .and_then(|minf| minf.gmhd())
        .expect("gmhd");
    assert!(gmhd.gmin.is_some());
}

#[test]
fn a_file_without_ftyp_still_reads() {
    let mut data = fixture("quicktime/lavf-aac-tmcd.mov");
    let at = find_top(&data, b"ftyp").expect("ftyp");
    data[at + 4..at + 8].copy_from_slice(b"free");
    let drive = drive(&data, ReaderConfig::default()).expect("read a brandless file");
    assert_eq!(drive.brand, None);
    assert_eq!(drive.tracks.len(), 3);
}

#[test]
fn a_chapter_title_sample_decodes() {
    // A text track sample: 16-bit length then the title bytes, optionally an
    // encoding atom this function ignores.
    let sample = b"\x00\x05Intro\x00\x00";
    assert_eq!(
        isobmff::quicktime::chapter_title(sample).expect("title"),
        "Intro"
    );
}

fn sample_timecode() -> TimecodeEntry {
    TimecodeEntry {
        reserved: [0; 6],
        data_reference_index: 1,
        reserved_1: 0,
        flags: 0,
        timescale: 12800,
        frame_duration: 512,
        number_of_frames: 25,
        reserved_tail: vec![0, 0, 0],
        name: None,
        others: Vec::new(),
    }
}

#[test]
fn a_typed_quicktime_entry_serialises_back_to_its_bytes() {
    let entry = sample_timecode();
    let tmcd = SampleEntryVariant::Tmcd(Box::new(entry.clone()));
    let bytes = tmcd.try_to_bytes().expect("writes");
    assert_eq!(&bytes[4..8], b"tmcd");
    assert_eq!(TimecodeEntry::parse(&bytes).expect("parses"), entry);
    assert_eq!(tmcd.serialized_len(), bytes.len());
}

/// Every quicktime box of `fourcc` in `data`, cut out by its own size field.
fn cut_boxes<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size = usize::try_from(u32::from_be_bytes([
            data[at],
            data[at + 1],
            data[at + 2],
            data[at + 3],
        ]))
        .unwrap_or(0);
        if size >= 8 && at + size <= data.len() && &data[at + 4..at + 8] == fourcc {
            out.push(&data[at..at + size]);
            at += size;
        } else {
            at += 1;
        }
    }
    out
}

const QUICKTIME_FIXTURES: &[&str] = &[
    "quicktime/lavf-aac-tmcd.mov",
    "quicktime/lavf-pcm24.mov",
    "quicktime/iphone-x-live-photo.mov",
    "camera/gopro-karma.mp4",
    "camera/gopro-hero6-ble.mp4",
    "camera/gopro-max-heromode.mp4",
];

#[test]
fn every_quicktime_atom_serialises_back_to_its_bytes() {
    for rel in QUICKTIME_FIXTURES {
        let data = fixture(rel);
        for box_bytes in cut_boxes(&data, b"gmhd") {
            let atom = GmhdAtom::parse_box(box_bytes).expect("gmhd");
            assert_eq!(atom.try_to_bytes().expect("gmhd bytes"), box_bytes, "{rel}");
        }
        for box_bytes in cut_boxes(&data, b"wave") {
            let atom = WaveAtom::parse_box(box_bytes).expect("wave");
            assert_eq!(atom.try_to_bytes().expect("wave bytes"), box_bytes, "{rel}");
        }
        for box_bytes in cut_boxes(&data, b"tapt") {
            let atom = TrackApertureModeDimensionsBox::parse_box(box_bytes).expect("tapt");
            assert_eq!(atom.try_to_bytes().expect("tapt bytes"), box_bytes, "{rel}");
        }
        for box_bytes in cut_boxes(&data, b"gmin") {
            let atom = GminAtom::parse(box_bytes).expect("gmin");
            assert_eq!(atom.try_to_bytes().expect("gmin bytes"), box_bytes, "{rel}");
        }
        for box_bytes in cut_boxes(&data, b"tcmi") {
            let atom = TcmiAtom::parse(box_bytes).expect("tcmi");
            assert_eq!(atom.try_to_bytes().expect("tcmi bytes"), box_bytes, "{rel}");
        }
    }
}

fn quicktime_entry(entry: &SampleEntryVariant) -> bool {
    match entry {
        SampleEntryVariant::Tmcd(_) | SampleEntryVariant::QuickTimeSound(_) => true,
        SampleEntryVariant::Mp4a(e) => e.quicktime.is_some(),
        SampleEntryVariant::Ac3(e) => e.quicktime.is_some(),
        SampleEntryVariant::Ec3(e) => e.quicktime.is_some(),
        SampleEntryVariant::Opus(e) => e.quicktime.is_some(),
        SampleEntryVariant::Flac(e) => e.quicktime.is_some(),
        SampleEntryVariant::Ac4(e) => e.quicktime.is_some(),
        SampleEntryVariant::Mha(e) => e.quicktime.is_some(),
        SampleEntryVariant::Dts(e) => e.quicktime.is_some(),
        _ => false,
    }
}

#[test]
fn quicktime_sample_entries_serialise_back_to_the_file_bytes() {
    for rel in QUICKTIME_FIXTURES {
        let data = fixture(rel);
        let drive = drive(&data, ReaderConfig::default()).expect("read");
        let mut checked = 0usize;
        for track in &drive.tracks {
            for entry in &track.entries {
                let Some(typed) = &entry.typed else {
                    continue;
                };
                if !quicktime_entry(typed) {
                    continue;
                }
                let bytes = typed.try_to_bytes().expect("serialise entry");
                assert!(!bytes.is_empty(), "{rel} {:?}", entry.fourcc);
                assert_eq!(bytes, entry.body, "{rel} {:?}", entry.fourcc);
                checked += 1;
            }
        }
        assert!(checked > 0, "{rel} carries no typed QuickTime entry");
    }
}

#[test]
fn track_info_bodies_match_the_file() {
    for rel in QUICKTIME_FIXTURES {
        let data = fixture(rel);
        let drive = drive(&data, ReaderConfig::default()).expect("read");
        let tracks = moov_tracks(&data);
        assert_eq!(tracks.len(), drive.tracks.len(), "{rel}");
        for (trak, info) in tracks.iter().zip(&drive.tracks) {
            let built = TrackInfo::from_trak(trak).expect("track info");
            assert_eq!(built.entries.len(), info.entries.len(), "{rel}");
            for (built_entry, raw) in built.entries.iter().zip(&info.entries) {
                assert!(
                    !built_entry.body.is_empty(),
                    "{rel} {:?} has an empty body",
                    raw.fourcc
                );
                assert_eq!(built_entry.fourcc, raw.fourcc, "{rel}");
                assert_eq!(built_entry.body, raw.body, "{rel} {:?}", raw.fourcc);
            }
        }
    }
}
