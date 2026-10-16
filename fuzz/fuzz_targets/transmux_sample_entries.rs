#![no_main]

use broadcast_common::Parse;
use libfuzzer_sys::fuzz_target;

// Fuzz the `stsd` sample-entry readers (every codec the dispatcher knows) in
// BOTH box-header forms. A box may carry `size = 1` + a 64-bit `largesize`
// (ISO/IEC 14496-12 §4.2), which moves its body from offset 8 to 16; a reader
// whose minimum-length guard still assumed 8 bytes indexed past a short
// `largesize` body and PANICKED on a hostile init segment (#1147 follow-up).
//
// First byte picks the entry fourcc; the rest is the entry body.
const FOURCCS: &[&[u8; 4]] = &[
    b"avc1", b"avc3", b"hvc1", b"hev1", b"vvc1", b"mp4v", b"mp4a", b"enca", b"ac-3", b"ec-3",
    b"stpp", b"wvtt", b"av01", b"vp09", b"ac-4", b"Opus", b"fLaC", b"mha1", b"dtsc", b"avs3",
    b"avst", b"lav3", b"lav2", b"xxxx",
];

fn stsd(entry: &[u8]) -> Vec<u8> {
    let mut b = ((16 + entry.len()) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(b"stsd");
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    b.extend_from_slice(entry);
    b
}

fuzz_target!(|data: &[u8]| {
    let Some((&sel, body)) = data.split_first() else {
        return;
    };
    let fourcc = FOURCCS[usize::from(sel) % FOURCCS.len()];
    let mut compact = ((8 + body.len()) as u32).to_be_bytes().to_vec();
    compact.extend_from_slice(fourcc);
    compact.extend_from_slice(body);
    let mut large = 1u32.to_be_bytes().to_vec();
    large.extend_from_slice(fourcc);
    large.extend_from_slice(&((16 + body.len()) as u64).to_be_bytes());
    large.extend_from_slice(body);
    for entry in [&compact, &large] {
        let _ = transmux::SampleDescriptionBox::parse(&stsd(entry));
    }
    let _ = transmux::TrackExtendsBox::parse(data);
    let _ = transmux::SchemeInformationBox::parse(data);
    let _ = transmux::ProtectionSchemeInfoBox::parse(data);
});
