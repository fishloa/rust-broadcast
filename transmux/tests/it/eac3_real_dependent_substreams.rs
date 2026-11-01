//! A REAL E-AC-3 stream with a dependent substream through transmux.
//!
//! Source: FFmpeg FATE `eac3/the_great_wall_7.1.eac3` (Dolby Digital Plus 7.1):
//! 157 access units, each an AC-3 core syncframe (bsid 6, independent substream 0
//! per ETSI TS 102 366 §E.1.3.1.2 / Annex J.1.5.2.2) followed by a dependent
//! E-AC-3 syncframe (`strmtyp` 1, `chanmap` = Ls, Rs, Lrs/Rrs). Its licence is not
//! stated, so it lives in the private submodule (`private/fixtures/eac3-dep/`,
//! with `PROVENANCE.md` and `make.sh` for the derived MP4/TS files produced by
//! MP4Box, ffmpeg and gpac); every test here skips loudly (`test_fixtures::require`)
//! when the private repo is not checked out.
//!
//! Independent oracles: MP4Box's own `dec3` box and gpac's per-sample parse of
//! MP4Box's MP4 (`gw_mp4box.gpac_inspect.txt`: size and CRC32 of each access
//! unit). Before the AC-3-core fix transmux recognised no access unit at all in
//! this stream (an AC-3 syncframe was not an independent substream), so the TS
//! demux produced no track.

use broadcast_common::{Package, Serialize, Unpackage};
use transmux::ac3::{Ec3SyncframeInfo, split_eac3_syncframes};
use transmux::pipeline::CodecConfig;
use transmux::{
    CmafMux, DemuxEvent, Ec3SpecificBox, Fmp4Demux, Media, ProgressiveDemux, StreamingTsDemux,
    TsDemux,
};

const AU_COUNT: usize = 157;
const AC3_CORE_LEN: usize = 2304;
const AU_LEN: usize = 2 * AC3_CORE_LEN;
const SAMPLES_PER_AU: u32 = 1536;

/// `private/fixtures/eac3-dep/<rel>` through the shared resolver: `None` after a
/// loud SKIPPED line when the private repo is not checked out.
fn local(rel: &str) -> Option<Vec<u8>> {
    test_fixtures::require_bytes(
        &format!("eac3-dep/{rel}"),
        "transmux/tests/it/eac3_real_dependent_substreams.rs",
    )
}

macro_rules! need {
    ($rel:expr) => {
        match local($rel) {
            Some(b) => b,
            None => return,
        }
    };
}

fn es() -> Option<Vec<u8>> {
    local("the_great_wall_7.1.eac3")
}

/// The CRC gpac prints per packet: its `gf_crc_32`, the MPEG-2 CRC-32.
fn crc32_gpac(data: &[u8]) -> u32 {
    broadcast_common::crc32_mpeg2::compute(data)
}

/// `dec3` box body (after the 8-byte header) of the first `ec-3` entry, found by
/// a plain byte search - independent of transmux's box parser.
fn dec3_body(mp4: &[u8]) -> Vec<u8> {
    let at = mp4
        .windows(4)
        .position(|w| w == b"dec3")
        .expect("a dec3 box");
    let size = u32::from_be_bytes(mp4[at - 4..at].try_into().unwrap()) as usize;
    mp4[at + 4..at - 4 + size].to_vec()
}

fn eac3_track(m: &Media) -> &transmux::Track {
    let mut it = m
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Eac3 { .. }));
    let t = it.next().expect("an E-AC-3 track");
    assert!(it.next().is_none());
    t
}

fn dec3_of(t: &transmux::Track) -> &Ec3SpecificBox {
    match &t.spec.config {
        CodecConfig::Eac3 { config, .. } => config,
        _ => unreachable!(),
    }
}

#[test]
fn the_real_stream_splits_into_ac3_core_plus_dependent_access_units() {
    let es = need!("the_great_wall_7.1.eac3");
    let units = split_eac3_syncframes(&es);
    assert_eq!(units.len(), AU_COUNT);
    let mut rebuilt = Vec::new();
    for u in &units {
        assert_eq!(u.data.len(), AU_LEN);
        let frames = Ec3SyncframeInfo::from_es_all(&u.data);
        assert_eq!(frames.len(), 2, "core + dependent");
        let (core, dep) = (&frames[0], &frames[1]);
        assert_eq!((core.strmtyp, core.substreamid, core.bsid), (0, 0, 6));
        assert_eq!(core.frmsiz as usize + 1, AC3_CORE_LEN / 2);
        assert_eq!(core.samples_per_frame(), SAMPLES_PER_AU);
        assert_eq!((dep.strmtyp, dep.substreamid, dep.bsid), (1, 0, 16));
        // chanmap bits (Table E.1.4, bit 0 = MSB): Ls, Rs and Lrs/Rrs.
        assert_eq!(dep.chanmap, Some(0x1A00));
        rebuilt.extend_from_slice(&u.data);
    }
    assert_eq!(
        rebuilt, es,
        "the access units tile the stream byte for byte"
    );
}

#[test]
fn access_units_equal_gpacs_own_samples_of_mp4boxs_file() {
    let es = need!("the_great_wall_7.1.eac3");
    let oracle = need!("gw_mp4box.gpac_inspect.txt");
    let units = split_eac3_syncframes(&es);
    let text = String::from_utf8(oracle).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        units.len(),
        "gpac sees one sample per access unit"
    );
    for (i, (line, u)) in lines.iter().zip(&units).enumerate() {
        let field = |k: &str| -> String {
            let key = format!("{k}=");
            let at = line.find(&key).unwrap() + key.len();
            line[at..].split_whitespace().next().unwrap().to_string()
        };
        assert_eq!(
            field("size").parse::<usize>().unwrap(),
            u.data.len(),
            "AU {i}"
        );
        assert_eq!(
            u32::from_str_radix(field("crc32").trim_start_matches("0x"), 16).unwrap(),
            crc32_gpac(&u.data),
            "AU {i} bytes"
        );
        assert_eq!(
            field("dur").parse::<u32>().unwrap(),
            SAMPLES_PER_AU,
            "AU {i}"
        );
    }
}

#[test]
fn dec3_for_the_real_access_unit_matches_mp4box() {
    let es = need!("the_great_wall_7.1.eac3");
    let mp4box = need!("gw_mp4box.mp4");
    let ffmpeg = need!("gw_ffmpeg.mp4");
    let first = split_eac3_syncframes(&es).remove(0);
    let frames = Ec3SyncframeInfo::from_es_all(&first.data);
    let ours = Ec3SpecificBox::from_syncframes(&frames)
        .expect("dec3")
        .try_to_bytes()
        .expect("serialize");
    // data_rate 1152 kbit/s (both substreams), 1 independent substream (id 0),
    // fscod 0, bsid 6, acmod 7 + LFE, ONE dependent substream whose chan_loc is
    // Lrs/Rrs (chanmap bit 6 -> Table F.6.1 bit 1).
    assert_eq!(ours, [0x24, 0x00, 0x0C, 0x0F, 0x02, 0x02]);
    let theirs = dec3_body(&mp4box);
    assert_eq!(
        &theirs[..ours.len()],
        &ours[..],
        "MP4Box's dec3 (GPAC 26.07) agrees on every field"
    );
    assert_eq!(
        &theirs[ours.len()..],
        [0x01, 0x0C],
        "MP4Box appends the Atmos extension"
    );
    // ffmpeg 8.1.2 writes data_rate 576 (independent substream only) and
    // chan_loc bit 4 (Lsd/Rsd) for the same stream. Its chan_loc contradicts the
    // dependent frame's own chanmap (Lrs/Rrs), so it is not the reference.
    assert_eq!(dec3_body(&ffmpeg), [0x12, 0x00, 0x0C, 0x0F, 0x02, 0x10]);
}

fn assert_ts_recovers_the_stream(name: &str) {
    let ts = need_ts(name);
    let es = es().expect("es");
    let want: Vec<Vec<u8>> = split_eac3_syncframes(&es)
        .into_iter()
        .map(|u| u.data)
        .collect();

    let media = TsDemux::new()
        .unpackage(&ts)
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    let track = eac3_track(&media);
    let got: Vec<&[u8]> = track.samples.iter().map(|s| &s.data[..]).collect();
    assert_eq!(got.len(), want.len(), "{name}: one sample per access unit");
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(*g, &w[..], "{name}: AU {i} byte-exact");
    }
    let dts0 = track.samples[0].dts.unwrap();
    for (i, s) in track.samples.iter().enumerate() {
        assert_eq!(s.duration, Some(SAMPLES_PER_AU), "{name}: AU {i}");
        assert_eq!(
            s.dts,
            Some(dts0 + i as i64 * i64::from(SAMPLES_PER_AU)),
            "{name}: AU {i}"
        );
    }
    // dec3 carries the dependent substream (7.1), identical to MP4Box's fields.
    let dec3 = dec3_of(track).try_to_bytes().unwrap();
    assert_eq!(dec3, [0x24, 0x00, 0x0C, 0x0F, 0x02, 0x02], "{name}");

    // The streaming demuxer, fed in uneven chunks, yields the same access units.
    for chunk in [188usize, 1000, 4096, ts.len()] {
        let mut d = StreamingTsDemux::new();
        let mut out: Vec<Vec<u8>> = Vec::new();
        let mut drain = |d: &mut StreamingTsDemux| {
            while let Some(e) = d.poll_event() {
                if let DemuxEvent::Sample { sample, .. } = e {
                    out.push(sample.data.to_vec());
                }
            }
        };
        for c in ts.chunks(chunk) {
            d.feed(c);
            drain(&mut d);
        }
        d.finish();
        drain(&mut d);
        assert_eq!(out, want, "{name}: streaming, chunk {chunk}");
    }
}

fn need_ts(name: &str) -> Vec<u8> {
    local(name).expect("checked by the caller")
}

#[test]
fn ffmpeg_ts_stream_type_0x87_recovers_every_access_unit() {
    if local("gw_ffmpeg.ts").is_none() {
        return;
    }
    assert_ts_recovers_the_stream("gw_ffmpeg.ts");
}

#[test]
fn gpac_ts_with_pes_boundaries_inside_access_units_recovers_every_access_unit() {
    if local("gw_gpac.ts").is_none() {
        return;
    }
    assert_ts_recovers_the_stream("gw_gpac.ts");
}

#[test]
fn mp4s_from_both_producers_demux_and_remux_unchanged() {
    let es = need!("the_great_wall_7.1.eac3");
    let want: Vec<Vec<u8>> = split_eac3_syncframes(&es)
        .into_iter()
        .map(|u| u.data)
        .collect();
    for name in ["gw_mp4box.mp4", "gw_ffmpeg.mp4"] {
        let bytes = need!(name);
        let media = ProgressiveDemux::new(usize::MAX)
            .unwrap()
            .unpackage(&bytes)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let track = eac3_track(&media);
        let got: Vec<Vec<u8>> = track.samples.iter().map(|s| s.data.to_vec()).collect();
        assert_eq!(got, want, "{name}: samples are the access units");
        let CodecConfig::Eac3 { config, .. } = &track.spec.config else {
            unreachable!()
        };
        assert_eq!(config.substreams[0].num_dep_sub, 1, "{name}");
        assert_eq!(config.substreams[0].bsid, 6, "{name}");

        let out = CmafMux::new(1)
            .package(&media)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let round = Fmp4Demux::new().unpackage(&out).unwrap();
        let again = eac3_track(&round);
        assert_eq!(dec3_of(again), config, "{name}: dec3 survives CMAF");
        let got2: Vec<Vec<u8>> = again.samples.iter().map(|s| s.data.to_vec()).collect();
        assert_eq!(got2, want, "{name}: samples survive CMAF");
    }
}

#[test]
fn a_real_orphan_dependent_frame_is_dropped() {
    let es = need!("the_great_wall_7.1.eac3");
    // Start the stream on the first access unit's DEPENDENT frame: its core was
    // "lost". The orphan must not become an access unit of its own.
    let units = split_eac3_syncframes(&es[AC3_CORE_LEN..]);
    assert_eq!(units.len(), AU_COUNT - 1);
    assert_eq!(units[0].data, es[AU_LEN..2 * AU_LEN]);
    assert_eq!(
        units[0].data[5] >> 3,
        6,
        "starts on the next AC-3 core frame"
    );
}
