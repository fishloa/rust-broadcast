use std::fs;

use broadcast_common::Parse;
use mpeg_ps::SystemHeader;
use mpeg_ps::program_stream;

// Fixture: ffmpeg -f lavfi -i testsrc2=duration=1:size=352x288:rate=25 -f lavfi -i sine=frequency=440:duration=1 -c:v mpeg2video -c:a mp2 -f vob -y mpeg-ps/tests/fixtures/ffmpeg-mpeg2-ps.mpg

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/mpeg-ps/ffmpeg-mpeg2-ps.mpg"
    );
    fs::read(path).expect("fixture ffmpeg-mpeg2-ps.mpg must be present")
}

#[test]
fn real_fixture_walk() {
    let data = fixture();
    let (packs, trailing) = program_stream::parse_all_packs(&data).unwrap();

    // 7 pack headers expected
    assert!(
        packs.len() >= 7,
        "expected at least 7 packs, got {}",
        packs.len()
    );
    assert!(
        trailing.is_empty(),
        "unexpected trailing bytes: {}",
        trailing.len()
    );

    // First pack must have a system header
    let sh: &SystemHeader = packs[0]
        .system_header
        .as_ref()
        .expect("first pack must have a system header");

    // Sanity checks
    assert!(sh.rate_bound > 0, "rate_bound should be positive");
    assert!(sh.audio_bound > 0, "audio_bound should be positive");
    assert!(sh.video_bound > 0, "video_bound should be positive");
    assert!(
        !sh.std_buffer_bounds.is_empty(),
        "should have P-STD buffer bounds"
    );

    let mut offset = 0usize;
    let mut scr_ticks_prev = 0u64;
    for (i, pack) in packs.iter().enumerate() {
        let ticks = pack.pack_header.scr.ticks();
        // SCR should be non-decreasing
        assert!(
            ticks >= scr_ticks_prev,
            "pack {i}: SCR went backwards ({ticks} < {scr_ticks_prev})"
        );
        scr_ticks_prev = ticks;

        // mux_rate must be non-zero
        assert!(
            pack.pack_header.program_mux_rate > 0,
            "pack {i}: mux_rate is zero"
        );

        // Oracle cross-check (issue #1049): H.222.0 requires rate_bound to be
        // an upper bound on program_mux_rate across all packs; ffmpeg emits a
        // constant-rate PS where every pack's mux_rate equals the system
        // header's rate_bound exactly.
        assert_eq!(
            pack.pack_header.program_mux_rate, sh.rate_bound,
            "pack {i}: program_mux_rate must match system_header.rate_bound"
        );

        // Byte-exact round-trip each pack header
        let orig_bytes = &data[offset..offset + pack.pack_header.serialized_len()];
        let mut round = vec![0u8; pack.pack_header.serialized_len()];
        pack.pack_header.serialize_into(&mut round).unwrap();
        assert_eq!(
            &round[..],
            orig_bytes,
            "pack {i}: pack_header round-trip mismatch"
        );

        offset += pack.pack_header.serialized_len()
            + pack
                .system_header
                .as_ref()
                .map_or(0, |sh| sh.serialized_len())
            + pack
                .pes_packets
                .iter()
                .map(|p| p.serialized_len())
                .sum::<usize>();
    }

    // Byte-exact round-trip the system header
    {
        let sh_offset = packs[0].pack_header.serialized_len();
        let sh = packs[0].system_header.as_ref().unwrap();
        let sh_len = sh.serialized_len();
        let orig_sh = &data[sh_offset..sh_offset + sh_len];
        let mut round = vec![0u8; sh_len];
        sh.serialize_into(&mut round).unwrap();
        assert_eq!(&round[..], orig_sh, "system_header round-trip mismatch");
    }

    // Verify the system header parses from the fixture bytes
    {
        let mut sh_offset = 0usize;
        for p in &packs {
            sh_offset += p.pack_header.serialized_len();
            if p.system_header.is_some() {
                break;
            }
        }
        let sh_bytes = &data[sh_offset..];
        let parsed = SystemHeader::parse(sh_bytes).unwrap();
        assert_eq!(&parsed, packs[0].system_header.as_ref().unwrap());
    }
}

use broadcast_common::Serialize;
use mpeg_ps::ProgramStreamMap;

/// Build a PSM per Table 2-41 and byte-exact round-trip.
#[test]
fn psm_unit_round_trip() {
    use mpeg_ps::EsMapEntry;

    let entries = vec![EsMapEntry {
        stream_type: 0x02, // MPEG-2 video
        elementary_stream_id: 0xE0,
        stream_id_extension: None,
        descriptors: &[0x0A, 0x04, b'H', b'E', b'L', b'L'],
    }];

    let psm = ProgramStreamMap {
        current_next_indicator: true,
        single_extension_stream_flag: false,
        version: 3,
        program_stream_info: &[],
        elementary_stream_map: entries,
        crc: 0,
    };

    let mut buf = vec![0u8; psm.serialized_len()];
    psm.serialize_into(&mut buf).unwrap();

    let parsed = ProgramStreamMap::parse(&buf).unwrap();
    assert!(parsed.current_next_indicator);
    assert_eq!(parsed.version, 3);
    assert_eq!(parsed.elementary_stream_map.len(), 1);
    assert_eq!(
        parsed.elementary_stream_map[0].descriptors,
        &[0x0A, 0x04, b'H', b'E', b'L', b'L'],
    );

    // Byte-exact round-trip
    let mut out2 = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut out2).unwrap();
    assert_eq!(&out2[..], &buf[..]);
}

/// Byte offsets of each pack start in the fixture, via the length-driven walk.
fn pack_offsets(data: &[u8]) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut pos = 0usize;
    while data.len() - pos >= 4 {
        let (pack, consumed) = program_stream::parse_pack(&data[pos..]).unwrap();
        if pack.is_none() {
            break;
        }
        offsets.push(pos);
        pos += consumed;
    }
    offsets
}

/// r14-MPS-W3 (deferred part): after a corrupt pack the walker resynchronises
/// on the next `pack_start_code` instead of dropping the rest of the stream,
/// and reports exactly what it skipped. Corruption 1: a destroyed pack start
/// code. Corruption 2: a broken PES start code after a good pack header.
#[test]
fn scan_packs_resyncs_after_a_corrupt_pack_and_reports_it() {
    let clean = fixture();
    let offsets = pack_offsets(&clean);
    assert!(offsets.len() >= 7);
    let (clean_packs, _) = program_stream::parse_all_packs(&clean).unwrap();

    // A clean stream scans identically to parse_all_packs, nothing skipped.
    let scan = program_stream::scan_packs(&clean);
    assert_eq!(scan.packs.len(), clean_packs.len());
    assert!(scan.skipped.is_empty());

    // 1. Destroy pack #2's start code (its first byte 0x00 -> 0xFF, so the
    //    preceding pack's PES loop also stops there).
    let mut bad = clean.clone();
    bad[offsets[2]] = 0xFF;
    assert!(program_stream::parse_all_packs(&bad).is_err());
    let scan = program_stream::scan_packs(&bad);
    assert_eq!(scan.packs.len(), clean_packs.len() - 1);
    assert_eq!(scan.skipped.len(), 1);
    assert_eq!(scan.skipped[0].offset, offsets[2]);
    assert_eq!(scan.skipped[0].len, offsets[3] - offsets[2]);
    assert!(matches!(
        scan.skipped[0].error,
        mpeg_ps::Error::BadPackStartCode(0xFF00_01BA)
    ));

    // 2. Break the first PES start code inside pack #4: the pack's PES loop
    //    stops at the first non-PES byte, the pack keeps its header, and the
    //    leftover span up to pack #5 is the skipped region.
    let hdr_len = clean_packs[4].pack_header.header_len();
    let pes_at = offsets[4] + hdr_len;
    assert_eq!(&clean[pes_at..pes_at + 3], &[0x00, 0x00, 0x01]);
    let mut bad2 = clean.clone();
    bad2[pes_at] = 0xFF;
    let scan = program_stream::scan_packs(&bad2);
    assert_eq!(scan.packs.len(), clean_packs.len());
    assert!(scan.packs[4].pes_packets.is_empty());
    assert_eq!(scan.skipped.len(), 1);
    assert_eq!(scan.skipped[0].offset, pes_at);
    assert_eq!(scan.skipped[0].len, offsets[5] - pes_at);

    // Pure garbage: no pack start anywhere -> one skipped region, no packs.
    let scan = program_stream::scan_packs(&[0x55; 64]);
    assert!(scan.packs.is_empty());
    assert_eq!(scan.skipped.len(), 1);
    assert_eq!(scan.skipped[0].len, 64);
    assert!(scan.remaining.is_empty());
}
