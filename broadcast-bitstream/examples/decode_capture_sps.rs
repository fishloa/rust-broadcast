//! Decode the sequence parameter set in force at a capture's first IDR access
//! unit and print what it states.
//!
//! The input is an MPEG-2 transport stream carrying H.264. The transport layer
//! is walked with the `mpeg-ts`/`mpeg-pes` dev-dependencies so the elementary
//! stream is real broadcast bytes; finding the parameter set inside it and
//! reading it is this crate's work alone. Packets are read one at a time, so a
//! long capture is never held whole.
//!
//! ```sh
//! cargo run -p broadcast-bitstream --example decode_capture_sps -- fixtures/ts/h264-aac.ts
//! ```

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::process;

use broadcast_bitstream::rfc6381::rfc6381_avc1;
use broadcast_bitstream::sps::{AvcSpsInfo, decode_avc_sps, is_high_profile};
use broadcast_bitstream::{AvcNalUnitType, NalCodec, iter_annexb_nals, nal_unit_type};
use mpeg_pes::PesAssembler;
use mpeg_ts::ts::TsPacket;

/// Length of one MPEG-2 TS packet (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_LEN: usize = 188;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!("Usage: decode_capture_sps <capture.ts>");
        process::exit(1);
    };
    let mut input = File::open(&path).unwrap_or_else(|e| {
        eprintln!("open {path}: {e}");
        process::exit(1);
    });

    match sps_in_force_at_first_idr(&mut input) {
        Ok(Some(sps)) => match decode_avc_sps(&sps) {
            Ok(info) => report(&info),
            Err(e) => {
                eprintln!("decode SPS: {e}");
                process::exit(1);
            }
        },
        Ok(None) => {
            eprintln!("no H.264 SPS before an IDR access unit in {path}");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("read {path}: {e}");
            process::exit(1);
        }
    }
}

/// The last SPS seen before the first IDR slice, walking the input's video PES
/// payloads in file order.
fn sps_in_force_at_first_idr(input: &mut File) -> io::Result<Option<Vec<u8>>> {
    let mut assemblers: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    let mut sps: Option<Vec<u8>> = None;
    let mut packet = [0u8; TS_PACKET_LEN];
    loop {
        match input.read_exact(&mut packet) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(sps),
            Err(e) => return Err(e),
        }
        let Ok(ts) = TsPacket::parse(&packet) else {
            continue;
        };
        let Some(payload) = ts.payload else {
            continue;
        };
        let Some(pes) = assemblers
            .entry(ts.header.pid.value())
            .or_default()
            .feed(ts.header.pusi, payload)
        else {
            continue;
        };
        for nal in iter_annexb_nals(&pes) {
            match nal_type(nal) {
                Some(AvcNalUnitType::Sps) => sps = Some(nal.to_vec()),
                Some(AvcNalUnitType::CodedSliceIdr) => return Ok(sps),
                _ => {}
            }
        }
    }
}

/// The H.264 NAL unit type of one Annex B NAL unit.
fn nal_type(nal: &[u8]) -> Option<AvcNalUnitType> {
    nal_unit_type(NalCodec::Avc, nal).map(AvcNalUnitType::from_u8)
}

fn report(info: &AvcSpsInfo) {
    println!("resolution: {}x{}", info.width(), info.height());
    println!(
        "profile_idc: {} (high-profile SPS syntax: {}), level_idc: {}",
        info.profile_idc(),
        is_high_profile(info.profile_idc()),
        info.level_idc()
    );
    println!(
        "codec string: {}",
        rfc6381_avc1(
            info.profile_idc(),
            info.constraint_flags(),
            info.level_idc()
        )
    );
    println!(
        "chroma_format_idc: {}, bit depths: {}/{}",
        info.chroma_format_idc(),
        info.bit_depth_luma(),
        info.bit_depth_chroma()
    );
}
