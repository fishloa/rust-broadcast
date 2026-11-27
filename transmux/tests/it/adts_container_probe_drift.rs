//! The ADTS header parser and `container-probe` must agree on what an ADTS
//! frame is: the probe scores a byte prefix as AAC-ADTS only when the header
//! parse accepts it, so a divergence silently misroutes a stream.

use broadcast_bitstream::aac_asc::{ADTS_HEADER_SIZE, build_adts_header, parse_adts_header};

#[test]
fn adts_agrees_with_container_probe() {
    let frame_len = (ADTS_HEADER_SIZE + 10) as u16;
    let mut frame = vec![0u8; frame_len as usize];
    frame[..ADTS_HEADER_SIZE].copy_from_slice(&build_adts_header(1, 4, 2, frame_len));
    let mut buf = Vec::new();
    for _ in 0..20 {
        buf.extend_from_slice(&frame);
    }
    let p = container_probe::probe(&buf);
    assert!(matches!(
        p,
        container_probe::Probe::Identified {
            format: container_probe::Format::AdtsAac,
            ..
        }
    ));
    assert!(parse_adts_header(&buf).is_ok());

    let mp3_frame = [0xFFu8, 0xFB, 0x90, 0x00, 0x00, 0x00, 0x00, 0x00];
    let mut buf2 = Vec::new();
    for _ in 0..20 {
        buf2.extend_from_slice(&mp3_frame);
    }
    let p2 = container_probe::probe(&buf2);
    assert!(!matches!(
        p2,
        container_probe::Probe::Identified {
            format: container_probe::Format::AdtsAac,
            ..
        }
    ));
    assert!(parse_adts_header(&mp3_frame).is_err());
}

#[test]
fn mp3_layer_iii_header_rejected() {
    let hdr = [0xFFu8, 0xFB, 0x90, 0x00, 0x00, 0x00, 0x00];
    let err = parse_adts_header(&hdr).unwrap_err();
    assert!(
        matches!(err, broadcast_bitstream::Error::InvalidValue(v) if v.field == "adts_layer" && v.value == 0x02)
    );
}

#[test]
fn adts_syncword_error_reports_the_offending_bytes() {
    let hdr = [0xABu8, 0xCD, 0x00, 0x00, 0x00, 0x00, 0x00];
    let err = parse_adts_header(&hdr).unwrap_err();
    assert!(
        matches!(err, broadcast_bitstream::Error::InvalidValue(v) if v.field == "adts_syncword" && v.value == 0xABCD)
    );
}

#[test]
fn frame_length_below_header_length_rejected() {
    let hdr = [0xFFu8, 0xF1, 0x00, 0x00, 0x00, 0x00, 0x00];
    let err = parse_adts_header(&hdr).unwrap_err();
    assert!(
        matches!(err, broadcast_bitstream::Error::InvalidValue(v) if v.field == "adts_frame_length")
    );
}
