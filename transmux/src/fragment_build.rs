//! Media-fragment builders: the per-track `tfhd`+`trun` run and the CENC
//! media-segment protection pass (issue #564 Task 3).
//!
//! The `moof`/`traf`/`trun` box syntax lives in [`crate::movie_fragment`];
//! this module keeps the builders over it that the demux/mux pipeline calls.

use crate::box_types::BOX_HEADER_MIN_SIZE;
use crate::error::{Error, Result};
use crate::ir::TrackId;
use crate::movie_fragment::{
    CENC_AUX_INFO_TYPES, MoofChild, MovieFragmentBox, SAIZ_SUBSAMPLE_COUNT_SIZE,
    SAIZ_SUBSAMPLE_ENTRY_SIZE, SAMPLE_FLAGS_NON_SYNC, SAMPLE_FLAGS_SYNC, SENC_ENTRIES_OFFSET,
    TFHD_DEFAULT_BASE_IS_MOOF, TRUN_DATA_OFFSET_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_FLAGS_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, TrackFragmentHeaderBox,
    TrackFragmentRunBox, TrafChild, TrunSample, aux_info_type_of,
};
use alloc::vec::Vec;
use broadcast_common::Serialize;

/// One sample of a media fragment's `trun`, as the segment writers describe it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MediaRunSample {
    /// `sample_duration`, in the track timescale.
    pub duration: u32,
    /// Coded size in bytes.
    pub size: usize,
    /// Sync sample (random-access point).
    pub is_sync: bool,
    /// `pts - dts` composition offset.
    pub composition_offset: i32,
}

/// The `tfhd` + `trun` every media-fragment writer (`build_media_segment`,
/// the LL-DASH chunk, the Smooth fragment) emits for one track: per-sample
/// duration/size/flags, with a v1 signed composition offset only when some
/// sample has a non-zero one (B-frames), `default-base-is-moof` addressing and a
/// placeholder `data_offset` of 0 for the caller to patch once the `moof` size
/// is known. One copy of what had been three (audit r05-O6).
pub(crate) fn media_fragment_run(
    track_id: TrackId,
    samples: impl Iterator<Item = MediaRunSample> + Clone,
) -> Result<(TrackFragmentHeaderBox, TrackFragmentRunBox)> {
    let any_cts = samples.clone().any(|s| s.composition_offset != 0);
    let mut run_samples = Vec::new();
    for s in samples {
        run_samples.push(TrunSample {
            sample_duration: Some(s.duration),
            sample_size: Some(u32::try_from(s.size).map_err(|_| {
                Error::InvalidInput("sample larger than the 32-bit trun sample_size field")
            })?),
            sample_flags: Some(if s.is_sync {
                SAMPLE_FLAGS_SYNC
            } else {
                SAMPLE_FLAGS_NON_SYNC
            }),
            sample_composition_time_offset: any_cts.then_some(i64::from(s.composition_offset)),
        });
    }
    let mut tr_flags = TRUN_DATA_OFFSET_PRESENT
        | TRUN_SAMPLE_DURATION_PRESENT
        | TRUN_SAMPLE_SIZE_PRESENT
        | TRUN_SAMPLE_FLAGS_PRESENT;
    // Version 1 carries a signed composition offset (needed for B-frames).
    let version = if any_cts {
        tr_flags |= TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT;
        1u8
    } else {
        0u8
    };
    let trun = TrackFragmentRunBox {
        version,
        tr_flags,
        data_offset: Some(0),
        first_sample_flags: None,
        samples: run_samples,
    };
    let tfhd = TrackFragmentHeaderBox {
        flags: TFHD_DEFAULT_BASE_IS_MOOF,
        track_id,
        base_data_offset: None,
        sample_description_index: None,
        default_sample_duration: None,
        default_sample_size: None,
        default_sample_flags: None,
    };
    Ok((tfhd, trun))
}

/// One protected track's per-sample CENC aux info for a single movie
/// fragment, for [`protect_media_segment`] — the movie-fragment half of
/// issue #564's muxer emission ([`crate::cenc_segment::protect_init_segment`]
/// is the init-segment half).
pub struct FragmentProtection<'a> {
    /// The `tfhd.track_id` of the `traf` to protect.
    pub track_id: TrackId,
    /// Per-sample IV + subsample map for **this fragment's samples only**, in
    /// decode order. Must have exactly as many entries as the matching
    /// `traf`'s total `trun` sample count (checked). A `Media` muxed across
    /// several CMAF media segments (e.g. via `Segmenter`) protects each
    /// segment with the slice of `Track::encryption`'s samples covering that
    /// segment.
    pub entries: &'a [crate::cenc::SampleEncryptionEntry],
    /// [`crate::cenc::TrackEncryptionBox::default_per_sample_iv_size`] for
    /// this track — the fixed IV length written into `senc`.
    pub per_sample_iv_size: u8,
}

/// The `senc`/`saiz`/`saio` triple built for one protected `traf`.
pub(crate) struct CencFragmentBoxes {
    pub(crate) senc: crate::cenc::SampleEncryptionBox,
    pub(crate) saiz: crate::cenc::SampleAuxInfoSizesBox,
    saio: crate::cenc::SampleAuxInfoOffsetsBox,
}

impl CencFragmentBoxes {
    fn added_len(&self) -> usize {
        self.senc.serialized_len() + self.saiz.serialized_len() + self.saio.serialized_len()
    }
}

/// Build the `senc`/`saiz`/`saio` boxes for one protected track's fragment,
/// or `None` when this track's fragment has nothing meaningful for them to
/// carry (issue R3, ISO/IEC 23001-7 §12.2/§12.3).
///
/// `senc`'s syntax (§12.3) writes, per sample, `InitializationVector[Per_Sample_IV_Size]`
/// then — only when `UseSubSampleEncryption` is set — a subsample map. When a
/// track uses a **constant IV** (`per_sample_iv_size == 0`, the per-sample IV
/// lives once in `tenc.default_constant_IV` instead — §12.2's
/// `default_isProtected==1 && default_Per_Sample_IV_Size==0` case) *and* whole-
/// sample protection (no subsample split needed), every one of `senc`'s
/// `sample_count` entries is **zero bytes wide**: nothing after `sample_count`
/// itself. That box asserts a sample count the bitstream carries no data to
/// corroborate, while recording no per-sample information a decryptor could
/// use — precisely the shape [`crate::cenc::SampleEncryptionBox::parse_body`]
/// rejects as [`Error::InvalidInput`] (a `senc` "declares samples but carries
/// no per-sample data"), and rightly so: a `sample_count`-only box with no
/// backing bytes is unverifiable, not merely unusual. There is nothing this
/// track's fragment needs `senc`/`saiz`/`saio` for at all in that case — a
/// decryptor already has everything it needs from `tenc`'s constant IV and the
/// (subsample-free) whole-sample protection convention — so the correct fix is
/// to omit the triple entirely, not to shrink it into an unparseable shape.
///
/// When the schema is `cbcs` with [`crate::ConstantIvSenc::Emit`] (the
/// default), a constant-IV track instead sets `per_sample_iv_size = 16` in
/// `tenc` and carries the constant IV replicated in every `senc` entry — so
/// this function's normal emission path (below) produces a regular `senc`
/// with 16-byte IVs per sample. The `None` return above is only reached for
/// the [`crate::ConstantIvSenc::Omit`] opt-out.
///
/// `saio.offsets[0]` (when `Some` is returned) is a placeholder (`0`) —
/// [`protect_media_segment`] back-patches it once every box's final position
/// in the rebuilt `moof` is known.
pub(crate) fn build_cenc_fragment_boxes(
    p: &FragmentProtection<'_>,
) -> Result<Option<CencFragmentBoxes>> {
    let use_subsamples = p.entries.iter().any(|e| !e.subsamples.is_empty());
    if p.per_sample_iv_size == 0 && !use_subsamples {
        return Ok(None);
    }
    let flags = if use_subsamples {
        crate::cenc::SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION
    } else {
        0
    };
    let senc = crate::cenc::SampleEncryptionBox {
        version: 0,
        flags,
        per_sample_iv_size: p.per_sample_iv_size,
        entries: p.entries.to_vec(),
    };

    let mut sizes = Vec::with_capacity(p.entries.len());
    for e in p.entries {
        let mut sz = p.per_sample_iv_size as usize;
        if use_subsamples {
            sz += SAIZ_SUBSAMPLE_COUNT_SIZE + e.subsamples.len() * SAIZ_SUBSAMPLE_ENTRY_SIZE;
        }
        if sz > u8::MAX as usize {
            return Err(Error::InvalidInput(
                "protect_media_segment: per-sample aux info size exceeds 255 bytes (saiz sample_info_size is u8)",
            ));
        }
        sizes.push(sz as u32);
    }
    let uniform = sizes
        .first()
        .copied()
        .filter(|first| sizes.iter().all(|s| s == first));
    let sample_count = broadcast_common::len::fit_u32(sizes.len(), "saiz sample_count")?;
    let saiz = crate::cenc::SampleAuxInfoSizesBox {
        version: 0,
        flags: 0,
        aux_info_type: None,
        aux_info_type_parameter: None,
        default_sample_info_size: uniform.unwrap_or(0),
        sample_count,
        sample_info_sizes: if uniform.is_some() { Vec::new() } else { sizes },
    };
    let saio = crate::cenc::SampleAuxInfoOffsetsBox {
        version: 0,
        flags: 0,
        aux_info_type: None,
        aux_info_type_parameter: None,
        offsets: alloc::vec![0u64],
    };
    Ok(Some(CencFragmentBoxes { senc, saiz, saio }))
}

/// Rewrite an already-built **single-fragment** CMAF media segment (`styp`
/// [+ `emsg`*] + one `moof` + `mdat`) so each track named in `protections`
/// gets CENC `senc`/`saiz`/`saio` boxes appended to its `traf` (issue #564
/// Task 3) — except a track whose fragment has nothing for those boxes to
/// carry (constant IV + whole-sample protection: see this module's private
/// `build_cenc_fragment_boxes`, issue R3), whose `traf` is left unmodified
/// entirely; its samples are still encrypted, decryptable from `tenc`'s
/// `default_constant_IV` alone.
///
/// This is the normal CMAF media-segment case (one `moof`/`mdat` pair per
/// segment). Only the buffer's *first* `moof` is located and rewritten — a
/// buffer containing more than one `moof` (a multi-fragment segment) is not
/// supported; any `moof`s beyond the first are copied through untouched as
/// part of the verbatim suffix, not protected.
///
/// Every `trun.data_offset` in the (possibly-grown) `moof` — protected track
/// or not — is shifted by the exact number of bytes the new boxes add, so
/// every sample in the unchanged `mdat` still resolves correctly against the
/// `default-base-is-moof` base (see the module docs above for why this, and
/// `saio.offset[0]`, are moof-relative). `media_segment` may be a bare
/// `styp`+`moof`+`mdat` triple or a larger buffer with more boxes before/after
/// (e.g. a whole `CmafMux` output including `ftyp`/`moov`) — only the (first)
/// `moof` span is touched; everything before and after it is copied through
/// verbatim. Returns the input unchanged when `protections` is empty.
pub fn protect_media_segment(
    media_segment: &[u8],
    protections: &[FragmentProtection<'_>],
) -> Result<Vec<u8>> {
    if protections.is_empty() {
        return Ok(media_segment.to_vec());
    }

    let mut prefix_len = 0usize;
    let mut moof_len = None;
    for step in crate::box_types::box_iter(media_segment) {
        let (box_ref, consumed) = step?;
        if box_ref.header.box_type.is(b"moof") {
            moof_len = Some(consumed);
            break;
        }
        prefix_len += consumed;
    }
    let moof_len = moof_len.ok_or(Error::UnexpectedBox { expected: "moof" })?;
    let moof_bytes = &media_segment[prefix_len..prefix_len + moof_len];
    let suffix = &media_segment[prefix_len + moof_len..];

    let mut moof =
        MovieFragmentBox::parse_body(&moof_bytes[crate::box_types::box_header_len(moof_bytes)?..])?;

    let mut built: Vec<Option<CencFragmentBoxes>> = (0..moof.traf.len()).map(|_| None).collect();
    for p in protections {
        let idx = moof
            .traf
            .iter()
            .position(|t| t.tfhd.track_id == p.track_id)
            .ok_or(Error::InvalidInput(
                "protect_media_segment: track_id not present in moof",
            ))?;
        let sample_count: usize = moof.traf[idx].trun.iter().map(|r| r.samples.len()).sum();
        if sample_count != p.entries.len() {
            return Err(Error::InvalidInput(
                "protect_media_segment: entries.len() must equal the traf's total trun sample count",
            ));
        }
        // A `traf` that already carries a CENC `senc`/`saiz`/`saio` —
        // preserved as opaque children since the W11 fix — would end up with a
        // *second* set appended, and a decryptor has no way to tell which pair
        // applies (ISO/IEC 23001-7 §12.3 defines one `senc` per `traf`).
        // Protection is idempotent-by-rejection: the caller must strip the
        // existing triple (or use the already-protected segment as-is).
        //
        // `senc` is CENC by definition; `saiz`/`saio` are generic
        // auxiliary-information boxes (§8.7.8/§8.7.9) that other schemes also
        // use, identified by `aux_info_type`. Only a CENC-typed pair (or one
        // with the field absent, which §8.7.8.2 defines as `cenc`) would
        // collide, so a differently-typed pair is left alone (audit item 7,
        // round 3).
        for child in &moof.traf[idx].order {
            let TrafChild::Opaque(o) = child else {
                continue;
            };
            let collides = match &o.box_type {
                b"senc" => true,
                b"saiz" | b"saio" => aux_info_type_of(&o.data, &o.box_type)
                    .is_none_or(|ty| CENC_AUX_INFO_TYPES.contains(&ty)),
                _ => false,
            };
            if collides {
                return Err(Error::InvalidInput(
                    "protect_media_segment: traf already carries a CENC senc/saiz/saio",
                ));
            }
        }
        built[idx] = build_cenc_fragment_boxes(p)?;
    }

    // Pass A: each traf's "base" (tfhd/tfdt/trun plus its own opaque
    // children) length, and its final length once its senc/saiz/saio (if
    // protected) are appended.
    let base_lens: Vec<usize> = moof.traf.iter().map(|t| t.serialized_len()).collect();
    let final_lens: Vec<usize> = base_lens
        .iter()
        .zip(&built)
        .map(|(&base, b)| base + b.as_ref().map(CencFragmentBoxes::added_len).unwrap_or(0))
        .collect();

    // The rebuilt `moof` in **wire order**. A `moof` may carry opaque children
    // (`meta`, `pssh`, a `uuid`) at any position, and pass C emits them; the
    // old length was `header + mfhd + Σ traf`, which under-counted by exactly
    // those bytes, so any `moof` with a moof-level opaque child failed the
    // consistency check at the end (audit item 2).
    let order = moof.child_order_for_rewrite();
    let new_moof_len = BOX_HEADER_MIN_SIZE
        + order
            .iter()
            .map(|child| match child {
                MoofChild::Mfhd => moof.mfhd.serialized_len(),
                MoofChild::Traf(i) => final_lens.get(*i).copied().unwrap_or(0),
                MoofChild::Opaque(o) => o.serialized_len(),
            })
            .sum::<usize>();
    let delta = new_moof_len as i64 - moof_len as i64;

    // Pass B: compute each protected traf's saio.offset[0] (moof-relative) in
    // the same wire order pass C writes, and shift every trun.data_offset by
    // `delta`.
    let mut running = BOX_HEADER_MIN_SIZE as u64;
    for child in &order {
        let traf_index = match child {
            MoofChild::Mfhd => {
                running += moof.mfhd.serialized_len() as u64;
                continue;
            }
            MoofChild::Opaque(o) => {
                running += o.serialized_len() as u64;
                continue;
            }
            MoofChild::Traf(i) => *i,
        };
        let Some(traf) = moof.traf.get_mut(traf_index) else {
            return Err(Error::InvalidInput(
                "protect_media_segment: moof child index out of range",
            ));
        };
        if let Some(b) = built[traf_index].as_mut() {
            let senc_start = running + base_lens[traf_index] as u64;
            b.saio.offsets[0] = senc_start + SENC_ENTRIES_OFFSET;
        }
        running += final_lens[traf_index] as u64;

        for run in &mut traf.trun {
            if run.tr_flags & TRUN_DATA_OFFSET_PRESENT != 0 {
                let current = run.data_offset.ok_or(Error::InvalidInput(
                    "protect_media_segment: trun declares data_offset but carries none",
                ))?;
                let shifted = i64::from(current)
                    .checked_add(delta)
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or(Error::InvalidInput(
                        "protect_media_segment: trun.data_offset leaves the 32-bit field",
                    ))?;
                run.data_offset = Some(shifted);
            }
        }
    }
    // Pass C: serialize the rebuilt moof. `TrackFragmentBox::serialize_into`
    // only knows about tfhd/tfdt/trun plus its preserved opaque children, so
    // its own leading size field (the "base" length) is back-patched to the
    // final length once senc/saiz/saio have been appended for a protected
    // traf. The `moof`'s own children are emitted in the order they were
    // parsed (opaque children included), so a rewrite cannot reorder or drop
    // them (audit r05-W11).
    let mut moof_out = alloc::vec![0u8; new_moof_len];
    let moof_size = broadcast_common::len::fit_u32(new_moof_len, "moof size")?;
    moof_out[0..4].copy_from_slice(&moof_size.to_be_bytes());
    moof_out[4..8].copy_from_slice(b"moof");
    let mut c = BOX_HEADER_MIN_SIZE;
    for child in moof.child_order_for_rewrite() {
        match child {
            MoofChild::Mfhd => c += moof.mfhd.serialize_into(&mut moof_out[c..])?,
            MoofChild::Traf(i) => {
                let start = c;
                let traf = moof.traf.get(i).ok_or(Error::InvalidInput(
                    "protect_media_segment: moof child index out of range",
                ))?;
                c += traf.serialize_into(&mut moof_out[c..])?;
                if let Some(b) = built.get(i).and_then(|b| b.as_ref()) {
                    c += b.senc.serialize_into(&mut moof_out[c..])?;
                    c += b.saiz.serialize_into(&mut moof_out[c..])?;
                    c += b.saio.serialize_into(&mut moof_out[c..])?;
                    let final_len = broadcast_common::len::fit_u32(c - start, "traf size")?;
                    moof_out[start..start + core::mem::size_of::<u32>()]
                        .copy_from_slice(&final_len.to_be_bytes());
                }
            }
            MoofChild::Opaque(ref o) => c += o.serialize_into(&mut moof_out[c..])?,
        }
    }
    if c != new_moof_len {
        return Err(Error::InvalidInput(
            "moof length/senc offset consistency check failed",
        ));
    }

    let mut out = Vec::with_capacity(prefix_len + new_moof_len + suffix.len());
    out.extend_from_slice(&media_segment[..prefix_len]);
    out.extend_from_slice(&moof_out);
    out.extend_from_slice(suffix);
    Ok(out)
}
