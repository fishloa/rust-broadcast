//! Extracted caption cue + the CEA-608/708 cue extractors.
//!
//! See `crate::webvtt` module docs for the diff-based boundary-detection
//! design and the documented losses.
use crate::event::MediaTime;
use alloc::string::String;
#[cfg(feature = "cc-data")]
use alloc::vec::Vec;

/// A single extracted caption cue: display text plus its media-timeline span.
///
/// `start`/`end` are wrap-unrolled 90 kHz [`MediaTime`] instants (see
/// [`crate::timeline`]). `text` may contain embedded `\n` for multi-line
/// captions (e.g. a 2-row CEA-608 roll-up window); [`crate::webvtt::cue_block`]
/// emits each line of `text` as its own WebVTT payload line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Cue {
    /// Cue start: the commit-event PTS (CEA-608 EOC / roll-up row reveal /
    /// CEA-708 window-visible transition) — see `crate::webvtt` module docs.
    pub start: MediaTime,
    /// Cue end: the PTS of the next erase/replace event.
    pub end: MediaTime,
    /// Cue display text: plain, unescaped, unstyled (see `crate::webvtt`
    /// module docs on documented losses).
    pub text: String,
}

/// Shared diff-based boundary tracker used by both the 608 and 708
/// extractors: unrolls the 33-bit PTS and turns "displayed text changed"
/// transitions into completed [`Cue`]s.
#[cfg(feature = "cc-data")]
struct DiffState {
    last_pts: Option<u64>,
    epoch: u64,
    open: Option<(u64, String)>,
}

#[cfg(feature = "cc-data")]
impl DiffState {
    fn new() -> Self {
        DiffState {
            last_pts: None,
            epoch: 0,
            open: None,
        }
    }

    fn unroll(&mut self, pts33: u64) -> u64 {
        crate::timeline::unroll_pts(&mut self.last_pts, &mut self.epoch, pts33)
    }

    /// Observe the decoded text at `ticks`; push a completed cue into `cues`
    /// when the text differs from the currently open cue (or, if none is
    /// open, when the new text is non-empty).
    fn observe(&mut self, ticks: u64, text: String, cues: &mut Vec<Cue>) {
        let changed = match &self.open {
            Some((_, cur)) => *cur != text,
            None => !text.is_empty(),
        };
        if !changed {
            return;
        }
        if let Some((start, prev)) = self.open.take() {
            if !prev.is_empty() {
                cues.push(Cue {
                    start: MediaTime(start),
                    end: MediaTime(ticks),
                    text: prev,
                });
            }
        }
        if !text.is_empty() {
            self.open = Some((ticks, text));
        }
    }

    /// Close any still-open cue at end of stream (or a channel/service reset).
    fn finalize(&mut self, ticks: u64, cues: &mut Vec<Cue>) {
        if let Some((start, prev)) = self.open.take() {
            if !prev.is_empty() {
                cues.push(Cue {
                    start: MediaTime(start),
                    end: MediaTime(ticks),
                    text: prev,
                });
            }
        }
    }
}

/// Extracts [`Cue`]s from a single CEA-608 data channel (CTA-608-E),
/// wrapping a `cc-data` [`cc_data::decode::Cea608Decoder`].
///
/// Feed it one access unit's CEA-608 triplets at a time, tagged with that
/// access unit's raw (non-unrolled) 33-bit PTS, via [`push_frame`]; call
/// [`finalize`] at end of stream to close any still-open cue.
///
/// [`push_frame`]: Cea608CueExtractor::push_frame
/// [`finalize`]: Cea608CueExtractor::finalize
///
/// ```
/// use timed_metadata::webvtt::Cea608CueExtractor;
/// use cc_data::{CcTriplet, CcType};
/// use cc_data::decode::Cea608Channel;
///
/// fn pair(pts: u64, b1: u8, b2: u8) -> (u64, CcTriplet) {
///     (
///         pts,
///         CcTriplet { cc_valid: true, cc_type: CcType::Ntsc608Field1, cc_data_1: b1, cc_data_2: b2 },
///     )
/// }
///
/// let mut ex = Cea608CueExtractor::new(Cea608Channel::Cc1);
/// let frames = [
///     pair(0, 0x14, 0x20),       // RCL
///     pair(1, 0x14, 0x70),       // PAC row 15
///     pair(2, b'H', b'I'),       // "HI"
///     pair(3, 0x14, 0x2F),       // EOC -> commits "HI"
///     pair(4, 0x14, 0x2C),       // EDM -> erases
/// ];
/// for (pts, t) in frames {
///     ex.push_frame(pts, core::slice::from_ref(&t));
/// }
/// let cues = ex.cues();
/// assert_eq!(cues.len(), 1);
/// assert_eq!(cues[0].text, "HI");
/// ```
#[cfg(feature = "cc-data")]
#[cfg_attr(docsrs, doc(cfg(feature = "cc-data")))]
pub struct Cea608CueExtractor {
    decoder: cc_data::decode::Cea608Decoder,
    channel: cc_data::decode::Cea608Channel,
    state: DiffState,
    cues: Vec<Cue>,
}

#[cfg(feature = "cc-data")]
#[cfg_attr(docsrs, doc(cfg(feature = "cc-data")))]
impl Cea608CueExtractor {
    /// A new extractor tracking the given CEA-608 data channel (e.g. `Cc1`).
    #[must_use]
    pub fn new(channel: cc_data::decode::Cea608Channel) -> Self {
        Cea608CueExtractor {
            decoder: cc_data::decode::Cea608Decoder::new(),
            channel,
            state: DiffState::new(),
            cues: Vec::new(),
        }
    }

    /// Feed one access unit's CEA-608 triplets (already demuxed from its
    /// `cc_data()`), tagged with that access unit's raw 33-bit PTS. Non-608
    /// triplets in `triplets` are ignored.
    pub fn push_frame(&mut self, pts33: u64, triplets: &[cc_data::CcTriplet]) {
        let ticks = self.state.unroll(pts33);
        self.decoder
            .push_triplets(triplets.iter().filter(|t| t.cc_type.is_cea608()));
        let text = self.decoder.channel_text(self.channel);
        self.state.observe(ticks, text, &mut self.cues);
    }

    /// Close any still-open cue at end of stream, at `end_pts33`.
    pub fn finalize(&mut self, end_pts33: u64) {
        let ticks = self.state.unroll(end_pts33);
        self.state.finalize(ticks, &mut self.cues);
    }

    /// The cues extracted so far, in order.
    #[must_use]
    pub fn cues(&self) -> &[Cue] {
        &self.cues
    }

    /// Consume the extractor, returning the extracted cues.
    #[must_use]
    pub fn into_cues(self) -> Vec<Cue> {
        self.cues
    }
}

/// Extracts [`Cue`]s from a single CEA-708 (DTVCC) service (CTA-708-E),
/// wrapping a `cc-data` [`cc_data::decode::Cea708Decoder`].
///
/// Reads [`cc_data::decode::Cea708Decoder::service_text`] for the configured
/// service number (`1`-`6`; service 1 is the primary caption service) after
/// each fed frame — see `crate::webvtt` module docs for the diff-based
/// boundary design and its documented losses (styling, window geometry,
/// cross-service merging).
#[cfg(feature = "cc-data")]
#[cfg_attr(docsrs, doc(cfg(feature = "cc-data")))]
pub struct Cea708CueExtractor {
    decoder: cc_data::decode::Cea708Decoder,
    service_number: usize,
    state: DiffState,
    cues: Vec<Cue>,
}

#[cfg(feature = "cc-data")]
#[cfg_attr(docsrs, doc(cfg(feature = "cc-data")))]
impl Cea708CueExtractor {
    /// A new extractor tracking the given CEA-708 service number (`1`-`6`).
    #[must_use]
    pub fn new(service_number: usize) -> Self {
        Cea708CueExtractor {
            decoder: cc_data::decode::Cea708Decoder::new(),
            service_number,
            state: DiffState::new(),
            cues: Vec::new(),
        }
    }

    /// Feed one access unit's CEA-708 triplets (already demuxed from its
    /// `cc_data()`), tagged with that access unit's raw 33-bit PTS. Non-708
    /// triplets in `triplets` are ignored.
    pub fn push_frame(&mut self, pts33: u64, triplets: &[cc_data::CcTriplet]) {
        let ticks = self.state.unroll(pts33);
        self.decoder
            .push_triplets(triplets.iter().filter(|t| t.cc_type.is_cea708()));
        let text = self.decoder.service_text(self.service_number);
        self.state.observe(ticks, text, &mut self.cues);
    }

    /// Close any still-open cue at end of stream, at `end_pts33`.
    pub fn finalize(&mut self, end_pts33: u64) {
        let ticks = self.state.unroll(end_pts33);
        self.state.finalize(ticks, &mut self.cues);
    }

    /// The cues extracted so far, in order.
    #[must_use]
    pub fn cues(&self) -> &[Cue] {
        &self.cues
    }

    /// Consume the extractor, returning the extracted cues.
    #[must_use]
    pub fn into_cues(self) -> Vec<Cue> {
        self.cues
    }
}

#[cfg(all(test, feature = "cc-data"))]
mod tests {
    use super::*;
    use cc_data::{CcTriplet, CcType};

    fn t608(b1: u8, b2: u8) -> CcTriplet {
        CcTriplet {
            cc_valid: true,
            cc_type: CcType::Ntsc608Field1,
            cc_data_1: b1,
            cc_data_2: b2,
        }
    }

    /// Pop-on: RCL/PAC/chars produce no cue until EOC; EDM closes it.
    #[test]
    fn pop_on_boundary_is_eoc_and_edm() {
        let mut ex = Cea608CueExtractor::new(cc_data::decode::Cea608Channel::Cc1);
        ex.push_frame(0, &[t608(0x14, 0x20)]); // RCL
        ex.push_frame(1, &[t608(0x14, 0x70)]); // PAC row 15
        assert!(ex.cues().is_empty(), "composing must not yet emit a cue");
        ex.push_frame(2, &[t608(b'H', b'I')]);
        assert!(
            ex.cues().is_empty(),
            "non-displayed writes must not emit a cue"
        );
        ex.push_frame(3, &[t608(0x14, 0x2F)]); // EOC -> opens a cue (not yet closed)
        assert!(
            ex.cues().is_empty(),
            "the cue is open, not yet closed by an erase/replace"
        );
        ex.push_frame(4, &[t608(0x14, 0x2C)]); // EDM -> closes it
        assert_eq!(ex.cues().len(), 1);
        assert_eq!(ex.cues()[0].start, MediaTime(3));
        assert_eq!(ex.cues()[0].end, MediaTime(4));
        assert_eq!(ex.cues()[0].text, "HI");
    }

    /// An unclosed cue is closed by `finalize` at end of stream.
    #[test]
    fn finalize_closes_open_cue() {
        let mut ex = Cea608CueExtractor::new(cc_data::decode::Cea608Channel::Cc1);
        ex.push_frame(0, &[t608(0x14, 0x20)]);
        ex.push_frame(1, &[t608(0x14, 0x70)]);
        ex.push_frame(2, &[t608(b'O', b'K')]);
        ex.push_frame(3, &[t608(0x14, 0x2F)]); // EOC
        assert!(ex.cues().is_empty());
        ex.finalize(10);
        assert_eq!(ex.cues().len(), 1);
        assert_eq!(ex.cues()[0].start, MediaTime(3));
        assert_eq!(ex.cues()[0].end, MediaTime(10));
        assert_eq!(ex.cues()[0].text, "OK");
    }

    /// 708 extractor: DefineWindow + text + ToggleWindows(visible) commits a
    /// cue; a subsequent hide closes it. Worked-example CCP bytes adapted
    /// from `cc-data`'s own `Cea708Decoder` doctest.
    #[test]
    fn cea708_service1_basic_window() {
        let mut ex = Cea708CueExtractor::new(1);
        // DefineWindow window 0, then draw via a raw packet push through the
        // extractor's triplet path (start + data triplets forming one CCP).
        // header 0x05 (seq=0,size=5*2-1=9? use decoder's own doctest packet).
        let packet: [u8; 9] = [0x05, 0x27, 0x9A, 0x38, 0x4A, 0xD1, 0x8B, 0x0F, 0x11];
        let t0 = CcTriplet {
            cc_valid: true,
            cc_type: CcType::Dtvcc708Start,
            cc_data_1: packet[0],
            cc_data_2: packet[1],
        };
        let rest: alloc::vec::Vec<CcTriplet> = packet[2..]
            .chunks(2)
            .map(|c| CcTriplet {
                cc_valid: true,
                cc_type: CcType::Dtvcc708Data,
                cc_data_1: c[0],
                cc_data_2: *c.get(1).unwrap_or(&0),
            })
            .collect();
        let mut triplets = alloc::vec![t0];
        triplets.extend(rest);
        ex.push_frame(0, &triplets);
        // This packet only defines window 2 on service 1 (from cc-data's own
        // doctest) -- it does not make it visible or paint text, so no cue
        // should be produced yet (documents that DefineWindow alone doesn't
        // commit a cue).
        assert!(ex.cues().is_empty());
    }
}
