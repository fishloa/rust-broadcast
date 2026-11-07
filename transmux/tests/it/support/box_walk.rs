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
