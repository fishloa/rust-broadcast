const TS_PACKET: usize = 188;
const PID_MASK: u16 = 0x1fff;
const PUSI_BIT: u8 = 0x40;
const PAT_PID: u16 = 0;

fn is_pat_start(packet: &[u8]) -> bool {
    let pid = (u16::from(packet[1]) << 8 | u16::from(packet[2])) & PID_MASK;
    pid == PAT_PID && packet[1] & PUSI_BIT != 0
}

pub fn split_at_pat(bytes: &[u8], parts: usize) -> Vec<Vec<u8>> {
    let packets = bytes.len() / TS_PACKET;
    let pats: Vec<usize> = (0..packets)
        .filter(|&i| is_pat_start(&bytes[i * TS_PACKET..]))
        .collect();
    let mut cuts = vec![0usize];
    for k in 1..parts {
        let target = packets * k / parts;
        if let Some(&c) = pats.iter().min_by_key(|&&c| c.abs_diff(target))
            && c > *cuts.last().unwrap()
        {
            cuts.push(c);
        }
    }
    cuts.push(packets);
    cuts.windows(2)
        .map(|w| bytes[w[0] * TS_PACKET..w[1] * TS_PACKET].to_vec())
        .collect()
}

pub fn drop_leading_packets(segment: &[u8], n: usize) -> Vec<u8> {
    segment[n * TS_PACKET..].to_vec()
}
