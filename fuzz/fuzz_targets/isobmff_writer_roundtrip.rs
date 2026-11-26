#![no_main]

use broadcast_core::Parse;
use isobmff::TrackId;
use isobmff::movie_fragment::MovieFragmentBox;
use isobmff::writer::{SampleEntry, SampleMeta, TrackSetup, Writer};
use libfuzzer_sys::fuzz_target;

/// Bounded track and sample counts: the writer must round-trip what it wrote,
/// and the fuzz input must not make the target allocate without limit.
const MAX_TRACKS: usize = 3;
const MAX_SAMPLES_PER_TRACK: usize = 8;

/// An empty `avc1` sample entry (box header only), written raw so the init
/// segment is assembled from caller-supplied bytes exactly as a private track
/// would be.
const RAW_ENTRY: [u8; 8] = [0, 0, 0, 8, b'a', b'v', b'c', b'1'];

fn timescale() -> broadcast_core::timescale::Timescale {
    broadcast_core::timescale::Timescale::try_new(1000).expect("a non-zero timescale")
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
    let mut writer =
        Writer::fragmented(setups, timescale(), isobmff::Limits::default()).expect("writer");
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
                        description_index: 1,
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
            .set_base_decode_time(
                TrackId::new(track as u32 + 1),
                u64::from(track as u32) * 100,
            )
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

    // A track that never received a sample was never activated, so it has no
    // `traf`; every track we did push into has exactly one.
    assert_eq!(moof.traf.len(), metas.len());
    let mdats = isobmff::frag_offsets::mdat_ranges(&fragment.bytes);
    let budget = isobmff::frag_offsets::sample_bytes_budget(fragment.bytes.len() as u64);
    let mut copied = 0u64;
    let mut seen: Vec<u32> = Vec::with_capacity(metas.len());
    for traf in &moof.traf {
        let track = usize::try_from(traf.tfhd.track_id.value())
            .ok()
            .and_then(|id| id.checked_sub(1))
            .expect("a track id the writer was given");
        assert!(track < metas.len(), "traf for a track with no samples");
        assert!(
            !seen.contains(&traf.tfhd.track_id.value()),
            "one traf per track"
        );
        seen.push(traf.tfhd.track_id.value());

        // Resolve through `frag_offsets`, the same reader a demuxer uses: it
        // applies the effective duration, flags and composition offset, so the
        // assertion does not depend on which optional `trun` field the writer
        // chose to emit.
        let ranges = isobmff::frag_offsets::sample_ranges_in_run(
            moof_item.start as u64,
            &moof,
            traf.tfhd.track_id,
            &mdats,
            budget,
            &mut copied,
        )
        .expect("resolve sample ranges");
        assert_eq!(ranges.len(), metas[track].len(), "sample count");
        for ((range, meta), expected) in ranges.iter().zip(&metas[track]).zip(&payloads[track]) {
            assert_eq!(range.duration, meta.duration, "duration");
            assert_eq!(
                range.composition_offset,
                i64::from(meta.composition_offset),
                "composition offset"
            );
            assert_eq!(
                range.flags & isobmff::movie_fragment::SAMPLE_FLAG_IS_NON_SYNC == 0,
                meta.sync,
                "sync flag"
            );
            let start = usize::try_from(range.start).expect("sample start");
            let end = usize::try_from(range.end).expect("sample end");
            assert_eq!(
                &fragment.bytes[start..end],
                expected.as_slice(),
                "sample bytes"
            );
        }
    }
    for track in 0..metas.len() {
        assert!(
            seen.contains(&(u32::try_from(track).expect("bounded") + 1)),
            "every pushed track has a traf"
        );
    }

    progressive_arm(data);
});

/// Place one writer output at its absolute offset in the assembled file.
fn place(out: &mut Vec<u8>, write: isobmff::writer::Write<'_>) {
    let start = usize::try_from(write.offset).expect("offset");
    let end = start + write.bytes.len();
    if out.len() < end {
        out.resize(end, 0);
    }
    out[start..end].copy_from_slice(&write.bytes);
}

/// The progressive arm: build a plan from the same input, write the whole
/// file, and prove the reader recovers every pushed sample byte for byte.
fn progressive_arm(data: &[u8]) {
    use isobmff::plan::{PlanBuilder, PlanSample};

    let setups = vec![TrackSetup::new(
        TrackId::new(1),
        *b"vide",
        b"VideoHandler\0".to_vec(),
        timescale(),
        SampleEntry::Raw {
            fourcc: *b"avc1",
            body: RAW_ENTRY.to_vec(),
        },
    )];
    let mut builder = PlanBuilder::new(setups, isobmff::Limits::default());
    let mut samples: Vec<Vec<u8>> = Vec::new();
    let mut cursor = data;
    for _ in 0..MAX_SAMPLES_PER_TRACK {
        let Some((&head, tail)) = cursor.split_first() else {
            break;
        };
        cursor = tail;
        let take = cursor.len().min(usize::from(head));
        let payload = cursor[..take].to_vec();
        cursor = &cursor[take..];
        builder
            .push(
                TrackId::new(1),
                PlanSample {
                    duration: 1 + u32::from(head),
                    composition_offset: 0,
                    sync: head & 1 == 0,
                    size: u32::try_from(take).expect("at most 255 bytes"),
                    description_index: 1,
                },
            )
            .expect("push within the sample cap");
        samples.push(payload);
    }
    if samples.is_empty() {
        return;
    }
    let count = u64::try_from(samples.len()).expect("bounded");
    builder
        .set_durations(TrackId::new(1), count, count)
        .expect("durations");
    let plan = builder.finish().expect("plan");
    let layout = isobmff::writer::Layout::new(
        true,
        isobmff::writer::Interleave::ByDuration { millis: 500 },
        isobmff::writer::Sink::Seekable,
    );
    let mut writer = isobmff::writer::Writer::progressive(
        plan,
        layout,
        isobmff::writer::FileType::default(),
        isobmff::writer::MovieHeader::new(timescale()),
        isobmff::Limits::default(),
    )
    .expect("progressive writer");
    let mut out = vec![0u8; usize::try_from(writer.file_len()).expect("length")];
    for write in writer.header() {
        place(&mut out, write);
    }
    for (index, sample) in samples.iter().enumerate() {
        let index = u32::try_from(index).expect("bounded");
        for write in writer.push(TrackId::new(1), index, sample).expect("push") {
            place(&mut out, write);
        }
    }
    for write in writer.finish().expect("finish") {
        place(&mut out, write);
    }

    let mut reader = isobmff::Reader::new(
        isobmff::ReaderConfig::default()
            .with_total_len(Some(out.len() as u64))
            .with_order(isobmff::SampleOrder::File),
    );
    let mut seen = Vec::new();
    loop {
        match reader.step().expect("reader step") {
            isobmff::Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end");
                reader
                    .supply(range.offset, &out[start..end])
                    .expect("supply");
            }
            isobmff::Step::Sample(sample) => seen.push(sample),
            isobmff::Step::Done => break,
            _ => {}
        }
    }
    assert_eq!(seen.len(), samples.len(), "every planned sample is indexed");
    for (sample, pushed) in seen.iter().zip(&samples) {
        let start = usize::try_from(sample.offset).expect("offset");
        let end = start + usize::try_from(sample.size).expect("size");
        assert_eq!(
            &out[start..end],
            pushed.as_slice(),
            "progressive sample bytes"
        );
    }
}
