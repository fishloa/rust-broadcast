const BOX_HEADER: usize = 8;
const LARGESIZE_HEADER: usize = 16;
const SIZE_IS_LARGE: u32 = 1;
const SIZE_TO_EOF: u32 = 0;
const TFHD_BASE_DATA_OFFSET_PRESENT: u32 = 1;
const TFHD_BASE_OFFSET_FIELD: usize = 16;

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

pub fn group(segments: &[Vec<u8>], count: usize) -> Vec<Vec<u8>> {
    let per = segments.len().div_ceil(count.max(1)).max(1);
    segments.chunks(per).map(|c| c.concat()).collect()
}

pub fn rebase_absolute_offsets(mut segment: Vec<u8>, base: u64) -> Vec<u8> {
    let mut pos = 0;
    while pos + BOX_HEADER <= segment.len() {
        let kind: [u8; 4] = segment[pos + 4..pos + 8].try_into().unwrap();
        if &kind == b"tfhd" {
            let flags =
                u32::from_be_bytes(segment[pos + 8..pos + 12].try_into().unwrap()) & 0x00ff_ffff;
            if flags & TFHD_BASE_DATA_OFFSET_PRESENT != 0 {
                segment[pos + TFHD_BASE_OFFSET_FIELD..pos + TFHD_BASE_OFFSET_FIELD + 8]
                    .copy_from_slice(&base.to_be_bytes());
            }
        }
        if &kind == b"moof" || &kind == b"traf" {
            pos += BOX_HEADER;
        } else {
            let size = u32::from_be_bytes(segment[pos..pos + 4].try_into().unwrap()) as usize;
            if size < BOX_HEADER {
                break;
            }
            pos += size;
        }
    }
    segment
}

const FULLBOX_HEADER: usize = 12;
const TKHD_TIMES_V0: usize = 8;
const TKHD_TIMES_V1: usize = 16;
const TKHD_TRACK_ID_V0: usize = FULLBOX_HEADER + TKHD_TIMES_V0;
const TKHD_TRACK_ID_V1: usize = FULLBOX_HEADER + TKHD_TIMES_V1;

fn tkhd_track_id(trak: &[u8]) -> Option<u32> {
    let mut pos = BOX_HEADER;
    while pos + BOX_HEADER <= trak.len() {
        let size = u32::from_be_bytes(trak[pos..pos + 4].try_into().ok()?) as usize;
        if size < BOX_HEADER || pos + size > trak.len() {
            return None;
        }
        if &trak[pos + 4..pos + 8] == b"tkhd" {
            let version = trak[pos + BOX_HEADER];
            let at = pos
                + if version == 1 {
                    TKHD_TRACK_ID_V1
                } else {
                    TKHD_TRACK_ID_V0
                };
            return Some(u32::from_be_bytes(trak.get(at..at + 4)?.try_into().ok()?));
        }
        pos += size;
    }
    None
}

pub fn drop_track(init: &[u8], track_id: u32) -> Vec<u8> {
    let boxes = top_level_boxes(init);
    let (_, moov_pos, moov_size) = *boxes.iter().find(|b| &b.0 == b"moov").expect("moov");
    let mut kept: Vec<u8> = Vec::new();
    let mut pos = moov_pos + BOX_HEADER;
    while pos < moov_pos + moov_size {
        let size = u32::from_be_bytes(init[pos..pos + 4].try_into().unwrap()) as usize;
        let child = &init[pos..pos + size];
        let is_target = &child[4..8] == b"trak" && tkhd_track_id(child) == Some(track_id);
        if !is_target {
            kept.extend_from_slice(child);
        }
        pos += size;
    }
    let mut out = init[..moov_pos].to_vec();
    out.extend_from_slice(&((kept.len() + BOX_HEADER) as u32).to_be_bytes());
    out.extend_from_slice(b"moov");
    out.extend_from_slice(&kept);
    out.extend_from_slice(&init[moov_pos + moov_size..]);
    out
}
