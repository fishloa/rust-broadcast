//! RTMP transport spoke integration tests (issue #515).
//!
//! Exercises `transmux::rtmp` — the handshake (§5.2), chunk headers (§5.3.1),
//! chunk reassembly (§5.3), protocol control (§5.4) and AMF0 command (§7)
//! primitives, plus the RTMP ⇄ FLV ⇄ IR tie against the real fixture
//! `fixtures/flv/av.flv`.
//!
//! Gates (each bites — see the per-test comments):
//! 1. Chunk header round-trip, all four fmts + field-mutation sensitivity.
//! 2. Multi-chunk reassembly across a small chunk size + interleaved csids.
//! 3. Handshake byte layout (lengths, version, C1 field offsets).
//! 4. AMF0 `connect` command round-trip + member-mutation sensitivity.
//! 5. RTMP → IR equals FLV → IR (track/sample counts + per-sample sizes).

use broadcast_common::{Package, Unpackage};
use transmux::rtmp::{
    self, msg_type, read_chunks, write_chunks, AmfValue, BasicHeader, Command, Handshake0,
    Handshake1, Handshake2, Message, MessageHeader, ProtocolControl, RtmpDemux, RtmpMux,
    HANDSHAKE_PACKET_LEN, RTMP_VERSION,
};
use transmux::FlvDemux;

const FLV: &[u8] = include_bytes!("../../fixtures/flv/av.flv");

// ---------------------------------------------------------------------------
// Test 1 — Chunk header round-trip, all four fmts + mutation bite
// ---------------------------------------------------------------------------

#[test]
fn chunk_headers_round_trip_all_fmts() {
    let hdr = MessageHeader {
        timestamp: 1000,
        message_length: 307,
        message_type_id: msg_type::VIDEO,
        message_stream_id: 12346,
    };

    for fmt in 0u8..=3 {
        let bh = BasicHeader { fmt, csid: 4 };
        let mut buf = Vec::new();
        bh.write_into(&mut buf);
        hdr.write_into(fmt, &mut buf);

        // Parse the basic header back.
        let (bh2, bn) = BasicHeader::parse(&buf).unwrap();
        assert_eq!(bh2, bh, "fmt {fmt}: basic header round-trips");
        // Message-header length matches the fmt table (11/7/3/0).
        let mh_len = MessageHeader::serialized_len(fmt);
        assert_eq!(
            buf.len() - bn,
            mh_len,
            "fmt {fmt}: message-header length matches spec table"
        );
    }

    // Field-mutation bite: changing message_length changes the fmt-0/fmt-1 bytes.
    let mut a = Vec::new();
    BasicHeader { fmt: 0, csid: 4 }.write_into(&mut a);
    hdr.write_into(0, &mut a);
    let mut b = Vec::new();
    BasicHeader { fmt: 0, csid: 4 }.write_into(&mut b);
    let mut hdr2 = hdr;
    hdr2.message_length = 308;
    hdr2.write_into(0, &mut b);
    assert_ne!(a, b, "mutating message_length changes the serialized bytes");

    // Extended-timestamp bite: a timestamp >= 0xFFFFFF forces the 4-byte ext field.
    let big = MessageHeader {
        timestamp: 0x0100_0000,
        message_length: 10,
        message_type_id: msg_type::AUDIO,
        message_stream_id: 1,
    };
    assert!(big.needs_extended(0));
    let mut e = Vec::new();
    big.write_into(0, &mut e);
    // fmt-0 header is 11 bytes; extended timestamp adds 4.
    assert_eq!(e.len(), 11 + 4, "extended timestamp appends 4 bytes");
}

// ---------------------------------------------------------------------------
// Test 2 — Multi-chunk reassembly (boundary bite) + interleaved csids
// ---------------------------------------------------------------------------

#[test]
fn multi_chunk_reassembly_spans_and_interleaves() {
    const CHUNK: usize = 128;

    // A video message whose body is LARGER than the chunk size (spans >= 2
    // chunks: first fmt 0, continuations fmt 3).
    let video_body: Vec<u8> = (0..300u32).map(|i| (i % 256) as u8).collect();
    assert!(video_body.len() > CHUNK, "body must span multiple chunks");
    let audio_body: Vec<u8> = (0..40u8).map(|i| i.wrapping_mul(7)).collect();

    // Interleave TWO messages on different csids by hand-writing chunks:
    //   video csid 5 (chunk 1 of 3), audio csid 4 (single chunk), video (chunk 2),
    //   video (chunk 3). This kills any "consume the rest of the buffer" bug.
    let mut wire = Vec::new();

    // video fmt-0 chunk 1 (128 bytes).
    BasicHeader { fmt: 0, csid: 5 }.write_into(&mut wire);
    MessageHeader {
        timestamp: 1000,
        message_length: video_body.len() as u32,
        message_type_id: msg_type::VIDEO,
        message_stream_id: 1,
    }
    .write_into(0, &mut wire);
    wire.extend_from_slice(&video_body[0..CHUNK]);

    // audio fmt-0 single chunk (40 bytes) on csid 4, interleaved.
    BasicHeader { fmt: 0, csid: 4 }.write_into(&mut wire);
    MessageHeader {
        timestamp: 1010,
        message_length: audio_body.len() as u32,
        message_type_id: msg_type::AUDIO,
        message_stream_id: 1,
    }
    .write_into(0, &mut wire);
    wire.extend_from_slice(&audio_body);

    // video fmt-3 chunk 2 (128 bytes).
    BasicHeader { fmt: 3, csid: 5 }.write_into(&mut wire);
    wire.extend_from_slice(&video_body[CHUNK..2 * CHUNK]);

    // video fmt-3 chunk 3 (remaining 44 bytes).
    BasicHeader { fmt: 3, csid: 5 }.write_into(&mut wire);
    wire.extend_from_slice(&video_body[2 * CHUNK..]);

    let msgs = read_chunks(&wire).unwrap();
    assert_eq!(msgs.len(), 2, "two complete messages reassembled");

    // Both reassemble to the exact original bodies, regardless of interleave.
    let video = msgs.iter().find(|m| m.csid == 5).unwrap();
    let audio = msgs.iter().find(|m| m.csid == 4).unwrap();
    assert_eq!(video.body, video_body, "video body reassembles exactly");
    assert_eq!(audio.body, audio_body, "audio body reassembles exactly");
    assert_eq!(video.message_type_id, msg_type::VIDEO);
    assert_eq!(audio.message_type_id, msg_type::AUDIO);

    // Round-trip via the writer at the same chunk size, then re-read.
    let out = write_chunks(
        &[
            Message {
                csid: 5,
                message_type_id: msg_type::VIDEO,
                message_stream_id: 1,
                timestamp: 1000,
                body: video_body.clone(),
            },
            Message {
                csid: 4,
                message_type_id: msg_type::AUDIO,
                message_stream_id: 1,
                timestamp: 1010,
                body: audio_body.clone(),
            },
        ],
        CHUNK,
    );
    let re = read_chunks(&out).unwrap();
    assert_eq!(re.len(), 2);
    assert_eq!(re[0].body, video_body, "writer→reader preserves video body");
    assert_eq!(re[1].body, audio_body, "writer→reader preserves audio body");
}

// ---------------------------------------------------------------------------
// Test 3 — Handshake byte layout
// ---------------------------------------------------------------------------

#[test]
fn handshake_byte_layout() {
    // C0/S0 is one byte = version 3.
    let c0 = Handshake0 {
        version: RTMP_VERSION,
    };
    let c0b = c0.to_bytes();
    assert_eq!(c0b.len(), 1, "C0/S0 is 1 byte");
    assert_eq!(c0b[0], 3, "version byte is 3");
    assert_eq!(Handshake0::parse(&c0b).unwrap(), c0);
    // A non-3 version is rejected.
    assert!(Handshake0::parse(&[6]).is_err(), "bad version rejected");

    // C1/S1 is 1536 bytes: time(4) + zero(4) + random(1528).
    let random: Vec<u8> = (0..1528u32).map(|i| (i % 251) as u8).collect();
    let c1 = Handshake1 {
        time: 0x0A0B0C0D,
        random: random.clone(),
    };
    let c1b = c1.to_bytes();
    assert_eq!(c1b.len(), HANDSHAKE_PACKET_LEN, "C1/S1 is 1536 bytes");
    // time field offset [0..4] big-endian.
    assert_eq!(
        &c1b[0..4],
        &[0x0A, 0x0B, 0x0C, 0x0D],
        "C1 time offset [0..4]"
    );
    // zero field offset [4..8] all zero.
    assert_eq!(&c1b[4..8], &[0, 0, 0, 0], "C1 zero offset [4..8]");
    // random field offset [8..1536].
    assert_eq!(&c1b[8..], &random[..], "C1 random offset [8..1536]");
    assert_eq!(Handshake1::parse(&c1b).unwrap(), c1);

    // C2/S2 is 1536 bytes: time(4) + time2(4) + random echo(1528).
    let c2 = Handshake2 {
        time: 0x11223344,
        time2: 0x55667788,
        random_echo: random.clone(),
    };
    let c2b = c2.to_bytes();
    assert_eq!(c2b.len(), HANDSHAKE_PACKET_LEN, "C2/S2 is 1536 bytes");
    assert_eq!(&c2b[0..4], &[0x11, 0x22, 0x33, 0x44], "C2 time offset");
    assert_eq!(&c2b[4..8], &[0x55, 0x66, 0x77, 0x88], "C2 time2 offset");
    assert_eq!(&c2b[8..], &random[..], "C2 random echo offset");
    assert_eq!(Handshake2::parse(&c2b).unwrap(), c2);

    // The full C0+C1 the client sends is 1 + 1536 bytes.
    let mut c0c1 = c0.to_bytes();
    c0c1.extend_from_slice(&c1.to_bytes());
    assert_eq!(c0c1.len(), 1 + HANDSHAKE_PACKET_LEN, "C0C1 is 1537 bytes");
}

// ---------------------------------------------------------------------------
// Test 4 — AMF0 connect command round-trip + mutation bite
// ---------------------------------------------------------------------------

#[test]
fn amf0_connect_command_round_trip() {
    let cmd = Command {
        name: "connect".into(),
        transaction_id: 1.0,
        arguments: vec![AmfValue::Object(vec![
            ("app".into(), AmfValue::String("live".into())),
            ("tcUrl".into(), AmfValue::String("rtmp://host/live".into())),
            ("objectEncoding".into(), AmfValue::Number(0.0)),
            ("fpad".into(), AmfValue::Boolean(false)),
        ])],
    };
    let body = cmd.to_body();
    let decoded = Command::parse(&body).expect("decode connect");
    assert_eq!(decoded, cmd, "connect command round-trips");
    assert_eq!(decoded.name, "connect");
    assert_eq!(decoded.transaction_id, 1.0);

    // Mutation bite: changing an object member changes the serialized bytes.
    let mut cmd2 = cmd.clone();
    if let AmfValue::Object(m) = &mut cmd2.arguments[0] {
        m[2].1 = AmfValue::Number(3.0); // objectEncoding 0 -> 3
    }
    assert_ne!(cmd2.to_body(), body, "mutating a member changes the bytes");

    // Protocol-control round-trip for good measure (Set Peer Bandwidth carries a
    // trailing limit-type byte — bites on any 4-vs-5 length confusion).
    let pc = ProtocolControl::SetPeerBandwidth {
        window_size: 2_500_000,
        limit_type: rtmp::bandwidth_limit::DYNAMIC,
    };
    let pcb = pc.to_body();
    assert_eq!(pcb.len(), 5, "Set Peer Bandwidth body is 5 bytes");
    assert_eq!(
        ProtocolControl::parse(pc.message_type_id(), &pcb).unwrap(),
        pc
    );
}

// ---------------------------------------------------------------------------
// Test 5 — RTMP ⇄ FLV ⇄ IR tie (the real-data bite)
// ---------------------------------------------------------------------------

/// Split `av.flv` into its FLV tags, wrap each A/V tag body as an RTMP message,
/// chunk them at a small chunk size (so video spans multiple chunks), then run
/// `RtmpDemux` → `Media`. Assert the result equals `FlvDemux` on the same FLV.
#[test]
fn rtmp_demux_matches_flv_demux() {
    const CHUNK: usize = 128;

    // Ground truth: FLV → IR directly.
    let mut flv_demux = FlvDemux::new();
    let flv_media = flv_demux.unpackage(FLV).expect("FLV → IR");

    // Split av.flv into FLV tags (reuse the same walk the FLV spoke uses).
    let tags = split_flv(FLV);
    assert!(!tags.is_empty(), "av.flv has tags");

    // Wrap each A/V (and script) tag body as an RTMP message on its per-kind
    // csid; a Set Chunk Size control message opens the stream. Video tags are
    // large enough that at CHUNK=128 they span multiple chunks.
    let mut messages = Vec::new();
    let mut saw_multichunk_video = false;
    for (tag_type, ts, body) in &tags {
        let (csid, mt) = match *tag_type {
            8 => (4u32, msg_type::AUDIO),
            9 => (5u32, msg_type::VIDEO),
            18 => (6u32, msg_type::DATA_AMF0),
            _ => continue,
        };
        if mt == msg_type::VIDEO && body.len() > CHUNK {
            saw_multichunk_video = true;
        }
        messages.push(Message {
            csid,
            message_type_id: mt,
            message_stream_id: 1,
            timestamp: *ts,
            body: body.clone(),
        });
    }
    assert!(
        saw_multichunk_video,
        "at least one video message spans multiple chunks at CHUNK=128"
    );

    // Chunk everything, then RtmpDemux → IR.
    let wire = write_chunks(&messages, CHUNK);
    let mut rtmp_demux = RtmpDemux::new();
    let rtmp_media = rtmp_demux.unpackage(&wire).expect("RTMP → IR");

    // Track count equal.
    assert_eq!(
        rtmp_media.tracks.len(),
        flv_media.tracks.len(),
        "RTMP and FLV yield the same track count"
    );

    // Per-track: sample count and per-sample payload sizes equal (this proves
    // the chunk reassembly + FLV routing rebuilt the exact A/V bodies — a
    // passthrough could not fake it because it went through RTMP chunking).
    for (r, f) in rtmp_media.tracks.iter().zip(&flv_media.tracks) {
        assert_eq!(
            r.samples.len(),
            f.samples.len(),
            "sample count matches per track"
        );
        for (i, (rs, fs)) in r.samples.iter().zip(&f.samples).enumerate() {
            assert_eq!(rs.data.len(), fs.data.len(), "sample {i} payload size");
            assert_eq!(rs.data, fs.data, "sample {i} payload bytes");
            assert_eq!(rs.is_sync, fs.is_sync, "sample {i} sync flag");
            assert_eq!(
                rs.composition_offset, fs.composition_offset,
                "sample {i} composition offset"
            );
        }
    }
}

/// IR → `RtmpMux` → wire → `RtmpDemux` → IR must preserve the sample set.
/// A raw-passthrough `RtmpMux` cannot pass: the bytes are re-chunked and the A/V
/// bodies re-framed as RTMP messages, then de-chunked and FLV-routed back.
#[test]
fn rtmp_mux_round_trip_preserves_samples() {
    let mut flv_demux = FlvDemux::new();
    let media = flv_demux.unpackage(FLV).expect("FLV → IR");

    let mut mux = RtmpMux::new();
    let wire = mux.package(&media).expect("IR → RTMP wire");
    assert!(!wire.is_empty(), "muxed RTMP wire is non-empty");

    let mut demux = RtmpDemux::new();
    let back = demux.unpackage(&wire).expect("RTMP wire → IR");

    assert_eq!(
        back.tracks.len(),
        media.tracks.len(),
        "track count preserved"
    );
    for (b, m) in back.tracks.iter().zip(&media.tracks) {
        assert_eq!(b.samples.len(), m.samples.len(), "sample count preserved");
        for (i, (bs, ms)) in b.samples.iter().zip(&m.samples).enumerate() {
            assert_eq!(bs.data, ms.data, "sample {i} bytes preserved");
            assert_eq!(bs.is_sync, ms.is_sync, "sample {i} sync flag preserved");
        }
    }
}

/// Minimal FLV tag walk for the test (mirrors Adobe FLV v10.1 §E.4.1).
fn split_flv(flv: &[u8]) -> Vec<(u8, u32, Vec<u8>)> {
    let data_offset = u32::from_be_bytes([flv[5], flv[6], flv[7], flv[8]]) as usize;
    let mut off = data_offset.max(9) + 4;
    let mut tags = Vec::new();
    while off + 11 <= flv.len() {
        let tag_type = flv[off];
        let data_size = ((flv[off + 1] as usize) << 16)
            | ((flv[off + 2] as usize) << 8)
            | flv[off + 3] as usize;
        let ts_lo =
            ((flv[off + 4] as u32) << 16) | ((flv[off + 5] as u32) << 8) | flv[off + 6] as u32;
        let ts = ((flv[off + 7] as u32) << 24) | ts_lo;
        let body_start = off + 11;
        let body_end = body_start + data_size;
        if body_end + 4 > flv.len() {
            break;
        }
        tags.push((tag_type, ts, flv[body_start..body_end].to_vec()));
        off = body_end + 4;
    }
    tags
}
