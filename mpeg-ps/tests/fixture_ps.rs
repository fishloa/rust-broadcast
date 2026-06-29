use broadcast_common::Parse;
use mpeg_ps::program_stream;
use mpeg_ps::SystemHeader;

// Fixture: ffmpeg -f lavfi -i testsrc2=duration=1:size=352x288:rate=25 -f lavfi -i sine=frequency=440:duration=1 -c:v mpeg2video -c:a mp2 -f vob -y mpeg-ps/tests/fixtures/ffmpeg-mpeg2-ps.mpg

#[test]
fn real_fixture_walk() {
    let data: &[u8] = include_bytes!("fixtures/ffmpeg-mpeg2-ps.mpg");
    let (packs, trailing) = program_stream::parse_all_packs(data).unwrap();

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
