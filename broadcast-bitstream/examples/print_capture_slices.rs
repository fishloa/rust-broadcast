//! Read a broadcast capture's coded pictures and print each one's slice type
//! and whether it is a frame or a field.
//!
//! The input is an MPEG-2 transport stream carrying H.264. The transport layer
//! is walked with the `mpeg-ts`/`mpeg-pes` dev-dependencies so the elementary
//! stream is real broadcast bytes; access-unit boundaries and the slice header
//! are this crate's work alone. Packets are read one at a time, so a long
//! capture is never held whole.
//!
//! ```sh
//! cargo run -p broadcast-bitstream --example print_capture_slices -- fixtures/ts/h264-aac.ts
//! ```

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::process;

use broadcast_bitstream::nal::{
    AVC_NAL_IDR, AVC_NAL_PPS, AVC_NAL_SLICE, AVC_NAL_SPS, NAL_TYPE_MASK, NalCodec, nal_unit_type,
};
use broadcast_bitstream::slice_header::AvcParamSets;
use broadcast_bitstream::{AccessUnitSplitter, iter_annexb_nals};
use mpeg_pes::{PesAssembler, PesPacket};
use mpeg_ts::ts::TsPacket;

/// Length of one MPEG-2 TS packet (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_LEN: usize = 188;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!("Usage: print_capture_slices <capture.ts>");
        process::exit(1);
    };
    let mut input = File::open(&path).unwrap_or_else(|e| {
        eprintln!("open {path}: {e}");
        process::exit(1);
    });
    let result = video_pid(&mut input).and_then(|pid| {
        let Some(pid) = pid else {
            return Ok(None);
        };
        input.seek(SeekFrom::Start(0))?;
        report(&mut input, pid).map(Some)
    });
    match result {
        Ok(Some(count)) => println!("{count} coded pictures"),
        Ok(None) => {
            eprintln!("{path}: no H.264 SPS in the capture");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("{path}: {e}");
            process::exit(1);
        }
    }
}

/// Print one line per coded picture of `pid` and return the picture count.
fn report(input: &mut File, pid: u16) -> std::io::Result<usize> {
    let mut splitter = AccessUnitSplitter::new(NalCodec::Avc);
    let mut params = AvcParamSets::new();
    let mut count = 0usize;
    for_each_pes(input, Some(pid), |_, pes| {
        let Some(es) = elementary_stream(&pes) else {
            return;
        };
        if splitter.push(es).is_err() {
            return;
        }
        while let Some(au) = splitter.pop() {
            count += describe(&au, &mut params);
        }
    })?;
    splitter.finish();
    while let Some(au) = splitter.pop() {
        count += describe(&au, &mut params);
    }
    Ok(count)
}

/// Print one access unit's coded picture (`1` when it carries one).
fn describe(au: &[u8], params: &mut AvcParamSets) -> usize {
    let nals: Vec<&[u8]> = iter_annexb_nals(au).collect();
    for nal in &nals {
        if matches!(
            nal.first().map(|b| b & NAL_TYPE_MASK),
            Some(AVC_NAL_SPS | AVC_NAL_PPS)
        ) {
            params.observe_nal(nal);
        }
    }
    let Some(first) = nals.iter().find(|nal| {
        matches!(
            nal.first().map(|b| b & NAL_TYPE_MASK),
            Some(AVC_NAL_SLICE | AVC_NAL_IDR)
        )
    }) else {
        return 0;
    };
    let Some((header, _)) = params.parse_slice(first) else {
        eprintln!("a slice header did not parse");
        return 0;
    };
    println!(
        "slice_type {} ({}), {}",
        header.slice_type(),
        slice_kind(header.slice_type()),
        if header.field_pic_flag() {
            "field"
        } else {
            "frame"
        }
    );
    1
}

/// `slice_type` reduced to its intra/inter class (Table 7-6: the type repeats
/// at `+5`).
fn slice_kind(slice_type: u8) -> &'static str {
    match slice_type % 5 {
        0 => "P",
        1 => "B",
        2 => "I",
        3 => "SP",
        4 => "SI",
        _ => "unknown",
    }
}

/// Read the input packet by packet, handing each completed PES of `only` (or of
/// every PID when `None`) to `visit`.
fn for_each_pes(
    input: &mut File,
    only: Option<u16>,
    mut visit: impl FnMut(u16, Vec<u8>),
) -> std::io::Result<()> {
    let mut assemblers: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    let mut packet = [0u8; TS_PACKET_LEN];
    loop {
        match input.read_exact(&mut packet) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let Ok(ts) = TsPacket::parse(&packet) else {
            continue;
        };
        let pid = ts.header.pid.value();
        if only.is_some_and(|wanted| wanted != pid) {
            continue;
        }
        let Some(payload) = ts.payload else {
            continue;
        };
        if let Some(pes) = assemblers
            .entry(pid)
            .or_default()
            .feed(ts.header.pusi, payload)
        {
            visit(pid, pes);
        }
    }
    for (pid, assembler) in &mut assemblers {
        if let Some(pes) = assembler.flush() {
            visit(*pid, pes);
        }
    }
    Ok(())
}

/// The elementary-stream bytes of one assembled PES packet.
fn elementary_stream(pes: &[u8]) -> Option<&[u8]> {
    PesPacket::parse_unit(pes)
        .ok()
        .map(|(packet, _)| packet.payload)
}

/// The PID whose PES payloads carry the H.264 elementary stream.
fn video_pid(input: &mut File) -> std::io::Result<Option<u16>> {
    let mut found = None;
    for_each_pes(input, None, |pid, pes| {
        if found.is_none()
            && elementary_stream(&pes).is_some_and(|es| {
                iter_annexb_nals(es)
                    .any(|nal| nal_unit_type(NalCodec::Avc, nal) == Some(AVC_NAL_SPS))
            })
        {
            found = Some(pid);
        }
    })?;
    Ok(found)
}
