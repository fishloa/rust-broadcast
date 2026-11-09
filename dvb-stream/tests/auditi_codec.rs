use bytes::BytesMut;
use dvb_stream::TsDecoder;
use tokio_util::codec::Decoder;

fn packet(tag: u8) -> [u8; 188] {
    let mut p = [tag; 188];
    p[0] = 0x47;
    p
}

fn drain(dec: &mut TsDecoder, buf: &mut BytesMut) -> Vec<u8> {
    let mut tags = Vec::new();
    while let Some(p) = dec.decode(buf).unwrap() {
        tags.push(p[1]);
    }
    tags
}

#[test]
fn one_corrupt_sync_byte_loses_only_that_packet() {
    let mut dec = TsDecoder::new();
    let mut buf = BytesMut::new();
    for tag in 1..=10u8 {
        let mut p = packet(tag);
        if tag == 3 {
            p[0] = 0x00;
        }
        buf.extend_from_slice(&p);
    }
    let got = drain(&mut dec, &mut buf);
    assert_eq!(
        got,
        vec![1, 2, 4, 5, 6, 7, 8, 9, 10],
        "packets after one bad sync byte are aligned and must survive"
    );
}

#[test]
fn false_sync_in_a_short_first_read_does_not_misalign_the_stream() {
    let mut dec = TsDecoder::new();
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&[0x11, 0x22, 0x47, 0x33]);
    let mut stream = Vec::new();
    for tag in 1..=4u8 {
        stream.extend_from_slice(&packet(tag));
    }
    buf.extend_from_slice(&stream[..60]);
    let mut got = drain(&mut dec, &mut buf);
    buf.extend_from_slice(&stream[60..]);
    got.extend(drain(&mut dec, &mut buf));
    assert_eq!(got, vec![1, 2, 3, 4]);
}
