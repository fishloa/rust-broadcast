//! CENC protection of an already-built init segment (issue #564 Task 3).
//!
//! The box-level wrapper lives in [`protect_sample_entry`];
//! this module keeps the IR-facing pass that takes a
//! [`TrackProtection`] and rewrites the `moov`
//! tree through its typed `Serialize` impl.

use crate::error::{Error, Result};
use crate::ir::TrackId;
use crate::ir::TrackProtection;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use isobmff::BoxIter;
use isobmff::init_segment::{MOOV, MovieBox, StblChild, protect_sample_entry};

/// Rewrite an already-built CMAF/fMP4 init segment (`ftyp` + `moov`) so the
/// given track's sample entry becomes CENC-protected (issue #564 Task 3):
/// [`protect_sample_entry`] renames the entry to `encv`/`enca` and adds
/// `sinf`; every ancestor box (`stsd`/`stbl`/`minf`/`mdia`/`trak`/`moov`) is
/// **recomputed** from its typed children by [`MovieBox`]'s own `Serialize`
/// impl (no manual size patching) — only the target track's sample entry and
/// its ancestors' size fields differ from the input; every other byte, and
/// every other track, round-trips unchanged.
///
/// Operates as a *post-processing* pass over an already-muxed init segment
/// rather than being wired into `pipeline::build_init_segment` itself, so it
/// composes with any caller that already has a
/// [`crate::ir::TrackProtection`] in hand (e.g. from
/// `CencEncryptor::encrypt`'s `TrackSpec::protection`) without that crypto
/// metadata needing to flow through the lower-level `TrackSpec`/pipeline
/// plumbing. `init_segment` may be the bare `ftyp`+`moov` pair or a larger
/// buffer with more boxes following (e.g. a whole `CmafMux` output including
/// `styp`/`moof`/`mdat`) — only the `moov` span is touched; everything
/// before and after it is copied through verbatim.
pub fn protect_init_segment(
    init_segment: &[u8],
    track_id: TrackId,
    protection: &TrackProtection,
) -> Result<Vec<u8>> {
    let mut prefix_len = 0usize;
    let mut moov_len = None;
    for step in BoxIter::new(init_segment) {
        let box_ref = step?;
        let size = box_ref.end - box_ref.start;
        if box_ref.header.box_type.is(&MOOV) {
            moov_len = Some(size);
            break;
        }
        prefix_len += size;
    }
    let moov_len = moov_len.ok_or(Error::UnexpectedBox { expected: "moov" })?;
    let moov_bytes = &init_segment[prefix_len..prefix_len + moov_len];
    let suffix = &init_segment[prefix_len + moov_len..];

    let mut moov = MovieBox::parse(moov_bytes)?;
    {
        let track = moov
            .tracks
            .iter_mut()
            .find(|t| t.tkhd.track_id == track_id.into())
            .ok_or(Error::InvalidInput(
                "protect_init_segment: track_id not found in moov",
            ))?;
        let stbl = track
            .mdia
            .as_mut()
            .and_then(|m| m.minf.as_mut())
            .and_then(|m| m.stbl.as_mut())
            .ok_or(Error::UnexpectedBox {
                expected: "trak/mdia/minf/stbl",
            })?;
        let stsd = stbl
            .children
            .iter_mut()
            .find_map(|c| match c {
                StblChild::Stsd(s) => Some(s),
                _ => None,
            })
            .ok_or(Error::UnexpectedBox { expected: "stsd" })?;
        let original = stsd
            .entries
            .first()
            .ok_or(Error::UnexpectedBox {
                expected: "stsd sample entry",
            })?
            .clone();
        stsd.entries[0] = protect_sample_entry(&original, protection.scheme, &protection.tenc)?;
    }

    let mut new_moov = alloc::vec![0u8; moov.serialized_len()];
    let n = moov.serialize_into(&mut new_moov)?;
    new_moov.truncate(n);

    let mut out = Vec::with_capacity(prefix_len + new_moov.len() + suffix.len());
    out.extend_from_slice(&init_segment[..prefix_len]);
    out.extend_from_slice(&new_moov);
    out.extend_from_slice(suffix);
    Ok(out)
}
