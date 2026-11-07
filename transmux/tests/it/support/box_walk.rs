#[cfg(feature = "cenc")]
pub const SAMPLE_TABLE_CONTAINERS: &[&[u8; 4]] = &[b"moov", b"trak", b"mdia", b"minf", b"stbl"];

pub fn find_box_range(
    data: &[u8],
    lo: usize,
    hi: usize,
    fourcc: &[u8; 4],
    containers: &[&[u8; 4]],
) -> Option<(usize, usize)> {
    let mut off = lo;
    while off + 8 <= hi {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > hi {
            break;
        }
        let t = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
        if &t == fourcc {
            return Some((off, size));
        }
        if containers.contains(&&t)
            && let Some(found) = find_box_range(data, off + 8, off + size, fourcc, containers)
        {
            return Some(found);
        }
        off += size;
    }
    None
}

pub struct Box4<'a> {
    pub ty: [u8; 4],
    pub offset: usize,
    pub size: usize,
    pub header: usize,
    pub body: &'a [u8],
}

/// Iterate the top-level boxes of `buf` (offsets absolute given `base`).
pub fn iter_boxes(buf: &[u8], base: usize) -> Vec<Box4<'_>> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= buf.len() {
        let size =
            u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
        let ty = [buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]];
        let (size, header) = if size == 1 {
            let s = u64::from_be_bytes([
                buf[off + 8],
                buf[off + 9],
                buf[off + 10],
                buf[off + 11],
                buf[off + 12],
                buf[off + 13],
                buf[off + 14],
                buf[off + 15],
            ]) as usize;
            (s, 16)
        } else {
            (size, 8)
        };
        if size < header || off + size > buf.len() {
            break;
        }
        out.push(Box4 {
            ty,
            offset: base + off,
            size,
            header,
            body: &buf[off + header..off + size],
        });
        off += size;
    }
    out
}
