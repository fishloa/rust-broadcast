//! MPEG-DASH pull ingest source (issue #758): fetches a remote MPD (ISO/IEC
//! 23009-1) via `transmux::dash_parse::Mpd`, resolves each selected
//! Representation's initialization + media segment URLs
//! ([`SegmentTemplate::resolve`]/[`SegmentTimeline::enumerate`]), and demuxes
//! the fetched fMP4/CMAF bytes via [`Fmp4Demux`] — mirrors
//! [`crate::source::ts_http`]'s own `reqwest` + `http_auth` fetch pattern
//! (there is no reusable DASH client the way [`crate::source::hls_pull`]
//! reuses `ll_hls_runtime::client::tokio_client::TokioClient`).
//!
//! # fMP4 demux without a `moov` per segment
//!
//! A DASH `SegmentTemplate` addresses a *separate* initialization resource
//! (`ftyp`+`moov`, fetched once per Representation) and media segments that
//! carry only `moof`+`mdat` — [`Fmp4Demux::unpackage`] needs a `moov` in the
//! same buffer it walks, so each media segment's bytes are demuxed
//! concatenated onto that Representation's cached init bytes, exactly the
//! pattern `ll_hls_runtime::client::engine`'s `demux_and_emit` uses for
//! CMAF parts/segments (see its module docs).
//!
//! # Track id remapping
//!
//! Each Representation's init segment is an independent CMAF file, so its
//! `moov` typically assigns the same local track_id (`1`) regardless of
//! which Representation it is — real init segments in `fixtures/dash/` do
//! exactly this (both `init-stream0.m4s`/`init-stream1.m4s` carry
//! `tkhd@track_ID=1`). [`DashPullSource::connect`] therefore remaps every
//! Representation's local track_id to a session-wide unique id at connect
//! time (recorded in a per-Representation local-to-global track id map),
//! and every subsequent segment demux translates back through that map
//! before a sample is surfaced.
//!
//! # v1 scope
//!
//! `SegmentTemplate`+`SegmentTimeline` (`$Time$` addressing) and
//! `SegmentTemplate` with a constant `@duration` (`$Number$` addressing,
//! segment count derived from the Period's/MPD's declared duration) are
//! supported, for static and dynamic (live) MPDs, one Representation per
//! `AdaptationSet` (the first — `Period`/`AdaptationSet`/`Representation`
//! selection by bitrate/language is out of scope). `SegmentList`/
//! `SegmentBase` addressing is deferred (matches `transmux::dash_parse`'s
//! own v1 scope). Multi-`Period` MPDs use only the first `Period`.
//!
//! # Dynamic (live) MPDs
//!
//! Once every Representation's initial segment plan is exhausted,
//! [`DashPullSession::next_samples`] re-fetches and re-parses the MPD (no
//! more often than `MPD@minimumUpdatePeriod`) and extends each
//! Representation's plan with any newly-revealed segment numbers. A `404`
//! for a not-yet-available live-edge segment is tolerated (the segment is
//! retried on the next call) rather than treated as a hard error; for a
//! static MPD the same `404` is a hard error, since every segment a static
//! MPD names is expected to already exist.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use broadcast_auth::Credentials;
use broadcast_common::Unpackage;
use reqwest::{Client, StatusCode};
use transmux::dash_parse::{Mpd, MpdType, SegmentTemplate, SegmentTimeline};
use transmux::media::Fmp4Demux;
use transmux::pipeline::{Sample, TrackSpec};
use url::Url;

use crate::error::{MultimuxError, Result};
use crate::source::IngestTimeouts;
use crate::source::Source;
use crate::source::http_auth::{
    authenticated_get, credentials_from_url, resolve_credentials, strip_userinfo,
};

/// A remote MPEG-DASH presentation to pull: its MPD URL, which may carry
/// `user:pass@` userinfo (see [`Debug`]'s redaction and
/// `crate::config::InputSpec::validate`).
#[derive(Clone)]
pub struct DashPullSource {
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
impl std::fmt::Debug for DashPullSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashPullSource")
            .field("name", &self.name)
            .field("url", &crate::redact::redact_url(&self.url))
            .field("auth", &self.auth.as_ref().map(|_| "***"))
            .finish()
    }
}

impl DashPullSource {
    /// Build a source descriptor. `url` is the MPD URL to pull.
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        DashPullSource {
            name: name.into(),
            url: url.into(),
            timeouts: IngestTimeouts::default(),
            auth: None,
        }
    }

    /// Overrides the default [`IngestTimeouts`] — see `RtspSource::with_timeouts`
    /// for the pattern this mirrors.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Attaches config-supplied credentials, overriding any URL userinfo at
    /// [`Self::connect`] time — see
    /// `crate::source::http_auth::resolve_credentials`.
    #[must_use]
    pub fn with_auth(mut self, auth: Option<Credentials>) -> Self {
        self.auth = auth;
        self
    }

    /// Fetches the MPD, resolves the first Period's Representations (the
    /// first Representation of each `AdaptationSet`), fetches + demuxes each
    /// one's initialization segment for its `TrackSpec`s, and builds the
    /// initial per-Representation segment plan — all bounded by
    /// [`IngestTimeouts::connect`].
    pub async fn connect(&self) -> Result<DashPullSession> {
        let parsed = Url::parse(&self.url).map_err(|e| MultimuxError::Connect {
            reason: format!(
                "bad DASH-pull URL {}: {e}",
                crate::redact::redact_url(&self.url)
            ),
        })?;
        let credentials = resolve_credentials(self.auth.clone(), credentials_from_url(&parsed)?);
        let clean_mpd_url = strip_userinfo(&parsed)?;

        let client = Client::builder()
            .build()
            .map_err(|e| MultimuxError::Connect {
                reason: format!("reqwest client: {e}"),
            })?;

        let connect_timeout = self.timeouts.connect;
        let outcome = tokio::time::timeout(
            connect_timeout,
            do_connect(&client, credentials.clone(), &clean_mpd_url),
        )
        .await;
        let (mpd_type, minimum_update_period, reps, specs) = match outcome {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(MultimuxError::Connect {
                    reason: format!("dash-pull: connect exceeded {connect_timeout:?}"),
                });
            }
        };

        Ok(DashPullSession {
            client,
            auth: credentials,
            mpd_url: clean_mpd_url,
            is_dynamic: matches!(mpd_type, MpdType::Dynamic),
            minimum_update_period,
            last_mpd_fetch: Instant::now(),
            reps,
            specs,
            read_timeout: self.timeouts.read,
        })
    }
}

impl Source for DashPullSource {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// One selected Representation's live state: its addressing inputs (for
/// dynamic-MPD plan extension), its cached init bytes, its local->global
/// track id map (see the module doc's "Track id remapping"), and its
/// pending `(number, time)` segment plan.
struct RepState {
    rep_id: String,
    media_template: Option<String>,
    bandwidth: u64,
    /// Cached initialization segment bytes — concatenated with each media
    /// segment's bytes before [`Fmp4Demux::unpackage`] (see the module
    /// doc's "fMP4 demux without a `moov` per segment").
    init_bytes: Vec<u8>,
    /// Maps this Representation's init-segment-local track_id to the
    /// session-wide unique id assigned at connect time.
    local_to_global: HashMap<u32, u32>,
    /// Pending `(number, time)` pairs not yet fetched, in presentation
    /// order. `time` is `None` under plain `$Number$`+`@duration`
    /// addressing (no `SegmentTimeline`).
    plan: VecDeque<(u64, Option<u64>)>,
    /// Highest segment number ever appended to [`Self::plan`] — the
    /// low-water mark a dynamic-MPD refresh extends beyond.
    last_number: u64,
}

/// Performs the MPD fetch + parse + per-Representation init fetch/demux
/// that [`DashPullSource::connect`] wraps in its connect timeout. Returns
/// the MPD's type/`minimumUpdatePeriod` (needed by the session for dynamic
/// refresh), the resolved [`RepState`]s, and every recovered [`TrackSpec`]
/// (track ids already remapped to session-wide unique ids).
async fn do_connect(
    client: &Client,
    credentials: Option<Credentials>,
    mpd_url: &Url,
) -> Result<(MpdType, Option<Duration>, Vec<RepState>, Vec<TrackSpec>)> {
    let mpd_text = fetch_text(client, mpd_url.as_str(), credentials.as_ref(), "mpd").await?;
    let mpd = Mpd::parse(&mpd_text).map_err(|e| MultimuxError::Connect {
        reason: format!("dash-pull: mpd parse: {e}"),
    })?;
    let period = mpd.periods.first().ok_or_else(|| MultimuxError::Connect {
        reason: "dash-pull: mpd has no Period".into(),
    })?;
    let total_duration = period.duration.or(mpd.media_presentation_duration);

    let mut reps = Vec::new();
    let mut specs = Vec::new();
    let mut next_track_id: u32 = 1;

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
        let init_url = mpd_url
            .join(&init_rel)
            .map_err(|e| MultimuxError::Connect {
                reason: format!("dash-pull: bad initialization URL {init_rel:?}: {e}"),
            })?;
        let init_bytes =
            fetch_bytes(client, init_url.as_str(), credentials.as_ref(), "init").await?;

        let media = Fmp4Demux::new().unpackage(init_bytes.as_slice())?;
        if media.tracks.is_empty() {
            return Err(MultimuxError::Connect {
                reason: format!(
                    "dash-pull: representation {:?} init segment carries no track",
                    rep.id
                ),
            });
        }
        let mut local_to_global = HashMap::new();
        for track in &media.tracks {
            let global_id = next_track_id;
            next_track_id += 1;
            local_to_global.insert(track.spec.track_id, global_id);
            let mut spec = track.spec.clone();
            spec.track_id = global_id;
            specs.push(spec);
        }

        let plan = build_plan(st, total_duration)?;
        let last_number = plan
            .last()
            .map(|(n, _)| *n)
            .unwrap_or_else(|| st.start_number.saturating_sub(1));

        reps.push(RepState {
            rep_id: rep.id.clone(),
            media_template: st.media.clone(),
            bandwidth: rep.bandwidth,
            init_bytes,
            local_to_global,
            plan: plan.into(),
            last_number,
        });
    }

    if specs.is_empty() {
        return Err(MultimuxError::Connect {
            reason: "dash-pull: mpd resolved no usable representation".into(),
        });
    }

    Ok((mpd.mpd_type, mpd.minimum_update_period, reps, specs))
}

/// Adapter around [`SegmentTimeline::enumerate`] — isolated to one call site
/// so a future signature change there (e.g. returning a `Result` once an
/// unbounded `@r` repeat count is capped) is a one-line change here rather
/// than scattered through [`build_plan`]/[`do_connect`].
fn enumerate_timeline(timeline: &SegmentTimeline, start_number: u64) -> Result<Vec<(u64, u64)>> {
    timeline
        .enumerate(start_number)
        .map_err(|e| MultimuxError::Connect {
            reason: format!("dash-pull: segment timeline: {e}"),
        })
}

/// Builds a Representation's initial ordered `(number, time)` segment plan
/// from its effective [`SegmentTemplate`] — via [`SegmentTimeline::enumerate`]
/// (`$Time$` addressing) if a timeline is present, else via
/// [`SegmentTemplate::number_sequence`] (`$Number$` addressing) sized from
/// `total_duration` (the Period's or MPD's declared duration) — `None`/no
/// `@duration` yields an empty plan (a dynamic MPD with neither is not yet
/// supported: nothing to fetch until a future MPD refresh reveals a
/// timeline).
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

/// Performs `GET url`, returning an error whose `reason` names `what` (e.g.
/// `"mpd"`/`"init"`) on any non-2xx status.
async fn fetch_bytes(
    client: &Client,
    url: &str,
    creds: Option<&Credentials>,
    what: &str,
) -> Result<Vec<u8>> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(status_error(what, status));
    }
    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| MultimuxError::Connect {
            reason: format!("dash-pull {what} read: {e}"),
        })
}

/// Text-body counterpart of [`fetch_bytes`], for the MPD itself (XML text,
/// not opaque segment bytes).
async fn fetch_text(
    client: &Client,
    url: &str,
    creds: Option<&Credentials>,
    what: &str,
) -> Result<String> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(status_error(what, status));
    }
    response.text().await.map_err(|e| MultimuxError::Connect {
        reason: format!("dash-pull {what} read: {e}"),
    })
}

/// Fetches one media segment. When `tolerate_404` (dynamic/live MPDs only —
/// see the module doc's "Dynamic (live) MPDs"), a `404` is reported as
/// `Ok(None)` ("not yet available, retry later") rather than an error; any
/// other non-2xx status is always a hard error.
async fn fetch_segment(
    client: &Client,
    url: &str,
    creds: Option<&Credentials>,
    tolerate_404: bool,
) -> Result<Option<Vec<u8>>> {
    let response = authenticated_get(client, url, creds).await?;
    let status = response.status();
    if tolerate_404 && status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(status_error("segment", status));
    }
    response
        .bytes()
        .await
        .map(|b| Some(b.to_vec()))
        .map_err(|e| MultimuxError::Connect {
            reason: format!("dash-pull segment read: {e}"),
        })
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

/// A live DASH-pull session: the fetched-and-parsed MPD's per-Representation
/// state, plus the connect-recovered [`TrackSpec`]s.
pub struct DashPullSession {
    client: Client,
    auth: Option<Credentials>,
    /// The (userinfo-stripped) MPD URL — also the base every relative
    /// `SegmentTemplate` URL resolves against, and what a dynamic-MPD
    /// refresh re-fetches.
    mpd_url: Url,
    is_dynamic: bool,
    minimum_update_period: Option<Duration>,
    last_mpd_fetch: Instant,
    reps: Vec<RepState>,
    specs: Vec<TrackSpec>,
    /// Bound on each network step in [`Self::next_samples`] — see
    /// [`IngestTimeouts::read`].
    read_timeout: Duration,
}

/// Default wait between exhausting a dynamic MPD's plan and its next refresh
/// attempt when `MPD@minimumUpdatePeriod` is absent.
const DEFAULT_MPD_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

impl DashPullSession {
    /// The `TrackSpec`s recovered during [`DashPullSource::connect`], track
    /// ids already remapped to session-wide unique values.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.specs.clone()
    }

    /// Fetches the next pending media segment for every Representation that
    /// still has one queued, demuxes each (concatenated onto that
    /// Representation's cached init bytes — see the module doc), and
    /// returns every recovered sample, track ids remapped to this session's
    /// global ids.
    ///
    /// When every Representation's plan is empty: a dynamic MPD triggers a
    /// (rate-limited) MPD refresh and returns an empty batch (not
    /// end-of-stream) so the caller keeps polling; a static MPD returns
    /// `Ok(None)` (true end-of-stream).
    ///
    /// Bounded by [`IngestTimeouts::read`] (issue #663 P5, audit-ingest #3
    /// / the #738/#739 ingest-hardening lesson): a stalled/unreachable
    /// server must not wedge the route — a timed-out fetch surfaces as an
    /// `Err`, reconnected by [`crate::origin::supervisor::supervise`] like
    /// any other read error.
    pub async fn next_samples(&mut self) -> Result<Option<Vec<(u32, Sample)>>> {
        let read_timeout = self.read_timeout;
        let mut out = Vec::new();
        let mut any_pending = false;

        for i in 0..self.reps.len() {
            let Some((number, time)) = self.reps[i].plan.pop_front() else {
                continue;
            };
            any_pending = true;

            let rep_id = self.reps[i].rep_id.clone();
            let bandwidth = self.reps[i].bandwidth;
            let template = self.reps[i].media_template.clone().unwrap_or_default();
            let rel =
                SegmentTemplate::resolve(&template, &rep_id, Some(number), time, Some(bandwidth));
            let seg_url = self
                .mpd_url
                .join(&rel)
                .map_err(|e| MultimuxError::Connect {
                    reason: format!("dash-pull: bad media segment URL {rel:?}: {e}"),
                })?;

            let fetch = fetch_segment(
                &self.client,
                seg_url.as_str(),
                self.auth.as_ref(),
                self.is_dynamic,
            );
            let fetched = match tokio::time::timeout(read_timeout, fetch).await {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(MultimuxError::Connect {
                        reason: format!(
                            "dash-pull: segment read (rep {rep_id:?}) timed out after {read_timeout:?}"
                        ),
                    });
                }
            };

            let Some(bytes) = fetched else {
                // Live-edge segment not yet available: retry next round.
                self.reps[i].plan.push_front((number, time));
                continue;
            };

            let rep = &self.reps[i];
            let mut combined = Vec::with_capacity(rep.init_bytes.len() + bytes.len());
            combined.extend_from_slice(&rep.init_bytes);
            combined.extend_from_slice(&bytes);
            let media = Fmp4Demux::new().unpackage(combined.as_slice())?;
            for track in media.tracks {
                if let Some(&global_id) = rep.local_to_global.get(&track.spec.track_id) {
                    for sample in track.samples {
                        out.push((global_id, sample));
                    }
                }
            }
        }

        if !any_pending {
            if self.is_dynamic {
                self.maybe_refresh_mpd().await?;
                return Ok(Some(Vec::new()));
            }
            return Ok(None);
        }

        Ok(Some(out))
    }

    /// Re-fetches and re-parses the MPD (no more often than
    /// `MPD@minimumUpdatePeriod`, or [`DEFAULT_MPD_REFRESH_INTERVAL`] absent
    /// one — sleeping out the remainder, capped by the read timeout, when
    /// called too soon) and extends every still-matching Representation's
    /// plan with any segment numbers beyond its low-water mark. A
    /// Representation no longer present in the refreshed MPD is left as-is
    /// (its plan simply stays empty).
    async fn maybe_refresh_mpd(&mut self) -> Result<()> {
        let min_update = self
            .minimum_update_period
            .unwrap_or(DEFAULT_MPD_REFRESH_INTERVAL);
        let elapsed = self.last_mpd_fetch.elapsed();
        if elapsed < min_update {
            let remaining = (min_update - elapsed).min(self.read_timeout);
            tokio::time::sleep(remaining).await;
            return Ok(());
        }

        let mpd_text = fetch_text(
            &self.client,
            self.mpd_url.as_str(),
            self.auth.as_ref(),
            "mpd refresh",
        )
        .await?;
        let mpd = Mpd::parse(&mpd_text).map_err(|e| MultimuxError::Connect {
            reason: format!("dash-pull: mpd refresh parse: {e}"),
        })?;
        self.last_mpd_fetch = Instant::now();
        self.is_dynamic = matches!(mpd.mpd_type, MpdType::Dynamic);
        self.minimum_update_period = mpd.minimum_update_period;

        let Some(period) = mpd.periods.first() else {
            return Ok(());
        };
        let total_duration = period.duration.or(mpd.media_presentation_duration);

        for rep in &mut self.reps {
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
        Ok(())
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
    use transmux::CodecConfig;

    /// Path to the real, committed DASH fixture (isoff-live profile, real
    /// ffmpeg-produced fMP4 — see `fixtures/dash/` at the workspace root):
    /// one video (AVC) + one audio (AAC) Representation, `SegmentTimeline`
    /// (`$Time$`) addressing, `type="static"`.
    fn fixture_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/dash"))
    }

    /// Starts a real axum server serving every file in `fixtures/dash/` by
    /// name under `/`, on an ephemeral loopback port. `auth`, if given,
    /// gates every request behind that scheme (see
    /// `crate::testutil::require_auth`); `stall_segment`, if given, makes
    /// that one filename hang forever instead of responding (the read-
    /// timeout biting test).
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
    /// each chunk concatenated, exactly as the source itself must) to know
    /// the real expected sample counts per stream, without going through
    /// `DashPullSource` at all.
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

    /// Biting loopback test: a real axum server serves the real, committed
    /// `fixtures/dash/` MPD + fMP4 segments; asserts `DashPullSource`
    /// resolves both the AVC and AAC `TrackSpec`s and, driven to
    /// end-of-stream, yields exactly the independently-demuxed oracle's
    /// sample counts — proving real samples for both tracks actually land,
    /// not just that `connect()` succeeds.
    #[tokio::test]
    async fn loopback_dash_pull_yields_real_avc_and_aac_samples() {
        let (url, server) = start_fixture_server(None, None).await;

        let source = DashPullSource::new("dash-cam", url);
        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect");

        let specs = session.track_specs();
        assert_eq!(specs.len(), 2, "one video + one audio track");
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Avc { .. })),
            "must recover the fixture's AVC video track: {specs:?}"
        );
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Aac { .. })),
            "must recover the fixture's AAC audio track: {specs:?}"
        );
        // Track ids must be unique despite both init segments locally
        // carrying tkhd@track_ID=1 (see the module doc's "Track id
        // remapping").
        let mut ids: Vec<u32> = specs.iter().map(|s| s.track_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 2, "track ids must be remapped to unique values");

        let want_total = oracle_sample_count(
            "init-stream0.m4s",
            &[
                "chunk-stream0-00001.m4s",
                "chunk-stream0-00002.m4s",
                "chunk-stream0-00003.m4s",
            ],
        ) + oracle_sample_count(
            "init-stream1.m4s",
            &[
                "chunk-stream1-00001.m4s",
                "chunk-stream1-00002.m4s",
                "chunk-stream1-00003.m4s",
                "chunk-stream1-00004.m4s",
            ],
        );
        assert!(want_total > 0, "sanity: fixture must carry real samples");

        let mut got_total = 0usize;
        while let Some(batch) = tokio::time::timeout(Duration::from_secs(5), session.next_samples())
            .await
            .expect("next_samples timed out")
            .expect("next_samples must not error")
        {
            if batch.is_empty() && got_total >= want_total {
                break;
            }
            got_total += batch.len();
        }
        assert_eq!(
            got_total, want_total,
            "must pull every real sample from both DASH representations, no gaps/duplicates"
        );

        server.abort();
    }

    /// Mutation-check counterpart (documented, not a separate `#[test]`):
    /// dropping the `Fmp4Demux::unpackage` feed in `next_samples`, or never
    /// advancing `RepState::plan`, makes `got_total` stay `0`/loop forever —
    /// this test's non-zero, exact-match assertion is what catches that.
    #[tokio::test]
    async fn connect_fails_on_missing_mpd() {
        let (base_url, server) = start_fixture_server(None, None).await;
        let bad_url = base_url.replace("manifest.mpd", "does-not-exist.mpd");

        let source = DashPullSource::new("dash-missing", bad_url);
        let result = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out");
        assert!(result.is_err(), "a 404 MPD must fail connect()");

        server.abort();
    }

    /// Biting test (issue #663 P5 / #738-#739 ingest-hardening lesson): a
    /// server that resolves the MPD + inits but then stalls on a segment
    /// fetch must fail `next_samples()` within [`IngestTimeouts::read`], not
    /// hang forever.
    #[tokio::test]
    async fn read_times_out_against_a_server_that_stalls_on_a_segment() {
        let (url, server) = start_fixture_server(None, Some("chunk-stream0-00001.m4s")).await;

        let source = DashPullSource::new("dash-stalled", url).with_timeouts(IngestTimeouts {
            connect: Duration::from_secs(5),
            read: Duration::from_millis(150),
        });
        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("connect must succeed (only the segment fetch stalls)");

        let result = tokio::time::timeout(Duration::from_secs(5), session.next_samples())
            .await
            .expect(
                "next_samples() must return on its own via IngestTimeouts::read, \
                 not hang until this test's own backstop timeout",
            );
        assert!(
            result.is_err(),
            "a server that stalls on a segment fetch must fail next_samples(), not hang forever"
        );

        server.abort();
    }

    const AUTH_USER: &str = "dash-user";
    const AUTH_PASS: &str = "dash-pass";

    /// Basic (RFC 7617) auth, credentials from URL userinfo: the server
    /// issues a Basic challenge for every request (MPD, inits, segments),
    /// `DashPullSource` answers via `source::http_auth::authenticated_get`,
    /// and real samples come out.
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

        let source = DashPullSource::new("dash-basic", credentialed);
        let mut session = tokio::time::timeout(Duration::from_secs(5), source.connect())
            .await
            .expect("connect timed out")
            .expect("Basic auth from URL userinfo must authenticate");

        let mut total = 0usize;
        while let Ok(Ok(Some(batch))) =
            tokio::time::timeout(Duration::from_millis(500), session.next_samples()).await
        {
            if batch.is_empty() {
                break;
            }
            total += batch.len();
        }
        assert!(total > 0, "expected real samples after Basic auth");

        server.abort();
    }
}
