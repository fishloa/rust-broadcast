const BOX_HEADER: usize = 8;
const LARGESIZE_HEADER: usize = 16;
const SIZE_IS_LARGE: u32 = 1;
const SIZE_TO_EOF: u32 = 0;

pub struct Split {
    pub init: Vec<u8>,
    pub segments: Vec<Vec<u8>>,
}

fn top_level_boxes(bytes: &[u8]) -> Vec<([u8; 4], usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + BOX_HEADER <= bytes.len() {
        let size32 = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap());
        let kind: [u8; 4] = bytes[pos + 4..pos + 8].try_into().unwrap();
        let size = if size32 == SIZE_IS_LARGE && pos + LARGESIZE_HEADER <= bytes.len() {
            u64::from_be_bytes(bytes[pos + 8..pos + 16].try_into().unwrap()) as usize
        } else if size32 == SIZE_TO_EOF {
            bytes.len() - pos
        } else {
            size32 as usize
        };
        if size < BOX_HEADER || pos + size > bytes.len() {
            break;
        }
        out.push((kind, pos, size));
        pos += size;
    }
    out
}

pub fn split(bytes: &[u8]) -> Option<Split> {
    let boxes = top_level_boxes(bytes);
    let first_moof = boxes.iter().position(|b| &b.0 == b"moof")?;
    let init_end = boxes[first_moof].1;
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (kind, pos, size) in &boxes[first_moof..] {
        match kind {
            b"moof" => spans.push((*pos, *pos + *size)),
            b"mdat" => {
                if let Some(last) = spans.last_mut() {
                    last.1 = *pos + *size;
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
