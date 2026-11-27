//! Tier 5 of issue #4167: hint tracks and file delivery.
//!
//! No installed tool writes `hinf`, the MPEG-2 TS hint entries, the reception
//! extras or the FLUTE file-delivery boxes (see `PROVENANCE.md`), so the bytes
//! here are assembled from the clause and labelled as such. The boxes a
//! committed fixture does carry (`hnti`, `rtp `, `sdp `, `tims`, `tsro`,
//! `snro`, `btrt`) are also asserted against those files in
//! `tier5_fixtures.rs`.

use broadcast_core::{Parse, Serialize};
use isobmff::file_delivery::{
    BlockPartition, ExtraDataBox, FdConstructor, FdItemInformationBox, FdPacketBox, FdSample,
    FdSessionGroupBox, FecInformationBox, FecReservoirBox, FilePartitionBox, FileReservoirBox,
    GroupIdToNameBox, GroupName, LctHeaderExtension, LctHeaderTemplate, PartitionEntryBox,
    ReservoirEntry, SessionGroup,
};
use isobmff::hint::{
    FdHintEntryChild, FdHintSampleEntry, HintInformationBox, HintInformationChild, HintSampleEntry,
    HintStatisticsBox, HintStatisticsChild, InitialSampleTimeBox, Mpeg2TsEntryChild,
    Mpeg2TsHintSampleEntry, PatBox, PmtBox, ReceivedCsrcBox, ReceivedSsrcBox, RtpConstructor,
    RtpEntryChild, RtpHdrExtBox, RtpHintInformation, RtpPacket, RtpSample, RtpSampleEntry,
    SdpHintInformation, SequenceOffsetBox, SrtpProcessBox, SrtpProcessChild, TimeOffsetBox,
    TimescaleEntryBox, TimestampSynchronyBox, TsTimingBox,
};
use isobmff::init_segment::{SampleEntryVariant, sample_entry_variant};
use isobmff::item::ItemInfoEntry;
use isobmff::movie_fragment::RedundantSampleOriginalTimingBox;
use isobmff::sample_entries::BitRateBox;
use isobmff::sample_tables_extra::{ShadowSyncEntry, ShadowSyncSampleBox};

fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

fn full(version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.push(version);
    out.extend_from_slice(&flags.to_be_bytes()[1..]);
    out.extend_from_slice(body);
    out
}

#[test]
fn hint_information_movie_and_track_round_trip() {
    let movie_rtp = boxed(*b"rtp ", b"sdp b=AS:141\r\nm=video 0 RTP/AVP 96\r\n");
    let movie_hnti = boxed(*b"hnti", &movie_rtp);
    let parsed = HintInformationBox::parse(&movie_hnti).expect("hnti");
    let rtp = parsed.rtp().expect("movie rtp");
    assert_eq!(rtp.description_format(), *b"sdp ");
    assert_eq!(rtp.sdp_text(), b"b=AS:141\r\nm=video 0 RTP/AVP 96\r\n");
    assert_eq!(parsed.try_to_bytes().expect("hnti bytes"), movie_hnti);

    let track_sdp = boxed(
        *b"sdp ",
        b"m=audio 0 RTP/AVP 97\r\na=rtpmap:97 mpeg4-generic/44100\r\n",
    );
    let track_hnti = boxed(*b"hnti", &track_sdp);
    let parsed = HintInformationBox::parse(&track_hnti).expect("track hnti");
    assert!(parsed.rtp().is_none());
    assert_eq!(
        parsed.sdp().expect("track sdp").sdp_text(),
        b"m=audio 0 RTP/AVP 97\r\na=rtpmap:97 mpeg4-generic/44100\r\n"
    );
    assert_eq!(parsed.try_to_bytes().expect("hnti bytes"), track_hnti);

    let mut built = HintInformationBox::new();
    built.push(HintInformationChild::Rtp(RtpHintInformation::new(
        *b"sdp ",
        b"b=AS:20\r\n",
    )));
    built.push(HintInformationChild::Sdp(SdpHintInformation::new(
        b"m=text 0 RTP/AVP 98\r\n",
    )));
    let bytes = built.try_to_bytes().expect("built hnti");
    let reparsed = HintInformationBox::parse(&bytes).expect("built hnti parses");
    assert_eq!(reparsed.rtp().expect("rtp").sdp_text(), b"b=AS:20\r\n");
    assert_eq!(
        reparsed.sdp().expect("sdp").sdp_text(),
        b"m=text 0 RTP/AVP 98\r\n"
    );
    assert_eq!(reparsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn hint_statistics_children_round_trip() {
    let payt = {
        let mut body = 96u32.to_be_bytes().to_vec();
        body.push(18);
        body.extend_from_slice(b"MP4V-ES/90000\0\0\0\0\0");
        boxed(*b"payt", &full(0, 0, &body))
    };
    let children = [
        boxed(*b"trpy", &full(0, 0, &12_345u64.to_be_bytes())),
        boxed(*b"nump", &full(0, 0, &99u64.to_be_bytes())),
        boxed(*b"tpyl", &full(0, 0, &10_000u64.to_be_bytes())),
        boxed(*b"totl", &full(0, 0, &9_999u32.to_be_bytes())),
        boxed(*b"npck", &full(0, 0, &77u32.to_be_bytes())),
        boxed(*b"tpay", &full(0, 0, &8_888u32.to_be_bytes())),
        boxed(
            *b"maxr",
            &full(
                0,
                0,
                &[1_000u32.to_be_bytes(), 50_000u32.to_be_bytes()].concat(),
            ),
        ),
        boxed(*b"dmed", &full(0, 0, &7_000u64.to_be_bytes())),
        boxed(*b"dimm", &full(0, 0, &600u64.to_be_bytes())),
        boxed(*b"drep", &full(0, 0, &500u64.to_be_bytes())),
        boxed(*b"tmin", &full(0, 0, &(-5i32).to_be_bytes())),
        boxed(*b"tmax", &full(0, 0, &250i32.to_be_bytes())),
        boxed(*b"pmax", &full(0, 0, &1_500u32.to_be_bytes())),
        boxed(*b"dmax", &full(0, 0, &320u32.to_be_bytes())),
        payt,
    ];
    let body: Vec<u8> = children.concat();
    let bytes = boxed(*b"hinf", &body);
    let parsed = HintStatisticsBox::parse(&bytes).expect("hinf");
    assert_eq!(parsed.children().len(), 15);
    assert_eq!(parsed.bytes_sent(), Some(12_345));
    assert_eq!(parsed.packets_sent(), Some(99));
    assert_eq!(parsed.largest_packet(), Some(1_500));
    assert!(matches!(
        parsed.children()[10],
        HintStatisticsChild::Tmin(b) if b.time() == -5
    ));
    assert!(matches!(
        parsed.children()[11],
        HintStatisticsChild::Tmax(b) if b.time() == 250
    ));
    let payt = parsed
        .children()
        .iter()
        .find_map(|child| match child {
            HintStatisticsChild::Payt(b) => Some(b),
            _ => None,
        })
        .expect("payt");
    assert_eq!(payt.payload_id(), 96);
    assert_eq!(&payt.rtpmap_string()[..8], b"MP4V-ES/");
    assert_eq!(parsed.try_to_bytes().expect("hinf bytes"), bytes);
}

#[test]
fn srtp_process_and_entry_children_round_trip() {
    let algorithms = [
        u32::from_be_bytes(*b"ACM1"),
        u32::from_be_bytes(*b"ACM1"),
        u32::from_be_bytes(*b"SHM2"),
        u32::from_be_bytes(*b"SHM2"),
    ];
    let mut srpp = SrtpProcessBox::new(algorithms[0], algorithms[1], algorithms[2], algorithms[3]);
    srpp.push(SrtpProcessChild::SchemeType(isobmff::cenc::SchemeTypeBox {
        version: 0,
        flags: 0,
        scheme_type: *b"srtp",
        scheme_version: 1,
        scheme_uri: None,
    }));
    srpp.push(SrtpProcessChild::SchemeInformation(
        isobmff::cenc::SchemeInformationBox {
            tenc: None,
            stvi: None,
            spki: None,
            extra_boxes: Vec::new(),
        },
    ));
    let bytes = srpp.try_to_bytes().expect("srpp bytes");
    let parsed = SrtpProcessBox::parse(&bytes).expect("srpp parses");
    assert_eq!(parsed.encryption_algorithm_rtp(), algorithms[0]);
    assert_eq!(parsed.integrity_algorithm_rtcp(), algorithms[3]);
    assert_eq!(parsed.scheme_type().expect("schm").scheme_type, *b"srtp");
    assert!(parsed.scheme_information().is_some());
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);

    let entry = RtpSampleEntry::new(*b"srtp", 1200);
    let mut entry = entry;
    entry.push(RtpEntryChild::Timescale(TimescaleEntryBox::new(90_000)));
    entry.push(RtpEntryChild::SrtpProcess(parsed));
    let entry_bytes = entry.try_to_bytes().expect("srtp entry");
    let SampleEntryVariant::Hint(entry) =
        sample_entry_variant(&entry_bytes).expect("srtp entry parses")
    else {
        panic!("expected an rtp-family hint entry");
    };
    let HintSampleEntry::Rtp(reparsed) = *entry else {
        panic!("expected an rtp-family hint entry");
    };
    assert_eq!(reparsed.format(), *b"srtp");
    assert_eq!(reparsed.max_packet_size(), 1200);
    assert_eq!(reparsed.timescale(), Some(90_000));
    assert!(reparsed.srtp_process().is_some());
    assert_eq!(reparsed.try_to_bytes().expect("bytes"), entry_bytes);
}

#[test]
fn rtp_offset_and_reception_children_round_trip() {
    assert_eq!(
        TimeOffsetBox::parse(&boxed(*b"tsro", &(-90_000i32).to_be_bytes()))
            .expect("tsro")
            .offset(),
        -90_000
    );
    assert_eq!(
        SequenceOffsetBox::parse(&boxed(*b"snro", &7i32.to_be_bytes()))
            .expect("snro")
            .offset(),
        7
    );
    let tssy = boxed(*b"tssy", &[0b1010_0010]);
    let parsed = TimestampSynchronyBox::parse(&tssy).expect("tssy");
    assert_eq!(parsed.reserved(), 0b10_1000);
    assert_eq!(parsed.timestamp_sync(), 2);
    assert_eq!(parsed.try_to_bytes().expect("tssy"), tssy);

    let rssr = boxed(*b"rssr", &0xDEAD_BEEFu32.to_be_bytes());
    assert_eq!(
        ReceivedSsrcBox::parse(&rssr).expect("rssr").ssrc(),
        0xDEAD_BEEF
    );
    assert_eq!(
        ReceivedSsrcBox::parse(&rssr)
            .expect("rssr")
            .try_to_bytes()
            .expect("bytes"),
        rssr
    );

    let rtpx = boxed(*b"rtpx", &[1, 2, 3, 4, 5]);
    assert_eq!(
        RtpHdrExtBox::parse(&rtpx).expect("rtpx").data(),
        &[1, 2, 3, 4, 5]
    );

    let rcsr = boxed(
        *b"rcsr",
        &[11u32.to_be_bytes(), 22u32.to_be_bytes()].concat(),
    );
    let parsed = ReceivedCsrcBox::parse(&rcsr).expect("rcsr");
    assert_eq!(parsed.csrc(), &[11, 22]);
    assert_eq!(parsed.try_to_bytes().expect("rcsr"), rcsr);
}

#[test]
fn mpeg2ts_hint_entry_and_ts_boxes_round_trip() {
    let pat = PatBox::new(0, &[0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00]);
    let pmt = PmtBox::new(0x1000, &[0x02, 0xB0, 0x12]);
    let timing = TsTimingBox::new(1, 0x0100);
    let initial = InitialSampleTimeBox::new(900_000);
    assert_eq!(pat.pid(), 0);
    assert_eq!(pmt.pid(), 0x1000);
    assert_eq!(timing.timing_derivation_method(), 1);
    assert_eq!(timing.pid(), 0x0100);
    assert_eq!(initial.initial_sample_time(), 900_000);

    let mut entry = Mpeg2TsHintSampleEntry::new(*b"sm2t");
    entry.push(Mpeg2TsEntryChild::Pat(pat));
    entry.push(Mpeg2TsEntryChild::Pmt(pmt));
    entry.push(Mpeg2TsEntryChild::TsTiming(timing));
    entry.push(Mpeg2TsEntryChild::InitialSampleTime(initial));
    let bytes = entry.try_to_bytes().expect("sm2t entry");
    let SampleEntryVariant::Hint(entry) = sample_entry_variant(&bytes).expect("sm2t parses") else {
        panic!("expected an MPEG-2 TS hint entry");
    };
    let HintSampleEntry::Mpeg2Ts(parsed) = *entry else {
        panic!("expected an MPEG-2 TS hint entry");
    };
    assert_eq!(parsed.format(), *b"sm2t");
    assert_eq!(parsed.pat().expect("tPAT").section_data().len(), 8);
    assert_eq!(parsed.pmts().len(), 1);
    assert_eq!(parsed.ts_timing().expect("tsti").reserved(), 0);
    assert_eq!(parsed.initial_sample_time().expect("istm").reserved(), 0);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn rtp_sample_constructors_round_trip() {
    let mut packet = RtpPacket::new(-300, 96, 0x1234, 2);
    packet.push_constructor(RtpConstructor::Noop { pad: [0u8; 15] });
    packet.push_constructor(RtpConstructor::Immediate {
        count: 3,
        data: {
            let mut data = [0u8; 14];
            data[..3].copy_from_slice(&[0xAA, 0xBB, 0xCC]);
            data
        },
    });
    packet.push_constructor(RtpConstructor::Sample {
        track_ref_index: 1,
        length: 188,
        sample_number: 42,
        sample_offset: 4,
        bytes_per_block: 1,
        samples_per_block: 1,
    });
    packet.push_constructor(RtpConstructor::SampleDescription {
        track_ref_index: 0,
        length: 16,
        sample_description_index: 1,
        sample_description_offset: 8,
        reserved: 0xDEAD_BEEF,
    });

    let mut sample = RtpSample::new();
    sample.push_packet(packet);
    let bytes = sample.try_to_bytes().expect("RTP sample bytes");
    let parsed = RtpSample::parse(&bytes).expect("RTP sample parses");
    assert_eq!(parsed.reserved(), 0);
    assert_eq!(parsed.packets().len(), 1);
    let packet = &parsed.packets()[0];
    assert_eq!(packet.relative_time(), -300);
    assert_eq!(packet.payload_type(), 96);
    assert_eq!(packet.sequence_seed(), 0x1234);
    assert_eq!(packet.csrc_count(), 2);
    assert_eq!(packet.constructors().len(), 4);
    assert!(matches!(
        packet.constructors()[2],
        RtpConstructor::Sample {
            length: 188,
            sample_number: 42,
            ..
        }
    ));
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);

    // A packet with extra information and its flags set, parsed from the wire.
    let mut raw_packet = Vec::new();
    raw_packet.extend_from_slice(&0i32.to_be_bytes());
    raw_packet.push(0b1001_0010);
    raw_packet.push(0x80 | 96);
    raw_packet.extend_from_slice(&0xBEEFu16.to_be_bytes());
    raw_packet.extend_from_slice(&(((5u16 << 3) | 0b111).to_be_bytes()));
    raw_packet.extend_from_slice(&0u16.to_be_bytes());
    let extra = [0xAAu8, 0xBB, 0xCC, 0xDD, 0, 0, 0, 0];
    raw_packet.extend_from_slice(&((4 + extra.len()) as u32).to_be_bytes());
    raw_packet.extend_from_slice(&extra);
    let mut raw_sample = Vec::new();
    raw_sample.extend_from_slice(&1u16.to_be_bytes());
    raw_sample.extend_from_slice(&0u16.to_be_bytes());
    raw_sample.extend_from_slice(&raw_packet);
    raw_sample.extend_from_slice(&[0xEE, 0xFF]);
    let parsed = RtpSample::parse(&raw_sample).expect("raw RTP sample");
    let packet = &parsed.packets()[0];
    assert!(packet.extension());
    assert!(packet.marker());
    assert!(packet.extra_flag());
    assert!(packet.bframe_flag());
    assert!(packet.repeat_flag());
    assert_eq!(packet.reserved(), 5);
    assert_eq!(packet.extra_information(), &extra[..]);
    assert_eq!(parsed.extra_data(), &[0xEE, 0xFF]);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), raw_sample);
}

#[test]
fn file_delivery_boxes_round_trip() {
    let mut fpar = FilePartitionBox::new(1, 1024)
        .with_fec(0, 0)
        .with_symbols(128, 512, 64);
    fpar.push_entry(BlockPartition::new(2, 4096));
    fpar.push_entry(BlockPartition::new(1, 1024));
    let fpar_bytes = fpar.try_to_bytes().expect("fpar");
    let parsed = FilePartitionBox::parse(&fpar_bytes).expect("fpar parses");
    assert_eq!(parsed.item_id(), 1);
    assert_eq!(parsed.packet_payload_size(), 1024);
    assert_eq!(parsed.max_source_block_length(), 128);
    assert_eq!(parsed.encoding_symbol_length(), 512);
    assert_eq!(parsed.entries().len(), 2);
    assert_eq!(parsed.entries()[0].block_size(), 4096);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), fpar_bytes);

    let mut fecr = FecReservoirBox::new();
    fecr.push_entry(ReservoirEntry::new(10, 20));
    let mut fire = FileReservoirBox::new();
    fire.push_entry(ReservoirEntry::new(11, 21));
    assert_eq!(fecr.entries()[0].item_id(), 10);
    assert_eq!(fire.entries()[0].symbol_count(), 21);
    let fecr_bytes = fecr.try_to_bytes().expect("fecr");
    let fire_bytes = fire.try_to_bytes().expect("fire");
    assert_eq!(FecReservoirBox::parse(&fecr_bytes).expect("fecr"), fecr);
    assert_eq!(FileReservoirBox::parse(&fire_bytes).expect("fire"), fire);

    let mut partition = PartitionEntryBox::new(fpar).with_fec_reservoir(fecr);
    partition = partition.with_file_reservoir(fire);
    let partition_bytes = partition.try_to_bytes().expect("paen");
    let parsed = PartitionEntryBox::parse(&partition_bytes).expect("paen");
    assert_eq!(parsed.file_partition().entries().len(), 2);
    assert!(parsed.fec_reservoir().is_some());
    assert!(parsed.file_reservoir().is_some());
    assert_eq!(parsed.try_to_bytes().expect("bytes"), partition_bytes);

    let mut segr = FdSessionGroupBox::new();
    segr.push_session(SessionGroup::new(vec![1, 2], vec![65_536, 65_537]));
    let segr_bytes = segr.try_to_bytes().expect("segr");
    assert_eq!(
        FdSessionGroupBox::parse(&segr_bytes)
            .expect("segr")
            .sessions()[0]
            .hint_track_ids(),
        &[65_536, 65_537]
    );

    let mut gitn = GroupIdToNameBox::new();
    gitn.push_name(GroupName::new(1, b"news"));
    gitn.push_name(GroupName::new(2, b"sports"));
    let gitn_bytes = gitn.try_to_bytes().expect("gitn");
    let parsed = GroupIdToNameBox::parse(&gitn_bytes).expect("gitn");
    assert_eq!(parsed.names()[1].name_str(), Some("sports"));
    assert_eq!(parsed.try_to_bytes().expect("bytes"), gitn_bytes);

    let mut fiin = FdItemInformationBox::new();
    fiin.push_partition(parsed_partition(&partition_bytes));
    fiin.set_session_info(FdSessionGroupBox::parse(&segr_bytes).expect("segr"));
    fiin.set_group_id_to_name(GroupIdToNameBox::parse(&gitn_bytes).expect("gitn"));
    let fiin_bytes = fiin.try_to_bytes().expect("fiin");
    let parsed = FdItemInformationBox::parse(&fiin_bytes).expect("fiin");
    assert_eq!(parsed.partition_entries().len(), 1);
    assert!(parsed.session_info().is_some());
    assert_eq!(
        parsed.group_id_to_name().expect("gitn").names()[0].name_str(),
        Some("news")
    );
    assert_eq!(parsed.try_to_bytes().expect("bytes"), fiin_bytes);
}

fn parsed_partition(bytes: &[u8]) -> PartitionEntryBox {
    PartitionEntryBox::parse(bytes).expect("paen")
}

#[test]
fn file_partition_v1_uses_wide_identifiers() {
    let mut fpar = FilePartitionBox::new(70_000, 1024).with_version(1);
    fpar = fpar.with_fec(1, 0).with_symbols(64, 256, 32);
    fpar.push_entry(BlockPartition::new(1, 256));
    let bytes = fpar.try_to_bytes().expect("fpar v1");
    let parsed = FilePartitionBox::parse(&bytes).expect("fpar v1");
    assert_eq!(parsed.version(), 1);
    assert_eq!(parsed.item_id(), 70_000);
    assert_eq!(parsed.entries().len(), 1);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), bytes);
}

#[test]
fn fd_sample_and_packets_round_trip() {
    let mut packet = FdPacketBox::new(LctHeaderTemplate::new(7));
    packet.push_header_extension(LctHeaderExtension::Variable {
        header_extension_type: 1,
        length: 1,
        content: vec![0xAA, 0xBB],
    });
    packet.push_header_extension(LctHeaderExtension::Fixed {
        header_extension_type: 192,
        content: [1, 2, 3],
    });
    packet.push_constructor(FdConstructor::Noop { pad: [0u8; 15] });
    packet.push_constructor(FdConstructor::Immediate {
        count: 4,
        data: {
            let mut data = [0u8; 14];
            data[..4].copy_from_slice(&[1, 2, 3, 4]);
            data
        },
    });
    packet.push_constructor(FdConstructor::Item {
        item_id: 9,
        extent_index: 2,
        data_offset: 1_000,
        data_length: 512,
    });
    packet.push_constructor(FdConstructor::Xml {
        data_offset: 64,
        data_length: 128,
        reserved: 0,
    });
    let packet_bytes = packet.try_to_bytes().expect("fdpa");
    let parsed = FdPacketBox::parse(&packet_bytes).expect("fdpa parses");
    assert_eq!(parsed.header_template().transport_object_identifier(), 7);
    assert_eq!(parsed.header_extensions().len(), 2);
    assert_eq!(parsed.header_extensions()[0].header_extension_type(), 1);
    assert_eq!(parsed.constructors().len(), 4);
    assert!(matches!(
        parsed.constructors()[2],
        FdConstructor::Item {
            item_id: 9,
            data_length: 512,
            ..
        }
    ));
    assert_eq!(parsed.try_to_bytes().expect("bytes"), packet_bytes);

    let feci = FecInformationBox::new(0, 0, 3, 4);
    let extra = ExtraDataBox::new(&[0xDE, 0xAD]).with_fec_information(feci);
    let mut sample = FdSample::new();
    sample.push_packet(parsed);
    sample.set_extra_data(extra);
    let sample_bytes = sample.try_to_bytes().expect("FD sample");
    let reparsed = FdSample::parse(&sample_bytes).expect("FD sample parses");
    assert_eq!(reparsed.packets().len(), 1);
    let extra = reparsed.extra_data().expect("extr");
    assert_eq!(
        extra.fec_information().expect("feci").encoding_symbol_id(),
        4
    );
    assert_eq!(extra.extra_data(), &[0xDE, 0xAD]);
    assert_eq!(reparsed.try_to_bytes().expect("bytes"), sample_bytes);

    let mut entry = FdHintSampleEntry::new(1, 0x0140);
    entry.push(FdHintEntryChild::Timescale(TimescaleEntryBox::new(1_000)));
    let entry_bytes = entry.try_to_bytes().expect("fdp entry");
    let SampleEntryVariant::Hint(entry) = sample_entry_variant(&entry_bytes).expect("fdp parses")
    else {
        panic!("expected an FD hint entry");
    };
    let HintSampleEntry::Fd(reparsed) = *entry else {
        panic!("expected an FD hint entry");
    };
    assert_eq!(reparsed.partition_entry_id(), 1);
    assert_eq!(reparsed.fec_overhead(), 0x0140);
    assert_eq!(reparsed.timescale(), Some(1_000));
    assert_eq!(reparsed.try_to_bytes().expect("bytes"), entry_bytes);
}

#[test]
fn bit_rate_shadow_sync_and_redundant_timing_round_trip() {
    let btrt = boxed(
        *b"btrt",
        &[
            0u32.to_be_bytes(),
            96_800u32.to_be_bytes(),
            48_400u32.to_be_bytes(),
        ]
        .concat(),
    );
    let parsed = BitRateBox::parse(&btrt).expect("btrt");
    assert_eq!(parsed.buffer_size_db(), 0);
    assert_eq!(parsed.max_bitrate(), 96_800);
    assert_eq!(parsed.avg_bitrate(), 48_400);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), btrt);

    let mut stsh = ShadowSyncSampleBox::new();
    stsh.push_entry(ShadowSyncEntry::new(3, 1));
    stsh.push_entry(ShadowSyncEntry::new(9, 7));
    let stsh_bytes = stsh.try_to_bytes().expect("stsh");
    let parsed = ShadowSyncSampleBox::parse(&stsh_bytes).expect("stsh");
    assert_eq!(parsed.entries()[1].shadowed_sample_number(), 9);
    assert_eq!(parsed.entries()[1].sync_sample_number(), 7);
    assert_eq!(parsed.try_to_bytes().expect("bytes"), stsh_bytes);

    let rsot = boxed(
        *b"rsot",
        &full(0, 3, &[300u32.to_be_bytes(), 120u32.to_be_bytes()].concat()),
    );
    let parsed = RedundantSampleOriginalTimingBox::parse(&rsot).expect("rsot");
    assert_eq!(parsed.original_duration(), Some(300));
    assert_eq!(parsed.elapsed_duration(), Some(120));
    assert_eq!(parsed.try_to_bytes().expect("bytes"), rsot);

    let invalid = boxed(*b"rsot", &full(0, 4, &300u32.to_be_bytes()));
    assert!(RedundantSampleOriginalTimingBox::parse(&invalid).is_err());
}

#[test]
fn fd_item_info_extension_reads_back() {
    let mut extension = Vec::new();
    extension.extend_from_slice(b"file.ts\0");
    extension.extend_from_slice(b"0123456789abcdef0123456789abcdef\0");
    extension.extend_from_slice(&1_024u64.to_be_bytes());
    extension.extend_from_slice(&1_040u64.to_be_bytes());
    extension.push(2);
    extension.extend_from_slice(&[1u32.to_be_bytes(), 2u32.to_be_bytes()].concat());

    let mut body = Vec::new();
    body.extend_from_slice(&3u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(b"file.ts\0");
    body.extend_from_slice(b"video/mp2t\0");
    body.push(0);
    body.extend_from_slice(b"fdel");
    body.extend_from_slice(&extension);
    let bytes = boxed(*b"infe", &full(1, 0, &body));

    let entry = ItemInfoEntry::parse(&bytes).expect("infe v1");
    let fd = entry.fd_extension().expect("fdel").expect("fdel present");
    assert_eq!(fd.content_location(), b"file.ts");
    assert_eq!(fd.content_length(), 1_024);
    assert_eq!(fd.transfer_length(), 1_040);
    assert_eq!(fd.group_ids(), &[1, 2]);
    assert_eq!(entry.try_to_bytes().expect("bytes"), bytes);
}
