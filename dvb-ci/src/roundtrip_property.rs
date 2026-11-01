//! Workspace-style guard (adversarial review `review/ci-cam-crypto`): for EVERY
//! `Parse` impl reachable through a dispatch enum, a deterministic sweep of
//! pseudo-random APDU bodies must either be refused by `parse` or re-serialize
//! byte-for-byte. A parser that silently drops or rewrites bytes it accepted
//! makes the CHANGELOG's byte-exact parse -> serialize claim false; this table
//! fails the build for it (no `#[ignore]`, no allowlist).

use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

use crate::error::Result;
use crate::tag::ApduTag;

fn iterations() -> usize {
    if option_env!("RUST_BROADCAST_SOAK").is_some() {
        40_000
    } else {
        8_000
    }
}
/// Longest random body (covers every fixed layout and a few list entries).
const MAX_BODY: usize = 64;

/// xorshift64* — deterministic, so a failure reproduces.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn byte(&mut self, pool: usize) -> u8 {
        let r = self.next();
        let b = (r >> 24) as u8;
        match pool {
            0 => b,
            // Small values: make counts/lengths consistent with the body size.
            1 => b & 0x03,
            // Flag-ish values.
            2 => [0x00, 0x01, 0x80, 0xFF, 0x7F, 0xE0, 0xF0, 0xC1][usize::from(b) % 8],
            _ => b & 0x0F,
        }
    }
}

fn sweep(
    name: &str,
    tags: &[ApduTag],
    rt: &dyn Fn(&[u8]) -> Result<Vec<u8>>,
    failures: &mut Vec<alloc::string::String>,
) {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (name.len() as u64));
    for &tag in tags {
        'tag: for it in 0..iterations() {
            let n = (rng.next() as usize) % (MAX_BODY + 1);
            let pool = it % 4;
            let mut wire = tag.to_bytes().to_vec();
            wire.push(n as u8);
            for _ in 0..n {
                wire.push(rng.byte(pool));
            }
            if let Ok(out) = rt(&wire)
                && out != wire
            {
                failures.push(alloc::format!(
                    "{name} {tag:?}: accepted {wire:02X?} but re-serialized {out:02X?}"
                ));
                break 'tag;
            }
        }
    }
}

macro_rules! sweep_enum {
    ($f:ident; $($ty:ty),+ $(,)?) => {$(
        sweep(stringify!($ty), <$ty>::TAGS, &|b| <$ty>::parse(b)?.try_to_bytes(), &mut $f);
    )+};
}

#[test]
fn every_parser_round_trips_byte_exactly_or_rejects_random_bodies() {
    use crate::ci_ext::broadcast_service_gateway::BroadcastServiceGatewayApdu;
    use crate::ci_ext::{
        application_info_v2::ApplicationInfoV2Apdu, application_mmi::ApplicationMmiApdu,
        ca_pipeline::CaPipelineApdu, copy_protection::CopyProtectionApdu,
        event_manager::EventManagerApdu, power_manager::PowerManagerApdu,
        resource_manager_v2::ResourceManagerV2Apdu, service_gateway::ServiceGatewayApdu,
        software_download::DownloadApdu, status_query::StatusQueryApdu,
        stream_input::StreamInputApdu,
    };
    use crate::ci_plus::multistream_host_control::{
        HostControlMode, MultistreamHostControlApdu, tag as mhc,
    };
    use crate::ci_plus::{
        ca_support::CaSupportApdu, cicam_player::CicamPlayerApdu,
        content_control::ContentControlApdu, file_retrieval::FileRetrievalApdu,
        low_speed_comms_v4::LscV4Apdu, multistream::MultistreamApdu,
        sample_decryption::SampleDecryptionApdu,
    };
    let mut f: Vec<alloc::string::String> = Vec::new();
    sweep_enum!(
        f;
        ApplicationInfoV2Apdu,
        ApplicationMmiApdu,
        CaPipelineApdu,
        CopyProtectionApdu,
        EventManagerApdu,
        PowerManagerApdu,
        ResourceManagerV2Apdu,
        ServiceGatewayApdu,
        DownloadApdu,
        StatusQueryApdu,
        StreamInputApdu,
        CaSupportApdu,
        CicamPlayerApdu,
        ContentControlApdu,
        FileRetrievalApdu,
        LscV4Apdu,
        MultistreamApdu,
        SampleDecryptionApdu,
    );
    // The base EN 50221 objects.
    sweep(
        "AnyApdu",
        crate::AnyApdu::DISPATCHED_TAGS,
        &|b| crate::AnyApdu::parse(b)?.try_to_bytes(),
        &mut f,
    );
    // Hand-written dispatchers.
    let mut bsg: Vec<ApduTag> = ServiceGatewayApdu::TAGS.to_vec();
    bsg.push(crate::ci_ext::broadcast_service_gateway::tag::EIT_SECTION_REQ);
    bsg.push(crate::ci_ext::broadcast_service_gateway::tag::EIT_SECTION_ACK);
    sweep(
        "BroadcastServiceGatewayApdu",
        &bsg,
        &|b| BroadcastServiceGatewayApdu::parse(b)?.try_to_bytes(),
        &mut f,
    );
    let tags = [
        mhc::TUNE_TRIPLET_REQ,
        mhc::TUNE_LCN_REQ,
        mhc::TUNE_IP_REQ,
        mhc::TUNER_STATUS_REQ,
        mhc::TUNER_STATUS_REPLY,
    ];
    for mode in [HostControlMode::MultiStream, HostControlMode::BaseV3] {
        sweep(
            "MultistreamHostControlApdu",
            &tags,
            &|b| MultistreamHostControlApdu::parse_mode(b, mode)?.try_to_bytes(),
            &mut f,
        );
    }
    spdus(&mut f);
    assert!(f.is_empty(), "{} violations:\n{}", f.len(), f.join("\n"));
}

/// SPDUs: one tag byte + `length_field` + body.
fn spdus(f: &mut Vec<alloc::string::String>) {
    use crate::spdu::{
        CloseSessionRequest, CloseSessionResponse, CreateSession, CreateSessionResponse,
        OpenSessionRequest, OpenSessionResponse, SessionNumber, tags,
    };
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    macro_rules! spdu {
        ($ty:ty, $tag:expr) => {
            for it in 0..iterations() {
                let n = (rng.next() as usize) % 12;
                let mut wire = alloc::vec![$tag, n as u8];
                for _ in 0..n {
                    wire.push(rng.byte(it % 4));
                }
                if let Ok(v) = <$ty>::parse(&wire) {
                    let out = v.try_to_bytes().unwrap();
                    if out != wire {
                        f.push(alloc::format!(
                            "{}: {wire:02X?} -> {out:02X?}",
                            stringify!($ty)
                        ));
                        break;
                    }
                }
            }
        };
    }
    spdu!(OpenSessionRequest, tags::OPEN_SESSION_REQUEST);
    spdu!(OpenSessionResponse, tags::OPEN_SESSION_RESPONSE);
    spdu!(CreateSession, tags::CREATE_SESSION);
    spdu!(CreateSessionResponse, tags::CREATE_SESSION_RESPONSE);
    spdu!(CloseSessionRequest, tags::CLOSE_SESSION_REQUEST);
    spdu!(CloseSessionResponse, tags::CLOSE_SESSION_RESPONSE);
    spdu!(SessionNumber, tags::SESSION_NUMBER);
}
