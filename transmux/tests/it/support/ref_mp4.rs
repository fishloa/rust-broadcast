use crate::box_iter::{Box4, iter_boxes};

fn find_box<'a>(buf: &'a [u8], base: usize, want: &[u8; 4]) -> Option<Box4<'a>> {
    for b in iter_boxes(buf, base) {
        if &b.ty == want {
            // Re-borrow with correct lifetimes.
            return Some(Box4 {
                ty: b.ty,
                offset: b.offset,
                size: b.size,
                header: b.header,
                body: b.body,
            });
        }
        let containers: &[[u8; 4]] = &[
            *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"mvex", *b"edts",
        ];
        if containers.contains(&b.ty)
            && let Some(found) = find_box(b.body, b.offset + b.header, want)
        {
            return Some(found);
        }
        if &b.ty == b"stsd" {
            // stsd: 8 bytes (version/flags + entry_count) before entries.
            if b.body.len() > 8
                && let Some(found) = find_box(&b.body[8..], b.offset + b.header + 8, want)
            {
                return Some(found);
            }
        }
        if &b.ty == b"avc1" || &b.ty == b"mp4a" {
            let skip = if &b.ty == b"avc1" { 78 } else { 28 };
            if b.body.len() > skip
                && let Some(found) = find_box(&b.body[skip..], b.offset + b.header + skip, want)
            {
                return Some(found);
            }
        }
    }
    None
}

pub struct RefTrack<'a> {
    pub handler: [u8; 4],
    pub trak_body: &'a [u8],
    pub trak_base: usize,
}

/// Collect every `trak` in the ref moov with its `hdlr` handler type.
pub fn ref_tracks(mp4: &[u8]) -> Vec<RefTrack<'_>> {
    let moov = find_box(mp4, 0, b"moov").expect("ref mp4 must have moov");
    let mut tracks = Vec::new();
    for b in iter_boxes(moov.body, moov.offset + moov.header) {
        if &b.ty == b"trak" {
            let hdlr = find_box(b.body, b.offset + b.header, b"hdlr").expect("trak has hdlr");
            // hdlr body: version/flags(4) + pre_defined(4) + handler_type(4).
            let handler = [hdlr.body[8], hdlr.body[9], hdlr.body[10], hdlr.body[11]];
            tracks.push(RefTrack {
                handler,
                trak_body: b.body,
                trak_base: b.offset + b.header,
            });
        }
    }
    tracks
}

/// Resolve the sample byte ranges of a track from stsz/stsc/stco (+ mdat), in
/// file order, returning each sample's bytes. Progressive-mp4 sample walk.
pub fn ref_track_samples(mp4: &[u8], trak_body: &[u8], trak_base: usize) -> Vec<Vec<u8>> {
    let stsz = find_box(trak_body, trak_base, b"stsz").expect("stsz");
    let stsc = find_box(trak_body, trak_base, b"stsc").expect("stsc");
    let stco = find_box(trak_body, trak_base, b"stco").expect("stco");

    // stsz: version/flags(4) + sample_size(4) + sample_count(4) + [size]*count.
    let default_size = u32::from_be_bytes([stsz.body[4], stsz.body[5], stsz.body[6], stsz.body[7]]);
    let sample_count =
        u32::from_be_bytes([stsz.body[8], stsz.body[9], stsz.body[10], stsz.body[11]]) as usize;
    let sizes: Vec<u32> = if default_size != 0 {
        vec![default_size; sample_count]
    } else {
        (0..sample_count)
            .map(|i| {
                let o = 12 + i * 4;
                u32::from_be_bytes([
                    stsz.body[o],
                    stsz.body[o + 1],
                    stsz.body[o + 2],
                    stsz.body[o + 3],
                ])
            })
            .collect()
    };

    // stco: version/flags(4) + entry_count(4) + [chunk_offset(4)]*.
    let chunk_count =
        u32::from_be_bytes([stco.body[4], stco.body[5], stco.body[6], stco.body[7]]) as usize;
    let chunk_offsets: Vec<u32> = (0..chunk_count)
        .map(|i| {
            let o = 8 + i * 4;
            u32::from_be_bytes([
                stco.body[o],
                stco.body[o + 1],
                stco.body[o + 2],
                stco.body[o + 3],
            ])
        })
        .collect();

    // stsc: version/flags(4) + entry_count(4) + [first_chunk, spc, sdi]*.
    let stsc_count =
        u32::from_be_bytes([stsc.body[4], stsc.body[5], stsc.body[6], stsc.body[7]]) as usize;
    let stsc_entries: Vec<(u32, u32)> = (0..stsc_count)
        .map(|i| {
            let o = 8 + i * 12;
            let first_chunk = u32::from_be_bytes([
                stsc.body[o],
                stsc.body[o + 1],
                stsc.body[o + 2],
                stsc.body[o + 3],
            ]);
            let spc = u32::from_be_bytes([
                stsc.body[o + 4],
                stsc.body[o + 5],
                stsc.body[o + 6],
                stsc.body[o + 7],
            ]);
            (first_chunk, spc)
        })
        .collect();

    // Expand stsc → samples-per-chunk for every chunk (1-based chunk index).
    let mut spc_per_chunk = vec![0u32; chunk_count];
    for (idx, &(first_chunk, spc)) in stsc_entries.iter().enumerate() {
        let last_chunk = if idx + 1 < stsc_entries.len() {
            stsc_entries[idx + 1].0 - 1
        } else {
            chunk_count as u32
        };
        for c in first_chunk..=last_chunk {
            if (c as usize) <= chunk_count {
                spc_per_chunk[(c - 1) as usize] = spc;
            }
        }
    }

    // Walk chunks, slicing sample bytes from the file at each chunk offset.
    let mut samples = Vec::with_capacity(sample_count);
    let mut sample_idx = 0usize;
    for (c, &chunk_off) in chunk_offsets.iter().enumerate() {
        let mut pos = chunk_off as usize;
        for _ in 0..spc_per_chunk[c] {
            if sample_idx >= sample_count {
                break;
            }
            let sz = sizes[sample_idx] as usize;
            samples.push(mp4[pos..pos + sz].to_vec());
            pos += sz;
            sample_idx += 1;
        }
    }
    assert_eq!(samples.len(), sample_count, "resolved all ref samples");
    samples
}
