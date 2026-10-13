//! `TsDecoder` framing edges (review-focus item 5), driven through `FramedRead`'s Decoder API.

use bytes::{Bytes, BytesMut};
use dvb_stream::TsDecoder;
use tokio_util::codec::Decoder;

fn pkt(tag: u8) -> [u8; 188] {
    let mut p = [0xFFu8; 188];
    p[0] = 0x47;
    p[1] = tag;
    p
}

fn drain(d: &mut TsDecoder, buf: &mut BytesMut) -> Vec<Bytes> {
    let mut out = Vec::new();
    while let Some(p) = d.decode(buf).unwrap() {
        out.push(p);
    }
    out
}

#[test]
fn one_byte_at_a_time_yields_every_packet_exactly_once() {
    let mut d = TsDecoder::new();
    let mut buf = BytesMut::new();
    let mut got = Vec::new();
    for tag in 1..=3u8 {
        for b in pkt(tag) {
            buf.extend_from_slice(&[b]);
            got.extend(drain(&mut d, &mut buf));
        }
    }
    assert_eq!(got.len(), 3);
    assert_eq!(got.iter().map(|p| p[1]).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(d.resync_stats().resyncs, 1);
    assert_eq!(d.resync_stats().bytes_discarded, 0);
}

/// The two-packet sync confirmation (`resync`) needs `buf[off + 188] == 0x47`, so the corruption
/// must come after two good packets for the stream to be synced when it hits.
#[test]
fn a_corrupt_sync_byte_mid_stream_counts_one_desync_and_recovers() {
    let mut d = TsDecoder::new();
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&pkt(1));
    buf.extend_from_slice(&pkt(2));
    let mut bad = pkt(3);
    bad[0] = 0x00;
    buf.extend_from_slice(&bad);
    buf.extend_from_slice(&pkt(4));
    let got = drain(&mut d, &mut buf);
    assert_eq!(
        got.len(),
        2,
        "only the packets before the corruption survive (old behaviour: rest of the buffer is dropped)"
    );
    assert_eq!(d.resync_stats().desyncs, 1);
    assert_eq!(d.resync_stats().bytes_discarded, 188 * 2);
    // the stream recovers on the next aligned data
    buf.extend_from_slice(&pkt(5));
    buf.extend_from_slice(&pkt(6));
    let got = drain(&mut d, &mut buf);
    assert_eq!(got.iter().map(|p| p[1]).collect::<Vec<_>>(), vec![5, 6]);
    assert_eq!(d.resync_stats().resyncs, 2);
}

#[test]
fn datagram_mode_resyncs_per_datagram_and_drops_the_stray_tail() {
    let mut d = TsDecoder::datagram();
    let mut buf = BytesMut::new();
    for tag in 0..10u8 {
        buf.extend_from_slice(&pkt(tag));
    }
    buf.extend_from_slice(&[0x47, 1, 2, 3, 4]); // 5 stray bytes starting like a sync byte
    let mut got = Vec::new();
    while let Some(p) = d.decode_eof(&mut buf).unwrap() {
        got.push(p);
    }
    assert_eq!(got.len(), 10, "all 10 packets of a >7x188 datagram");
    assert!(
        buf.is_empty(),
        "the stray tail is consumed, never stitched onto the next datagram"
    );
    assert_eq!(d.resync_stats().bytes_discarded, 5);
    // next datagram starts clean even though the previous tail looked like a sync byte
    buf.extend_from_slice(&pkt(77));
    assert_eq!(d.decode_eof(&mut buf).unwrap().unwrap()[1], 77);
}

#[test]
fn a_pure_junk_datagram_does_not_stall_or_grow_the_buffer() {
    let mut d = TsDecoder::datagram();
    let mut buf = BytesMut::from(&[0x00u8; 1316][..]);
    assert!(d.decode_eof(&mut buf).unwrap().is_none());
    assert!(buf.is_empty());
    assert_eq!(d.resync_stats().bytes_discarded, 1316);
}

#[test]
fn a_partial_packet_at_eof_in_stream_mode_is_a_clean_end_not_an_error() {
    let mut d = TsDecoder::new();
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&pkt(1));
    buf.extend_from_slice(&pkt(2)[..100]);
    assert_eq!(drain(&mut d, &mut buf).len(), 1);
    assert!(
        d.decode_eof(&mut buf).unwrap().is_none(),
        "old behaviour: trailing partial packet ignored"
    );
}

/// W-DS-2: every datagram resyncs from scratch. A stale `synced = true` carried over from the
/// previous datagram would treat the junk prefix of datagram 2 as a mid-stream desync and drop
/// the whole datagram.
#[test]
fn a_datagram_with_a_junk_prefix_is_resynced_independently_of_the_previous_one() {
    let mut d = TsDecoder::datagram();
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&pkt(1));
    buf.extend_from_slice(&pkt(2));
    let mut got = Vec::new();
    while let Some(p) = d.decode_eof(&mut buf).unwrap() {
        got.push(p[1]);
    }
    assert_eq!(got, vec![1, 2]);
    // second datagram: 10 junk bytes, then two good packets
    buf.extend_from_slice(&[0x00; 10]);
    buf.extend_from_slice(&pkt(3));
    buf.extend_from_slice(&pkt(4));
    while let Some(p) = d.decode_eof(&mut buf).unwrap() {
        got.push(p[1]);
    }
    assert_eq!(
        got,
        vec![1, 2, 3, 4],
        "junk-prefixed datagram fully recovered"
    );
    assert_eq!(
        d.resync_stats().desyncs,
        0,
        "a new datagram is not a desync"
    );
    assert_eq!(d.resync_stats().resyncs, 2);
    assert_eq!(d.resync_stats().bytes_discarded, 10);
}

/// Review item 6: one corrupted sync byte must not cost more than the old framer's 1316-byte read,
/// however much the `FramedRead` buffer holds. 2 good packets, a corrupt one, then 100 good ones in
/// a single 8 KiB-class buffer: the stream resyncs in-buffer and the vast majority survives.
#[test]
fn a_mid_stream_desync_drops_at_most_one_old_read_not_the_whole_buffer() {
    let mut d = TsDecoder::new();
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&pkt(1));
    buf.extend_from_slice(&pkt(2));
    let mut bad = pkt(3);
    bad[0] = 0x00;
    buf.extend_from_slice(&bad);
    for t in 0..100u8 {
        buf.extend_from_slice(&pkt(10 + t));
    }
    let got = drain(&mut d, &mut buf);
    let s = d.resync_stats();
    assert_eq!(s.desyncs, 1);
    // 1316 dropped by the desync arm, plus < 188 skipped by the following resync
    assert!(
        s.bytes_discarded <= 1316 + 187,
        "discarded {} bytes for one corrupt sync byte",
        s.bytes_discarded
    );
    assert!(got.len() >= 2 + 85, "recovered only {} packets", got.len());
}

// ── RTP-encapsulated TS datagrams (#1099 / W-DS-2) ──────────────────────────

fn real_packets() -> Vec<[u8; 188]> {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/ts/m6-single.ts"
    ))
    .expect("committed fixture");
    bytes
        .chunks_exact(188)
        .map(|c| <[u8; 188]>::try_from(c).unwrap())
        .collect()
}

fn decode_datagram(d: &mut TsDecoder, datagram: &[u8]) -> Vec<Bytes> {
    let mut buf = BytesMut::from(datagram);
    let mut got = Vec::new();
    while let Some(p) = d.decode_eof(&mut buf).unwrap() {
        got.push(p);
    }
    got
}

/// A 12-byte RTP header (RFC 3550 §5.1: V=2, PT 33, no padding/ext/CSRC).
fn rtp_header() -> [u8; 12] {
    [
        0x80, 33, 0x12, 0x34, 0xAA, 0xBB, 0xCC, 0xDD, 0x01, 0x02, 0x03, 0x04,
    ]
}

/// Real packets, RTP-wrapped, where the RTP header carries a `0x47` exactly
/// 188 bytes before a `0x47` inside the first TS packet — a false two-packet
/// sync lattice for the stateless resync. Pre-fix the decoder latched onto
/// that header byte and emitted misaligned garbage instead of the 7 packets.
#[test]
fn rtp_wrapped_datagram_is_unwrapped_not_misframed_by_a_0x47_in_the_header() {
    let pkts = real_packets();
    // First packet with a 0x47 in bytes 178..=187 (header byte k = j - 176).
    let start = pkts
        .windows(7)
        .position(|w| w[0][178..188].contains(&0x47))
        .expect("a real packet with 0x47 near its end");
    let j = 178
        + pkts[start][178..188]
            .iter()
            .position(|&b| b == 0x47)
            .unwrap();
    let mut hdr = rtp_header();
    hdr[j - 176] = 0x47;
    let mut datagram = hdr.to_vec();
    for p in &pkts[start..start + 7] {
        datagram.extend_from_slice(p);
    }
    let mut d = TsDecoder::datagram();
    let got = decode_datagram(&mut d, &datagram);
    assert_eq!(got.len(), 7);
    for (g, want) in got.iter().zip(&pkts[start..start + 7]) {
        assert_eq!(&g[..], &want[..]);
    }
    assert_eq!(
        d.resync_stats().bytes_discarded,
        0,
        "nothing hunted/dropped"
    );
}

/// CSRC list + header extension + trailing padding are all skipped.
#[test]
fn rtp_csrc_extension_and_padding_are_skipped() {
    let pkts = real_packets();
    let mut datagram = vec![0xB2, 33, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3]; // V=2, P=1, X=1, CC=2
    datagram.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 2]); // two CSRCs
    datagram.extend_from_slice(&[0xBE, 0xDE, 0x00, 0x01, 0xDE, 0xAD, 0xBE, 0xEF]); // ext, 1 word
    for p in &pkts[..3] {
        datagram.extend_from_slice(p);
    }
    datagram.extend_from_slice(&[0, 0, 3]); // 3 bytes padding (last byte = count)
    let mut d = TsDecoder::datagram();
    let got = decode_datagram(&mut d, &datagram);
    assert_eq!(got.len(), 3);
    for (g, want) in got.iter().zip(&pkts) {
        assert_eq!(&g[..], &want[..]);
    }
}

/// A bare TS datagram (first byte `0x47`, RTP version bits `01`) is never
/// mistaken for RTP, and an RTP-looking header with a non-TS payload is left
/// to the ordinary resync/discard path.
#[test]
fn bare_ts_and_non_ts_rtp_payloads_are_not_unwrapped() {
    let pkts = real_packets();
    let mut d = TsDecoder::datagram();
    let mut bare = Vec::new();
    for p in &pkts[..5] {
        bare.extend_from_slice(p);
    }
    assert_eq!(decode_datagram(&mut d, &bare).len(), 5);

    let mut junk = rtp_header().to_vec();
    junk.extend_from_slice(&[0x55u8; 200]); // not a whole number of TS packets
    assert!(decode_datagram(&mut d, &junk).is_empty());
}
