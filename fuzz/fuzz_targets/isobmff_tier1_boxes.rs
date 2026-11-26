#![no_main]

use broadcast_core::Parse;
use libfuzzer_sys::fuzz_target;

// Every tier 1 box parser, reached from one selector byte. A box body is
// untrusted wire data: a count or a bit width read from it must never drive an
// allocation or an out-of-bounds read.
fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

fuzz_target!(|data: &[u8]| {
    let Some((&selector, body)) = data.split_first() else {
        return;
    };
    match selector % 18 {
        0 => {
            let _ = isobmff::ChannelLayoutBox::parse(&boxed(*b"chnl", body));
        }
        1 => {
            let _ = isobmff::DownMixInstructionsBox::parse(&boxed(*b"dmix", body));
        }
        2 => {
            let _ = isobmff::LoudnessBox::parse(&boxed(*b"ludt", body));
        }
        3 => {
            let _ = isobmff::LabelBox::parse(&boxed(*b"labl", body));
        }
        4 => {
            let _ = isobmff::ExtendedLanguageBox::parse(&boxed(*b"elng", body));
        }
        5 => {
            let _ = isobmff::KindBox::parse(&boxed(*b"kind", body));
        }
        6 => {
            let _ = isobmff::TrackGroupBox::parse(&boxed(*b"trgr", body));
        }
        7 => {
            let _ = isobmff::TrackSelectionBox::parse(&boxed(*b"tsel", body));
        }
        8 => {
            let _ = isobmff::StereoVideoBox::parse(&boxed(*b"stvi", body));
        }
        9 => {
            let _ = isobmff::SubsegmentIndexBox::parse(&boxed(*b"ssix", body));
        }
        10 => {
            let _ = isobmff::LevelAssignmentBox::parse(&boxed(*b"leva", body));
        }
        11 => {
            let _ = isobmff::ProgressiveDownloadInfoBox::parse(&boxed(*b"pdin", body));
        }
        12 => {
            let _ = isobmff::TrackExtensionPropertiesBox::parse(&boxed(*b"trep", body));
        }
        13 => {
            let _ = isobmff::sample_groups::CompactSampleToGroupBox::parse(&boxed(*b"csgp", body));
        }
        14 => {
            let _ = isobmff::DegradationPriorityBox::parse(&boxed(*b"stdp", body));
        }
        15 => {
            let _ = isobmff::PaddingBitsBox::parse(&boxed(*b"padb", body));
        }
        16 => {
            let _ = isobmff::SubTrackSampleGroupBox::parse(&boxed(*b"stsg", body));
        }
        _ => {
            // A `ludt` child and the compact sample-to-group mapping: the
            // two parsers whose bit widths and pattern counts expand.
            let _ = isobmff::TrackLoudnessInfo::parse(&boxed(*b"tlou", body));
            if let Ok(csgp) =
                isobmff::sample_groups::CompactSampleToGroupBox::parse(&boxed(*b"csgp", body))
            {
                let _ = csgp.mapping_with_limit(4096);
            }
        }
    }
});
