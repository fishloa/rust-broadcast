//! Shared helpers for the read-ahead gate tests (issue #4086): the media-time
//! ledger a test mirrors the session's measure with, and the 27 MHz clock the
//! playback position is reported on.

use std::collections::BTreeMap;

use broadcast_client::{Event, Sample, TrackId};
use broadcast_common::clock33::{Clock27, PCR_HZ};

/// A playback position from 27 MHz ticks.
pub fn clock(ticks: u64) -> Clock27 {
    Clock27::from_27mhz_wrapping(ticks)
}

/// The media the session has delivered, rebuilt from the events it hands the
/// host, so a test knows what the session's own measure can see.
#[derive(Default)]
pub struct Media {
    timescales: BTreeMap<u32, u32>,
    /// Latest presentation end delivered, in 27 MHz ticks.
    pub latest_end: Option<i64>,
    /// Samples delivered.
    pub samples: usize,
}

impl Media {
    pub fn observe(&mut self, event: &Event) {
        match event {
            Event::Tracks(specs) => {
                for spec in specs {
                    self.timescales
                        .insert(spec.track_id.value(), spec.timescale.get());
                }
            }
            Event::Sample { track, sample, .. } => self.sample(*track, sample),
            _ => {}
        }
    }

    fn sample(&mut self, track: TrackId, sample: &Sample) {
        let Some(end) = self.sample_end(track.value(), sample) else {
            return;
        };
        self.samples = self.samples.saturating_add(1);
        self.latest_end = Some(self.latest_end.map_or(end, |latest| latest.max(end)));
    }

    fn sample_end(&self, track: u32, sample: &Sample) -> Option<i64> {
        let timescale = self.timescales.get(&track).copied()?;
        let pts = sample.timing.and_then(|t| t.pts())?;
        let duration = i64::from(sample.timing.and_then(|t| t.duration()).unwrap_or(0));
        let end =
            i128::from(pts.saturating_add(duration)) * i128::from(PCR_HZ) / i128::from(timescale);
        i64::try_from(end).ok()
    }

    /// The latest presentation end, on the 27 MHz clock.
    pub fn end_ticks(&self) -> u64 {
        self.latest_end
            .and_then(|end| u64::try_from(end).ok())
            .unwrap_or(0)
    }

    /// A position `behind` ticks before the latest presentation end: the lead a
    /// gate configured with that bound sees.
    pub fn position_behind(&self, behind: u64) -> u64 {
        self.end_ticks().saturating_sub(behind)
    }
}

/// One paced step of an HTTP host: the request it answered, and the lead the
/// session's own measure had when the step began.
pub struct Step {
    pub requests: Vec<String>,
    pub lead: Option<u64>,
}

fn drain_events(host: &mut super::hls_host::Host, media: &mut Media) {
    for event in core::mem::take(&mut host.events) {
        media.observe(&event);
    }
}

/// Drive the host for at most `steps`, reporting the player's position from
/// `pace` before each one and answering whatever request the session made.
pub fn drive(
    host: &mut super::hls_host::Host,
    media: &mut Media,
    steps: usize,
    mut pace: impl FnMut(&Media) -> Option<Clock27>,
) -> Vec<Step> {
    let mut log = Vec::new();
    for _ in 0..steps {
        let before = host.seen.len();
        host.collect();
        let position = pace(media);
        let lead = position.map(|position| media.end_ticks().saturating_sub(position.ticks()));
        host.session
            .on_playback_position(super::ts(host.now), position);
        host.collect();
        drain_events(host, media);
        let stepped = host.step();
        host.collect();
        drain_events(host, media);
        let requests: Vec<String> = host.seen[before..]
            .iter()
            .map(|seen| seen.url.clone())
            .collect();
        let quiet = requests.is_empty();
        log.push(Step { requests, lead });
        if !stepped && quiet {
            break;
        }
    }
    log
}

pub fn requests(log: &[Step]) -> Vec<String> {
    log.iter()
        .flat_map(|step| step.requests.iter().cloned())
        .collect()
}

pub fn segment_count(requested: &[String]) -> u64 {
    u64::try_from(requested.iter().filter(|url| url.contains("/seg")).count()).unwrap_or(0)
}

/// The gate allowance: media held may reach the cap plus the one segment already
/// in flight, so at most one request may appear at a step whose entry lead is at
/// or over the cap. A gate that leaks shows many.
pub fn assert_gate_held(log: &[Step], cap: u64) {
    let over = log
        .iter()
        .filter(|step| !step.requests.is_empty() && step.lead.is_some_and(|lead| lead >= cap))
        .count();
    assert!(
        over <= 1,
        "{over} requests were issued with the lead at or over the {cap}-tick cap"
    );
}
