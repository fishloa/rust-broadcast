//! RFC 3550 §A.1 RTP sequence-number validity state.
//!
//! Transcription of the RFC's `source` fields that `update_seq` reads and
//! writes (`private/specs-verified/ietf/specs/rfc3550_rtp.txt`, Appendix A.1;
//! the adapted discipline is written up in
//! `private/specs-verified/mpeg/transmux/rtp/rtp-sequence-validation.md`),
//! shared by every TS-over-RTP consumer in the workspace so the validity
//! decision is made in one place (issue #2473).

/// The 16-bit sequence-number space, RFC 3550 §A.1's `RTP_SEQ_MOD`.
pub const RTP_SEQ_MOD: u32 = 1 << 16;

/// `RTP_SEQ_MOD + 1`, the value RFC 3550 §A.1's `init_seq` stores in
/// `bad_seq` "so seq == bad_seq is false".
pub const RTP_SEQ_MOD_PLUS_ONE: u32 = RTP_SEQ_MOD + 1;

/// RFC 3550 §A.1's `MAX_DROPOUT`.
pub const MAX_DROPOUT: u16 = 3000;

/// RFC 3550 §A.1's `MAX_MISORDER`.
pub const MAX_MISORDER: u32 = 100;

/// RFC 3550 §A.1's `MIN_SEQUENTIAL`.
pub const MIN_SEQUENTIAL: u8 = 2;

/// `RTP_SEQ_MOD - MAX_MISORDER`, narrowed to `u16`.
pub const MAX_WILD_JUMP_U16: u16 = (RTP_SEQ_MOD - MAX_MISORDER) as u16;

/// RFC 3550 §A.1's per-source validity state.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SeqValidity {
    /// §A.1 `max_seq`.
    pub max_seq: u16,
    /// §A.1 `bad_seq`.
    pub bad_seq: u32,
    /// §A.1 `cycles`.
    pub cycles: u32,
    /// §A.1 `probation`.
    pub probation: u8,
}

/// Outcome of one RFC 3550 §A.1 `update_seq` call.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqVerdict {
    /// §A.1 returned 1.
    Valid {
        /// Set when two sequential packets confirmed a restart.
        restarted: bool,
    },
    /// §A.1 returned 0 from the probation branch.
    Probation,
    /// §A.1 returned 0 having stored `bad_seq`.
    WildJumpArmed,
}

impl SeqVerdict {
    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Valid { .. } => "valid",
            Self::Probation => "probation",
            Self::WildJumpArmed => "wild-jump-armed",
        }
    }
}

broadcast_common::impl_spec_display!(SeqVerdict);

impl SeqValidity {
    /// §A.1 `init_seq(s, seq)`.
    pub fn init_seq(&mut self, seq: u16) {
        self.max_seq = seq;
        self.bad_seq = RTP_SEQ_MOD_PLUS_ONE;
        self.cycles = 0;
        self.probation = 0;
    }

    /// State used when a source is first seen: `max_seq = seq - 1`,
    /// `probation = MIN_SEQUENTIAL`.
    pub fn new_source(&mut self, seq: u16) {
        self.max_seq = seq.wrapping_sub(1);
        self.bad_seq = RTP_SEQ_MOD_PLUS_ONE;
        self.cycles = 0;
        self.probation = MIN_SEQUENTIAL;
    }

    /// §A.1 `update_seq(s, seq)`.
    pub fn update_seq(&mut self, seq: u16) -> SeqVerdict {
        let udelta = seq.wrapping_sub(self.max_seq);

        if self.probation != 0 {
            if seq == self.max_seq.wrapping_add(1) {
                self.probation -= 1;
                self.max_seq = seq;
                if self.probation == 0 {
                    self.init_seq(seq);
                    return SeqVerdict::Valid { restarted: false };
                }
            } else {
                self.probation = MIN_SEQUENTIAL - 1;
                self.max_seq = seq;
            }
            return SeqVerdict::Probation;
        }

        if udelta < MAX_DROPOUT {
            if seq < self.max_seq {
                self.cycles = self.cycles.wrapping_add(RTP_SEQ_MOD);
            }
            self.max_seq = seq;
            SeqVerdict::Valid { restarted: false }
        } else if udelta <= MAX_WILD_JUMP_U16 {
            if u32::from(seq) == self.bad_seq {
                self.init_seq(seq);
                SeqVerdict::Valid { restarted: true }
            } else {
                self.bad_seq = u32::from(seq.wrapping_add(1)) & (RTP_SEQ_MOD - 1);
                SeqVerdict::WildJumpArmed
            }
        } else {
            SeqVerdict::Valid { restarted: false }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn valid(verdict: SeqVerdict) -> bool {
        matches!(verdict, SeqVerdict::Valid { .. })
    }

    #[test]
    fn a_new_source_is_on_probation_until_two_sequential_packets() {
        let mut v = SeqValidity::default();
        v.new_source(1000);
        assert_eq!(v.update_seq(1000), SeqVerdict::Probation);
        assert_eq!(v.update_seq(1001), SeqVerdict::Valid { restarted: false });
        assert_eq!(v.probation, 0);
        assert_eq!(v.max_seq, 1001);
    }

    #[test]
    fn a_restart_while_on_probation_restarts_the_probation() {
        let mut v = SeqValidity::default();
        v.new_source(1000);
        assert_eq!(v.update_seq(500), SeqVerdict::Probation);
        assert_eq!(v.probation, MIN_SEQUENTIAL - 1);
        assert_eq!(v.max_seq, 500);
    }

    #[test]
    fn a_gap_below_max_dropout_is_in_order() {
        let mut v = SeqValidity::default();
        v.init_seq(1000);
        let verdict = v.update_seq(1000u16.wrapping_add(MAX_DROPOUT - 1));
        assert_eq!(verdict, SeqVerdict::Valid { restarted: false });
        assert_eq!(v.max_seq, 1000 + (MAX_DROPOUT - 1));
    }

    #[test]
    fn a_packet_at_or_beyond_max_dropout_ahead_arms_bad_seq() {
        let mut v = SeqValidity::default();
        v.init_seq(1000);
        let jump = 1000u16.wrapping_add(MAX_DROPOUT);
        assert_eq!(v.update_seq(jump), SeqVerdict::WildJumpArmed);
        assert_eq!(v.bad_seq, u32::from(jump) + 1);
        assert_eq!(v.max_seq, 1000, "the wild jump must not move max_seq");
    }

    #[test]
    fn two_sequential_packets_confirm_the_restart() {
        let mut v = SeqValidity::default();
        v.init_seq(1000);
        let jump = 1000u16.wrapping_add(MAX_DROPOUT);
        assert_eq!(v.update_seq(jump), SeqVerdict::WildJumpArmed);
        assert_eq!(
            v.update_seq(jump + 1),
            SeqVerdict::Valid { restarted: true }
        );
        assert_eq!(v.max_seq, jump + 1);
        assert_eq!(
            v.bad_seq, RTP_SEQ_MOD_PLUS_ONE,
            "init_seq re-arms the sentinel"
        );
    }

    #[test]
    fn one_stray_far_behind_packet_is_not_a_restart() {
        let mut v = SeqValidity::default();
        v.init_seq(1000);
        let stray = 1000u16.wrapping_sub(MAX_MISORDER as u16 + 1);
        assert_eq!(v.update_seq(stray), SeqVerdict::WildJumpArmed);
        assert_eq!(v.max_seq, 1000);
        assert!(valid(v.update_seq(1001)));
        assert_eq!(v.max_seq, 1001);
    }

    #[test]
    fn a_large_backward_step_arms_bad_seq_and_a_small_one_is_misordering() {
        let mut v = SeqValidity::default();
        v.init_seq(1000);
        let before = v;
        let behind = 1000 - MAX_MISORDER as u16;
        assert_eq!(v.update_seq(behind), SeqVerdict::WildJumpArmed);
        assert_eq!(v.max_seq, before.max_seq, "the jump must not move max_seq");
        let mut reordered = SeqValidity::default();
        reordered.init_seq(1000);
        let before = reordered;
        assert_eq!(
            reordered.update_seq(1000 - MAX_MISORDER as u16 + 1),
            SeqVerdict::Valid { restarted: false }
        );
        assert_eq!(
            reordered, before,
            "a misordered packet inside MAX_MISORDER changes no state"
        );
        let mut w = SeqValidity::default();
        w.init_seq(1000);
        assert_eq!(
            w.update_seq(behind - 1),
            SeqVerdict::WildJumpArmed,
            "past MAX_MISORDER §A.1 calls it a wild jump"
        );
        assert_eq!(w.max_seq, 1000);
    }

    #[test]
    fn a_wrap_counts_one_64k_cycle() {
        let mut v = SeqValidity::default();
        v.init_seq(u16::MAX - 1);
        assert!(valid(v.update_seq(u16::MAX)));
        assert!(valid(v.update_seq(0)));
        assert_eq!(v.cycles, RTP_SEQ_MOD);
    }

    #[test]
    fn the_wild_jump_window_cannot_underflow() {
        assert_eq!(
            u32::from(MAX_WILD_JUMP_U16),
            RTP_SEQ_MOD - MAX_MISORDER,
            "MAX_MISORDER must be far below RTP_SEQ_MOD"
        );
        // MAX_DROPOUT must stay below the wild-jump threshold, or an
        // ordinary gap would be read as a restart.
        assert_eq!(
            MAX_WILD_JUMP_U16.max(MAX_DROPOUT),
            MAX_WILD_JUMP_U16,
            "MAX_DROPOUT must sit below the wild-jump window"
        );
    }

    #[test]
    fn verdict_labels_are_spec_tokens() {
        assert_eq!(SeqVerdict::Probation.name(), "probation");
        assert_eq!(SeqVerdict::WildJumpArmed.to_string(), "wild-jump-armed");
        assert_eq!(SeqVerdict::Valid { restarted: true }.to_string(), "valid");
    }
}
