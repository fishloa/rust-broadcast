//! Serializers must reject a value wider than its wire bit-field instead of
//! shifting/masking it into a different, valid-looking segment (the #1129
//! class: `(field << 4)` on a `u8` silently drops the high bits). The segments
//! come from the real M6 capture; each is mutated so exactly one field is one
//! step over its range and must fail with `Error::FieldOverflow`, while the
//! UNMUTATED segment must still serialize byte-exactly (so rejecting
//! everything cannot satisfy the test).

use broadcast_common::{Parse, Serialize};
use dvb_subtitle::{AnySegment, Error, PesDataField};
fn is_overflow<T>(r: Result<T, Error>) -> bool {
    matches!(r, Err(Error::FieldOverflow(_)))
}

fn ser<T: Serialize<Error = Error>>(t: &T) -> Result<Vec<u8>, Error> {
    let mut buf = vec![0u8; t.serialized_len()];
    t.serialize_into(&mut buf)?;
    Ok(buf)
}

#[test]
fn over_range_fields_of_real_segments_are_rejected() {
    let mut pages = 0;
    let mut regions = 0;
    let mut cluts = 0;
    for raw in crate::support::data_fields(64) {
        let field = PesDataField::parse(&raw).unwrap();
        for seg in &field.segments {
            match seg {
                AnySegment::PageComposition(p) => {
                    pages += 1;
                    assert!(ser(p).is_ok());
                    let mut a = p.clone();
                    a.page_version_number = 16;
                    assert!(is_overflow(ser(&a)), "page_version_number");
                    let mut b = p.clone();
                    b.reserved = 4;
                    assert!(is_overflow(ser(&b)), "page reserved");
                }
                AnySegment::RegionComposition(r) => {
                    regions += 1;
                    assert!(ser(r).is_ok());
                    let edits: [fn(&mut dvb_subtitle::RegionCompositionSegment); 5] = [
                        |s| s.region_version_number = 16,
                        |s| s.reserved_byte1 = 8,
                        |s| s.region_4bit_pixel_code = 16,
                        |s| s.region_2bit_pixel_code = 4,
                        |s| s.reserved_byte9 = 4,
                    ];
                    for (i, edit) in edits.iter().enumerate() {
                        let mut m = r.clone();
                        edit(&mut m);
                        assert!(is_overflow(ser(&m)), "region edit {i}");
                    }
                }
                AnySegment::ClutDefinition(c) => {
                    cluts += 1;
                    assert!(ser(c).is_ok());
                    let mut a = c.clone();
                    a.clut_version_number = 16;
                    assert!(is_overflow(ser(&a)), "clut_version_number");
                    let mut b = c.clone();
                    b.reserved = 16;
                    assert!(is_overflow(ser(&b)), "clut reserved");
                }
                _ => {}
            }
        }
    }
    assert!(
        pages > 0 && regions > 0 && cluts > 0,
        "the capture must carry page ({pages}), region ({regions}) and CLUT ({cluts}) segments"
    );
}
