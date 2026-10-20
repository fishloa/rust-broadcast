//! MPEG-2 video per-access-unit display tracking (Phase 3b of
//! `docs/display-geometry/DESIGN.md` §6.4, §6.5): sequence geometry, AFD and
//! bar data, with the H.262 persistence rule always in force.
//!
//! One [`Mpeg2DisplayTracker::observe`] per input access unit answers: what
//! geometry is in force, and what area of interest (AFD + bar data) do this
//! picture's `extension_and_user_data(2)` ask for.
//!
//! # Spec citations
//!
//! - ETSI TS 101 154 §B.6.1 / A/53 Part 4 §6.2.3-6.2.4 (`docs/codec/afd-bar-data.md`
//!   §4.4): AFD and bar data ride in `extension_and_user_data(2)` (after the
//!   picture header); they "remain in effect until the next sequence_header()
//!   or until a new AFD"; after `sequence_header()` the area of interest
//!   reverts to the sequence's own aspect (`afd = None`, not
//!   `SameAsCodedFrame`); a new AFD without bar data clears bar data.
//! - H.262 §6.2.2.1 / §6.2.2.3 / §6.2.2.4 for the sequence syntax (see
//!   [`crate::h262`]).
//!
//! # Ordering (§6.5)
//!
//! `random_access` is the access unit's own random-access bit
//! ([`crate::ts_demux::mpeg2_is_sync`]: a sequence header or an I picture).
//! AFD is not RAP-bound, so an AFD change on a mid-GOP picture raises
//! `DisplayChanged` with `random_access = false`. The event and the
//! [`Sample`](crate::Sample) tag share one `Arc<DisplayInfo>`.

use crate::h262::{
    afd_from_mpeg2_user_data, find_sequence_header, parse_sequence_at, picture_user_data_payloads,
};
use crate::ir::{
    ActiveFormat, ActiveFormatDescription, BarData, DisplayChangeSet, DisplayGeometry, DisplayInfo,
    UserDataCarriage,
};
use crate::user_data::{
    BAR_DATA_USER_DATA_TYPE_CODE, DTG1_USER_IDENTIFIER, GA94_USER_IDENTIFIER, USER_IDENTIFIER_LEN,
};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// The outcome of [`Mpeg2DisplayTracker::observe`] for one access unit.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mpeg2Outcome {
    /// The access unit raised nothing (the state in force is unchanged).
    Unchanged,
    /// The first state, or one that differs from the previous access unit's.
    Changed {
        /// The new per-access-unit state (shared with the sample).
        display: Arc<DisplayInfo>,
        /// The track's first state (`changed.initial`).
        initial: bool,
        /// The access unit's own random-access bit.
        random_access: bool,
        /// Which parts of the state differ from the previous one.
        changed: DisplayChangeSet,
    },
}

/// Per-track MPEG-2 display state.
#[derive(Debug, Clone, Default)]
pub struct Mpeg2DisplayTracker {
    /// Geometry of the sequence in force (`None` before the first
    /// `sequence_header()`).
    geometry: Option<DisplayGeometry>,
    /// The picture-level state in force.
    afd: Option<ActiveFormatDescription>,
    bar_data: Option<BarData>,
    /// Whether any AFD or bar data has ever been seen (so a reset back to
    /// "no area of interest" is a change, not the initial state).
    active_seen: bool,
    /// The state every sample carries until a new change is raised.
    state: Option<Arc<DisplayInfo>>,
}

impl Mpeg2DisplayTracker {
    /// A tracker with no sequence and no state.
    pub fn new() -> Self {
        Self::default()
    }

    /// The display state in force, shared with every sample that is not the
    /// first picture of a change.
    pub fn state(&self) -> Option<&Arc<DisplayInfo>> {
        self.state.as_ref()
    }

    /// Seed the state in force (`TrackAdded`'s level-triggered start), so the
    /// first access unit that resolves to it emits nothing.
    pub fn seed_state(&mut self, geometry: Option<DisplayGeometry>) {
        if self.state.is_none() {
            self.geometry = geometry.clone();
            self.state = DisplayInfo::from_geometry(geometry).map(Arc::new);
        }
    }

    /// The `DisplayInfo` formed by the parts currently in force.
    fn compose(&self) -> DisplayInfo {
        let active_format = (self.active_seen || self.afd.is_some() || self.bar_data.is_some())
            .then(|| {
                ActiveFormat::new(
                    self.afd,
                    self.bar_data.clone(),
                    UserDataCarriage::Mpeg2PictureUserData,
                )
            });
        DisplayInfo::new(self.geometry.clone()).with_active_format(active_format)
    }

    /// Fold one MPEG-2 video access unit (a `picture_header()` plus its
    /// `extension_and_user_data(2)`; a sequence header may precede it).
    pub fn observe(&mut self, au: &[u8]) -> Mpeg2Outcome {
        let mut state = self.clone();
        let outcome = state.step(au);
        *self = state;
        outcome
    }

    /// [`observe`](Self::observe) on `self` itself (the real step).
    fn step(&mut self, au: &[u8]) -> Mpeg2Outcome {
        let random_access = crate::ts_demux::mpeg2_is_sync(au);
        // A sequence_header() resets the area of interest (A/53 §6.2.4) and
        // re-defines the sequence geometry.
        if let Some(start) = find_sequence_header(au)
            && let Ok(parsed) = parse_sequence_at(au, start)
        {
            self.geometry = Some(parsed.to_display_geometry());
            self.afd = None;
            self.bar_data = None;
        }
        let (_, payloads) = picture_user_data_payloads(au);
        self.picture_active_format(&payloads);
        let next = self.compose();
        let Some(previous) = self.state.clone() else {
            let display = Arc::new(next);
            self.state = Some(display.clone());
            return Mpeg2Outcome::Changed {
                display,
                initial: true,
                random_access,
                changed: DisplayChangeSet::initial(),
            };
        };
        if previous.as_ref() == &next {
            return Mpeg2Outcome::Unchanged;
        }
        let changed = next.changes_from(Some(&previous));
        let display = Arc::new(next);
        self.state = Some(display.clone());
        Mpeg2Outcome::Changed {
            display,
            initial: false,
            random_access,
            changed,
        }
    }

    /// Interpret this picture's user data, updating the persistent AFD / bar
    /// state. The first AFD (and the first bar-data) message in the picture
    /// wins; a short or unrecognised payload is silently discarded (TS 101 154
    /// §B.6.2).
    fn picture_active_format(&mut self, payloads: &[&[u8]]) {
        let mut afd_message: Option<Option<ActiveFormatDescription>> = None;
        let mut bar: Option<Option<BarData>> = None;
        for payload in payloads {
            if afd_message.is_some() && bar.is_some() {
                continue;
            }
            if payload.len() < USER_IDENTIFIER_LEN {
                continue;
            }
            let identifier = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            let body = &payload[USER_IDENTIFIER_LEN..];
            match identifier {
                DTG1_USER_IDENTIFIER => {
                    if afd_message.is_none()
                        && let Some(parsed) = afd_from_mpeg2_user_data(body)
                    {
                        afd_message = Some(parsed);
                    }
                }
                GA94_USER_IDENTIFIER => {
                    if bar.is_none()
                        && let Some(parsed) = bar_data_from_dvb1(body)
                    {
                        bar = Some(parsed);
                    }
                }
                _ => {}
            }
        }
        if let Some(afd) = afd_message {
            self.afd = afd;
            self.active_seen = true;
            // A new AFD without bar data clears bar data (TS 101 154 §B.6.1
            // item 3) — unless this very picture also carries bar data.
            if bar.is_none() {
                self.bar_data = None;
            }
        }
        if let Some(bar) = bar {
            self.bar_data = bar;
            self.active_seen = true;
        }
    }
}

/// Bytes before the first bar value: `user_data_type_code` + `bar_data()`
/// flags (TS 101 154 Table B.4/B.7).
const MIN_BAR_MESSAGE_LEN: usize = 2;

/// Parse a `DVB1_data()` bar-data message: `user_data_type_code` then
/// `bar_data()` (TS 101 154 Table B.3/B.4/B.7). `Some(Some(..))` for a bar
/// message carrying a signal, `Some(None)` for one with no bar flag set,
/// `None` for another `user_data_type_code` or a truncated payload.
fn bar_data_from_dvb1(body: &[u8]) -> Option<Option<BarData>> {
    let (&type_code, bar) = body.split_first()?;
    if type_code != BAR_DATA_USER_DATA_TYPE_CODE {
        return None;
    }
    // The shared decoder reads the T.35 metadata form (`GA94` + type code +
    // `bar_data()`); the `DVB1_data()` body is exactly that tail, so a
    // canonical T.35 prefix in front of it reuses one bar-data parser.
    let mut tail = Vec::with_capacity(MIN_BAR_MESSAGE_LEN + bar.len());
    tail.push(BAR_DATA_USER_DATA_TYPE_CODE);
    tail.extend_from_slice(bar);
    crate::user_data::bar_data_from_ga94_payload(&tail).map(Some)
}

#[cfg(test)]
mod tests;
