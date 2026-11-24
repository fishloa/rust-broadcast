//! Fragmented-MP4 sample-data addressing — ISO/IEC 14496-12:2026 §8.8.7 / §8.8.8.
//!
//! The single place in this crate that decides *where a fragment's sample
//! bytes live*. Both the fMP4 demuxer and the CENC decryptor (in the muxing
//! hub above this crate) resolve `moof`/`trun` sample offsets through
//! [`sample_ranges_in`], so the two cannot drift (audit r05-W7).
//!
//! Addressing takes the parsed `moof`, its absolute offset and the `mdat`
//! payload ranges that follow — never a slice of the file — so a reader that
//! walked only box headers can address a fragment without holding its media.
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
//! sample data; and the bytes handed out over a whole demux run are capped at
//! [`sample_bytes_budget`] (one input length), so neither many runs pointing at
//! the same offset nor many `moof`s each claiming one `mdat` can amplify the
//! work: every claimant would copy those bytes again (audit 20 findings 6 and
//! 13). Both are checked before a byte is sliced.

use crate::TrackId;
use alloc::vec::Vec;

use crate::box_types::BoxIter;
use crate::error::{Error, Result};
use crate::movie_fragment::{MovieFragmentBox, TFHD_DEFAULT_BASE_IS_MOOF};

/// The `(start, end)` byte range of one sample, plus its timing/flags as the
/// containing `trun`/`tfhd` declare them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentSampleRange {
    /// File offset of the sample's first byte.
    pub start: u64,
    /// File offset one past the sample's last byte.
    pub end: u64,
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

/// An `mdat` payload range (the box's first payload byte through its last),
/// in absolute file offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MdatRange {
    start: u64,
    end: u64,
}

impl MdatRange {
    /// Build one from the payload's first byte and one past its last byte.
    #[must_use]
    pub const fn new(start: u64, end: u64) -> Self {
        Self { start, end }
    }

    /// File offset of the payload's first byte.
    #[must_use]
    pub const fn start(&self) -> u64 {
        self.start
    }

    /// File offset one past the payload's last byte.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.end
    }
}

// Test instrumentation only (feature `scan-counters`): number of whole-file
// `mdat` scans on this thread, so the muxing hub above this crate can assert
// its demuxer scans once per file. Compiled out of the default build.
#[cfg(feature = "scan-counters")]
std::thread_local! {
    static MDAT_SCANS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Number of whole-file `mdat` scans performed on the current thread.
#[cfg(feature = "scan-counters")]
#[doc(hidden)]
pub fn mdat_scans_on_thread() -> usize {
    MDAT_SCANS.with(core::cell::Cell::get)
}

#[cfg(test)]
thread_local! {
    /// Number of `mdat` ranges [`inside_mdat`]'s search compared on this thread
    /// (work-counter for the "log mdats per sample, not mdats x samples"
    /// guarantee).
    static MDAT_PROBES: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Collect every top-level `mdat`'s payload range from `file`.
///
/// The convenience for a caller that holds the whole file: scan once per file
/// and hand the result to [`sample_ranges_in`]; rescanning per `traf` made
/// demuxing O(fragments x boxes). A caller walking box headers itself (the
/// reader) builds the same ranges with [`MdatRange::new`].
///
/// A box whose declared size runs past the file (a truncated capture) is
/// clamped to the file end rather than rejected: the caller may legitimately
/// hold a cut-short recording whose early fragments are intact, which is why
/// the walk is [`BoxIter::clamped`] and not [`BoxIter::new`].
pub fn mdat_ranges(file: &[u8]) -> Vec<MdatRange> {
    #[cfg(feature = "scan-counters")]
    MDAT_SCANS.with(|c| c.set(c.get() + 1));
    BoxIter::clamped(file)
        .filter_map(|b| b.ok())
        .filter(|b| b.header.box_type.is(b"mdat"))
        .map(|b| MdatRange::new(b.payload_start() as u64, b.end as u64))
        .collect()
}

/// Whether `[start, end)` lies inside at least one `mdat` payload.
///
/// The ranges are file-ordered and disjoint, so at most the **first** range
/// whose end reaches `end` can contain the sample: a binary search makes the
/// check O(log mdats) instead of a scan of every payload for every sample. A
/// file of 8-byte empty `mdat`s with a `trun` of zero-size samples made the
/// scan O(samples x mdats) — 2e11 comparisons for 1.6 MB (audit 20 finding 39).
fn inside_mdat(mdats: &[MdatRange], start: u64, end: u64) -> bool {
    let i = mdats.partition_point(|m| {
        #[cfg(test)]
        MDAT_PROBES.with(|c| c.set(c.get() + 1));
        m.end < end
    });
    mdats.get(i).is_some_and(|m| start >= m.start)
}

/// How many times the input's own length one demux run may resolve as sample
/// bytes.
///
/// Sample data that does not overlap cannot total more than the container
/// holds, so the honest allowance is exactly one input length. A larger total
/// is the same bytes handed out again — an overlapping `stco`/`stsc` chunk
/// layout, overlapping `trun` runs, or several `moof`s each claiming one
/// `mdat` — and every claimant copies them, turning a linear input into
/// quadratic memory (audit 20 findings 6 and 13).
pub const MAX_SAMPLE_BYTES_PER_INPUT_LENGTH: u64 = 1;

/// The aggregate sample-byte allowance for one demux run over an
/// `input_len`-byte input: [`MAX_SAMPLE_BYTES_PER_INPUT_LENGTH`] times its
/// length, which is what [`Error::SampleDataBudgetExceeded`] reports as its
/// `limit`.
#[must_use]
pub fn sample_bytes_budget(input_len: u64) -> u64 {
    input_len.saturating_mul(MAX_SAMPLE_BYTES_PER_INPUT_LENGTH)
}

/// Resolve a `moof`'s sample ranges for one track, in (traf, trun, sample)
/// order, applying §8.8.7/§8.8.8 and the bounds above, with a fresh run-wide
/// sample-byte counter.
///
/// `moof_off` is the absolute file offset of the `moof` box itself (its header
/// included) and `mdats` are the payload ranges the fragment's data may live
/// in; no byte of the file is needed, so a reader that walked only box headers
/// can address the fragment.
///
/// Errors are [`Error::InvalidInput`] or [`Error::SampleDataBudgetExceeded`] —
/// never a panic — for a `trun` sample with no size, an offset/length that
/// overflows, a range outside every `mdat` payload, or a total exceeding
/// `budget`.
pub fn sample_ranges_in(
    moof_off: u64,
    moof: &MovieFragmentBox,
    track_id: TrackId,
    mdats: &[MdatRange],
    budget: u64,
) -> Result<Vec<FragmentSampleRange>> {
    let mut copied_bytes = 0u64;
    sample_ranges_in_run(moof_off, moof, track_id, mdats, budget, &mut copied_bytes)
}

/// [`sample_ranges_in`] with the caller's run-wide sample-byte counter.
///
/// `copied_bytes` is the running total for **one demux run** and is never
/// refilled: pass the same counter to every `moof` of a file, so many fragments
/// that each claim the same `mdat` cannot copy it again per fragment. It is
/// bounded by [`sample_bytes_budget`], which non-overlapping fragments can
/// never exceed — one `mdat` read once, however it is split into fragments
/// (audit 20 finding 13).
pub fn sample_ranges_in_run(
    moof_off: u64,
    moof: &MovieFragmentBox,
    track_id: TrackId,
    mdats: &[MdatRange],
    budget: u64,
    copied_bytes: &mut u64,
) -> Result<Vec<FragmentSampleRange>> {
    let (ranges, truncated) =
        sample_ranges_in_run_until(moof_off, moof, track_id, mdats, budget, copied_bytes)?;
    if truncated {
        return Err(Error::InvalidInput(
            "fragment sample range is outside every mdat payload",
        ));
    }
    Ok(ranges)
}

/// The same walk as [`sample_ranges_in_run`], stopping at the first sample
/// whose range leaves every `mdat` payload located so far.
///
/// The flag is `true` when such a sample was met; the returned ranges then
/// cover only the samples fully inside those payloads. This is how a reader at
/// the end of a truncated input hands out the complete prefix of a cut final
/// fragment instead of dropping the whole fragment.
pub fn sample_ranges_in_run_until(
    moof_off: u64,
    moof: &MovieFragmentBox,
    track_id: TrackId,
    mdats: &[MdatRange],
    budget: u64,
    copied_bytes: &mut u64,
) -> Result<(Vec<FragmentSampleRange>, bool)> {
    let mut out = Vec::new();
    // Running end of the data consumed so far in this moof, used for a trun
    // without a `data_offset` and for the next traf with no explicit base.
    let mut data_end: u64 = moof_off;

    for traf in &moof.traf {
        let tfhd = &traf.tfhd;
        let explicit_base = match tfhd.base_data_offset {
            Some(bdo) => Some(bdo),
            None if tfhd.flags & TFHD_DEFAULT_BASE_IS_MOOF != 0 => Some(moof_off),
            None => None,
        };
        let traf_base: u64 = explicit_base.unwrap_or(data_end);
        let mut run_cursor = traf_base;

        for trun in &traf.trun {
            // A trun's data_offset is relative to the **traf base**, never to
            // the previous run's end (§8.8.8).
            let run_base = match trun.data_offset {
                Some(off) => shift(traf_base, off)?,
                None => run_cursor,
            };
            let mut cursor = run_base;
            for (i, ts) in trun.samples.iter().enumerate() {
                let size = u64::from(ts.sample_size.or(tfhd.default_sample_size).ok_or(
                    Error::InvalidInput(
                        "trun sample has no size (no trun.sample_size, no tfhd default_sample_size)",
                    ),
                )?);
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

                let start = cursor;
                let end = start
                    .checked_add(size)
                    .ok_or(Error::InvalidInput("sample offset + size overflow"))?;
                if tfhd.track_id == track_id {
                    *copied_bytes = copied_bytes
                        .checked_add(size)
                        .ok_or(Error::SampleDataBudgetExceeded { limit: budget })?;
                    if *copied_bytes > budget {
                        return Err(Error::SampleDataBudgetExceeded { limit: budget });
                    }
                    if !inside_mdat(mdats, start, end) {
                        return Ok((out, true));
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
                    .checked_add(size)
                    .ok_or(Error::InvalidInput("fragment data size overflow"))?;
            }
            run_cursor = cursor;
        }
        data_end = run_cursor.max(data_end);
    }
    Ok((out, false))
}

/// Apply a `trun.data_offset` to the track fragment's base, checked.
///
/// The offset is a signed 32-bit field (§8.8.8.2) relative to an unsigned base,
/// so a negative offset that lands before the start of the file is
/// [`Error::InvalidInput`] rather than a wrapped huge offset.
fn shift(base: u64, delta: i32) -> Result<u64> {
    if delta >= 0 {
        base.checked_add(u64::from(delta.unsigned_abs()))
            .ok_or(Error::InvalidInput("trun.data_offset overflow"))
    } else {
        base.checked_sub(u64::from(delta.unsigned_abs()))
            .ok_or(Error::InvalidInput(
                "trun.data_offset before the start of the file",
            ))
    }
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

    fn tfhd(flags: u32, track_id: TrackId, base: Option<u64>) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&flags.to_be_bytes());
        p.extend_from_slice(&track_id.value().to_be_bytes());
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

    /// Resolve one `moof` against the whole file's `mdat` payloads, the way a
    /// demuxer that holds the file does.
    fn ranges(
        file: &[u8],
        moof_off: usize,
        moof: &MovieFragmentBox,
        track: TrackId,
    ) -> Result<Vec<FragmentSampleRange>> {
        sample_ranges_in(
            moof_off as u64,
            moof,
            track,
            &mdat_ranges(file),
            sample_bytes_budget(file.len() as u64),
        )
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
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None),
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
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None),
                trun(&[2, 2], Some(off)),
                trun(&[2, 2], Some(off)),
            ],
        );
        let (file, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&file, moof_off);
        let r = ranges(&file, moof_off, &moof, TrackId::new(1)).expect("ranges");
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].start, mdat_payload as u64);
        assert_eq!(
            r[2].start, mdat_payload as u64,
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
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None),
                trun(&[2, 2], Some(0)),
                trun(&[2, 2], None),
            ],
        );
        let (pf, moof_off) = file_with(&[probe], &mdat_body);
        let off = mdat_rel(&pf, moof_off, mdat_body.len());
        let traf = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None),
                trun(&[2, 2], Some(off)),
                trun(&[2, 2], None),
            ],
        );
        let (file, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&file, moof_off);
        let r = ranges(&file, moof_off, &moof, TrackId::new(1)).expect("ranges");
        assert_eq!(r[2].start, r[1].end);
    }

    /// A run pointing into the `moof` (outside every `mdat`) is rejected.
    #[test]
    fn run_pointing_into_moof_is_rejected() {
        let mdat_body: Vec<u8> = (0u8..8).collect();
        let traf = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None),
                trun(&[2], Some(0)),
            ],
        );
        let (file, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&file, moof_off);
        assert!(matches!(
            ranges(&file, moof_off, &moof, TrackId::new(1)),
            Err(Error::InvalidInput(_))
        ));
    }

    const TFHD_BASE_DATA_OFFSET_PRESENT: u32 = 0x000001;

    /// The search keeps the containment rule exact: a sample must fit inside
    /// ONE payload, so one that straddles two adjacent ranges — or ends in a
    /// gap between them — is not contained.
    #[test]
    fn mdat_lookup_rejects_ranges_straddling_payloads() {
        let mdats = [
            MdatRange::new(100, 200),
            MdatRange::new(300, 400),
            MdatRange::new(500, 600),
        ];
        assert!(inside_mdat(&mdats, 100, 200));
        assert!(inside_mdat(&mdats, 300, 301));
        assert!(inside_mdat(&mdats, 500, 501));
        assert!(!inside_mdat(&mdats, 150, 250), "straddles a gap");
        assert!(!inside_mdat(&mdats, 250, 350), "starts in a gap");
        assert!(!inside_mdat(&mdats, 450, 550), "spans two payloads");
        assert!(!inside_mdat(&mdats, 0, 0), "before the first payload");
        assert!(!inside_mdat(&mdats, 700, 700), "after the last payload");
    }

    /// Locating a sample in its `mdat` is a binary search: with R payloads and
    /// S samples that all land in the last one, the comparisons stay O(S log R)
    /// instead of O(S x R) — the linear scan made a file of 8-byte empty
    /// `mdat`s quadratic in `samples x mdats` (audit 20 finding 39).
    #[test]
    fn mdat_lookup_work_is_logarithmic_in_payloads() {
        const MDATS: usize = 4096;
        const SAMPLES: usize = 512;
        const BOX_HEADER: usize = 8;
        let mut file = Vec::new();
        for _ in 0..MDATS {
            file.extend_from_slice(&box_bytes(b"mdat", &[]));
        }
        let moof_off = file.len();
        // Zero-size samples that all land in the trailing payload: every one of
        // them is still located in an `mdat`, and none advances the cursor, so
        // the count (not the byte total) drives the lookups.
        let sizes = alloc::vec![0u32; SAMPLES];
        let moof_len = container(
            b"moof",
            &[
                mfhd(),
                container(
                    b"traf",
                    &[
                        tfhd(TFHD_BASE_DATA_OFFSET_PRESENT, TrackId::new(1), Some(0)),
                        trun(&sizes, Some(0)),
                    ],
                ),
            ],
        )
        .len();
        // The trailing `mdat` carries no payload, so its payload range is the
        // single position one past its header: the explicit base points there.
        let payload = (moof_off + moof_len + BOX_HEADER) as u64;
        let traf = container(
            b"traf",
            &[
                tfhd(
                    TFHD_BASE_DATA_OFFSET_PRESENT,
                    TrackId::new(1),
                    Some(payload),
                ),
                trun(&sizes, Some(0)),
            ],
        );
        file.extend_from_slice(&container(b"moof", &[mfhd(), traf]));
        file.extend_from_slice(&box_bytes(b"mdat", &[]));

        let moof = parse_moof(&file, moof_off);
        MDAT_PROBES.with(|c| c.set(0));
        let resolved = ranges(&file, moof_off, &moof, TrackId::new(1))
            .expect("zero-size samples in the trailing payload resolve");
        let probes = MDAT_PROBES.with(core::cell::Cell::get);
        assert_eq!(resolved.len(), SAMPLES);
        assert!(
            probes <= SAMPLES * 16,
            "expected ~log2({MDATS}) probes per sample, got {probes} for {SAMPLES} samples"
        );
    }

    /// Total handed-out bytes are capped: 10k overlapping runs cannot amplify.
    #[test]
    fn many_overlapping_runs_are_capped() {
        let mdat_body: Vec<u8> = alloc::vec![0xAA; 8];
        let mut kids = alloc::vec![tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None)];
        for _ in 0..10_000 {
            kids.push(trun(&[8], Some(40)));
        }
        let traf = container(b"traf", &kids);
        let (file, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&file, moof_off);
        let err = ranges(&file, moof_off, &moof, TrackId::new(1)).unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "10k overlapping runs must be capped, got {err:?}"
        );
    }

    /// One run's byte budget is never refilled: fragments that each claim the
    /// same `mdat` are refused once their total passes the input's length, so
    /// N `moof`s over one `mdat` cannot copy it N times (audit 20 findings
    /// 13 / 57).
    #[test]
    fn repeated_moofs_share_one_sample_byte_budget() {
        let mdat_body: Vec<u8> = (0u8..8).collect();
        let probe = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None),
                trun(&[2, 2], Some(0)),
            ],
        );
        let (pf, moof_off) = file_with(&[probe], &mdat_body);
        let off = mdat_rel(&pf, moof_off, mdat_body.len());
        let traf = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None),
                trun(&[2, 2], Some(off)),
            ],
        );
        let (file, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&file, moof_off);
        let mdats = mdat_ranges(&file);
        let budget = sample_bytes_budget(file.len() as u64);
        let mut copied_bytes = 0u64;
        let mut handed_out = 0u64;
        let mut refused = None;
        // Bounded: each round claims 4 bytes, so the budget is crossed well
        // before this loop ends.
        for _ in 0..64 {
            match sample_ranges_in_run(
                moof_off as u64,
                &moof,
                TrackId::new(1),
                &mdats,
                budget,
                &mut copied_bytes,
            ) {
                Ok(ranges) => {
                    handed_out += ranges.iter().map(|r| r.end - r.start).sum::<u64>();
                }
                Err(err) => {
                    refused = Some(err);
                    break;
                }
            }
        }
        assert!(
            matches!(refused, Some(Error::SampleDataBudgetExceeded { limit }) if limit == budget),
            "fragments claiming one mdat must be refused past the input length, got {refused:?}"
        );
        assert!(
            handed_out <= budget,
            "handed out {handed_out} bytes of a {budget}-byte budget"
        );
    }

    /// The allowance is inclusive: a run copying exactly the budget is
    /// accepted, and the same run one byte over the budget is refused.
    #[test]
    fn sample_byte_budget_boundary_is_inclusive() {
        const SAMPLE: usize = 64;
        const RUNS: usize = 16;
        let mdat_body: Vec<u8> = alloc::vec![0x5A; SAMPLE];
        let mut probe_kids = alloc::vec![tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None)];
        for _ in 0..RUNS {
            probe_kids.push(trun(&[SAMPLE as u32], Some(0)));
        }
        let (pf, moof_off) = file_with(&[container(b"traf", &probe_kids)], &mdat_body);
        let off = mdat_rel(&pf, moof_off, mdat_body.len());
        let mut kids = alloc::vec![tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None)];
        for _ in 0..RUNS {
            kids.push(trun(&[SAMPLE as u32], Some(off)));
        }
        let traf = container(b"traf", &kids);
        let (base_file, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&base_file, moof_off);
        let total = RUNS * SAMPLE;

        // A trailing `free` box pads the input so `sample_bytes_budget` lands
        // on a chosen byte count (the box header itself counts).
        let padded = |budget: usize| {
            let pad = budget
                .checked_sub(base_file.len() + 8)
                .expect("the sample total must exceed the unpadded file");
            let mut file = base_file.clone();
            file.extend_from_slice(&box_bytes(b"free", &alloc::vec![0u8; pad]));
            assert_eq!(sample_bytes_budget(file.len() as u64), budget as u64);
            file
        };

        let file = padded(total);
        let mut copied_bytes = 0u64;
        let ranges = sample_ranges_in_run(
            moof_off as u64,
            &moof,
            TrackId::new(1),
            &mdat_ranges(&file),
            sample_bytes_budget(file.len() as u64),
            &mut copied_bytes,
        )
        .expect("a run copying exactly the budget is accepted");
        assert_eq!(ranges.len(), RUNS);
        assert_eq!(copied_bytes, total as u64);

        let file = padded(total - 1);
        let mut copied_bytes = 0u64;
        let err = sample_ranges_in_run(
            moof_off as u64,
            &moof,
            TrackId::new(1),
            &mdat_ranges(&file),
            sample_bytes_budget(file.len() as u64),
            &mut copied_bytes,
        )
        .expect_err("a run one byte over the budget is refused");
        assert!(
            matches!(err, Error::SampleDataBudgetExceeded { limit } if limit == total as u64 - 1),
            "expected the {}-byte limit, got {err:?}",
            total - 1
        );
    }

    /// A `tfdt` above `i64::MAX` is rejected, not wrapped.
    #[test]
    fn hostile_tfdt_is_rejected() {
        assert!(matches!(
            tfdt_as_i64(u64::MAX),
            Err(crate::error::Error::InvalidInput(..))
        ));
        assert_eq!(tfdt_as_i64(5).unwrap(), 5);
    }

    /// Decode-time arithmetic is checked, not wrapping.
    #[test]
    fn dts_arithmetic_is_checked() {
        assert!(matches!(
            add_duration(i64::MAX, 1),
            Err(crate::error::Error::InvalidInput(..))
        ));
        assert!(matches!(
            add_offset(i64::MAX, 1),
            Err(crate::error::Error::InvalidInput(..))
        ));
        assert!(matches!(
            add_offset(i64::MIN, -1),
            Err(crate::error::Error::InvalidInput(..))
        ));
        assert_eq!(add_duration(10, 3).unwrap(), 13);
        assert_eq!(add_offset(10, -3).unwrap(), 7);
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
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None),
                trun(&[2, 2], Some(0)),
            ],
        );
        let (pf, moof_off) = file_with(&[probe], &mdat_body);
        let off = mdat_rel(&pf, moof_off, mdat_body.len());
        let traf = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None),
                trun(&[2, 2], Some(off)),
            ],
        );
        let (full, moof_off) = file_with(&[traf], &mdat_body);
        let moof = parse_moof(&full, moof_off);
        let payload = full.len() - mdat_body.len();
        // Keep 5 of the 8 payload bytes: both 2-byte samples survive.
        let cut = &full[..payload + 5];
        let mdats = mdat_ranges(cut);
        assert_eq!(
            mdats,
            alloc::vec![MdatRange::new(payload as u64, cut.len() as u64)]
        );
        let r = ranges(cut, moof_off, &moof, TrackId::new(1)).expect("early samples resolve");
        assert_eq!((r[0].start, r[0].end), (payload as u64, payload as u64 + 2));
        assert_eq!(
            (r[1].start, r[1].end),
            (payload as u64 + 2, payload as u64 + 4)
        );
        // A sample past the cut is still rejected.
        let traf = container(
            b"traf",
            &[
                tfhd(TFHD_DEFAULT_BASE_IS_MOOF, TrackId::new(1), None),
                trun(&[2, 2, 2], Some(off)),
            ],
        );
        let (full3, moof_off3) = file_with(&[traf], &mdat_body);
        let moof3 = parse_moof(&full3, moof_off3);
        assert!(matches!(
            ranges(&full3[..payload + 5], moof_off3, &moof3, TrackId::new(1)),
            Err(Error::InvalidInput(_))
        ));
    }
}
