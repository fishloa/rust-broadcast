#![no_main]

use broadcast_common::Parse;
use isobmff::movie_fragment::MovieFragmentBox;
use isobmff::writer::{SampleEntry, SampleMeta, TrackSetup, Writer};
use isobmff::TrackId;
use libfuzzer_sys::fuzz_target;

/// Bounded track and sample counts: the writer must round-trip what it wrote,
/// and the fuzz input must not make the target allocate without limit.
const MAX_TRACKS: usize = 3;
const MAX_SAMPLES_PER_TRACK: usize = 8;

/// An empty `avc1` sample entry (box header only), written raw so the init
/// segment is assembled from caller-supplied bytes exactly as a private track
/// would be.
const RAW_ENTRY: [u8; 8] = [0, 0, 0, 8, b'a', b'v', b'c', b'1'];

fn timescale() -> broadcast_common::timescale::Timescale {
    broadcast_common::timescale::Timescale::try_new(1000).expect("a non-zero timescale")
}

fuzz_target!(|data: &[u8]| {
    let Some((&track_count, rest)) = data.split_first() else {
        return;
    };
    let track_count = usize::from(track_count) % MAX_TRACKS + 1;
    let mut setups = Vec::with_capacity(track_count);
    for i in 0..track_count {
        setups.push(TrackSetup::new(
            TrackId::new(u32::try_from(i + 1).expect("small track id")),
            *b"vide",
            b"VideoHandler\0".to_vec(),
            timescale(),
            SampleEntry::Raw {
                fourcc: *b"avc1",
                body: RAW_ENTRY.to_vec(),
            },
        ));
    }
    let mut writer = Writer::fragmented(setups, timescale(), isobmff::Limits::default())
        .expect("writer");
    let init = writer.init().expect("init segment");
    let moov = isobmff::init_segment::MovieBox::parse(
        isobmff::box_types::find_box(&init, b"moov").expect("moov"),
    )
    .expect("parse moov");
    assert_eq!(moov.tracks.len(), track_count);

    let mut metas: Vec<Vec<SampleMeta>> = Vec::with_capacity(track_count);
    let mut payloads: Vec<Vec<Vec<u8>>> = Vec::with_capacity(track_count);
    let mut cursor = rest;
    for track in 0..track_count {
        let Some((&count, tail)) = cursor.split_first() else {
            break;
        };
        cursor = tail;
        let count = usize::from(count) % MAX_SAMPLES_PER_TRACK + 1;
        let mut track_metas = Vec::with_capacity(count);
        let mut track_payloads = Vec::with_capacity(count);
        for sample in 0..count {
            let (meta, tail) = match cursor.split_first() {
                Some((&flags, tail)) => (
                    SampleMeta {
                        duration: u32::from(sample as u8) + 1,
                        composition_offset: i32::from(flags as i8),
                        sync: flags & 1 == 0,
                        aux: None,
                    },
                    tail,
                ),
                None => (SampleMeta::default(), cursor),
            };
            cursor = tail;
            let take = cursor.len().min(16);
            track_payloads.push(cursor[..take].to_vec());
            cursor = &cursor[take..];
            track_metas.push(meta);
        }
        writer
            .set_base_decode_time(TrackId::new(track as u32 + 1), u64::from(track as u32) * 100)
            .expect("base decode time");
        for (meta, payload) in track_metas.iter().zip(&track_payloads) {
            writer
                .push(TrackId::new(track as u32 + 1), meta.clone(), payload)
                .expect("push within the fragment cap");
        }
        metas.push(track_metas);
        payloads.push(track_payloads);
    }

    let Some(fragment) = writer.flush().expect("flush") else {
        return;
    };
    let moof_item = isobmff::box_types::BoxIter::new(&fragment.bytes)
        .filter_map(|b| b.ok())
        .find(|b| b.four_cc() == *b"moof")
        .expect("moof");
    let moof = MovieFragmentBox::parse_body(moof_item.body).expect("parse moof");
    assert_eq!(moof.traf.len(), track_count);
    let mdats = isobmff::frag_offsets::mdat_ranges(&fragment.bytes);
    let mut copied = 0usize;
    for (track, traf) in moof.traf.iter().enumerate() {
        assert_eq!(traf.tfhd.track_id, TrackId::new(track as u32 + 1));
        let samples = &traf.trun[0].samples;
        assert_eq!(samples.len(), metas[track].len());
        for (sample, meta) in samples.iter().zip(&metas[track]) {
            assert_eq!(sample.sample_duration, Some(meta.duration));
            assert_eq!(
                sample.sample_composition_time_offset,
                Some(i64::from(meta.composition_offset))
            );
        }
        let ranges = isobmff::frag_offsets::sample_ranges_in_run(
            &fragment.bytes,
            &mdats,
            moof_item.start,
            &moof,
            traf.tfhd.track_id,
            &mut copied,
        )
        .expect("resolve sample ranges");
        assert_eq!(ranges.len(), payloads[track].len());
        for ((range, expected), meta) in ranges.iter().zip(&payloads[track]).zip(&metas[track]) {
            assert_eq!(&fragment.bytes[range.start..range.end], expected.as_slice());
            assert_eq!(
                range.flags & isobmff::movie_fragment::SAMPLE_FLAG_IS_NON_SYNC == 0,
                meta.sync
            );
        }
    }
});
