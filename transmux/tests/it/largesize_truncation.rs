//! Property test for the #1147 follow-up: every sample-entry type the `stsd`
//! dispatcher knows, in BOTH box-header forms (compact 8-byte and the
//! `size = 1` + 64-bit `largesize` 16-byte form, ISO/IEC 14496-12 §4.2), at
//! EVERY body length from 0 up past the longest fixed layout, with several
//! fill patterns, must return `Ok`/`Err` and never panic. A reader whose
//! minimum-length guard assumed an 8-byte header indexed out of bounds on a
//! short `largesize` body (hostile init segment).

use broadcast_common::Parse;
use transmux::{
    ProtectionSchemeInfoBox, SampleDescriptionBox, SchemeInformationBox, TrackExtendsBox,
};

const FOURCCS: &[&[u8; 4]] = &[
    b"avc1", b"avc3", b"avc2", b"hvc1", b"hev1", b"vvc1", b"mp4v", b"mp4a", b"enca", b"ac-3",
    b"ec-3", b"stpp", b"wvtt", b"av01", b"vp09", b"ac-4", b"Opus", b"fLaC", b"mha1", b"dtsc",
    b"avs3", b"avst", b"lav3", b"lav2", b"xxxx",
];

fn stsd(entry: &[u8]) -> Vec<u8> {
    let mut b = ((16 + entry.len()) as u32).to_be_bytes().to_vec();
    b.extend_from_slice(b"stsd");
    b.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    b.extend_from_slice(entry);
    b
}

fn boxed(fourcc: &[u8; 4], body: &[u8], large: bool) -> Vec<u8> {
    let mut b = Vec::new();
    if large {
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(fourcc);
        b.extend_from_slice(&((16 + body.len()) as u64).to_be_bytes());
    } else {
        b.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        b.extend_from_slice(fourcc);
    }
    b.extend_from_slice(body);
    b
}

#[test]
fn truncated_sample_entries_never_panic_in_either_header_form() {
    let mut panicked = Vec::new();
    for fourcc in FOURCCS {
        for large in [false, true] {
            for fill in [0x00u8, 0x01, 0x7F, 0xFF] {
                for len in 0..=160usize {
                    let entry = boxed(fourcc, &vec![fill; len], large);
                    let stsd = stsd(&entry);
                    if std::panic::catch_unwind(|| {
                        let _ = SampleDescriptionBox::parse(&stsd);
                    })
                    .is_err()
                    {
                        panicked.push(format!(
                            "{} large={large} fill={fill:#x} body_len={len}",
                            String::from_utf8_lossy(&fourcc[..])
                        ));
                    }
                }
            }
        }
    }
    assert!(panicked.is_empty(), "panics: {panicked:#?}");
}

#[test]
fn truncated_fixed_layout_boxes_never_panic_in_either_header_form() {
    let mut panicked = Vec::new();
    for large in [false, true] {
        for len in 0..=64usize {
            for (name, fourcc) in [("trex", b"trex"), ("schi", b"schi"), ("sinf", b"sinf")] {
                let bytes = boxed(fourcc, &vec![0u8; len], large);
                if std::panic::catch_unwind(|| match name {
                    "trex" => {
                        let _ = TrackExtendsBox::parse(&bytes);
                    }
                    "schi" => {
                        let _ = SchemeInformationBox::parse(&bytes);
                    }
                    _ => {
                        let _ = ProtectionSchemeInfoBox::parse(&bytes);
                    }
                })
                .is_err()
                {
                    panicked.push(format!("{name} large={large} body_len={len}"));
                }
            }
        }
    }
    assert!(panicked.is_empty(), "panics: {panicked:#?}");
}
