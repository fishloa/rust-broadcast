//! Real A/53 `cc_data()` captures: the per-SEI `<pts_90k> <hex cc_data()>` lists
//! the independent Python extractor `fixtures/captions/gen_cc_data_reference.py`
//! wrote from the mux.js multi-channel-608 and shaka-player `captions-test` TS
//! captures (private submodule, see `private/fixtures/captions/PROVENANCE.md`).
//! Tests skip loudly when `private/` is not checked out.
//!
//! The expected counts below come from a separate hand count of the same lists
//! (`cc_count` = byte 0 low five bits, `cc_type` = triplet byte 0 low two bits),
//! not from the code under test.

use broadcast_common::{Parse, Serialize};
use cc_data::CcData;

struct Capture {
    rel: &'static str,
    frames: usize,
    triplets: usize,
    valid: usize,
    cea608: usize,
    cea708: usize,
    em_data_clear: usize,
}

const CAPTURES: &[Capture] = &[
    Capture {
        rel: "captions/mux.js/multi-channel-608-captions.cc_data.txt",
        frames: 368,
        triplets: 3680,
        valid: 368,
        cea608: 368,
        cea708: 3312,
        em_data_clear: 0,
    },
    Capture {
        rel: "captions/shaka/captions-test.cc_data.txt",
        frames: 599,
        triplets: 11980,
        valid: 1307,
        cea608: 1198,
        cea708: 10782,
        em_data_clear: 599,
    },
];

fn frames(cap: &Capture) -> Option<Vec<(u64, Vec<u8>)>> {
    let Some(path) = test_fixtures::private(cap.rel) else {
        eprintln!(
            "SKIP: private fixtures {} absent (the `private` submodule is not checked out)",
            cap.rel
        );
        return None;
    };
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Some(
        text.lines()
            .map(|line| {
                let (pts, body) = line.split_once(' ').expect("`<pts> <hex>` line");
                (pts.parse().expect("pts"), test_fixtures::hex_bytes(body))
            })
            .collect(),
    )
}

#[test]
fn every_real_cc_data_round_trips_byte_exact_with_the_hand_counted_totals() {
    for cap in CAPTURES {
        let Some(frames) = frames(cap) else { return };
        assert_eq!(frames.len(), cap.frames, "{}: frame count", cap.rel);
        let (mut triplets, mut valid, mut c608, mut c708, mut em_clear) = (0, 0, 0, 0, 0);
        for (i, (_, bytes)) in frames.iter().enumerate() {
            let cc = CcData::parse(bytes).unwrap_or_else(|e| panic!("{} frame {i}: {e}", cap.rel));
            assert_eq!(
                usize::from(bytes[0] & 0x1F),
                cc.triplets.len(),
                "{} frame {i}: cc_count",
                cap.rel
            );
            assert_eq!(
                &cc.try_to_bytes().expect("serialize"),
                bytes,
                "{} frame {i}: byte-exact round trip",
                cap.rel
            );
            assert_eq!(CcData::parse(&cc.try_to_bytes().unwrap()).unwrap(), cc);
            triplets += cc.triplets.len();
            valid += cc.triplets.iter().filter(|t| t.cc_valid).count();
            c608 += cc.cea608().count();
            c708 += cc.cea708().count();
            em_clear += usize::from(!cc.process_em_data_flag);
        }
        assert_eq!(triplets, cap.triplets, "{}: triplets", cap.rel);
        assert_eq!(valid, cap.valid, "{}: cc_valid triplets", cap.rel);
        assert_eq!(c608, cap.cea608, "{}: CEA-608 triplets", cap.rel);
        assert_eq!(c708, cap.cea708, "{}: CEA-708 triplets", cap.rel);
        assert_eq!(
            c608 + c708,
            triplets,
            "{}: every triplet classified",
            cap.rel
        );
        assert_eq!(
            em_clear, cap.em_data_clear,
            "{}: process_em_data clear",
            cap.rel
        );
    }
}
