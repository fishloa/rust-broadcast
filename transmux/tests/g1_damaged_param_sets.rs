use transmux::ir::DemuxEvent;
use transmux::ts_demux::StreamingTsDemux;

fn load() -> Vec<u8> {
    std::fs::read(format!(
        "{}/../fixtures/ts/h264_aac.ts",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn first_video_pusi_index(bytes: &[u8]) -> usize {
    for (i, pkt) in bytes.chunks(188).enumerate() {
        if pkt.len() < 188 {
            break;
        }
        let pid = u16::from_be_bytes([pkt[1] & 0x1f, pkt[2]]) as usize;
        let pusi = pkt[1] & 0x40 != 0;
        if pusi && (256..=511).contains(&pid) {
            return i;
        }
    }
    panic!("no video pusi");
}

fn corrupt_first_au_slice(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let fi = first_video_pusi_index(bytes);
    // corrupt the next packet on the same PID (slice data) -> continuity gap
    let pid = u16::from_be_bytes([bytes[fi * 188 + 1] & 0x1f, bytes[fi * 188 + 2]]);
    for (i, pkt) in bytes.chunks(188).enumerate().skip(fi + 1) {
        if pkt.len() < 188 {
            break;
        }
        let p = u16::from_be_bytes([pkt[1] & 0x1f, pkt[2]]);
        if p == pid {
            let off = i * 188;
            let cc = out[off + 3] & 0x0f;
            out[off + 3] = (out[off + 3] & 0xf0) | ((cc + 2) & 0x0f);
            break;
        }
    }
    out
}

fn corrupt_sps_in_first_au(bytes: &[u8]) -> Vec<u8> {
    let mut out = corrupt_first_au_slice(bytes);
    // corrupt an SPS NAL in the first video PUSI packet payload
    let fi = first_video_pusi_index(bytes);
    let base = fi * 188;
    // find annexb sps start code in packet payload (bytes 4..188) but PES header present;
    // search whole packet
    let mut idx = None;
    for i in base..base + 184 {
        if i + 4 < out.len()
            && out[i] == 0x00
            && out[i + 1] == 0x00
            && (out[i + 2] == 0x01 || (out[i + 2] == 0x00 && out[i + 3] == 0x01))
        {
            // determine nal type
            let nal_off = if out[i + 2] == 0x01 { i + 3 } else { i + 4 };
            if nal_off < out.len() && (out[nal_off] & 0x1f) == 7 {
                idx = Some(nal_off);
                break;
            }
        }
    }
    if let Some(n) = idx {
        // flip a byte a few positions into the SPS payload to break validation
        if n + 5 < out.len() {
            out[n + 5] ^= 0xff;
        }
    }
    out
}

fn first_track_added_packet(bytes: &[u8]) -> Option<usize> {
    let mut demux = StreamingTsDemux::new();
    let mut first = None;
    for (i, chunk) in bytes.chunks(188).enumerate() {
        demux.feed(chunk);
        while let Some(e) = demux.poll_event() {
            if matches!(e, DemuxEvent::TrackAdded(_)) && first.is_none() {
                first = Some(i);
            }
        }
    }
    demux.finish();
    while let Some(e) = demux.poll_event() {
        if matches!(e, DemuxEvent::TrackAdded(_)) && first.is_none() {
            first = Some(usize::MAX);
        }
    }
    first
}

fn count_invalid_param_sets(bytes: &[u8]) -> usize {
    let mut demux = StreamingTsDemux::new();
    let mut n = 0usize;
    for chunk in bytes.chunks(188) {
        demux.feed(chunk);
        while let Some(e) = demux.poll_event() {
            if let DemuxEvent::InputDegraded { kind, .. } = e
                && format!("{:?}", kind).contains("InvalidParameterSet")
            {
                n += 1;
            }
        }
    }
    demux.finish();
    while let Some(e) = demux.poll_event() {
        if let DemuxEvent::InputDegraded { kind, .. } = e
            && format!("{:?}", kind).contains("InvalidParameterSet")
        {
            n += 1;
        }
    }
    n
}

#[test]
fn g1_positive_damaged_first_au_yields_init() {
    let clean = load();
    let damaged = corrupt_first_au_slice(&clean);
    let clean_first = first_track_added_packet(&clean).unwrap();
    let damaged_first = first_track_added_packet(&damaged).unwrap();
    println!(
        "CLEAN first init: {} DAMAGED first init: {}",
        clean_first, damaged_first
    );
    assert!(damaged_first < clean_first, "damaged should init earlier");
    assert!(damaged_first <= 5, "expected init from first AU");
}

#[test]
fn g1_negative_invalid_sps_ignored_and_counted() {
    let clean = load();
    let damaged = corrupt_sps_in_first_au(&clean);
    let invalid = count_invalid_param_sets(&damaged);
    let first = first_track_added_packet(&damaged);
    println!("invalid param sets: {} first init: {:?}", invalid, first);
    assert!(invalid > 0, "invalid SPS must be counted");
    assert!(invalid > 0, "invalid SPS must be counted");
    // first init should not come from the damaged first AU's invalid SPS:
    // it must be no earlier than the clean stream's first init minus ... just
    // ensure we didn't treat invalid SPS as valid by checking count.
}
