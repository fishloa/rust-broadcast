#![no_main]

use broadcast_core::Parse;
use libfuzzer_sys::fuzz_target;

// Every tier 5 box parser of issue #4167, reached from one selector byte: the
// hint-track family, the RTP and FD sample formats, and the file-delivery
// family. A count or a presence flag read from the body must never drive an
// allocation or an out-of-bounds read.
fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

fn sample_entry(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut payload = vec![0u8; 6];
    payload.extend_from_slice(&1u16.to_be_bytes());
    payload.extend_from_slice(body);
    boxed(fourcc, &payload)
}

fuzz_target!(|data: &[u8]| {
    let Some((&selector, body)) = data.split_first() else {
        return;
    };
    match selector % 24 {
        0 => {
            let _ = isobmff::hint::HintInformationBox::parse(&boxed(*b"hnti", body));
            let _ = isobmff::hint::RtpHintInformation::parse(&boxed(*b"rtp ", body));
            let _ = isobmff::hint::SdpHintInformation::parse(&boxed(*b"sdp ", body));
        }
        1 => {
            let _ = isobmff::hint::HintStatisticsBox::parse(&boxed(*b"hinf", body));
        }
        2 => {
            let _ = isobmff::hint::TimescaleEntryBox::parse(&boxed(*b"tims", body));
            let _ = isobmff::hint::TimeOffsetBox::parse(&boxed(*b"tsro", body));
            let _ = isobmff::hint::SequenceOffsetBox::parse(&boxed(*b"snro", body));
        }
        3 => {
            let _ = isobmff::hint::TimestampSynchronyBox::parse(&boxed(*b"tssy", body));
            let _ = isobmff::hint::ReceivedSsrcBox::parse(&boxed(*b"rssr", body));
            let _ = isobmff::hint::RtpHdrExtBox::parse(&boxed(*b"rtpx", body));
            let _ = isobmff::hint::ReceivedCsrcBox::parse(&boxed(*b"rcsr", body));
        }
        4 => {
            let _ = isobmff::hint::SrtpProcessBox::parse(&boxed(*b"srpp", body));
        }
        5 => {
            let _ = isobmff::hint::PatBox::parse(&boxed(*b"tPAT", body));
            let _ = isobmff::hint::PmtBox::parse(&boxed(*b"tPMT", body));
            let _ = isobmff::hint::TsTimingBox::parse(&boxed(*b"tsti", body));
            let _ = isobmff::hint::InitialSampleTimeBox::parse(&boxed(*b"istm", body));
        }
        6 => {
            let _ = isobmff::hint::RtpSampleEntry::parse(&sample_entry(*b"rtp ", body));
            let _ = isobmff::hint::RtpSampleEntry::parse(&sample_entry(*b"rrtp", body));
        }
        7 => {
            let _ = isobmff::hint::Mpeg2TsHintSampleEntry::parse(&sample_entry(*b"rm2t", body));
            let _ = isobmff::hint::Mpeg2TsHintSampleEntry::parse(&sample_entry(*b"pm2t", body));
            let _ = isobmff::hint::FdHintSampleEntry::parse(&sample_entry(*b"fdp ", body));
        }
        8 => {
            let _ = isobmff::hint::RtpSample::parse(body);
        }
        9 => {
            let _ = isobmff::file_delivery::FdSample::parse(body);
        }
        10 => {
            let _ = isobmff::file_delivery::FdItemInformationBox::parse(&boxed(*b"fiin", body));
            let _ = isobmff::file_delivery::PartitionEntryBox::parse(&boxed(*b"paen", body));
        }
        11 => {
            let _ = isobmff::file_delivery::FilePartitionBox::parse(&boxed(*b"fpar", body));
        }
        12 => {
            let _ = isobmff::file_delivery::FecReservoirBox::parse(&boxed(*b"fecr", body));
            let _ = isobmff::file_delivery::FileReservoirBox::parse(&boxed(*b"fire", body));
        }
        13 => {
            let _ = isobmff::file_delivery::FdSessionGroupBox::parse(&boxed(*b"segr", body));
            let _ = isobmff::file_delivery::GroupIdToNameBox::parse(&boxed(*b"gitn", body));
        }
        14 => {
            let _ = isobmff::file_delivery::FecInformationBox::parse(&boxed(*b"feci", body));
            let _ = isobmff::file_delivery::ExtraDataBox::parse(&boxed(*b"extr", body));
            let _ = isobmff::file_delivery::FdPacketBox::parse(&boxed(*b"fdpa", body));
        }
        15 => {
            let _ = isobmff::sample_entries::BitRateBox::parse(&boxed(*b"btrt", body));
        }
        16 => {
            let _ = isobmff::sample_tables_extra::ShadowSyncSampleBox::parse(&boxed(*b"stsh", body));
        }
        17 => {
            let _ = isobmff::movie_fragment::RedundantSampleOriginalTimingBox::parse(&boxed(
                *b"rsot", body,
            ));
        }
        18 => {
            if let Ok(entry) = isobmff::ItemInfoEntry::parse(&boxed(*b"infe", body)) {
                let _ = entry.fd_extension();
            }
        }
        19 => {
            let _ = isobmff::hint::HintSampleEntry::parse_box(&sample_entry(*b"srtp", body));
        }
        20 => {
            let _ = isobmff::hint::HintSampleEntry::parse_box(&sample_entry(*b"stcp", body));
        }
        21 => {
            let _ = isobmff::hint::HintSampleEntry::parse_box(&sample_entry(*b"sm2t", body));
        }
        22 => {
            if let Ok(parsed) = isobmff::file_delivery::FdPacketBox::parse(&boxed(*b"fdpa", body)) {
                let _ = parsed.header_extensions();
                let _ = parsed.constructors();
            }
        }
        _ => {
            if let Ok(parsed) = isobmff::hint::RtpSampleEntry::parse(&sample_entry(*b"rtp ", body))
            {
                let _ = parsed.timescale();
                let _ = parsed.srtp_process();
            }
        }
    }
});
