//! No-panic hostility sweep (audit S3) of the caption decoders and the
//! `cc_data()` parser: every `(b1, b2)` byte pair through the CEA-608 decoder
//! from a fresh and from a mode-primed state, every single-byte mutation /
//! truncation of a real CCP through the CEA-708 decoder, and a long
//! deterministic pseudo-random packet stream (a fixed LCG, so a failure is
//! reproducible). The contract is only "never panics": these decoders are
//! byte-driven state machines whose operands index fixed-size arrays.
#![cfg(feature = "decode")]

use broadcast_common::Parse;
use cc_data::decode::{Cea608Decoder, Cea708Decoder};
use cc_data::{CcData, CcTriplet, CcType};

/// A correctly framed CCP (the CTA-708-E DefineWindow worked example).
const FULL_CCP: [u8; 10] = [0x05, 0x27, 0x9A, 0x38, 0x4A, 0xD1, 0x8B, 0x0F, 0x11, 0x00];

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn cea608_every_byte_pair_never_panics() {
    // Primed states: after each control family, so the pair is interpreted in
    // roll-up / pop-on / paint-on / text mode, not just the idle state.
    let primers: [&[(u8, u8)]; 5] = [
        &[],
        &[(0x14, 0x25)],               // RU2
        &[(0x14, 0x20), (0x14, 0x2E)], // RCL, EOC
        &[(0x14, 0x29)],               // RDC (paint-on)
        &[(0x14, 0x2A)],               // TR (text restart)
    ];
    for primer in primers {
        for field2 in [false, true] {
            for b1 in 0u8..=255 {
                for b2 in 0u8..=255 {
                    let mut d = Cea608Decoder::new();
                    for &(p1, p2) in primer {
                        d.push_pair(field2, p1, p2);
                    }
                    d.push_pair(field2, b1, b2);
                    // twice in a row: control codes repeat on the wire
                    d.push_pair(field2, b1, b2);
                }
            }
        }
    }
}

#[test]
fn cea708_truncation_and_mutation_of_a_real_ccp_never_panic() {
    for n in 0..=FULL_CCP.len() {
        let mut d = Cea708Decoder::new();
        d.push_packet(&FULL_CCP[..n]);
    }
    for pos in 0..FULL_CCP.len() {
        for v in 0u8..=255 {
            let mut m = FULL_CCP;
            m[pos] = v;
            let mut d = Cea708Decoder::new();
            d.push_packet(&m);
            // and the decoder's own accessors on whatever state resulted
            for svc in 0..8 {
                let _ = d.service_text(svc);
                let _ = d.windows(svc);
            }
        }
    }
}

#[test]
fn cea708_long_pseudo_random_stream_never_panics() {
    let mut rng = Lcg(0x5EED_CAFE);
    let mut d = Cea708Decoder::new();
    for _ in 0..40_000 {
        let len = 1 + (rng.next() % 40) as usize;
        let mut pkt: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        // Bias the header toward a plausible size_code so the service-block
        // loop is reached, not just rejected.
        pkt[0] = (pkt[0] & 0xC0) | ((len as u8).div_ceil(2) & 0x3F);
        d.push_packet(&pkt);
    }
    for svc in 0..8 {
        let _ = d.service_text(svc);
    }
}

#[test]
fn random_cc_data_and_triplet_streams_never_panic() {
    let mut rng = Lcg(0xC0DE_D00D);
    let mut dec608 = Cea608Decoder::new();
    let mut dec708 = Cea708Decoder::new();
    for _ in 0..20_000 {
        let len = (rng.next() % 40) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        if let Ok(cc) = CcData::parse(&bytes) {
            dec608.push_triplets(cc.triplets.iter());
            dec708.push_triplets(cc.triplets.iter());
        }
        let t = CcTriplet {
            cc_valid: rng.next() & 1 == 1,
            cc_type: match rng.next() % 4 {
                0 => CcType::Ntsc608Field1,
                1 => CcType::Ntsc608Field2,
                2 => CcType::Dtvcc708Data,
                _ => CcType::Dtvcc708Start,
            },
            cc_data_1: rng.next() as u8,
            cc_data_2: rng.next() as u8,
        };
        dec608.push_triplets([&t]);
        dec708.push_triplets([&t]);
    }
}
