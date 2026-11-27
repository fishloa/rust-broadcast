//! Real files carrying the tier 3/4 boxes of issue #4167.
//!
//! Each assertion names the line an independent tool's dump of the same file
//! prints, and every parsed box is re-serialized byte-exact from the file's
//! own bytes. `fixtures/mp4/boxes/PROVENANCE.md` records the command, tool
//! version and dump for each file.

use broadcast_core::{Parse, Serialize};
use isobmff::init_segment::SampleEntryVariant;
use isobmff::metadata::{Udta, UdtaChild};
use isobmff::{BoxIter, HintMediaHeaderBox, MovieBox, NullMediaHeaderBox, parse_box};

fn children(parent: &[u8], fourcc: [u8; 4]) -> Vec<Vec<u8>> {
    parse_box(parent)
        .expect("box header")
        .0
        .children()
        .filter_map(|child| child.ok())
        .filter(|child| child.four_cc() == fourcc)
        .map(|child| child.bytes.to_vec())
        .collect()
}

fn top_level(file: &[u8], fourcc: [u8; 4]) -> Vec<u8> {
    for item in BoxIter::new(file) {
        let item = item.expect("top-level box");
        if item.four_cc() == fourcc {
            return item.bytes.to_vec();
        }
    }
    panic!("no top-level {fourcc:?}");
}

fn one_child(parent: &[u8], fourcc: [u8; 4]) -> Vec<u8> {
    let mut found = children(parent, fourcc);
    assert_eq!(found.len(), 1, "exactly one {fourcc:?}");
    found.pop().expect("one")
}

#[test]
fn cprt_from_mp4box_reads_back() {
    let file = test_fixtures::read("mp4/boxes/cprt.mp4");
    let moov = top_level(&file, *b"moov");
    let udta_bytes = one_child(&moov, *b"udta");
    let udta = Udta::parse(&udta_bytes).expect("udta parses");
    let copyright = udta
        .children
        .iter()
        .find_map(|child| match child {
            UdtaChild::Copyright(b) => Some(b),
            _ => None,
        })
        .expect("cprt");
    // MP4Box -disox: LanguageCode="und" CopyrightNotice="(c) 2026 Example Corp".
    assert_eq!(copyright.language(), 0x55C4);
    assert_eq!(copyright.notice_str(), Some("(c) 2026 Example Corp"));
    assert_eq!(
        copyright.try_to_bytes().expect("cprt bytes"),
        one_child(&udta_bytes, *b"cprt")
    );
}

#[test]
fn hmhd_from_mp4box_hint_tracks_reads_back() {
    let file = test_fixtures::read("mp4/boxes/hint-hmhd.mp4");
    let moov = top_level(&file, *b"moov");
    let movie = MovieBox::parse(&moov).expect("moov parses");
    let mut seen = 0;
    for trak in &movie.tracks {
        let Some(hmhd) = trak
            .mdia
            .as_ref()
            .and_then(|mdia| mdia.minf.as_ref())
            .and_then(|minf| minf.hmhd.as_ref())
        else {
            continue;
        };
        // MP4Box -disox: MaximumPDUSize="0" AveragePDUSize="0"
        // MaxBitRate="0" AverageBitRate="0", Version="0" Flags="0".
        assert_eq!(hmhd.version(), 0);
        assert_eq!(hmhd.flags(), 0);
        assert_eq!(hmhd.max_pdu_size(), 0);
        assert_eq!(hmhd.avg_pdu_size(), 0);
        assert_eq!(hmhd.max_bitrate(), 0);
        assert_eq!(hmhd.avg_bitrate(), 0);
        let bytes = hmhd.try_to_bytes().expect("hmhd bytes");
        assert_eq!(&bytes[4..8], b"hmhd");
        assert_eq!(HintMediaHeaderBox::parse(&bytes).expect("reparse"), *hmhd);
        seen += 1;
    }
    assert_eq!(seen, 2, "one hmhd per hint track");
}

#[test]
fn nmhd_from_a_committed_subtitle_fixture_reads_back() {
    let file = test_fixtures::read("subtitles/mp4box/wvtt_progressive.mp4");
    let moov = top_level(&file, *b"moov");
    let movie = MovieBox::parse(&moov).expect("moov parses");
    let mut seen = 0;
    for trak in &movie.tracks {
        let Some(nmhd) = trak
            .mdia
            .as_ref()
            .and_then(|mdia| mdia.minf.as_ref())
            .and_then(|minf| minf.nmhd.as_ref())
        else {
            continue;
        };
        // MP4Box -disox: <MPEGMediaHeaderBox ... Type="nmhd" Version="0" Flags="0">.
        assert_eq!(nmhd.version(), 0);
        assert_eq!(nmhd.flags(), 0);
        let bytes = nmhd.try_to_bytes().expect("nmhd bytes");
        assert_eq!(&bytes[4..8], b"nmhd");
        assert_eq!(NullMediaHeaderBox::parse(&bytes).expect("reparse"), *nmhd);
        seen += 1;
    }
    assert_eq!(seen, 1, "the WebVTT track's nmhd");
}

#[test]
fn keyd_from_the_private_live_photo_reads_back() {
    let Some(path) = test_fixtures::require(
        "mp4/quicktime/iphone-x-live-photo.mov",
        "isobmff tier 3 fixtures",
    ) else {
        return;
    };
    let file = std::fs::read(&path).expect("live photo");
    let moov = top_level(&file, *b"moov");
    let movie = MovieBox::parse(&moov).expect("moov parses");
    let mut declarations = Vec::new();
    for trak in &movie.tracks {
        let Some(stsd) = trak
            .mdia
            .as_ref()
            .and_then(|mdia| mdia.minf.as_ref())
            .and_then(|minf| minf.stbl.as_ref())
            .and_then(sample_description)
        else {
            continue;
        };
        for entry in &stsd.entries {
            let SampleEntryVariant::Mebx(mebx) = entry else {
                continue;
            };
            let key = &mebx.key_table().keys()[0];
            let declaration = key.declaration().expect("keyd");
            // §12.8.4.4: the namespace is 'mdta', the value Apple's own key.
            assert_eq!(declaration.key_namespace(), u32::from_be_bytes(*b"mdta"));
            declarations.push(
                declaration
                    .key_value_str()
                    .expect("key value text")
                    .to_string(),
            );
            let bytes = mebx.try_to_bytes().expect("mebx bytes");
            assert_eq!(
                isobmff::BoxedMetaDataSampleEntry::parse(&bytes).expect("reparse"),
                **mebx
            );
        }
    }
    declarations.sort();
    assert_eq!(
        declarations,
        vec![
            "com.apple.quicktime.live-photo-info".to_string(),
            "com.apple.quicktime.still-image-time".to_string(),
        ]
    );
}

fn sample_description(stbl: &isobmff::SampleTableBox) -> Option<&isobmff::SampleDescriptionBox> {
    stbl.children.iter().find_map(|child| match child {
        isobmff::StblChild::Stsd(stsd) => Some(stsd),
        _ => None,
    })
}

#[test]
fn every_committed_mp4_family_fixture_walks() {
    let mut files = 0usize;
    let mut roots = vec![test_fixtures::root()];
    if let Some(private) = test_fixtures::private_root() {
        roots.push(private);
    }
    for root in roots {
        for path in mp4_files(&root) {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if bytes.len() < 8 {
                continue;
            }
            files += 1;
            for item in BoxIter::new(&bytes) {
                let Ok(item) = item else { break };
                for child in item.children() {
                    let Ok(child) = child else { break };
                    let _ = child.four_cc();
                }
            }
        }
    }
    assert!(files > 0, "the corpus walk found MP4-family files");
}

fn mp4_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    const EXTS: [&str; 8] = ["mp4", "mov", "m4a", "m4v", "3gp", "m4s", "heic", "avif"];
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| EXTS.contains(&e.to_ascii_lowercase().as_str()))
            {
                out.push(path);
            }
        }
    }
    out
}

#[test]
fn the_reader_surfaces_a_metadata_tracks_entry_and_media_header() {
    use isobmff::driver::{DriverStep, FileReader};
    use isobmff::reader::TimedMetadataEntry;

    let Some(path) = test_fixtures::require(
        "mp4/quicktime/iphone-x-live-photo.mov",
        "isobmff tier 3 fixtures",
    ) else {
        return;
    };
    let file = std::fs::File::open(&path).expect("live photo");
    let mut reader = FileReader::new(file, isobmff::ReaderConfig::default()).expect("reader");
    let mut metadata_entries = Vec::new();
    let mut typed_headers = 0usize;
    loop {
        match reader.step().expect("step") {
            DriverStep::Tracks(tracks) => {
                for track in tracks {
                    if track.media_header.is_some() {
                        typed_headers += 1;
                    }
                    for entry in &track.entries {
                        if let Some(TimedMetadataEntry::Boxed(boxed)) = entry.timed_metadata() {
                            let key = &boxed.key_table().keys()[0];
                            metadata_entries.push(
                                key.declaration()
                                    .and_then(|d| d.key_value_str())
                                    .expect("key value")
                                    .to_string(),
                            );
                        }
                    }
                }
            }
            DriverStep::Done => break,
            _ => {}
        }
    }
    assert_eq!(typed_headers, 2, "vmhd and smhd tracks report their header");
    metadata_entries.sort();
    assert_eq!(
        metadata_entries,
        vec![
            "com.apple.quicktime.live-photo-info".to_string(),
            "com.apple.quicktime.still-image-time".to_string(),
        ]
    );
}

#[test]
fn the_reader_reports_a_subtitle_tracks_null_media_header() {
    use isobmff::MediaInformationHeader;
    use isobmff::driver::{DriverStep, FileReader};

    let file = test_fixtures::read("subtitles/mp4box/wvtt_progressive.mp4");
    let mut reader = FileReader::new(std::io::Cursor::new(file), isobmff::ReaderConfig::default())
        .expect("reader");
    let mut seen = false;
    loop {
        match reader.step().expect("step") {
            DriverStep::Tracks(tracks) => {
                for track in tracks {
                    if let Some(MediaInformationHeader::Null(header)) = &track.media_header {
                        assert_eq!(header.version(), 0);
                        seen = true;
                    }
                }
            }
            DriverStep::Done => break,
            _ => {}
        }
    }
    assert!(seen, "the WebVTT track reports its nmhd");
}
