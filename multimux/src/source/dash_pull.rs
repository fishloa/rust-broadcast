//! MPEG-DASH pull ingest source (issue #758; re-ported onto the media-plane
//! ingress traits at plan step 5a round 3): a sans-IO [`DashIngestSession`]
//! (fetch a remote MPD, resolve each selected Representation's
//! initialization + media segment URLs, demux the fetched fMP4/CMAF bytes)
//! plus [`run_dash_pull`], the tokio drive loop that performs the real GETs.
//!
//! There is no reusable sans-IO DASH client the way `hls_pull` reuses
//! `ll_hls_runtime::client::LlHlsClient` — this module's [`DashAction`]/
//! [`DashResourceId`] are its own request/response identity, in the same
//! *shape* `ll-hls-runtime` uses (an opaque request type + a correlating id),
//! chosen entirely by this module; `media-plane` never sees either.
//!
//! # Round 3: the in-read-path sleep is gone
//!
//! The pre-port `DashPullSession::maybe_refresh_mpd` buried a wall-clock
//! `Instant::elapsed` + `tokio::time::sleep` inside what was nominally a
//! "compute the next samples" step — a sans-IO session that sleeps
//! internally is not sans-IO. `Stage::next_deadline` now
//! *reports* when a live-MPD refresh is due (as an absolute [`Timestamp`] on
//! the driver's own clock, exactly like [`HandshakePolicy::establish_by`]),
//! and `Stage::on_deadline` — called by [`run_dash_pull`] once
//! that time has passed — is what turns it into a real
//! `DashAction::FetchMpd` request. The session never reads a clock or sleeps;
//! [`run_dash_pull`]'s own loop is the only place time is observed or waited
//! on, via `tokio::time::sleep_until`.
//!
//! # fMP4 demux without a `moov` per segment
//!
//! A DASH `SegmentTemplate` addresses a *separate* initialization resource
//! (`ftyp`+`moov`, fetched once per Representation) and media segments that
//! carry only `moof`+`mdat` — [`Fmp4Demux::unpackage`] needs a `moov` in the
//! same buffer it walks, so each media segment's bytes are demuxed
//! concatenated onto that Representation's cached init bytes, exactly the
//! pattern `ll_hls_runtime::client::engine`'s `demux_and_emit` uses for
//! CMAF parts/segments.
//!
//! # Round 3: the three `RepState` fields, judged individually
//!
//! - **`init_bytes`** (cached init segment, re-concatenated onto every media
//!   segment) — **kept**. Genuinely per-source parsing state: the `Trunk`
//!   stores decoded `Sample`s, never raw container bytes, so there is
//!   nothing to duplicate. This is a structural necessity of DASH's own wire
//!   format (separate init resource, `moof`+`mdat`-only media segments), not
//!   a cache of anything the plane already tracks.
//! - **`plan`** (the resolved-but-not-yet-fetched `(number, time)` sequence)
//!   — **kept**. This is fetch *scheduling* state — what to request next —
//!   which the `Trunk` has no concept of at all (it only ever sees samples
//!   after they arrive). Not a duplicate of anything downstream.
//! - **`last_number`** (high-water mark the plan has been extended to) —
//!   **kept**. Bookkeeping for *this session's own* addressing decisions (so
//!   a live-MPD refresh doesn't re-enqueue an already-planned segment
//!   number) — again a fact about pending fetches, which nothing else in the
//!   pipeline records.
//!
//! None of the three overlaps what a [`media_plane::trunk::Trunk`] holds
//! (decoded samples/segments/parts once ingested); all three are inputs to
//! *reaching* that point, not a parallel copy of it.
//!
//! # Track id remapping
//!
//! Each Representation's init segment is an independent CMAF file, so its
//! `moov` typically assigns the same local track_id (`1`) regardless of
//! which Representation it is. Global ids are assigned once every
//! Representation's init has resolved (not as each arrives), walking
//! Representations in their fixed MPD order, so the assignment is deterministic regardless of
//! which Representation's init happens to complete its fetch first (they are
//! now fetched concurrently, unlike the pre-port sequential `do_connect`).
//!
//! # Bounded in-flight fetches
//!
//! Each Representation only ever has **one** segment fetch outstanding at a
//! time (`RepState::in_flight`) — mirroring the pre-port `next_samples`'
//! per-round-per-Representation pacing — so the natural fan-out here is
//! already small (one per Representation, typically 1-2). [`run_dash_pull`]
//! additionally enforces the same [`crate::source::MAX_INFLIGHT_FETCHES`]
//! global cap every pull source uses, defensively.
//!
//! # v1 scope / Dynamic (live) MPDs / PlayReady
//!
//! Unchanged from the pre-port module: `SegmentTemplate`+`SegmentTimeline`
//! (`$Time$`) and `SegmentTemplate` with constant `@duration` (`$Number$`)
//! addressing; one Representation per `AdaptationSet` (the first); first
//! `Period` only. A `404` for a not-yet-available live-edge segment is
//! tolerated (retried) on a dynamic MPD; a static MPD treats the same `404`
//! as a hard error.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use broadcast_auth::Credentials;
use broadcast_common::{Demand, Stage, Timestamp, Unpackage};
use media_plane::ingress::{
    Dialer, HandshakePolicy, IngestSession, ProgramId, SessionEvent, run_dial,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};
use reqwest::{Client as HttpClient, StatusCode};
use tokio::task::JoinSet;
use transmux::dash_parse::{Mpd, MpdType, SegmentTemplate, SegmentTimeline};
use transmux::media::Fmp4Demux;
use transmux::pipeline::TrackSpec;
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::http_auth::{
    authenticated_get, credentials_from_url, resolve_credentials, strip_userinfo,
};
use crate::source::{IngestTimeouts, MAX_INFLIGHT_FETCHES, Source};

/// Default wait between exhausting a dynamic MPD's plan and its next refresh
/// attempt when `MPD@minimumUpdatePeriod` is absent — unchanged from the
/// pre-port module.
const DEFAULT_MPD_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// How long a `run_*_pull` drive loop parks when its session has, momentarily,
/// neither an outbound request queued nor a fetch in flight — and has not
/// ended. A bare `continue` there would spin the loop with no `.await` in it,
/// which on a current-thread runtime starves every other task on the executor
/// (including the in-flight fetches this loop is waiting for). Short enough
/// that it costs no observable latency, long enough that it is not a spin.
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// A remote MPEG-DASH presentation to pull: its MPD URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
#[derive(Clone)]
pub struct DashPullRoute {
    name: String,
    url: String,
    timeouts: IngestTimeouts,
    /// Config-supplied credentials, taking precedence over any URL userinfo
    /// — see `crate::source::http_auth::resolve_credentials`.
    auth: Option<Credentials>,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`): `url` may carry a live
/// origin's `user:pass@` userinfo, so it must never render verbatim; `auth`
/// (if present) carries a raw password/token, also never rendered verbatim.
impl std::fmt::Debug for DashPullRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashPullRoute")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl DashPullRoute {
    /// Build a route descriptor. `url` is the MPD URL to pull.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        DashPullRoute {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
        }
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Attaches config-supplied credentials, overriding any URL userinfo.
    #[must_use]
    pub fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.auth = auth;
        self
    }
}

impl Source for DashPullRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// Identifies one selected Representation by its position in MPD document
/// order — stable across a session's lifetime (representations are never
/// added/removed after connect, only their plans extended).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RepIndex(pub usize);

/// This session's own request/response identity — see the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashResourceId {
    /// The MPD itself — initial fetch, or a live-MPD refresh.
    Mpd,
    /// A Representation's initialization segment.
    Init(RepIndex),
    /// One media segment, by its (already-resolved) segment number.
    Segment(RepIndex, u64),
}

/// One unit of IO [`run_dash_pull`] must perform.
#[derive(Debug, Clone)]
pub enum DashAction {
    /// Fetch (or re-fetch, for a live refresh) the MPD.
    FetchMpd { url: String },
    /// Fetch a Representation's initialization segment.
    FetchInit { rep: RepIndex, url: String },
    /// Fetch one media segment. `tolerate_404`: a dynamic MPD's live edge may
    /// legitimately not exist yet — see the module doc.
    FetchSegment {
        rep: RepIndex,
        number: u64,
        time: Option<u64>,
        url: String,
        tolerate_404: bool,
    },
}

/// A Representation resolved from the MPD but not yet initialized (its init
/// fetch is outstanding) — see `Phase::AwaitingInits`.
struct PendingRep {
    rep_id: String,
    media_template: Option<String>,
    bandwidth: u64,
    plan: VecDeque<(u64, Option<u64>)>,
    last_number: u64,
    init_bytes: Option<Vec<u8>>,
    /// Local (init-segment-relative) track specs, once fetched.
    local_tracks: Option<Vec<TrackSpec>>,
}

/// One selected Representation's live state — see the module doc's "the
/// three `RepState` fields, judged individually".
struct RepState {
    rep_id: String,
    media_template: Option<String>,
    bandwidth: u64,
    init_bytes: Vec<u8>,
    local_to_global: HashMap<u32, u32>,
    plan: VecDeque<(u64, Option<u64>)>,
    last_number: u64,
    /// At most one outstanding segment fetch per Representation at a time —
    /// see the module doc's "Bounded in-flight fetches".
    in_flight: bool,
}

/// Live (post-Established) session state.
struct LiveState {
    mpd_url: Url,
    is_dynamic: bool,
    minimum_update_period: Option<Duration>,
    last_mpd_fetch: Timestamp,
    reps: Vec<RepState>,
    mpd_refresh_in_flight: bool,
}

impl LiveState {
    fn all_reps_idle_and_exhausted(&self) -> bool {
        self.reps.iter().all(|r| !r.in_flight && r.plan.is_empty())
    }

    fn refresh_interval(&self) -> Duration {
        self.minimum_update_period
            .unwrap_or(DEFAULT_MPD_REFRESH_INTERVAL)
    }
}

enum Phase {
    AwaitingMpd,
    /// Every Representation resolved from the initial MPD, plus that MPD's
    /// own `@type`/`@minimumUpdatePeriod` — carried through this phase
    /// because [`LiveState`] needs them the moment the last init resolves,
    /// and re-parsing the MPD just to recover them would mean a second fetch.
    AwaitingInits {
        reps: Vec<PendingRep>,
        is_dynamic: bool,
        minimum_update_period: Option<Duration>,
    },
    Live(LiveState),
}

/// The sans-IO DASH-pull [`IngestSession`]. [`run_dash_pull`] performs every
/// real GET and feeds responses in.
pub struct DashIngestSession {
    mpd_url: Url,
    phase: Phase,
    pending_requests: VecDeque<DashAction>,
    pending_events: VecDeque<SessionEvent>,
}

impl DashIngestSession {
    /// Construct a fresh session for `mpd_url` — performs no I/O; queues
    /// only the first `DashAction::FetchMpd`.
    pub fn new(mpd_url: Url) -> Self {
        let mut pending_requests = VecDeque::new();
        pending_requests.push_back(DashAction::FetchMpd {
            url: mpd_url.to_string(),
        });
        DashIngestSession {
            mpd_url,
            phase: Phase::AwaitingMpd,
            pending_requests,
            pending_events: VecDeque::new(),
        }
    }

    /// True once every Representation's plan is empty, none is in flight, and
    /// the MPD is static — see [`run_dash_pull`]'s use via
    /// [`media_plane::ingress::IngestDriver::session`].
    pub fn ended(&self) -> bool {
        matches!(&self.phase, Phase::Live(live) if !live.is_dynamic && live.all_reps_idle_and_exhausted())
    }

    fn on_mpd(&mut self, bytes: &[u8]) -> Result<()> {
        let text = std::str::from_utf8(bytes).map_err(|e| MultimuxError::Connect {
            reason: format!("dash-pull: mpd is not valid UTF-8: {e}"),
        })?;
        let mpd = Mpd::parse(text).map_err(|e| MultimuxError::Connect {
            reason: format!("dash-pull: mpd parse: {e}"),
        })?;
        match std::mem::replace(&mut self.phase, Phase::AwaitingMpd) {
            Phase::AwaitingMpd => self.on_initial_mpd(mpd),
            other @ Phase::AwaitingInits { .. } => {
                // A second MPD delivery while the first round of init fetches
                // is still outstanding — a driver bug or a duplicate delivery.
                // Ignored rather than restarting resolution: re-running
                // `on_initial_mpd` would re-queue a `FetchInit` for every
                // Representation on top of the ones already in flight, so the
                // "bounded in-flight fetches" property would depend on the
                // driver never double-delivering.
                self.phase = other;
                Ok(())
            }
            Phase::Live(mut live) => {
                live.is_dynamic = matches!(mpd.mpd_type, MpdType::Dynamic);
                live.minimum_update_period = mpd.minimum_update_period;
                live.mpd_refresh_in_flight = false;
                if let Some(period) = mpd.periods.first() {
                    let total_duration = period.duration.or(mpd.media_presentation_duration);
                    for rep in &mut live.reps {
                        let found = period
                            .adaptation_sets
                            .iter()
                            .find_map(|a| a.representations.iter().find(|r| r.id == rep.rep_id));
                        let Some(found_rep) = found else { continue };
                        let Some(st) = &found_rep.segment_template else {
                            continue;
                        };
                        let plan = build_plan(st, total_duration)?;
                        for (number, time) in plan {
                            if number > rep.last_number {
                                rep.last_number = number;
                                rep.plan.push_back((number, time));
                            }
                        }
                    }
                }
                self.phase = Phase::Live(live);
                self.pump_segment_fetches();
                Ok(())
            }
        }
    }

    fn on_initial_mpd(&mut self, mpd: Mpd) -> Result<()> {
        let period = mpd.periods.first().ok_or_else(|| MultimuxError::Connect {
            reason: "dash-pull: mpd has no Period".into(),
        })?;
        let total_duration = period.duration.or(mpd.media_presentation_duration);

        let mut reps = Vec::new();
        for aset in &period.adaptation_sets {
            let Some(rep) = aset.representations.first() else {
                continue;
            };
            let Some(st) = &rep.segment_template else {
                continue;
            };
            let init_template = st
                .initialization
                .as_deref()
                .ok_or_else(|| MultimuxError::Connect {
                    reason: format!(
                        "dash-pull: representation {:?} has no SegmentTemplate@initialization",
                        rep.id
                    ),
                })?;
            let init_rel =
                SegmentTemplate::resolve(init_template, &rep.id, None, None, Some(rep.bandwidth));
            let init_url = self.mpd_url.join(&init_rel).map_err(|e| MultimuxError::Connect {
                reason: format!("dash-pull: bad initialization URL {init_rel:?}: {e}"),
            })?;

            let plan = build_plan(st, total_duration)?;
            let last_number = plan
                .last()
                .map(|(n, _)| *n)
                .unwrap_or_else(|| st.start_number.saturating_sub(1));

            let idx = RepIndex(reps.len());
            self.pending_requests.push_back(DashAction::FetchInit {
                rep: idx,
                url: init_url.to_string(),
            });
            reps.push(PendingRep {
                rep_id: rep.id.clone(),
                media_template: st.media.clone(),
                bandwidth: rep.bandwidth,
                plan: plan.into(),
                last_number,
                init_bytes: None,
                local_tracks: None,
            });
        }
        if reps.is_empty() {
            return Err(MultimuxError::Connect {
                reason: "dash-pull: mpd resolved no usable representation".into(),
            });
        }
        self.phase = Phase::AwaitingInits {
            reps,
            is_dynamic: matches!(mpd.mpd_type, MpdType::Dynamic),
            minimum_update_period: mpd.minimum_update_period,
        };
        // `Established` is queued the moment the MPD parses: the *connection*
        // is up, and there is no media-level handshake left to negotiate --
        // the byte-stream sources' precedent (see `ts_program`'s module doc).
        // `NewProgram` waits until every Representation's init has arrived,
        // because only then are the global track ids assignable.
        self.pending_events.push_back(SessionEvent::Established);
        Ok(())
    }

    fn on_init(&mut self, rep: RepIndex, bytes: &[u8]) -> Result<()> {
        let Phase::AwaitingInits { reps, .. } = &mut self.phase else {
            return Ok(()); // late/duplicate delivery once already Live: ignore.
        };
        let Some(pending) = reps.get_mut(rep.0) else {
            return Ok(());
        };
        let media = Fmp4Demux::new().unpackage(bytes)?;
        if media.tracks.is_empty() {
            return Err(MultimuxError::Connect {
                reason: format!(
                    "dash-pull: representation {:?} init segment carries no track",
                    pending.rep_id
                ),
            });
        }
        pending.init_bytes = Some(bytes.to_vec());
        pending.local_tracks = Some(media.tracks.into_iter().map(|t| t.spec).collect());

        if reps.iter().all(|r| r.init_bytes.is_some()) {
            self.finish_awaiting_inits()?;
        }
        Ok(())
    }

    /// Every Representation's init has resolved: assign global track ids in
    /// fixed MPD order (see the module doc's "Track id remapping"), announce
    /// the program, and kick off the first round of segment fetches.
    fn finish_awaiting_inits(&mut self) -> Result<()> {
        let Phase::AwaitingInits {
            reps: pending,
            is_dynamic,
            minimum_update_period,
        } = std::mem::replace(&mut self.phase, Phase::AwaitingMpd)
        else {
            unreachable!("caller only invokes this from Phase::AwaitingInits");
        };
        let mut next_track_id: u32 = 1;
        let mut specs = Vec::new();
        let mut reps = Vec::new();
        for p in pending {
            let local_tracks = p.local_tracks.expect("checked all-Some by the caller");
            let init_bytes = p.init_bytes.expect("checked all-Some by the caller");
            let mut local_to_global = HashMap::new();
            for spec in &local_tracks {
                let global_id = next_track_id;
                next_track_id += 1;
                local_to_global.insert(spec.track_id, global_id);
                let mut remapped = spec.clone();
                remapped.track_id = global_id;
                specs.push(remapped);
            }
            reps.push(RepState {
                rep_id: p.rep_id,
                media_template: p.media_template,
                bandwidth: p.bandwidth,
                init_bytes,
                local_to_global,
                plan: p.plan,
                last_number: p.last_number,
                in_flight: false,
            });
        }
        self.phase = Phase::Live(LiveState {
            mpd_url: self.mpd_url.clone(),
            // Carried through `Phase::AwaitingInits` from the initial MPD —
            // getting this wrong would make a *dynamic* MPD report `ended()`
            // the moment its first plan drained, ending a live route after
            // one pass instead of refreshing.
            is_dynamic,
            minimum_update_period,
            last_mpd_fetch: Timestamp::ZERO,
            reps,
            mpd_refresh_in_flight: false,
        });
        self.pending_events.push_back(SessionEvent::NewProgram {
            program: ProgramId(0),
            tracks: specs,
        });
        self.pump_segment_fetches();
        Ok(())
    }

    /// For every idle Representation with a pending plan entry, dequeue it
    /// and enqueue its `DashAction::FetchSegment` — see the module doc's
    /// "Bounded in-flight fetches".
    fn pump_segment_fetches(&mut self) {
        let Phase::Live(live) = &mut self.phase else {
            return;
        };
        let is_dynamic = live.is_dynamic;
        let mpd_url = live.mpd_url.clone();
        for (i, rep) in live.reps.iter_mut().enumerate() {
            if rep.in_flight {
                continue;
            }
            let Some((number, time)) = rep.plan.pop_front() else {
                continue;
            };
            let template = rep.media_template.clone().unwrap_or_default();
            let rel = SegmentTemplate::resolve(&template, &rep.rep_id, Some(number), time, Some(rep.bandwidth));
            let Ok(url) = mpd_url.join(&rel) else {
                // Malformed template output: drop this entry rather than
                // wedge the Representation forever on a URL that will never
                // resolve.
                continue;
            };
            rep.in_flight = true;
            self.pending_requests.push_back(DashAction::FetchSegment {
                rep: RepIndex(i),
                number,
                time,
                url: url.to_string(),
                tolerate_404: is_dynamic,
            });
        }
    }

    fn on_segment(&mut self, rep_idx: RepIndex, bytes: &[u8]) -> Result<()> {
        let Phase::Live(live) = &mut self.phase else {
            return Ok(());
        };
        let Some(rep) = live.reps.get_mut(rep_idx.0) else {
            return Ok(());
        };
        rep.in_flight = false;
        let mut combined = Vec::with_capacity(rep.init_bytes.len() + bytes.len());
        combined.extend_from_slice(&rep.init_bytes);
        combined.extend_from_slice(bytes);
        let media = Fmp4Demux::new().unpackage(combined.as_slice())?;
        for track in media.tracks {
            if let Some(&global_id) = rep.local_to_global.get(&track.spec.track_id) {
                for sample in track.samples {
                    self.pending_events.push_back(SessionEvent::Sample {
                        program: ProgramId(0),
                        track_id: global_id,
                        retention: RetentionClass::Timed,
                        sample,
                    });
                }
            }
        }
        self.pump_segment_fetches();
        Ok(())
    }

}

impl Stage for DashIngestSession {
    type In<'a> = (DashResourceId, &'a [u8]);
    type Out = SessionEvent;
    type Error = MultimuxError;

    fn feed(&mut self, (id, bytes): (DashResourceId, &[u8]), now: Timestamp) -> Result<()> {
        let was_mpd = matches!(id, DashResourceId::Mpd);
        match id {
            DashResourceId::Mpd => self.on_mpd(bytes)?,
            DashResourceId::Init(rep) => self.on_init(rep, bytes)?,
            DashResourceId::Segment(rep, _number) => self.on_segment(rep, bytes)?,
        }
        if let Phase::Live(live) = &mut self.phase {
            // Stamp the refresh clock on an MPD delivery, and also on the very
            // first feed that reaches `Live` at all: the *initial* MPD arrives
            // while still `AwaitingInits`, so without this second case
            // `last_mpd_fetch` would stay `ZERO` and the first live refresh
            // would be considered overdue the instant the session went live.
            if was_mpd || live.last_mpd_fetch == Timestamp::ZERO {
                live.last_mpd_fetch = now;
            }
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending_events.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        let Phase::Live(live) = &self.phase else {
            return None;
        };
        if !live.is_dynamic || live.mpd_refresh_in_flight || !live.all_reps_idle_and_exhausted() {
            return None;
        }
        Some(live.last_mpd_fetch.saturating_add(live.refresh_interval()))
    }

    fn on_deadline(&mut self, now: Timestamp) {
        let Phase::Live(live) = &mut self.phase else {
            return;
        };
        if !live.is_dynamic || live.mpd_refresh_in_flight || !live.all_reps_idle_and_exhausted() {
            return;
        }
        if now < live.last_mpd_fetch.saturating_add(live.refresh_interval()) {
            return;
        }
        live.mpd_refresh_in_flight = true;
        self.pending_requests.push_back(DashAction::FetchMpd {
            url: live.mpd_url.to_string(),
        });
    }

    fn demand(&self) -> Demand {
        Demand::new(crate::source::MAX_TS_READ)
    }
}

impl IngestSession for DashIngestSession {
    type Request = DashAction;

    fn poll_transmit(&mut self) -> Option<DashAction> {
        self.pending_requests.pop_front()
    }
}

/// Constructs a [`DashIngestSession`] — performs **no I/O**.
pub struct DashPullDialer {
    mpd_url: Url,
}

impl Dialer for DashPullDialer {
    type Session = DashIngestSession;
    type Error = MultimuxError;

    fn dial(&mut self) -> Result<DashIngestSession> {
        Ok(DashIngestSession::new(self.mpd_url.clone()))
    }
}

/// Adapter around [`SegmentTimeline::enumerate`] — see the pre-port module's
/// own doc for why this is isolated to one call site.
fn enumerate_timeline(timeline: &SegmentTimeline, start_number: u64) -> Result<Vec<(u64, u64)>> {
    timeline
        .enumerate(start_number)
        .map_err(|e| MultimuxError::Connect {
            reason: format!("dash-pull: segment timeline: {e}"),
        })
}

/// Builds a Representation's `(number, time)` segment plan from its
/// effective [`SegmentTemplate`] — unchanged from the pre-port module.
fn build_plan(
    st: &SegmentTemplate,
    total_duration: Option<Duration>,
) -> Result<Vec<(u64, Option<u64>)>> {
    if let Some(timeline) = &st.timeline {
        return Ok(enumerate_timeline(timeline, st.start_number)?
            .into_iter()
            .map(|(n, t)| (n, Some(t)))
            .collect());
    }
    let Some(duration) = st.duration else {
        return Ok(Vec::new());
    };
    if duration == 0 {
        return Ok(Vec::new());
    }
    let count = match total_duration {
        Some(total) => {
            let total_ticks = (total.as_secs_f64() * st.timescale as f64).ceil() as u64;
            total_ticks.div_ceil(duration) as usize
        }
        None => 0,
    };
    Ok(st
        .number_sequence(count)
        .into_iter()
        .map(|n| (n, None))
        .collect())
}

fn status_error(what: &str, status: StatusCode) -> MultimuxError {
    if status == StatusCode::UNAUTHORIZED {
        MultimuxError::Auth {
            reason: format!("dash-pull {what}: {status}"),
        }
    } else {
        MultimuxError::Connect {
            reason: format!("dash-pull {what}: HTTP {status}"),
        }
    }
}

/// How long [`run_dash_pull`] waits before retrying a tolerated `404` for a
/// live-edge segment not yet available — avoids hot-looping the same
/// not-yet-available request. The retry runs as its own delayed task (see
/// `spawn_segment_fetch`), so it never blocks the main loop from observing
/// other in-flight fetches meanwhile.
const SEGMENT_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Outcome of one fetch task, as seen by [`run_dash_pull`]'s join loop.
enum FetchOutcome {
    Bytes(Vec<u8>),
    /// A tolerated `404` for a live-edge segment — retried by
    /// [`run_dash_pull`] itself (re-spawning the same `FetchSegment`, after
    /// [`SEGMENT_RETRY_DELAY`]) without ever touching the session: from
    /// [`DashIngestSession`]'s point of view this Representation's fetch was
    /// never *not* in flight, matching `RepState::in_flight`'s meaning.
    NotReady,
}

async fn fetch_one(
    client: &HttpClient,
    url: &str,
    creds: Option<&Credentials>,
    what: &str,
    tolerate_404: bool,
) -> Result<FetchOutcome> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if tolerate_404 && status == StatusCode::NOT_FOUND {
        return Ok(FetchOutcome::NotReady);
    }
    if !status.is_success() {
        return Err(status_error(what, status));
    }
    response
        .bytes()
        .await
        .map(|b| FetchOutcome::Bytes(b.to_vec()))
        .map_err(|e| MultimuxError::Connect {
            reason: format!("dash-pull {what} read: {e}"),
        })
}

/// One join result: the resource id, and (for a segment, so a tolerated
/// `404` can be retried without the session's help) the number/time/url/
/// tolerate_404 that produced it.
struct JoinedFetch {
    id: DashResourceId,
    number: u64,
    time: Option<u64>,
    url: String,
    tolerate_404: bool,
    outcome: Result<FetchOutcome>,
}

/// Spawns one fetch (after `delay`, for a retry) into `inflight`, tagging the
/// result with everything [`run_dash_pull`]'s join loop needs to either feed
/// it to the session or retry it.
#[allow(clippy::too_many_arguments)]
fn spawn_fetch(
    inflight: &mut JoinSet<JoinedFetch>,
    http: HttpClient,
    creds: Option<Credentials>,
    id: DashResourceId,
    number: u64,
    time: Option<u64>,
    url: String,
    what: &'static str,
    tolerate_404: bool,
    read_timeout: Duration,
    delay: Duration,
) {
    inflight.spawn(async move {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let outcome = tokio::time::timeout(
            read_timeout,
            fetch_one(&http, &url, creds.as_ref(), what, tolerate_404),
        )
        .await
        .unwrap_or_else(|_| {
            Err(MultimuxError::Connect {
                reason: format!("dash-pull {what} ({id:?}) read exceeded {read_timeout:?}"),
            })
        });
        JoinedFetch {
            id,
            number,
            time,
            url,
            tolerate_404,
            outcome,
        }
    });
}

fn build_client(route: &DashPullRoute) -> Result<(HttpClient, Url, Option<Credentials>)> {
    let parsed = Url::parse(&route.url).map_err(|e| MultimuxError::Connect {
        reason: format!(
            "bad DASH-pull URL {}: {e}",
            crate::redact::redact_url(&route.url)
        ),
    })?;
    let credentials = resolve_credentials(route.auth.clone(), credentials_from_url(&parsed)?);
    let clean_url = strip_userinfo(&parsed)?;
    let http = HttpClient::builder()
        .build()
        .map_err(|e| MultimuxError::Connect {
            reason: format!("reqwest client: {e}"),
        })?;
    Ok((http, clean_url, credentials))
}

/// Turns a driver that has reached a terminal [`HealthState`] into this
/// crate's own `Result`, moving the concrete session error out via
/// [`media_plane::ingress::IngestDriver::into_health`].
///
/// **This is why a pull drive loop must check `health()` after every feed at
/// all**: `IngestDriver::feed` records a session error in `health` and
/// returns `()`, so a loop that only ever calls `feed` never observes it. A
/// `smooth_pull` session rejecting a PlayReady-protected manifest, or an
/// `hls_pull` session rejecting a malformed playlist, would otherwise leave
/// the loop spinning against a session that can never make progress.
fn terminal_result<S>(driver: media_plane::ingress::IngestDriver<S>, what: &str) -> Result<()>
where
    S: media_plane::ingress::IngestSession<Error = MultimuxError>,
{
    match driver.into_health() {
        media_plane::ingress::HealthState::Failed(e) => Err(e),
        media_plane::ingress::HealthState::HandshakeTimedOut { deadline } => {
            Err(MultimuxError::Connect {
                reason: format!("{what}: handshake deadline {deadline:?} passed"),
            })
        }
        // `Ended` is a clean finish; the two running states are unreachable
        // here (callers only call this once `health().is_running()` is false)
        // but map to `Ok` rather than panicking on a future variant.
        _ => Ok(()),
    }
}

/// Drives `route` to completion: dial (no I/O), then pump `poll_transmit` →
/// fetch → `feed` until a static MPD's every Representation is exhausted, or
/// a hard fetch failure occurs. A dynamic MPD never ends on its own (matches
/// the pre-port module). A live-edge `404` that this loop tolerates (a
/// dynamic MPD's not-yet-available segment) is retried by this loop directly,
/// after a fixed short delay, without ever calling into the session.
pub async fn run_dash_pull(
    route: &DashPullRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
) -> Result<()> {
    let (http, clean_url, credentials) = build_client(route)?;
    let mut dialer = DashPullDialer { mpd_url: clean_url };
    let mut driver = run_dial(
        &mut dialer,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    )?;

    let read_timeout = route.timeouts.read;
    let mut backlog: VecDeque<DashAction> = VecDeque::new();
    let mut inflight: JoinSet<JoinedFetch> = JoinSet::new();
    let start = std::time::Instant::now();

    loop {
        while let Some(action) = driver.poll_transmit() {
            backlog.push_back(action);
        }

        while inflight.len() < MAX_INFLIGHT_FETCHES {
            let Some(action) = backlog.pop_front() else {
                break;
            };
            match action {
                DashAction::FetchMpd { url } => spawn_fetch(
                    &mut inflight,
                    http.clone(),
                    credentials.clone(),
                    DashResourceId::Mpd,
                    0,
                    None,
                    url,
                    "mpd",
                    false,
                    read_timeout,
                    Duration::ZERO,
                ),
                DashAction::FetchInit { rep, url } => spawn_fetch(
                    &mut inflight,
                    http.clone(),
                    credentials.clone(),
                    DashResourceId::Init(rep),
                    0,
                    None,
                    url,
                    "init",
                    false,
                    read_timeout,
                    Duration::ZERO,
                ),
                DashAction::FetchSegment {
                    rep,
                    number,
                    time,
                    url,
                    tolerate_404,
                } => spawn_fetch(
                    &mut inflight,
                    http.clone(),
                    credentials.clone(),
                    DashResourceId::Segment(rep, number),
                    number,
                    time,
                    url,
                    "segment",
                    tolerate_404,
                    read_timeout,
                    Duration::ZERO,
                ),
            }
        }

        if inflight.is_empty() {
            if driver.session().ended() {
                driver.finish();
                return terminal_result(driver, "dash-pull");
            }
            match driver.next_deadline() {
                Some(deadline) => {
                    let now = Timestamp::from_instant(start, std::time::Instant::now());
                    if now < deadline {
                        tokio::time::sleep(deadline.saturating_sub(now)).await;
                    }
                    let now = Timestamp::from_instant(start, std::time::Instant::now());
                    driver.on_deadline(now);
                }
                // No scheduled work and nothing in flight: park briefly
                // rather than spinning — see `IDLE_POLL_INTERVAL`.
                None => tokio::time::sleep(IDLE_POLL_INTERVAL).await,
            }
            continue;
        }

        let joined = inflight.join_next().await;
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        match joined {
            Some(Ok(JoinedFetch {
                id,
                outcome: Ok(FetchOutcome::Bytes(bytes)),
                ..
            })) => {
                driver.feed((id, bytes.as_slice()), now);
            }
            Some(Ok(JoinedFetch {
                id: DashResourceId::Segment(rep, _),
                number,
                time,
                url,
                tolerate_404,
                outcome: Ok(FetchOutcome::NotReady),
            })) => {
                // Retried directly, without touching the session — see the
                // module doc's `FetchOutcome::NotReady`.
                spawn_fetch(
                    &mut inflight,
                    http.clone(),
                    credentials.clone(),
                    DashResourceId::Segment(rep, number),
                    number,
                    time,
                    url,
                    "segment",
                    tolerate_404,
                    read_timeout,
                    SEGMENT_RETRY_DELAY,
                );
            }
            Some(Ok(JoinedFetch {
                outcome: Ok(FetchOutcome::NotReady),
                ..
            })) => {
                // Only segment fetches are ever tolerant of 404 -- see
                // `fetch_one`'s callers.
            }
            Some(Ok(JoinedFetch {
                outcome: Err(e), ..
            })) => return Err(e),
            Some(Err(join_err)) => {
                return Err(MultimuxError::Connect {
                    reason: format!("dash-pull: fetch task failed: {join_err}"),
                });
            }
            None => unreachable!("checked inflight.is_empty() above"),
        }

        if !driver.health().is_running() {
            // The feed above drove the session terminal (a rejected
            // playlist/manifest/resource) — see `terminal_result`.
            return terminal_result(driver, "dash-pull");
        }

        if driver.session().ended() {
            driver.finish();
            return terminal_result(driver, "dash-pull");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::Path as AxumPath;
    use axum::http::StatusCode as AxumStatusCode;
    use axum::response::{IntoResponse, Response as AxumResponse};
    use axum::routing::get;
    use media_plane::ingress::HandshakePolicy;
    use media_plane::trunk::{SampleCursor, SampleCursorItem, TrunkConfig};
    use std::num::NonZeroUsize;
    use transmux::CodecConfig;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    fn trunk_config() -> TrunkConfig {
        TrunkConfig::new(nz(64), nz(16), nz(8), nz(8), nz(8))
    }

    fn handshake() -> HandshakePolicy {
        HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
    }

    fn drain(cursor: &mut SampleCursor) -> usize {
        let mut n = 0;
        while let Some(item) = cursor.poll() {
            if matches!(item, SampleCursorItem::Timed { .. }) {
                n += 1;
            }
        }
        n
    }

    /// Path to the real, committed DASH fixture (isoff-live profile, real
    /// ffmpeg-produced fMP4 — `fixtures/dash/` at the workspace root): one
    /// video (AVC) + one audio (AAC) Representation, `SegmentTimeline`
    /// (`$Time$`) addressing, `type="static"`.
    fn fixture_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/dash"))
    }

    /// Starts a real axum server serving every file in `fixtures/dash/` by
    /// name under `/`. `auth`, if given, gates every request; `stall_segment`,
    /// if given, makes that one filename hang forever instead of responding.
    async fn start_fixture_server(
        auth: Option<crate::testutil::MockAuthScheme>,
        stall_segment: Option<&'static str>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(
            AxumPath(name): AxumPath<String>,
            axum::extract::State((dir, stall)): axum::extract::State<(
                std::path::PathBuf,
                Option<&'static str>,
            )>,
        ) -> AxumResponse {
            if stall == Some(name.as_str()) {
                std::future::pending::<()>().await;
                unreachable!("pending future never resolves");
            }
            let path = dir.join(&name);
            match std::fs::read(&path) {
                Ok(bytes) => bytes.into_response(),
                Err(_) => AxumStatusCode::NOT_FOUND.into_response(),
            }
        }
        let mut app = Router::new()
            .route("/:name", get(handler))
            .with_state((fixture_dir(), stall_segment));
        if let Some(scheme) = auth {
            app = crate::testutil::require_auth(app, scheme);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("axum server");
        });
        (format!("http://{addr}/manifest.mpd"), server)
    }

    /// Independent oracle: demuxes every fixture segment directly (init +
    /// each chunk concatenated) to know the real expected sample counts per
    /// stream, without going through `DashIngestSession` at all.
    fn oracle_sample_count(init_name: &str, chunk_names: &[&str]) -> usize {
        let dir = fixture_dir();
        let init = std::fs::read(dir.join(init_name)).expect("read init fixture");
        let mut total = 0usize;
        for chunk_name in chunk_names {
            let chunk = std::fs::read(dir.join(chunk_name)).expect("read chunk fixture");
            let mut combined = init.clone();
            combined.extend_from_slice(&chunk);
            let media = Fmp4Demux::new()
                .unpackage(combined.as_slice())
                .expect("oracle demux");
            total += media.tracks.iter().map(|t| t.samples.len()).sum::<usize>();
        }
        total
    }

    /// Manually drives the raw [`DashIngestSession`] (dial → poll_transmit →
    /// fetch → feed), bypassing [`media_plane::ingress::IngestDriver`]
    /// entirely, so the test can inspect [`SessionEvent::NewProgram`]'s
    /// `tracks` and each [`SessionEvent::Sample`]'s `track_id` directly —
    /// `IngestDriver::drain` consumes those events internally (dispatching
    /// into a `Trunk`), so there is no way to observe them through a driver.
    /// Returns the recovered `TrackSpec`s and a per-`track_id` sample count.
    async fn drive_and_collect(
        route: &DashPullRoute,
    ) -> Result<(Vec<TrackSpec>, HashMap<u32, usize>)> {
        let (http, clean_url, credentials) = build_client(route)?;
        let mut session = DashIngestSession::new(clean_url);
        let mut backlog: VecDeque<DashAction> = VecDeque::new();
        let mut specs: Vec<TrackSpec> = Vec::new();
        let mut per_track: HashMap<u32, usize> = HashMap::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);

        loop {
            while let Some(a) = session.poll_transmit() {
                backlog.push_back(a);
            }
            while let Some(event) = session.poll() {
                match event {
                    SessionEvent::NewProgram { tracks, .. } => specs = tracks,
                    SessionEvent::Sample { track_id, .. } => {
                        *per_track.entry(track_id).or_insert(0) += 1;
                    }
                    SessionEvent::Established => {}
                    _ => {}
                }
            }
            let Some(action) = backlog.pop_front() else {
                if session.ended() || tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(IDLE_POLL_INTERVAL).await;
                continue;
            };
            let now = Timestamp::from_nanos(0);
            match action {
                DashAction::FetchMpd { url } => {
                    if let FetchOutcome::Bytes(b) = tokio::time::timeout(
                        route.timeouts.read,
                        fetch_one(&http, &url, credentials.as_ref(), "mpd", false),
                    )
                    .await
                    .map_err(|_| MultimuxError::Connect {
                        reason: "mpd fetch timed out".into(),
                    })??
                    {
                        session.feed((DashResourceId::Mpd, b.as_slice()), now)?;
                    }
                }
                DashAction::FetchInit { rep, url } => {
                    if let FetchOutcome::Bytes(b) = tokio::time::timeout(
                        route.timeouts.read,
                        fetch_one(&http, &url, credentials.as_ref(), "init", false),
                    )
                    .await
                    .map_err(|_| MultimuxError::Connect {
                        reason: "init fetch timed out".into(),
                    })??
                    {
                        session.feed((DashResourceId::Init(rep), b.as_slice()), now)?;
                    }
                }
                DashAction::FetchSegment {
                    rep,
                    number,
                    url,
                    tolerate_404,
                    ..
                } => {
                    if let FetchOutcome::Bytes(b) = tokio::time::timeout(
                        route.timeouts.read,
                        fetch_one(&http, &url, credentials.as_ref(), "segment", tolerate_404),
                    )
                    .await
                    .map_err(|_| MultimuxError::Connect {
                        reason: "segment fetch timed out".into(),
                    })??
                    {
                        session.feed((DashResourceId::Segment(rep, number), b.as_slice()), now)?;
                    }
                }
            }
        }
        Ok((specs, per_track))
    }

    /// Biting loopback test: a real axum server serves the real, committed
    /// `fixtures/dash/` MPD + fMP4 segments; asserts driving the session to
    /// completion recovers exactly the right `TrackSpec`s (one AVC + one AAC,
    /// unique remapped ids despite both init segments locally colliding on
    /// `tkhd@track_ID=1` — issue #738 gap 2 regression) and every real sample
    /// for each track, matching an independent oracle's per-track counts.
    ///
    /// MUTATION-CHECKED: dropping the `Fmp4Demux::unpackage` feed in
    /// `on_segment`, or never advancing `RepState::plan` in
    /// `pump_segment_fetches`, makes every count stay `0`/loop forever;
    /// disabling the local->global remap (or its translate-back in
    /// `on_segment`) collapses both tracks onto the same id, changing
    /// `per_track.len()` from 2 to 1 — this test's assertions catch either.
    #[tokio::test]
    async fn loopback_dash_pull_yields_real_avc_and_aac_samples() {
        let (url, server) = start_fixture_server(None, None).await;
        let route = DashPullRoute::new("dash-cam", url);

        let want_video = oracle_sample_count(
            "init-stream0.m4s",
            &[
                "chunk-stream0-00001.m4s",
                "chunk-stream0-00002.m4s",
                "chunk-stream0-00003.m4s",
            ],
        );
        let want_audio = oracle_sample_count(
            "init-stream1.m4s",
            &[
                "chunk-stream1-00001.m4s",
                "chunk-stream1-00002.m4s",
                "chunk-stream1-00003.m4s",
                "chunk-stream1-00004.m4s",
            ],
        );
        assert!(
            want_video > 0 && want_audio > 0,
            "sanity: fixture must carry real samples for both streams"
        );

        let (specs, per_track) = tokio::time::timeout(Duration::from_secs(15), drive_and_collect(&route))
            .await
            .expect("drive_and_collect timed out")
            .expect("drive_and_collect");

        assert_eq!(specs.len(), 2, "one video + one audio track: {specs:?}");
        let mut ids: Vec<u32> = specs.iter().map(|s| s.track_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 2, "track ids must be remapped to unique values");

        let video_id = specs
            .iter()
            .find(|s| matches!(s.config, CodecConfig::Avc { .. }))
            .expect("an AVC track")
            .track_id;
        let audio_id = specs
            .iter()
            .find(|s| matches!(s.config, CodecConfig::Aac { .. }))
            .expect("an AAC track")
            .track_id;
        assert_eq!(per_track.len(), 2, "samples must land on exactly 2 distinct track ids: {per_track:?}");
        assert_eq!(
            per_track.get(&video_id).copied().unwrap_or(0),
            want_video,
            "video sample count must match the independent oracle exactly"
        );
        assert_eq!(
            per_track.get(&audio_id).copied().unwrap_or(0),
            want_audio,
            "audio sample count must match the independent oracle exactly"
        );
        server.abort();
    }

    /// The full `run_dash_pull` drive loop against the same fixture: a static
    /// MPD must end cleanly once every Representation's plan is exhausted.
    #[tokio::test]
    async fn run_dash_pull_completes_cleanly_on_a_static_mpd() {
        let (url, server) = start_fixture_server(None, None).await;
        let route = DashPullRoute::new("dash-cam", url);
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            run_dash_pull(&route, trunk_config(), handshake()),
        )
        .await
        .expect("run_dash_pull must not hang against a static MPD");
        assert!(result.is_ok(), "a static MPD must end cleanly: {result:?}");
        server.abort();
    }

    /// A missing MPD must fail the run, not hang or silently proceed.
    #[tokio::test]
    async fn missing_mpd_fails_run_dash_pull() {
        let (base_url, server) = start_fixture_server(None, None).await;
        let bad_url = base_url.replace("manifest.mpd", "does-not-exist.mpd");
        let route = DashPullRoute::new("dash-missing", bad_url);
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_dash_pull(&route, trunk_config(), handshake()),
        )
        .await
        .expect("must not hang");
        assert!(result.is_err(), "a 404 MPD must fail run_dash_pull");
        server.abort();
    }

    /// Biting test (issue #663 P5 / #738-#739 ingest-hardening lesson): a
    /// server that resolves the MPD + inits but then stalls on a segment
    /// fetch must fail within `IngestTimeouts::read`, not hang forever.
    #[tokio::test]
    async fn read_times_out_against_a_server_that_stalls_on_a_segment() {
        let (url, server) =
            start_fixture_server(None, Some("chunk-stream0-00001.m4s")).await;
        let route = DashPullRoute::new("dash-stalled", url).with_timeouts(IngestTimeouts {
            connect: Duration::from_secs(5),
            read: Duration::from_millis(150),
        });
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_dash_pull(&route, trunk_config(), handshake()),
        )
        .await
        .expect(
            "run_dash_pull must return on its own via IngestTimeouts::read, \
             not hang until this test's own backstop timeout",
        );
        assert!(
            result.is_err(),
            "a server that stalls on a segment fetch must fail run_dash_pull, not hang forever"
        );
        server.abort();
    }

    const AUTH_USER: &str = "dash-user";
    const AUTH_PASS: &str = "dash-pass";

    /// Basic (RFC 7617) auth, credentials from URL userinfo: the server
    /// issues a Basic challenge for every request (MPD, inits, segments),
    /// and real samples come out via `run_dash_pull`.
    #[tokio::test]
    async fn basic_auth_from_url_userinfo_authenticates_and_pulls_samples() {
        let (url, server) = start_fixture_server(
            Some(crate::testutil::MockAuthScheme::Basic {
                username: AUTH_USER.into(),
                password: AUTH_PASS.into(),
            }),
            None,
        )
        .await;
        let credentialed = url.replacen("http://", &format!("http://{AUTH_USER}:{AUTH_PASS}@"), 1);
        let route = DashPullRoute::new("dash-basic", credentialed);
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            run_dash_pull(&route, trunk_config(), handshake()),
        )
        .await
        .expect("must not hang");
        assert!(
            result.is_ok(),
            "Basic auth from URL userinfo must authenticate: {result:?}"
        );
        server.abort();
    }

}
