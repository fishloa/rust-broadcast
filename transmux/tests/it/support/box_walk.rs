#[cfg(feature = "cenc")]
pub const SAMPLE_TABLE_CONTAINERS: &[&[u8; 4]] = &[b"moov", b"trak", b"mdia", b"minf", b"stbl"];

pub fn find_box_range(
    data: &[u8],
    lo: usize,
    hi: usize,
    fourcc: &[u8; 4],
    containers: &[&[u8; 4]],
) -> Option<(usize, usize)> {
    for b in isobmff::BoxIter::new(data.get(lo..hi)?) {
        let Ok(b) = b else { break };
        let start = lo + b.start;
        let end = lo + b.end;
        if b.four_cc() == *fourcc {
            return Some((start, end - start));
        }
        if containers.contains(&&b.four_cc())
            && let Some(found) = find_box_range(
                data,
                start + b.header.header_size(),
                end,
                fourcc,
                containers,
            )
        {
            return Some(found);
        }
    }
    None
}
