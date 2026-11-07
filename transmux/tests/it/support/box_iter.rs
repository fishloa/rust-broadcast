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
