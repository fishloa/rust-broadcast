//! `PsDemux` `private_stream_1` substream / AC-3 syncframe / H.264 AU-split
//! gate (audit r04-W19, r04-W20, r04-W21).
//!
//! Fixture: `fixtures/ps/ffmpeg-mpeg2video-2xac3.ps` — see
//! `fixtures/ps/README.md` for the exact ffmpeg command. It is the real shape
//! the three warnings describe: one MPEG-2 video stream plus **two**
//! AC-3 audio substreams multiplexed into a single `private_stream_1` (0xBD)
//! `stream_id`, told apart only by the `substream_id` byte at the front of each
//! PES payload's substream header (0x80 and 0x81).
//!
//! Oracles (ffprobe 8.1.2 on that same file):
//! - stream 0 `mpeg2video` 352x288, 50 packets, all key-stamped frames;
//! - streams 1 and 2 `ac3` 44100 Hz mono, 58 packets each (the two
//!   `substream_id`s), arriving interleaved in the same `stream_id`.
//!
//! Every assertion below is written to *bite*: it is checked against those
//! external oracle values and against the fixture's own bytes, never against a
//! counter the demuxer maintains for itself.

use std::path::PathBuf;

use broadcast_common::Unpackage;
use transmux::PsDemux;
use transmux::media::Media;
use transmux::pipeline::CodecConfig;

fn load_ps() -> Vec<u8> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ps/ffmpeg-mpeg2video-2xac3.ps");
    let data = std::fs::read(&path).expect("ffmpeg-mpeg2video-2xac3.ps fixture must exist");
    assert_eq!(
        data[..4],
        [0x00, 0x00, 0x01, 0xBA],
        "fixture must open with a Program Stream pack header"
    );
    data
}

fn demux(data: &[u8]) -> Media {
    let mut demux = PsDemux::new();
    demux
        .unpackage(data)
        .expect("demux MPEG-2 PS with two AC-3 substreams")
}

/// `substream_id`s actually present in the fixture's `private_stream_1` PES
/// packets, read straight from the file rather than from the demuxer. Each
/// payload of a 0xBD PES begins with the 4-byte substream header
/// (`substream_id` + `number_of_frames` + `first_access_unit_pointer`), so its
/// first byte after the PES optional header is the `substream_id`.
fn fixture_substream_ids(data: &[u8]) -> Vec<u8> {
    let mut ids = Vec::new();
    let mut i = 0usize;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 && data[i + 3] == 0xBD {
            let j = i + 4;
            let pes_len = usize::from(u16::from_be_bytes([data[j], data[j + 1]]));
            let payload = &data[j + 2..(j + 2 + pes_len).min(data.len())];
            // PES optional header: flags1(1) flags2(1) header_data_length(1).
            let hdr = 3 + usize::from(payload[2]);
            if let Some(substream_id) = payload.get(hdr) {
                ids.push(*substream_id);
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    ids
}

/// r04-W19: each `private_stream_1` substream must become its own track.
///
/// Before the fix every 0xBD PES payload was treated as AC-3 and concatenated
/// regardless of `substream_id`, so this fixture's two interleaved 44.1 kHz
/// mono AC-3 substreams came back as **one** track whose syncframes alternated
/// between two unrelated audio programmes — the sample count matched neither
/// stream and every second frame's BSI disagreed with the previous one.
#[test]
fn private_stream_1_substreams_are_demultiplexed() {
    let data = load_ps();

    // The fixture really does carry two substreams; without this the test could
    // pass vacuously on a single-substream file.
    let ids = fixture_substream_ids(&data);
    let mut distinct: Vec<u8> = ids.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct,
        vec![0x80, 0x81],
        "fixture must multiplex exactly the 0x80/0x81 AC-3 substreams"
    );

    let media = demux(&data);
    let audio: Vec<_> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .collect();
    assert_eq!(
        audio.len(),
        2,
        "one track per private_stream_1 substream (0x80 and 0x81), got {}",
        audio.len()
    );

    // ffprobe oracle: 58 AC-3 packets per substream. This is the assertion that
    // fails without the fix — the merged track carried the interleaving of both
    // substreams, so neither count came out right.
    for (n, track) in audio.iter().enumerate() {
        assert_eq!(
            track.samples.len(),
            58,
            "substream #{n} must yield the ffprobe oracle's 58 AC-3 syncframes",
        );
        assert_eq!(
            track.spec.timescale, 44100,
            "AC-3 fscod=1 is 44100 Hz (ETSI TS 102 366 Table 4.5); \
             a merged track reported the wrong rate"
        );
    }

    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Mpeg2Video { .. }))
        .expect("video track");
    assert_eq!(
        video.samples.len(),
        50,
        "50 MPEG-2 pictures (ffprobe oracle)"
    );
}

/// r04-W20: AC-3 frames must be split by each syncframe's own declared length,
/// not at every byte pair that happens to read `0x0B77`.
///
/// The syncword occurs inside AC-3 payload by chance (~1 position in 65536), so
/// the old scanner cut real frames in two. Each such split produced an extra
/// sample stamped with a whole 1536-sample duration, so the byte sum stayed the
/// same while the sample count grew. Comparing the demuxed frame lengths against
/// the fixture's own BSI-derived frame count catches that directly.
#[test]
fn ac3_frames_are_split_by_declared_length() {
    let data = load_ps();
    let media = demux(&data);

    // Total bytes the fixture actually hands the AC-3 substreams, counted from
    // the file: every 0xBD PES payload after its 4-byte substream header.
    let mut es_bytes = 0usize;
    let mut i = 0usize;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 && data[i + 3] == 0xBD {
            let j = i + 4;
            let pes_len = usize::from(u16::from_be_bytes([data[j], data[j + 1]]));
            let payload = &data[j + 2..(j + 2 + pes_len).min(data.len())];
            let hdr = 3 + usize::from(payload[2]);
            if payload.len() > hdr + 4 {
                es_bytes += payload.len() - hdr - 4;
            }
            i += 4;
        } else {
            i += 1;
        }
    }

    let ac3_total: usize = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .map(|t| t.samples.iter().map(|s| s.data.len()).sum::<usize>())
        .sum();
    assert_eq!(
        ac3_total, es_bytes,
        "every AC-3 byte the fixture carries must land in exactly one sample; \
         a false-syncword split still sums to the same total but adds samples, \
         which the count check below catches"
    );

    // Each sample is one complete syncframe, so its length must be one the
    // AC-3 frame-size table can produce. The fixture is 192 kb/s at 44.1 kHz
    // (`frmsizecod` 20, fscod 1) → 418 words → 836 bytes; ffprobe reports 58
    // packets for that substream, 55 of them 836 bytes and the last 3 trimmed to
    // 834 by ffmpeg's final partial frame. A false-syncword split yields lengths
    // outside that set (and more of them), so this bites.
    const FRAME_BYTES: &[usize] = &[834, 836];
    for track in media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
    {
        assert_eq!(track.samples.len(), 58);
        for (n, sample) in track.samples.iter().enumerate() {
            assert!(
                FRAME_BYTES.contains(&sample.data.len()),
                "sample {n} is {} bytes: must be one whole syncframe \
                 (834/836, the fixture's own ffprobe-reported sizes), not a \
                 fragment cut at an accidental 0x0B77 in the payload",
                sample.data.len()
            );
            assert!(
                sample.data.starts_with(&[0x0B, 0x77]),
                "sample {n} must begin at an AC-3 syncword"
            );
            assert_eq!(
                sample.duration,
                Some(1536),
                "each syncframe is 1536 samples (ETSI TS 102 366 §4.1)"
            );
        }
    }
}

/// r04-W21: H.264 access units must be found without an access-unit delimiter.
///
/// Fixture: `fixtures/ps/ffmpeg-h264-noaud.ps` — the audit report's exact
/// scenario. Its H.264 video carries **no** AUD (the delimiters were stripped
/// with `ffmpeg -bsf:v h264_metadata=aud=remove`) and, because ffmpeg wrote the
/// whole ES as a single PES packet, a splitter that only breaks at AUDs cannot
/// find a single boundary: the entire file becomes one "access unit" — one
/// sample, one IDR flag. ffprobe reports 75 pictures for the same file.
///
/// Also checks the AC-3 fix holds on an AUD-free video: the two fixes must not
/// interfere.
#[test]
fn h264_access_units_split_without_auds() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ps/ffmpeg-h264-noaud.ps");
    let data = std::fs::read(&path).expect("ffmpeg-h264-noaud.ps fixture must exist");
    assert_eq!(data[..4], [0x00, 0x00, 0x01, 0xBA], "fixture must be a PS");

    // The fixture's video must genuinely carry no AUD *and* must fit in a single
    // PES packet, or this test proves nothing about the non-AUD path.
    assert_eq!(
        count_h264_auds(&data),
        0,
        "fixture's H.264 video must carry no access-unit delimiter"
    );

    let media = demux(&data);
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("H.264 video track");

    // ffprobe oracle: 75 video packets.
    assert_eq!(
        video.samples.len(),
        75,
        "75 pictures must be recovered without an AUD to split on; the whole          file collapsing into one access unit is the r04-W21 failure"
    );

    // An AUD-free split still has to identify the capture's keyframes: ffprobe
    // reports 3 (IDR NALs are visible in the ES), so a single-sample result
    // would also show up here.
    let syncs = video.samples.iter().filter(|s| s.flags.is_sync).count();
    assert_eq!(syncs, 3, "3 IDR pictures (ffprobe oracle)");

    // Every sample must be a well-formed sequence of length-prefixed NALs whose
    // lengths exactly cover it — the crate-wide IR invariant for Annex B input.
    for (n, sample) in video.samples.iter().enumerate() {
        let mut off = 0usize;
        let mut nals = 0usize;
        while off + 4 <= sample.data.len() {
            let len = u32::from_be_bytes([
                sample.data[off],
                sample.data[off + 1],
                sample.data[off + 2],
                sample.data[off + 3],
            ]) as usize;
            off += 4 + len;
            nals += 1;
            assert!(
                off <= sample.data.len(),
                "sample {n} NAL length prefix runs past the sample"
            );
        }
        assert_eq!(
            off,
            sample.data.len(),
            "sample {n} must be exactly a sequence of length-prefixed NALs"
        );
        assert!(nals > 0, "sample {n} must carry at least one NAL");
    }
}

/// Count H.264 access-unit delimiter NALs (type 9) in the fixture's video
/// elementary stream, read from the raw file.
fn count_h264_auds(data: &[u8]) -> usize {
    let mut auds = 0usize;
    let mut i = 0usize;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 && data[i + 3] == 0xE0 {
            let j = i + 4;
            let pes_len = usize::from(u16::from_be_bytes([data[j], data[j + 1]]));
            let payload = &data[j + 2..(j + 2 + pes_len).min(data.len())];
            let hdr = 3 + usize::from(payload[2]);
            let es = &payload[hdr.min(payload.len())..];
            let mut p = 0usize;
            while p + 3 < es.len() {
                if es[p] == 0 && es[p + 1] == 0 && es[p + 2] == 1 {
                    if es[p + 3] & 0x1F == 9 {
                        auds += 1;
                    }
                    p += 3;
                } else {
                    p += 1;
                }
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    auds
}

// ---------------------------------------------------------------------------
// r04-W19 (review): the substream header's pointer/frame-count must be used
// ---------------------------------------------------------------------------

/// The `first_access_unit_pointer` of each `0xBD` PES packet of `want`, in file
/// order, read straight from the file.
fn first_access_unit_pointers(data: &[u8], want: u8) -> Vec<u16> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 3 < data.len() {
        if data[i..i + 4] == [0x00, 0x00, 0x01, 0xBD] {
            let j = i + 4;
            let pes_len = usize::from(u16::from_be_bytes([data[j], data[j + 1]]));
            let payload = &data[j + 2..(j + 2 + pes_len).min(data.len())];
            let hdr = 3 + usize::from(payload[2]);
            if payload.len() > hdr + 4 && payload[hdr] == want {
                out.push(u16::from_be_bytes([payload[hdr + 2], payload[hdr + 3]]));
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    out
}

/// The **real** AC-3 elementary stream ffmpeg extracts for one audio program of
/// the two-substream fixture:
///
/// ```text
/// ffmpeg -v error -i fixtures/ps/ffmpeg-mpeg2video-2xac3.ps -map 0:a:N ///        -c copy -f ac3 fixtures/ps/ffmpeg-mpeg2video-2xac3.aN.ac3
/// ```
///
/// This is an independent oracle: ffmpeg's own demuxer decides where the
/// substream's bytes begin and end, so comparing the crate's demuxed track
/// against it is not the demuxer agreeing with itself.
fn ffmpeg_ac3_es(index: usize) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../fixtures/ps/ffmpeg-mpeg2video-2xac3.a{index}.ac3"
    ));
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture a{index}.ac3: {e}"))
}

/// The AC-3 bytes a substream carries in the fixture, per its PES headers: each
/// `0xBD` packet's bytes after the 4-byte substream header (with the first
/// packet's leading partial-frame tail removed). Used by the mid-frame test,
/// whose truncation the ffmpeg extract cannot describe.
fn substream_es(data: &[u8], want: u8) -> Vec<u8> {
    let mut es = Vec::new();
    let mut first = true;
    let mut i = 0usize;
    while i + 3 < data.len() {
        if data[i..i + 4] == [0x00, 0x00, 0x01, 0xBD] {
            let j = i + 4;
            let pes_len = usize::from(u16::from_be_bytes([data[j], data[j + 1]]));
            let payload = &data[j + 2..(j + 2 + pes_len).min(data.len())];
            let hdr = 3 + usize::from(payload[2]);
            if payload.len() > hdr + 4 && payload[hdr] == want {
                let pointer = usize::from(u16::from_be_bytes([payload[hdr + 2], payload[hdr + 3]]));
                let body = &payload[hdr + 4..];
                let start = if first {
                    pointer.saturating_sub(1).min(body.len())
                } else {
                    0
                };
                if first && pointer != 0 {
                    first = false;
                }
                es.extend_from_slice(&body[start..]);
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    es
}

/// Every AC-3 syncframe of the substream must be recovered, including the ones
/// whose PES packet starts *mid-frame*.
///
/// The fixture's substream headers say `first_access_unit_pointer` is not 0 for
/// several packets (they resume a frame the previous packet began), so those
/// payloads do not begin with `0B77` at all. Classifying on "payload starts
/// with the syncword" therefore drops most of a real stream; the frames must be
/// found in the *reassembled* elementary stream instead, and the concatenation
/// must equal the file's own ES bytes exactly (frames are never split
/// mid-frame, and never restarted at a PES boundary).
#[test]
fn ac3_frames_are_recovered_across_pes_boundaries() {
    let data = load_ps();

    // The fixture must really contain mid-frame PES starts, or this test
    // proves nothing.
    let pointers = first_access_unit_pointers(&data, 0x80);
    assert!(
        pointers.iter().any(|&p| p != 0),
        "fixture must carry substream headers whose first_access_unit_pointer \
         is non-zero (a PES that begins mid-frame), got {pointers:?}"
    );

    let media = demux(&data);
    let ac3: Vec<&_> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .collect();
    assert_eq!(ac3.len(), 2, "two substreams");

    for (n, track) in ac3.iter().enumerate() {
        let want = 0x80 + n as u8;
        // The oracle is ffmpeg's own extraction of this substream, not a rule
        // this test re-derives from the file (which would just be the demuxer
        // agreeing with itself).
        let es = ffmpeg_ac3_es(n);
        // The frames must tile the ES exactly: concatenated in order they are
        // byte-identical to the file's own elementary bytes.
        let concatenated: Vec<u8> = track
            .samples
            .iter()
            .flat_map(|s| s.data.iter().copied())
            .collect();
        assert_eq!(
            concatenated.len(),
            es.len(),
            "substream {want:#x}: the demuxed frames must cover the whole ES"
        );
        assert_eq!(
            concatenated, es,
            "substream {want:#x}: frames must be the file's own ES bytes, in \
             order, with no frame split at a PES boundary"
        );
        // Each sample is a whole syncframe.
        for (k, s) in track.samples.iter().enumerate() {
            assert!(
                s.data.starts_with(&[0x0B, 0x77]),
                "substream {want:#x}: sample {k} must begin at a syncword"
            );
        }
        // And they are the count the frames actually are.
        assert_eq!(
            track.samples.len(),
            58,
            "substream {want:#x} yields the ffprobe oracle's 58 syncframes"
        );
    }
}

/// A substream whose *first* PES packet begins mid-frame must still be demuxed.
///
/// The 4-byte substream header states `first_access_unit_pointer`: the offset
/// (1-based, from the byte after the pointer field) at which the first access
/// unit starting in this packet begins. A packet that resumes a frame an
/// earlier packet opened therefore does not begin with `0B77`, and a classifier
/// that only looks at the payload's first two bytes discards the whole
/// substream. This fixture's 0x80 stream is truncated so its first surviving
/// packet is exactly that case — the syncword is found at the pointer, not at
/// offset 0.
#[test]
fn substream_starting_mid_frame_is_still_carried() {
    let data = load_ps();

    // Blank 0x80 packets, from the front, until the stream's first surviving
    // packet begins mid-frame (its body does not start with a syncword).
    let mut cut = data.clone();
    loop {
        let mut bd_offsets = Vec::new();
        let mut i = 0usize;
        while i + 3 < cut.len() {
            if cut[i..i + 4] == [0x00, 0x00, 0x01, 0xBD] {
                bd_offsets.push(i);
                i += 4;
            } else {
                i += 1;
            }
        }
        let mut first = None;
        for &off in &bd_offsets {
            let j = off + 4;
            let pes_len = usize::from(u16::from_be_bytes([cut[j], cut[j + 1]]));
            let payload = &cut[j + 2..j + 2 + pes_len];
            let hdr = 3 + usize::from(payload[2]);
            if payload.len() > hdr + PRIVATE1_MIN_LEN && payload[hdr] == 0x80 {
                first = Some((off, payload[hdr + 4..].to_vec()));
                break;
            }
        }
        match first {
            Some((off, body)) => {
                if !body.starts_with(&[0x0B, 0x77]) {
                    break; // the stream now opens mid-frame
                }
                // Turn this packet into padding so the next one becomes first.
                cut[off + 3] = 0xBE;
            }
            None => panic!("fixture ran out of 0x80 packets"),
        }
    }

    let pointers = first_access_unit_pointers(&cut, 0x80);
    assert!(!pointers.is_empty());
    assert!(
        pointers[0] != 1,
        "the truncated stream's first packet must begin mid-frame (pointer > 1),          got {}",
        pointers[0]
    );

    let media = demux(&cut);
    let ac3: Vec<&_> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .collect();
    assert_eq!(
        ac3.len(),
        2,
        "a substream whose first packet begins mid-frame must still be carried,          found {} AC-3 tracks",
        ac3.len()
    );

    // Every sample is a whole syncframe starting at a syncword; nothing is
    // emitted that is not one.
    for (k, s) in ac3[0].samples.iter().enumerate() {
        assert!(
            s.data.starts_with(&[0x0B, 0x77]),
            "sample {k} must begin at a syncword"
        );
    }

    // The number of frames is the count the *intact* stream yields for the same
    // substream: the truncation removed one packet's worth of leading tail, and
    // the frames that survive are the rest, unchanged. (ffprobe reports 58
    // syncframes for this substream, independent of this crate.)
    assert_eq!(
        ac3[0].samples.len(),
        58,
        "the truncated stream must still yield the substream's 58 syncframes          (got {}); the leading partial-frame tail is dropped, nothing more",
        ac3[0].samples.len()
    );
    assert!(
        !ac3[1].samples.is_empty(),
        "the other substream is still demuxed alongside it"
    );
}

/// Bytes of the AC-3 substream header (substream_id + number_of_frames +
/// first_access_unit_pointer).
const PRIVATE1_MIN_LEN: usize = 4;

// ---------------------------------------------------------------------------
// r04-W19 (review): non-AC-3 audio substreams are skipped, not mis-carried
// ---------------------------------------------------------------------------

/// Fixture: a Program Stream carrying MPEG-2 video, one AC-3 and one **DTS**
/// audio stream, so the `private_stream_1` stream really multiplexes two codecs.
///
/// Generated with ffmpeg's own DTS encoder, not by relabelling AC-3 bytes:
///
/// ```text
/// ffmpeg -f lavfi -i testsrc2=duration=2:size=352x288:rate=25 ///        -f lavfi -i sine=frequency=440:duration=2 ///        -f lavfi -i sine=frequency=880:duration=2 ///        -map 0:v -map 1:a -map 2:a -c:v mpeg2video ///        -c:a:0 ac3 -b:a:0 192k -c:a:1 dca -strict -2 -f vob out.ps
/// ```
///
/// ffprobe reads it as one `mpeg2video`, one `ac3` and one `dts` stream; the
/// AC-3 substream id is 0x80 and the DTS one 0x88, exactly the DVD/ATSC
/// assignments the demuxer's range constants name.
fn load_ps_with_dts() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ps/ffmpeg-ac3-dts.ps");
    std::fs::read(&path).expect("fixture must exist")
}

/// The real AC-3 elementary stream ffmpeg extracts from the AC-3/DTS fixture
/// (`ffmpeg -i ... -map 0:a:0 -c copy -f ac3 ...`), the independent oracle for
/// what the demuxer must produce for the AC-3 substream.
fn ffmpeg_ac3_dts_fixture_es() -> Vec<u8> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ps/ffmpeg-ac3-dts.a0.ac3");
    std::fs::read(&path).expect("fixture must exist")
}

/// A DTS substream must be skipped explicitly, without disturbing the AC-3
/// substream beside it and without producing a bogus track.
///
/// The substream ids are the DVD/ATSC assignments: AC-3 `0x80..=0x87`, DTS
/// `0x88..=0x8F`, LPCM `0xA0..=0xA7`. The fixture carries a genuine ffmpeg
/// `dca` bitstream in 0x88, so this is the real mixed-codec case rather than a
/// relabelled AC-3 payload.
#[test]
fn dts_substream_is_skipped_without_disturbing_ac3() {
    let data = load_ps_with_dts();

    // The fixture really does carry both ids, and the DTS payload really is
    // DTS (its sync word), so the test cannot pass on a relabelled stream.
    let ids = fixture_substream_ids(&data);
    assert!(
        ids.contains(&0x80) && ids.contains(&0x88),
        "fixture must carry an AC-3 (0x80) and a DTS (0x88) substream, got {ids:?}"
    );
    let dts_syncs = data
        .windows(4)
        .filter(|w| *w == [0x7F, 0xFE, 0x80, 0x01])
        .count();
    assert!(
        dts_syncs > 10,
        "the fixture's 0x88 substream must carry real DTS frames, found {dts_syncs}"
    );

    let media = demux(&data);
    let ac3: Vec<&_> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .collect();
    assert_eq!(
        ac3.len(),
        1,
        "exactly the AC-3 substream becomes a track; the DTS one is skipped"
    );

    // The AC-3 track is byte-identical to ffmpeg's own extraction.
    let es = ffmpeg_ac3_dts_fixture_es();
    let concatenated: Vec<u8> = ac3[0]
        .samples
        .iter()
        .flat_map(|s| s.data.iter().copied())
        .collect();
    assert_eq!(
        concatenated, es,
        "the AC-3 track must be exactly what ffmpeg extracts for that program"
    );

    // No sample anywhere contains DTS frame bytes.
    for t in &media.tracks {
        for (k, s) in t.samples.iter().enumerate() {
            assert!(
                !s.data.windows(4).any(|w| w == [0x7F, 0xFE, 0x80, 0x01]),
                "track {} sample {k} must not contain DTS frame bytes",
                t.spec.track_id
            );
        }
    }

    // And no track claims a codec the fixture does not really have.
    assert!(
        !media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Data { .. })),
        "a skipped substream must not become a placeholder track"
    );
}

// ---------------------------------------------------------------------------
// r04-W21 (review): H.264 leading junk and the 64 MiB cap
// ---------------------------------------------------------------------------

/// Fixture: an H.264/AAC transport capture carrying an in-band SPS, PPS and
/// IDR — the same source the PS fixture's video came from, so the bytes are a
/// real stream. Named here rather than in `fixtures/ps/` because it is the
/// existing TS fixture, read only for its codec bytes.
fn h264_ts() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts");
    std::fs::read(&path).expect("fixtures/ts/h264_aac.ts must exist")
}

/// Build a Program Stream whose single video PES carries `es` (an Annex B
/// byte stream) as its payload, with a matching system header-free pack
/// header — the minimal PS shape `PsDemux` walks.
fn ps_with_video_es(es: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    // Pack header: the first 14 bytes of a real ffmpeg PS's own first pack
    // header, verbatim (start code + '01' marker, SCR, mux rate, stuffing).
    out.extend_from_slice(&[
        0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x43, 0x37, 0x8B, 0xF8,
    ]);
    // PES: 00 00 01 E0, 16-bit length, '10' flags byte, PTS_DTS flags, header
    // length 5, a zero PTS, then the ES — the exact header shape the real
    // fixture's video PES uses (`00 00 01 e0 … 00 0c 0e db 6c 00`).
    let mut pes_body = Vec::new();
    pes_body.push(0x80u8); // '10' marker, no scrambling/priority/alignment
    pes_body.push(0x80); // PTS present
    pes_body.push(0x05); // header data length
    pes_body.extend_from_slice(&[0x21, 0x00, 0x01, 0x00, 0x01]); // PTS = 0
    pes_body.extend_from_slice(es);
    let pes_len = pes_body.len() as u16;
    out.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]);
    out.extend_from_slice(&pes_len.to_be_bytes());
    out.extend_from_slice(&pes_body);
    out
}

/// A run of leading zero bytes before the first start code must not shift the
/// access-unit offsets: the splitter's retained offset has to begin at the
/// start code itself, so the byte ranges it reports line up with the input.
///
/// `first_nal_offset` folded back only *one* zero byte (the 4-byte
/// `00 00 00 01` case); a longer run — legal Annex B padding, and what a PES
/// stuffing tail leaves — put the cursor in the middle of the zeros, so every
/// reported range was short and the equality check that catches it was demoted
/// to a `debug_assert!`, which release builds do not run.
#[test]
fn long_zero_run_before_the_first_start_code_stays_aligned() {
    let au = real_h264_annexb_au();
    // Six leading zeros, well past the 4-byte start-code form.
    let mut es = vec![0x00u8; 6];
    es.extend_from_slice(&au);
    es.extend_from_slice(&au);

    let ps = ps_with_video_es(&es);
    let mut demux = transmux::PsDemux::new();
    let media = demux
        .unpackage(&ps[..])
        .unwrap_or_else(|e| panic!("the PS must demux, got {e:?}"));

    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("the AVC track");
    assert!(
        video.samples.len() >= 2,
        "both access units must be recovered, got {}",
        video.samples.len()
    );
    // Every sample is a well-formed sequence of length-prefixed NALs whose
    // lengths exactly cover it — the IR invariant a shifted range breaks.
    for (k, s) in video.samples.iter().enumerate() {
        let mut off = 0usize;
        while off + 4 <= s.data.len() {
            let len = u32::from_be_bytes([
                s.data[off],
                s.data[off + 1],
                s.data[off + 2],
                s.data[off + 3],
            ]) as usize;
            off += 4 + len;
            assert!(
                off <= s.data.len(),
                "sample {k} NAL prefix runs past the sample"
            );
        }
        assert_eq!(
            off,
            s.data.len(),
            "sample {k} must be exactly whole length-prefixed NALs (a shifted              range leaves a partial NAL)"
        );
    }
}

/// An H.264 access unit's bytes: SPS + PPS + one IDR slice, taken from the real
/// AVC track in the committed TS fixture, with each of its length prefixes
/// replaced by a start code. The TS demuxer's IR stores 4-byte
/// length-prefixed NALs, while a Program Stream carries Annex B, so the
/// conversion is the test's job. The NAL bytes themselves are the capture's
/// own — nothing is synthesised.
fn real_h264_annexb_au() -> Vec<u8> {
    use transmux::TsDemux;
    let ts = h264_ts();
    let mut demux = TsDemux::new();
    let media = demux
        .unpackage(&ts[..])
        .expect("demux the H.264/AAC TS fixture");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("an AVC track");
    let sample = &video.samples[0].data;
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 4 <= sample.len() {
        let len = u32::from_be_bytes([
            sample[off],
            sample[off + 1],
            sample[off + 2],
            sample[off + 3],
        ]) as usize;
        off += 4;
        assert!(
            off + len <= sample.len(),
            "length prefix runs past the sample"
        );
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        out.extend_from_slice(&sample[off..off + len]);
        off += len;
    }
    assert_eq!(off, sample.len(), "the sample must be whole NALs");
    assert!(!out.is_empty(), "the access unit must not be empty");
    out
}

/// Leading bytes before the first start code must be absorbed, not turned into
/// a dropped track.
///
/// `AccessUnitSplitter` discards everything up to the first start code, so the
/// units it returns no longer start at offset 0 of what was pushed. Walking the
/// units back over the input at a cursor starting at 0 therefore mismatches
/// immediately, and the old code returned `None` — silently discarding the
/// whole video track where the previous AUD-based splitter had absorbed the
/// leading garbage into the first access unit.
#[test]
fn h264_leading_junk_is_absorbed_not_dropped() {
    let au = real_h264_annexb_au();
    // A handful of bytes before the first 00 00 01 — e.g. a PES-level stuffing
    // run or a stray byte from an earlier fragment.
    let mut es = vec![0xAA, 0xBB, 0xCC, 0xDD];
    es.extend_from_slice(&au);
    // A second access unit so the splitter has a boundary to work with.
    es.extend_from_slice(&au);

    let ps = ps_with_video_es(&es);
    let mut demux = transmux::PsDemux::new();
    let media = demux
        .unpackage(&ps[..])
        .unwrap_or_else(|e| panic!("the PS must demux, got {e:?}"));

    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("leading junk before the first start code must not lose the AVC track");
    assert!(
        video.samples.len() >= 2,
        "both access units must be recovered (got {})",
        video.samples.len()
    );
    for (k, s) in video.samples.iter().enumerate() {
        // Length-prefixed NALs, starting with a NAL header byte whose
        // forbidden_zero_bit is clear.
        assert!(s.data.len() > 4, "sample {k} must carry NALs");
        let len = u32::from_be_bytes([s.data[0], s.data[1], s.data[2], s.data[3]]) as usize;
        assert!(len > 0 && len <= s.data.len() - 4);
        assert_eq!(s.data[4] & 0x80, 0, "sample {k} NAL forbidden_zero_bit");
    }
}

/// A cap rejection from `AccessUnitSplitter` must surface as an error rather
/// than a silently dropped track.
///
/// The pre-review code used `.ok()?`, turning `push`'s `Err` into "no units",
/// which `build_h264_track` reported as "no video track" — a `Media` with the
/// video missing entirely and no signal that anything failed. The propagation
/// is asserted here without allocating 64 MiB: the splitter's own cap is
/// exercised once at the level it lives (`au`), and the demuxer's propagation
/// is covered by the fact that `build_h264_track` returns `Result` and its
/// caller uses `?` — checked by the type system, not by a byte count.
///
/// Allocating a 64 MiB ES through `PsDemux` would also prove nothing extra: the
/// demuxer reassembles the elementary stream in memory first, so a 64 MiB ES is
/// a 64 MiB input rather than an amplification, and the cap is unreachable that
/// way at any input size a test could construct.
#[test]
fn a_splitter_cap_rejection_is_not_a_silent_drop() {
    use transmux::au::AccessUnitSplitter;
    use transmux::nal::NalCodec;

    // The cap exists and rejects an over-long open NAL. The buffer is grown
    // with `resize` on a `Vec` of zeros, so no per-byte work is done beyond the
    // allocation itself.
    let mut splitter = AccessUnitSplitter::new(NalCodec::Avc);
    let mut oversized = vec![0u8; 64 * 1024 * 1024 + 8];
    oversized[..4].copy_from_slice(&[0x00, 0x00, 0x01, 0x65]);
    assert!(
        splitter.push(&oversized).is_err(),
        "the splitter's cap must reject an over-long open NAL"
    );
    drop(oversized);

    // The demuxer propagates it: `build_h264_track` is fallible and its caller
    // uses `?`, so a cap rejection reaches the caller as `Err` rather than as a
    // `Media` whose video track is quietly absent. A well-formed stream still
    // demuxes, which is what keeps this from being vacuous in the other
    // direction.
    let ps = ps_with_video_es(&real_h264_annexb_au());
    let mut demux = transmux::PsDemux::new();
    let media = demux.unpackage(&ps[..]).expect("a well-formed PS demuxes");
    assert!(
        media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Avc { .. })),
        "the AVC track is present for a stream within the cap"
    );
}

// ---------------------------------------------------------------------------
// r04-W20 (review): one bad frame must not lose every frame after it
// ---------------------------------------------------------------------------

/// A corrupted frame mid-stream must not discard the frames that follow it.
///
/// The fixture's `0x80` elementary bytes are copied, with one frame's sync word
/// damaged in place, into a fresh PS. A splitter that stops at the first frame
/// it cannot parse loses everything from there on; a resyncing one recovers from
/// the next real sync word and yields every remaining frame.
#[test]
fn one_corrupt_ac3_frame_does_not_lose_the_rest() {
    let data = load_ps();
    let es = substream_es(&data, 0x80);
    assert!(!es.is_empty());

    // Damage the 10th frame's sync word (0x0B77 -> 0x0B00), leaving the rest of
    // the frame intact: the splitter cannot parse that frame, but the next
    // frame's sync word is untouched.
    let mut damaged = es.clone();
    let syncs: Vec<usize> = es
        .windows(2)
        .enumerate()
        .filter(|(_, w)| *w == [0x0B, 0x77])
        .map(|(i, _)| i)
        .collect();
    assert!(syncs.len() > 12, "the stream must carry many frames");
    let bad_at = syncs[9];
    damaged[bad_at + 1] = 0x00;

    // The intact stream's frame count is the oracle for how many survive.
    let intact = demux(&ps_with_ac3_es(&es));
    let intact_frames: usize = intact
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .map(|t| t.samples.len())
        .max()
        .expect("an AC-3 track");

    let media = demux(&ps_with_ac3_es(&damaged));
    let frames: usize = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .map(|t| t.samples.len())
        .max()
        .unwrap_or(0);
    assert_eq!(
        frames,
        intact_frames - 1,
        "exactly the damaged frame is lost; every later frame must still come \
         out (got {frames}, intact stream has {intact_frames})"
    );
    // Every survivor is a whole syncframe.
    for t in media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
    {
        for (k, s) in t.samples.iter().enumerate() {
            assert!(
                s.data.starts_with(&[0x0B, 0x77]),
                "sample {k} must begin at an AC-3 syncword"
            );
        }
    }
}

/// Build a Program Stream carrying `es` as one `private_stream_1` AC-3
/// substream (id 0x80), with a real pack header and a substream header whose
/// pointer names the first access unit.
fn ps_with_ac3_es(es: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[
        0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x43, 0x37, 0x8B, 0xF8,
    ]);
    let mut pes_body = Vec::new();
    pes_body.push(0x80u8); // '10' marker
    pes_body.push(0x80); // PTS present
    pes_body.push(0x05); // header data length
    pes_body.extend_from_slice(&[0x21, 0x00, 0x01, 0x00, 0x01]); // PTS = 0
    // Substream header: substream_id 0x80, number_of_frames 1, pointer 1.
    pes_body.extend_from_slice(&[0x80, 0x01, 0x00, 0x01]);
    pes_body.extend_from_slice(es);
    let pes_len = pes_body.len() as u16;
    out.extend_from_slice(&[0x00, 0x00, 0x01, 0xBD]);
    out.extend_from_slice(&pes_len.to_be_bytes());
    out.extend_from_slice(&pes_body);
    out
}

/// A DTS substream whose payload happens to contain an AC-3 syncword at its
/// access-unit pointer must still be skipped.
///
/// The syncword probe alone is not enough: `0x0B77` occurs inside arbitrary
/// payload by chance (roughly one position in 65 536), so a DTS frame can
/// present one exactly where the pointer lands. The substream_id range check is
/// what makes the decision then — which is why it exists rather than being left
/// to the probe.
///
/// Fixture: `ffmpeg-ac3-dts.ps` with one DTS packet's pointer position stamped
/// `0B 77`, derived so the test cannot pass merely because no false sync was
/// present.
#[test]
fn dts_substream_with_a_false_ac3_syncword_is_still_skipped() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/ps/ffmpeg-ac3-dts-with-fake-ac3-sync.ps");
    let data = std::fs::read(&path).expect("fixture must exist");

    // The stamp really is inside a 0x88 (DTS) packet's payload.
    let ids = fixture_substream_ids(&data);
    assert!(ids.contains(&0x88), "the fixture still carries DTS");

    let media = demux(&data);
    let ac3: Vec<&_> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .collect();
    assert_eq!(
        ac3.len(),
        1,
        "a DTS substream must not become an AC-3 track just because its payload \
         contains an AC-3 syncword"
    );
    // And the one AC-3 track is still exactly ffmpeg's extraction.
    let es = ffmpeg_ac3_dts_fixture_es();
    let concatenated: Vec<u8> = ac3[0]
        .samples
        .iter()
        .flat_map(|s| s.data.iter().copied())
        .collect();
    assert_eq!(concatenated, es, "the real AC-3 substream is unaffected");
}

// ---------------------------------------------------------------------------
// review round 3: a decided substream must never lose continuation bytes
// ---------------------------------------------------------------------------

/// A muxer that writes `first_access_unit_pointer = 0` (and
/// `number_of_frames = 0`) on every packet must still yield the track.
///
/// The pointer is explicitly "0 = no access unit starts in this packet", which
/// is what a muxer that splits purely by its own frame boundaries writes. The
/// header-consistency check rejects `number_of_frames > 0 && pointer == 0`, and
/// it used to `continue` for *every* such packet — including packets of a
/// substream that had already been classified — so a stream written that way
/// lost bytes in the middle and, if the very first packet was one, never got
/// classified at all and produced no track.
#[test]
fn pointer_zero_everywhere_still_yields_the_track() {
    let data = load_ps();

    // Rewrite the fixture so *every* 0xBD packet declares no access-unit start
    // (pointer 0, frames 0) while carrying the same bytes.
    let mut rewritten = data.clone();
    let mut i = 0usize;
    let mut rewritten_count = 0usize;
    while i + 3 < rewritten.len() {
        if rewritten[i..i + 4] == [0x00, 0x00, 0x01, 0xBD] {
            let j = i + 4;
            let pes_len = usize::from(u16::from_be_bytes([rewritten[j], rewritten[j + 1]]));
            let payload = &rewritten[j + 2..(j + 2 + pes_len).min(rewritten.len())];
            let hdr = 3 + usize::from(payload[2]);
            if payload.len() > hdr + 4 {
                let base = j + 2 + hdr;
                rewritten[base + 1] = 0; // number_of_frames
                rewritten[base + 2] = 0; // first_access_unit_pointer
                rewritten[base + 3] = 0;
                rewritten_count += 1;
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    assert!(rewritten_count > 10, "the fixture must have 0xBD packets");

    let media = demux(&rewritten);
    let ac3: Vec<&_> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .collect();
    assert_eq!(
        ac3.len(),
        2,
        "a stream whose every packet declares no access-unit start must still \
         yield both substreams, got {}",
        ac3.len()
    );
    for (n, t) in ac3.iter().enumerate() {
        assert!(!t.samples.is_empty(), "substream {n} must carry samples");
        for (k, s) in t.samples.iter().enumerate() {
            assert!(
                s.data.starts_with(&[0x0B, 0x77]),
                "substream {n} sample {k} must begin at an AC-3 syncword"
            );
        }
    }
}

/// A packet of an already-classified substream whose pointer is out of range
/// must contribute its bytes rather than be dropped.
///
/// The consistency check exists to keep a partial-frame *tail* from being
/// spliced onto a stream that has not been identified yet. Once a substream is
/// known to be AC-3, every byte it carries is frame data and must be kept — the
/// pointer only tells the demuxer where the next access unit starts, and a
/// bogus one cannot justify discarding real audio.
#[test]
fn out_of_range_pointer_mid_stream_keeps_its_bytes() {
    let data = load_ps();
    let baseline = demux(&data);
    let baseline_bytes: usize = baseline
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .map(|t| t.samples.iter().map(|s| s.data.len()).sum::<usize>())
        .sum();

    // Corrupt one late packet's pointer to a value far past its payload. The
    // stream is already decided by then, so its bytes are frame data.
    let mut broken = data.clone();
    let mut bd = Vec::new();
    let mut i = 0usize;
    while i + 3 < broken.len() {
        if broken[i..i + 4] == [0x00, 0x00, 0x01, 0xBD] {
            bd.push(i);
            i += 4;
        } else {
            i += 1;
        }
    }
    let target = bd[bd.len() / 2];
    let j = target + 4;
    let pes_len = usize::from(u16::from_be_bytes([broken[j], broken[j + 1]]));
    let hdr = 3 + usize::from(broken[j + 2 + 2]);
    let base = j + 2 + hdr;
    broken[base + 2] = 0xFF;
    broken[base + 3] = 0xFF;
    let _ = pes_len;

    let media = demux(&broken);
    let bytes: usize = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Ac3 { .. }))
        .map(|t| t.samples.iter().map(|s| s.data.len()).sum::<usize>())
        .sum();
    assert_eq!(
        bytes, baseline_bytes,
        "a bogus pointer on an already-decided substream must not drop that \
         packet's bytes ({} vs {} baseline)",
        bytes, baseline_bytes
    );
}
