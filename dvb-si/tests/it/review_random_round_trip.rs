//! Adversarial review (branch `review/dvbsi-descriptors`): property test for
//! the workspace invariant "whatever a descriptor parser accepts re-serializes
//! byte-identically" over deterministic pseudo-random descriptor bodies for
//! every auto-dispatched tag (and every DVB/MPEG-2 extension tag), plus the
//! registry-only descriptors added this cycle. On the reviewed `main`
//! (9fe7a022) it fails for 0x48 service, 0x4A linkage (extended event
//! linkage flags), 0x4D short_event, 0x57 telephone, 0x7F/0x05
//! SH_delivery_system and 0x7F/0x19 audio_preselection; the targeted
//! reproducers are in `review_descriptor_findings.rs`.

use broadcast_common::{Parse, Serialize};
use std::collections::BTreeMap;
#[cfg(feature = "serde")]
use std::collections::BTreeMap as Counts;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn descriptor(&mut self, tag: u8, max_len: u64) -> Vec<u8> {
        let len = (self.next() % max_len) as usize;
        let mut v = vec![tag, len as u8];
        for _ in 0..len {
            v.push(self.next() as u8);
        }
        v
    }
}

fn soak() -> bool {
    std::env::var_os("RUST_BROADCAST_SOAK").is_some()
}

#[cfg(feature = "serde")]
const SERDE_PER_TAG: usize = 100;

/// Descriptors whose serde `Serialize` is covered only by this sweep.
#[cfg(feature = "serde")]
const SERDE_COVERED_TAGS: [u8; 28] = [
    0x72, 0x7D, 0x75, 0x6D, 0x6C, 0x69, 0x65, 0x7E, 0x6E, 0x49, 0x5F, 0x78, 0x4B, 0x4C, 0x74, 0x70,
    0x4F, 0x6B, 0x7C, 0x6F, 0x51, 0x53, 0x7B, 0x46, 0x77, 0x60, 0x71, 0x63,
];

type Fails = BTreeMap<String, (usize, Vec<u8>)>;

fn record(fails: &mut Fails, key: String, input: &[u8]) {
    fails.entry(key).or_insert((0, input.to_vec())).0 += 1;
}

fn typed<T>(name: &str, tag: u8, iters: usize, fails: &mut Fails)
where
    T: for<'a> Parse<'a, Error = dvb_si::Error> + Serialize<Error = dvb_si::Error>,
{
    let mut r = Rng(0x1234_5678_9abc_def1 ^ u64::from(tag));
    for _ in 0..iters {
        let v = r.descriptor(tag, 64);
        let Ok(d) = T::parse(&v) else { continue };
        let mut out = vec![0u8; d.serialized_len()];
        if !matches!(d.serialize_into(&mut out), Ok(n) if out[..n] == v[..]) {
            record(fails, name.to_string(), &v);
        }
    }
}

#[test]
fn every_accepted_descriptor_reserializes_byte_identically() {
    let mut fails = Fails::new();
    #[cfg(feature = "serde")]
    let mut serialized_tags: Counts<u8, usize> = Counts::new();
    let (plain_iters, ext_iters, typed_iters) = if soak() {
        (20_000, 400_000, 50_000)
    } else {
        (5_000, 100_000, 12_500)
    };
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    for tag in 0u8..=255 {
        let ext = tag == 0x7F || tag == 0x3F;
        let iters = if ext { ext_iters } else { plain_iters };
        for i in 0..iters {
            let mut v = r.descriptor(tag, 128);
            if ext && v.len() > 2 {
                v[2] = (i % 32) as u8; // cover every allocated extension tag
            }
            for item in parse_loop(&v) {
                let Ok(d) = item else { continue };
                #[cfg(feature = "serde")]
                {
                    let n = serialized_tags.entry(tag).or_insert(0);
                    if *n < SERDE_PER_TAG {
                        serde_json::to_string(&d)
                            .unwrap_or_else(|e| panic!("serde failed for tag 0x{tag:02X}: {e}"));
                    }
                    *n += 1;
                }
                if let Some(res) = d.reserialize()
                    && !matches!(&res, Ok(b) if b[..] == v[..])
                {
                    let e = if ext { v[2] } else { 0 };
                    record(&mut fails, format!("tag 0x{tag:02X}/0x{e:02X}"), &v);
                }
            }
        }
    }
    use dvb_si::descriptors::*;
    typed::<avs2_video::Avs2VideoDescriptor>("avs2_video", 0x40, typed_iters, &mut fails);
    typed::<avs2_audio::Avs2AudioDescriptor>("avs2_audio", 0xD3, typed_iters, &mut fails);
    typed::<avs3_video::Avs3VideoDescriptor>("avs3_video", 0xD1, typed_iters, &mut fails);
    typed::<avs3_audio::Avs3AudioDescriptor>("avs3_audio", 0xD2, typed_iters, &mut fails);
    typed::<tuwa_avs3_video::TuwaAvs3VideoDescriptor>(
        "tuwa_avs3_video",
        62,
        typed_iters,
        &mut fails,
    );
    typed::<china_caption_service::ChinaCaptionServiceDescriptor>(
        "china_caption_service",
        0x86,
        typed_iters,
        &mut fails,
    );
    typed::<logical_channel::LogicalChannelDescriptor>(
        "logical_channel",
        0x83,
        typed_iters,
        &mut fails,
    );
    typed::<nordig::NordigLogicalChannelV1>("nordig_v1", 0x83, typed_iters, &mut fails);
    typed::<nordig::NordigLogicalChannelV2>("nordig_v2", 0x87, typed_iters, &mut fails);
    #[cfg(feature = "serde")]
    for tag in SERDE_COVERED_TAGS {
        assert!(
            serialized_tags.contains_key(&tag),
            "no descriptor with tag 0x{tag:02X} was accepted, so its serde Serialize went unchecked"
        );
    }
    let report: Vec<String> = fails
        .iter()
        .map(|(k, (n, ex))| format!("{k}: {n} inputs, e.g. {ex:02X?}"))
        .collect();
    assert!(
        report.is_empty(),
        "parse accepted but did not re-serialize byte-identically:\n{}",
        report.join("\n")
    );
}
