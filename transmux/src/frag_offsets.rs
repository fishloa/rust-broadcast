//! Fragmented-MP4 sample-data addressing — ISO/IEC 14496-12 §8.8.7 / §8.8.8.
//!
//! The single place in this crate that decides *where a fragment's sample
//! bytes live*. Both [`crate::media::Fmp4Demux`] and
//! [`crate::cenc_decrypt::CencDecryptor`] resolve `moof`/`trun` sample offsets
//! through [`sample_ranges`], so the two cannot drift (audit r05-W7).
//!
//! # The rules (§8.8.7 / §8.8.8)
//!
//! The **base data offset** for a track fragment is, in order of precedence:
//!
//! 1. `tfhd.base_data_offset`, when the `base-data-offset-present` flag
//!    (`0x000001`) is set — an **absolute** file offset ("an explicit anchor
//!    for the data offsets in each track run");
//! 2. otherwise, with `default-base-is-moof` (`0x020000`) set, the first byte of
//!    the enclosing `moof`;
//! 3. otherwise, the first byte of the enclosing `moof` for the **first** track
//!    fragment, and the end of the previous fragment's data for each later one
//!    ("Fragments 'inheriting' their offset in this way must all use the same
//!    data-reference").
//!
//! A `trun`'s `data_offset`, when present, is **relative to that traf base**
//! ("it is relative to the base-data-offset established in the track fragment
//! header") — never to the previous run's end. When absent, "the data for this
//! run starts immediately after the data of the previous run, or at the
//! base-data-offset defined by the track fragment header if this is the first
//! run in a track fragment".
//!
//! # Bounds
//!
//! Each resolved sample range must lie inside an `mdat` payload of the file
//! (§8.8.7's data-reference / "the data must be in the same file"), so a
//! hostile `trun` cannot make the demuxer hand back `moov`/`moof` bytes as
//! sample data; and the total bytes handed out is capped at the file length, so
//! many runs pointing at the same offset cannot amplify work. Both are checked
//! before a byte is sliced.

use alloc::vec::Vec;

use broadcast_common::Parse;

use crate::box_types::{BOX_HEADER_MIN_SIZE, BoxHeader, SIZE_TO_EOF};
use crate::error::{Error, Result};
use crate::movie_fragment::{MovieFragmentBox, TFHD_DEFAULT_BASE_IS_MOOF};

/// The `(start, end)` byte range of one sample, plus its timing/flags as the
/// containing `trun`/`tfhd` declare them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentSampleRange {
    /// File offset of the sample's first byte.
    pub start: usize,
    /// File offset one past the sample's last byte.
    pub end: usize,
    /// `trun.sample_duration`, else `tfhd.default_sample_duration`, else 0.
    pub duration: u32,
    /// `trun.sample_flags`, else `first_sample_flags` (sample 0), else the
    /// `tfhd` default. A sample is a sync sample when bit `[16]` is clear.
    pub flags: u32,
    /// `trun.sample_composition_time_offset`, else 0 — as `i64`, since the
    /// field is unsigned under `trun` version 0 (§8.8.8.2) and so can exceed
    /// `i32::MAX`.
    pub composition_offset: i64,
}

/// An `mdat` payload range (the box's first payload byte through its last).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MdatRange {
    start: usize,
    end: usize,
}

#[cfg(test)]
thread_local! {
    /// Number of whole-file `mdat` scans on this thread (work-counter for the
    /// "scan once per file, not once per `traf`" guarantee).
    pub(crate) static MDAT_SCANS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Collect every top-level `mdat`'s payload range from `file`.
///
/// Scan once per file and hand the result to [`sample_ranges_in`]; rescanning
/// per `traf` made demuxing O(fragments x boxes).
///
/// A box whose declared size runs past the file (a truncated capture) is
/// clamped to the file end rather than rejected: the caller may legitimately
/// hold a cut-short recording whose early fragments are intact. (`parse_box`
/// itself reports such a box as `BufferTooShort`, so the header is read with
/// [`BoxHeader::parse`] and the extent computed here.)
pub fn mdat_ranges(file: &[u8]) -> Vec<MdatRange> {
    #[cfg(test)]
    MDAT_SCANS.with(|c| c.set(c.get() + 1));
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset + BOX_HEADER_MIN_SIZE <= file.len() {
        let rest = &file[offset..];
        let Ok(header) = BoxHeader::parse(rest) else {
            break;
        };
        let hdr_sz = header.header_size();
        let total = if header.size == SIZE_TO_EOF as u64 {
            rest.len()
        } else {
            match usize::try_from(header.size) {
                Ok(t) if t >= hdr_sz => t,
                // A size below the header (framing is wrong) or one that does
                // not fit `usize`: nothing after it can be trusted.
                Ok(_) => break,
                Err(_) => rest.len(),
            }
        };
        if hdr_sz > rest.len() {
            break;
        }
        if header.box_type.is(b"mdat") {
            out.push(MdatRange {
                start: offset + hdr_sz,
                end: offset + total.min(rest.len()),
            });
        }
        offset += total.min(rest.len());
    }
    out
}

/// Whether `[start, end)` lies inside at least one `mdat` payload.
fn inside_mdat(mdats: &[MdatRange], start: usize, end: usize) -> bool {
    mdats.iter().any(|m| start >= m.start && end <= m.end)
}

/// Resolve a `moof`'s sample ranges for one track, in (traf, trun, sample)
/// order, applying §8.8.7/§8.8.8 and the bounds above.
///
/// `file` is the whole MP4; `moof_off` is the byte offset of the `moof` box
/// itself (its 8-byte header included).
///
/// Errors are [`Error::InvalidInput`] or [`Error::BufferTooShort`] — never a
/// panic — for a `trun` sample with no size, an offset/length that overflows,
/// a range outside every `mdat` payload, or a total exceeding the file length.
pub fn sample_ranges(
    file: &[u8],
    moof_off: usize,
    moof: &MovieFragmentBox,
    track_id: u32,
) -> Result<Vec<FragmentSampleRange>> {
    sample_ranges_in(file, &mdat_ranges(file), moof_off, moof, track_id)
}

/// [`sample_ranges`] with the file's `mdat` ranges supplied by the caller
/// (from one [`mdat_ranges`] call per file), so resolving many fragments does
/// not rescan the file for each.
pub fn sample_ranges_in(
    file: &[u8],
    mdats: &[MdatRange],
    moof_off: usize,
    moof: &MovieFragmentBox,
    track_id: u32,
) -> Result<Vec<FragmentSampleRange>> {
    let mut out = Vec::new();
    // Running end of the data consumed so far in this moof, used for a trun
    // without a `data_offset` and for the next traf with no explicit base.
    let mut data_end: i64 = moof_off as i64;
    // Total bytes handed out, so many runs aimed at one offset cannot amplify.
    let mut total: usize = 0;

    for traf in &moof.traf {
        let tfhd = &traf.tfhd;
        let explicit_base = match tfhd.base_data_offset {
            Some(bdo) => Some(
                i64::try_from(bdo)
                    .map_err(|_| Error::InvalidInput("tfhd.base_data_offset overflows i64"))?,
            ),
            None if tfhd.flags & TFHD_DEFAULT_BASE_IS_MOOF != 0 => Some(moof_off as i64),
            None => None,
        };
        let traf_base: i64 = explicit_base.unwrap_or(data_end);
        let mut run_cursor = traf_base;

        for trun in &traf.trun {
            // A trun's data_offset is relative to the **traf base**, never to
            // the previous run's end (§8.8.8).
            let run_base = match trun.data_offset {
                Some(off) => traf_base
                    .checked_add(off as i64)
                    .ok_or(Error::InvalidInput("trun.data_offset overflow"))?,
                None => run_cursor,
            };
            let mut cursor = run_base;
            for (i, ts) in trun.samples.iter().enumerate() {
                let size = ts
                    .sample_size
                    .or(tfhd.default_sample_size)
                    .ok_or(Error::InvalidInput(
                    "trun sample has no size (no trun.sample_size, no tfhd default_sample_size)",
                ))? as usize;
                let duration = ts
                    .sample_duration
                    .or(tfhd.default_sample_duration)
                    .unwrap_or(0);
                let flags = ts
                    .sample_flags
                    .or(if i == 0 {
                        trun.first_sample_flags
                    } else {
                        None
                    })
                    .or(tfhd.default_sample_flags)
                    .unwrap_or(0);
                let composition_offset = ts.sample_composition_time_offset.unwrap_or(0);

                let start = usize::try_from(cursor)
                    .map_err(|_| Error::InvalidInput("negative sample data offset"))?;
                let end = start
                    .checked_add(size)
                    .ok_or(Error::InvalidInput("sample offset + size overflow"))?;
                if tfhd.track_id == track_id {
                    total = total
                        .checked_add(size)
                        .ok_or(Error::InvalidInput("fragment sample bytes overflow"))?;
                    if total > file.len() {
                        return Err(Error::InvalidInput(
                            "fragment sample data exceeds the file length (overlapping runs?)",
                        ));
                    }
                    if end > file.len() {
                        return Err(Error::BufferTooShort {
                            need: end,
                            have: file.len(),
                            what: "fragment sample data",
                        });
                    }
                    if !inside_mdat(mdats, start, end) {
                        return Err(Error::InvalidInput(
                            "fragment sample range is outside every mdat payload",
                        ));
                    }
                    out.push(FragmentSampleRange {
                        start,
                        end,
                        duration,
                        flags,
                        composition_offset,
                    });
                }
                cursor = cursor
                    .checked_add(size as i64)
                    .ok_or(Error::InvalidInput("fragment data size overflow"))?;
            }
            run_cursor = cursor;
        }
        data_end = run_cursor.max(data_end);
    }
    Ok(out)
}

/// The base-media-decode-time of a `tfdt`, as a checked `i64`.
///
/// `base_media_decode_time()` is a `u64` off the wire; a value above
/// `i64::MAX` is [`Error::InvalidInput`] rather than a wrapped negative.
pub fn tfdt_as_i64(raw: u64) -> Result<i64> {
    i64::try_from(raw).map_err(|_| {
        Error::InvalidInput("tfdt base_media_decode_time overflows the signed decode-time range")
    })
}

/// Add a signed composition offset to a decode time, checked.
pub fn add_offset(dts: i64, offset: i64) -> Result<i64> {
    dts.checked_add(offset)
        .ok_or(Error::InvalidInput("sample pts overflow"))
}

/// Add a sample duration to a running decode-time cursor, checked.
pub fn add_duration(dts: i64, duration: u32) -> Result<i64> {
    dts.checked_add(i64::from(duration))
        .ok_or(Error::InvalidInput("sample dts overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_bytes(ty: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + payload.len());
        out.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
        out.extend_from_slice(ty);
        out.extend_from_slice(payload);
        out
    }

    fn container(ty: &[u8; 4], kids: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        for k in kids {
            p.extend_from_slice(k);
        }
        box_bytes(ty, &p)
    }

    fn tfhd(flags: u32, track_id: u32, base: Option<u64>) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&flags.to_be_bytes());
        p.extend_from_slice(&track_id.to_be_bytes());
        if let Some(b) = base {
            p.extend_from_slice(&b.to_be_bytes());
        }
        box_bytes(b"tfhd", &p)
    }

    const TRUN_DATA_OFFSET: u32 = 0x000001;
    const TRUN_SAMPLE_SIZE: u32 = 0x000200;
    fn trun(sizes: &[u32], data_offset: Option<i32>) -> Vec<u8> {
        let mut flags = TRUN_SAMPLE_SIZE;
        if data_offset.is_some() {
            flags |= TRUN_DATA_OFFSET;
        }
        let mut p = Vec::new();
        p.extend_from_slice(&flags.to_be_bytes());
        p.extend_from_slice(&(sizes.len() as u32).to_be_bytes());
        if let Some(o) = data_offset {
            p.extend_from_slice(&o.to_be_bytes());
        }
        for s in sizes {
            p.extend_from_slice(&s.to_be_bytes());
        }
        box_bytes(b"trun", &p)
    }

    fn mfhd() -> Vec<u8> {
        // FullBox: version(1) + flags(3) + sequence_number(4).
        let mut p = alloc::vec![0u8; 4];
        p.extend_from_slice(&1u32.to_be_bytes());
        box_bytes(b"mfhd", &p)
    }

    /// Build `[free][moof][mdat]`; returns the file and the moof's offset.
    fn file_with(trafs: &[Vec<u8>], mdat_body: &[u8]) -> (Vec<u8>, usize) {
        let mut kids = alloc::vec![mfhd()];
        kids.extend_from_slice(trafs);
        let moof = container(b"moof", &kids);
        let mdat = box_bytes(b"mdat", mdat_body);
        let mut file = box_bytes(b"free", &alloc::vec![0u8; 16]);
        let moof_off = file.len();
        file.extend_from_slice(&moof);
        file.extend_from_slice(&mdat);
        (file, moof_off)
    }

    fn parse_moof(file: &[u8], moof_off: usize) -> MovieFragmentBox {
        let sz = u32::from_be_bytes([
            file[moof_off],
            file[moof_off + 1],
            file[moof_off + 2],
            file[moof_off + 3],
        ]) as usize;
        MovieFragmentBox::parse_body(&file[moof_off + 8..moof_off + sz]).expect("moof")
    }

    /// The moof-relative offset of the mdat payload for a probe file.
    fn mdat_rel(file: &[u8], moof_off: usize, mdat_len: usize) -> i32 {
        let mdat_payload = file.len() - mdat_len;
        (mdat_payload - moof_off) as i32
    }

    /// A `trun`'s `data_offset` is relative to the **traf base**, not the
    /// previous run's end: two runs pointing at the same mdat byte both read it.
    #[test]
    fn second_run_data_offset_is_relative_to_traf_base() {
        let mdat_body: Vec<u8> = (0u8..8).collect();
        let probe = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, 1, None),
                trun(&[2, 2], Some(0)),
                trun(&[2, 2], Some(0)),
            ],
        );
        let (pf, moof_off) = file_with(&[probe], &mdat_body);
        let off = mdat_rel(&pf, moof_off, mdat_body.len());
        let mdat_payload = pf.len() - mdat_body.len();

        let traf = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, 1, None),
                trun(&[2, 2], Some(off)),
                trun(&[2, 2], Some(off)),
            ],
        );
        let (file, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&file, moof_off);
        let r = sample_ranges(&file, moof_off, &moof, 1).expect("ranges");
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].start, mdat_payload);
        assert_eq!(
            r[2].start, mdat_payload,
            "run 2 restarts at base + its offset"
        );
        assert_ne!(
            r[2].start, r[1].end,
            "a data_offset'd run must not continue after the previous run"
        );
    }

    /// A `trun` without `data_offset` continues after the previous run.
    #[test]
    fn implicit_run_continues_after_previous() {
        let mdat_body: Vec<u8> = (0u8..8).collect();
        let probe = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, 1, None),
                trun(&[2, 2], Some(0)),
                trun(&[2, 2], None),
            ],
        );
        let (pf, moof_off) = file_with(&[probe], &mdat_body);
        let off = mdat_rel(&pf, moof_off, mdat_body.len());
        let traf = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, 1, None),
                trun(&[2, 2], Some(off)),
                trun(&[2, 2], None),
            ],
        );
        let (file, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&file, moof_off);
        let r = sample_ranges(&file, moof_off, &moof, 1).expect("ranges");
        assert_eq!(r[2].start, r[1].end);
    }

    /// A run pointing into the `moof` (outside every `mdat`) is rejected.
    #[test]
    fn run_pointing_into_moof_is_rejected() {
        let mdat_body: Vec<u8> = (0u8..8).collect();
        let traf = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, 1, None),
                trun(&[2], Some(0)),
            ],
        );
        let (file, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&file, moof_off);
        assert!(sample_ranges(&file, moof_off, &moof, 1).is_err());
    }

    /// Total handed-out bytes are capped: 10k overlapping runs cannot amplify.
    #[test]
    fn many_overlapping_runs_are_capped() {
        let mdat_body: Vec<u8> = alloc::vec![0xAA; 8];
        let mut kids = alloc::vec![tfhd(TFHD_DEFAULT_BASE_IS_MOOF, 1, None)];
        for _ in 0..10_000 {
            kids.push(trun(&[8], Some(40)));
        }
        let traf = container(b"traf", &kids);
        let (file, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&file, moof_off);
        let err = sample_ranges(&file, moof_off, &moof, 1).unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "10k overlapping runs must be capped, got {err:?}"
        );
    }

    /// A `tfdt` above `i64::MAX` is rejected, not wrapped.
    #[test]
    fn hostile_tfdt_is_rejected() {
        assert!(tfdt_as_i64(u64::MAX).is_err());
        assert_eq!(tfdt_as_i64(5).unwrap(), 5);
    }

    /// Decode-time arithmetic is checked, not wrapping.
    #[test]
    fn dts_arithmetic_is_checked() {
        assert!(add_duration(i64::MAX, 1).is_err());
        assert!(add_offset(i64::MAX, 1).is_err());
        assert!(add_offset(i64::MIN, -1).is_err());
        assert_eq!(add_duration(10, 3).unwrap(), 13);
        assert_eq!(add_offset(10, -3).unwrap(), 7);
    }

    /// Number of whole-file `mdat` scans so far on this thread.
    fn scans() -> usize {
        MDAT_SCANS.with(|c| c.get())
    }

    /// A final `mdat` cut short mid-payload is clamped to the file end (the
    /// documented behaviour), so samples that lie inside the surviving bytes
    /// still resolve. `parse_box` reports such a box as `BufferTooShort`, which
    /// the old `mdat_ranges` treated as end-of-walk, dropping the range.
    #[test]
    fn truncated_final_mdat_is_clamped_not_dropped() {
        let mdat_body: Vec<u8> = (0u8..8).collect();
        let probe = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, 1, None),
                trun(&[2, 2], Some(0)),
            ],
        );
        let (pf, moof_off) = file_with(&[probe], &mdat_body);
        let off = mdat_rel(&pf, moof_off, mdat_body.len());
        let traf = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, 1, None),
                trun(&[2, 2], Some(off)),
            ],
        );
        let (full, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&full, moof_off);
        let payload = full.len() - mdat_body.len();
        // Keep 5 of the 8 payload bytes: both 2-byte samples survive.
        let cut = &full[..payload + 5];
        let ranges = mdat_ranges(cut);
        assert_eq!(
            ranges,
            alloc::vec![MdatRange {
                start: payload,
                end: cut.len()
            }]
        );
        let r = sample_ranges(cut, moof_off, &moof, 1).expect("early samples resolve");
        assert_eq!((r[0].start, r[0].end), (payload, payload + 2));
        assert_eq!((r[1].start, r[1].end), (payload + 2, payload + 4));
        // A sample past the cut is still rejected.
        let traf = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, 1, None),
                trun(&[2, 2, 2], Some(off)),
            ],
        );
        let (full3, moof_off3) = file_with(&[traf], &mdat_body);
        let moof3 = parse_moof(&full3, moof_off3);
        assert!(sample_ranges(&full3[..payload + 5], moof_off3, &moof3, 1).is_err());
    }

    /// `Fmp4Demux` scans the file's `mdat`s exactly once however many
    /// fragments/trafs it holds (was once per traf: O(fragments x boxes)).
    #[test]
    fn demux_scans_mdats_once_per_file() {
        use broadcast_common::Unpackage;
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/cenc_frag_layouts/clear_multitrun_omit_implicit.mp4"
        );
        let bytes = std::fs::read(path).expect("fixture");
        let before = scans();
        let media = crate::media::Fmp4Demux::new()
            .unpackage(bytes.as_slice())
            .expect("demux");
        assert!(media.tracks.iter().map(|t| t.samples.len()).sum::<usize>() > 1);
        assert_eq!(scans() - before, 1, "one mdat scan per file");
    }
}
