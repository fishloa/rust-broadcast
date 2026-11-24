use isobmff::segments::{MDAT, MOOF};

pub struct Split {
    pub init: Vec<u8>,
    pub segments: Vec<Vec<u8>>,
}

pub fn split(bytes: &[u8]) -> Option<Split> {
    let boxes: Vec<isobmff::BoxItem<'_>> = isobmff::BoxIter::new(bytes)
        .filter_map(|b| b.ok())
        .collect();
    let first_moof = boxes.iter().position(|b| b.four_cc() == MOOF)?;
    let init_end = boxes[first_moof].start;
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for b in &boxes[first_moof..] {
        match b.four_cc() {
            MOOF => spans.push((b.start, b.end)),
            MDAT => {
                if let Some(last) = spans.last_mut() {
                    last.1 = b.end;
                }
            }
            _ => {}
        }
    }
    Some(Split {
        init: bytes[..init_end].to_vec(),
        segments: spans
            .into_iter()
            .map(|(a, e)| bytes[a..e].to_vec())
            .collect(),
    })
}
