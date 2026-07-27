#![no_main]

use std::collections::VecDeque;
use std::convert::Infallible;
use std::num::NonZeroUsize;

use broadcast_common::{Demand, Stage, Timestamp};
use bytes::Bytes;
use libfuzzer_sys::fuzz_target;
use media_plane::{
    HandshakePolicy, IngestDriver, IngestSession, ProgramId, RetentionClass, SessionEvent,
    TrunkConfig,
};
use transmux::pipeline::DataCarriage;
use transmux::{CodecConfig, Sample, TrackSpec};

/// A session whose entire event stream is fuzzer-controlled: `feed` decodes
/// its input into zero or more [`SessionEvent`]s and never fails. This
/// drives [`IngestDriver`]'s program/track dispatch (`drain()`) -- the part
/// of `media-plane` that turns a remote peer's *reported* program/track
/// identifiers straight into per-program bookkeeping -- under an arbitrary
/// announce/sample/finish sequence, not just the handful of hand-written
/// scripts the unit tests cover.
struct FuzzSession {
    pending: VecDeque<SessionEvent>,
}

impl Stage for FuzzSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = Infallible;

    fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
        // One 5-byte record per event: [kind, program_lo, program_hi, track_id, sample_byte].
        for rec in input.chunks(5) {
            if rec.len() < 5 {
                break;
            }
            let program = ProgramId(u16::from_le_bytes([rec[1], rec[2]]) as u32);
            match rec[0] % 3 {
                0 => self.pending.push_back(SessionEvent::Established),
                1 => self.pending.push_back(SessionEvent::NewProgram {
                    program,
                    tracks: vec![TrackSpec::new(
                        rec[3] as u32,
                        90_000,
                        CodecConfig::Data {
                            stream_type: 0x06,
                            descriptors: Vec::new(),
                            carriage: DataCarriage::Pes,
                        },
                    )],
                }),
                _ => self.pending.push_back(SessionEvent::Sample {
                    program,
                    track_id: rec[3] as u32,
                    retention: if rec[4] % 2 == 0 {
                        RetentionClass::Timed
                    } else {
                        RetentionClass::Sparse
                    },
                    sample: Sample::new(Bytes::copy_from_slice(&[rec[4]; 1]), Some(0), Some(0), Some(1), true),
                }),
            }
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(4096)
    }
}

/// Takes the default `poll_transmit` (nothing to send).
impl IngestSession for FuzzSession {}

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).unwrap()
}

// `IngestDriver` (media-plane step 3f acceptance): must never panic under an
// arbitrary sequence of `Established`/`NewProgram`/`Sample` events, however
// many distinct `ProgramId`s a (fuzzer-modelled) session reports. Small,
// fixed `TrunkConfig` capacities and a capped input length keep *this
// harness's* memory bounded regardless of how many distinct programs a
// given input announces -- this target is checking `IngestDriver` doesn't
// panic, not re-litigating the unbounded-Trunk-per-distinct-`ProgramId`
// allocation already reported (not fixed) in the step 3f report; see that
// report for why no such per-session program cap exists in the driver
// itself.
fuzz_target!(|data: &[u8]| {
    let capped = &data[..data.len().min(4096)];
    let session = FuzzSession {
        pending: VecDeque::new(),
    };
    let trunk_config = TrunkConfig::new(nz(8), nz(4), nz(4), nz(4), nz(4));
    let handshake = HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX));
    let mut driver = IngestDriver::new(session, trunk_config, handshake);

    driver.feed(capped, Timestamp::ZERO);
    driver.on_deadline(Timestamp::from_nanos(1));
    driver.finish();
});
