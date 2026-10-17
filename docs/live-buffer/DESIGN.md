# live-buffer: design

Status: PROPOSAL for owner decision. Docs and analysis only; no crate code exists and nothing is added to the workspace, crate index or release lanes by this PR. Requested by the zenith-tv project (tvOS live-TV client over HTTP-TS). Evidence: [`TRACE-ANALYSIS.md`](TRACE-ANALYSIS.md) (numbers quoted below as TA-x come from its tables; scripts in [`analysis/`](analysis/)).

Tags on every default: `[tuned]` = chosen from a number computed on the real traces; `[sourced]` = taken from a cited, permissively licensed implementation or RFC (not validated here); `[guess]` = engineering judgement, to be validated by the synthetic suite and the extra channels (section 8). The traces contain ONE stall (tf-1); anything derived from it is n=1.

## 1. Purpose and non-goals

`live-buffer` decides **when a live-TS player should pause, wait, and resume**, and how deep the buffer must be to survive the delivery stalls of a real network. It turns "chunk arrived at time T carrying media time M" and "the playback clock reads P" into: a target buffer depth, a state (Buffering / Playing / Starved), and events telling the caller exactly when to start, pause and resume and at which media position.

What the research and the measurements pin down:

- The caller cannot time-scale playback (passthrough AC-3/E-AC-3 has no rate control), so the **only** way to rebuild a buffer is to pause and wait; the only way to survive a stall is to have enough buffer already (TA finding 3: a live-paced sender never catches up, a stall removes buffer permanently).
- On tvOS nothing owns underrun (zenith-tv measurement, tvOS 27.0, 1.5 s delivery gap, own re-buffer disabled): the synchronizer clock **keeps running at rate 1.0 through the gap** with 0 s buffered; when data resumes its PTS is already behind the clock, buffered stays 0 and the picture never recovers; no auto-pause, no auto-resume, no notification. `hasSufficientMediaDataForReliablePlaybackStart` is useless as a health signal (display layer toggles 0/1 while playing; the AC-3 passthrough renderer is 0 for the whole run). **The controller therefore owns pause, hold, refill and resume completely** (section 2.4); the wrapper is a dumb actuator.

Non-goals (hard): no I/O, no clock reads, no threads, no timers (the caller passes `now`; same sans-IO style as `srt-runtime`'s TSBPD, which also takes `now` from the caller); no demuxing, no transmux dependency (inputs are plain integers); no playback-rate control; no ABR/variant switching; no f32/f64 anywhere in the public API or the core (bit-exact determinism, C-ABI mirroring); no panics on any input. `#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`, MIT OR Apache-2.0, MSRV 1.95, depends only on `broadcast-common` (for `clock33`). `Parse`/`Serialize` do not apply (no wire format).

## 2. Public API

Time units, fixed and documented once, **caller obligations**:

| quantity | type | unit | requirement |
|---|---|---|---|
| arrival clock `now_us`, `arrival_us` | `u64` | microseconds, monotonic, any origin | same clock for `on_chunk` and `on_tick`; never required to be monotonic for memory safety, but a regression is counted (`Stats::clock_regressions`) and clamped to the last value |
| media time `media_27mhz` | `u64` | 27 MHz ticks in the PCR domain, `0..PCR_WRAP_27MHZ` | raw (wrapped) PCR; the crate unrolls it with `broadcast_common::clock33::pcr_signed_distance`. A 90 kHz PTS caller passes `pts * 300` (33-bit PTS wrap x 300 equals `PCR_WRAP_27MHZ` exactly). Never use ad-hoc `abs_diff`/`1<<33` (the `clock33_guard` tripwire applies) |
| playback position `playback_27mhz` | `u64` | same domain and offset as `media_27mhz` | the caller maps its renderer clock into the PCR domain once (offset between first PTS and first PCR is a constant it already knows) |

`type Micros = u64; type Ticks27 = u64;` are public aliases; they are `u64` on the C ABI.

### 2.1 Configuration (all `#[non_exhaustive]`, validated constructors, no panics)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)] #[non_exhaustive]
pub struct JitterConfig {
    pub min_window_us: Micros,        // sliding-minimum window for D (2 s)
    pub interval_us: Micros,          // per-interval max resampling period (500 ms)
    pub bucket_us: u32,               // histogram bucket width (10_000)
    pub buckets: u16,                 // histogram buckets (400 => 0..4 s)
    pub forget_q15: u16,              // per-interval forgetting factor, Q15 (0.98 => 32113)
    pub quantile_permille: u16,       // baseline quantile (990)
    pub hold_us: Micros,              // stall-memory hold before decay (60 s)
    pub decay_us: Micros,             // linear decay duration after hold (60 s)
    pub max_samples: u16,             // hard cap of the sliding-min deque (4096)
    pub discontinuity_us: Micros,     // PCR jump threshold (1 s): media delta ahead of the arrival delta by more, or backwards by more
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)] #[non_exhaustive]
pub struct PolicyConfig {
    pub min_us: Micros,               // floor of every target (300 ms)
    pub initial_us: Micros,           // first start target (500 ms)
    pub max_us: Micros,               // ceiling of every target (3 s)
    pub k1_q8: u16,                   // baseline multiplier, Q8 (2.0 => 512)
    pub margin_us: Micros,            // additive margin (100 ms)
    pub k2_q8: u16,                   // stall-peak multiplier, Q8 (1.2 => 307)
    pub after_rebuffer_q8: u16,       // 1.5 => 384: resume target = max(initial, x * last_stall_depth)
    pub max_step_down_us_per_s: u32,  // target decrease slew (100_000)
    pub stability_us: Micros,         // stall-free playback after which a stall increment decays (60 s)
    pub reprime_floor_us: Micros,     // voluntary re-prime below this lead in Playing (0 = disabled)
    pub reprime_hold_us: Micros,      // how long below the floor before re-priming (2 s)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)] #[non_exhaustive]
pub struct ControllerConfig {
    pub jitter: JitterConfig,
    pub policy: PolicyConfig,
    pub starve_threshold_us: Micros,  // starved when lead <= this while the clock advances (20 ms)
    pub nodata_after_us: Micros,      // no chunk for this long => reason NoData (200 ms)
    pub start_hold_us: Micros,        // platform start latency the caller adds on top (0; informational, see 7)
}
impl JitterConfig  { pub const DEFAULT: Self; pub fn validate(&self) -> Result<(), Error>; }
impl PolicyConfig  { pub const DEFAULT: Self; pub fn validate(&self) -> Result<(), Error>; }
impl ControllerConfig { pub const DEFAULT: Self; pub fn new(jitter: JitterConfig, policy: PolicyConfig, starve_threshold_us: Micros, nodata_after_us: Micros) -> Result<Self, Error>; }
```

Validation rules (each returns a typed `Error`): `min <= initial <= max`; `max_us <= buckets*bucket_us`; `forget_q15 < 32768`; `0 < quantile_permille <= 1000`; `interval_us > 0`; `min_window_us >= 2*interval_us`; `starve_threshold_us < min_us`; `max_samples >= 8`; multipliers non-zero; the overflow bound of 3.5 holds.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)] #[non_exhaustive]
pub enum Error { InvalidConfig(ConfigField), ZeroLengthWindow, ... }   // thiserror-free (no_std): Display via impl_spec_display!
#[non_exhaustive] pub enum ConfigField { MinWindow, Interval, Bucket, Forget, Quantile, Order, Capacity, Threshold, Multiplier }
```

### 2.2 Delivery-jitter estimator (usable alone)

```rust
pub struct DeliveryJitter { /* O(window) deque + fixed histogram + 3 scalars */ }
impl DeliveryJitter {
    pub fn new(cfg: JitterConfig) -> Result<Self, Error>;
    /// One call per PCR-carrying chunk. Chunks without a PCR are not fed (they carry no media time).
    pub fn on_chunk(&mut self, arrival_us: Micros, media_27mhz: Ticks27, bytes: u32) -> SampleOutcome;
    pub fn estimate(&self) -> JitterEstimate;
    pub fn reset_baseline(&mut self);          // PCR discontinuity: drops min window + histogram, KEEPS stall memory
    pub fn snapshot(&self) -> StallMemory;     // for per-channel persistence (7)
    pub fn restore(&mut self, m: StallMemory);
}
#[non_exhaustive] pub struct JitterEstimate {
    pub pdv_last_us: Micros,        // latest D - Dmin
    pub baseline_us: Micros,        // histogram quantile (default q99) of interval maxima
    pub stall_peak_us: Micros,      // max-hold with linear decay
    pub rfc3550_jitter_us: Micros,  // RFC 3550 A.8 J, reporting only
    pub samples: u64,
}
#[non_exhaustive] pub enum SampleOutcome { Accepted, Discontinuity, NonMonotonicArrival, Duplicate }
#[derive(Clone, Copy, PartialEq, Eq, Default)] #[non_exhaustive]
pub struct StallMemory { pub peak_us: Micros, pub age_us: Micros }   // serialisable by the caller as two u64
```

### 2.3 Controller

```rust
pub struct BufferController { /* DeliveryJitter + state machine + event queue (bounded 16, oldest-dropped counted) */ }
impl BufferController {
    pub fn new(cfg: ControllerConfig, seed: Option<StallMemory>) -> Result<Self, Error>;
    pub fn on_chunk(&mut self, arrival_us: Micros, media_27mhz: Option<Ticks27>, bytes: u32);
    pub fn on_tick(&mut self, now_us: Micros, playback_27mhz: Option<Ticks27>);   // None while not rendering yet
    pub fn on_hint(&mut self, now_us: Micros, hint: Hint);
    pub fn poll_event(&mut self) -> Option<Event>;
    pub fn state(&self) -> State;
    pub fn target_us(&self) -> Micros;       // current desired lead
    pub fn stats(&self) -> Stats;
    pub fn reset(&mut self, keep_memory: bool);   // new channel (keep_memory = false) or reconnect (true)
}
#[derive(Clone, Copy, PartialEq, Eq)] #[non_exhaustive]
pub enum State { Buffering, Playing, Starved }                 // name() + impl_spec_display!
#[derive(Clone, Copy, PartialEq, Eq)] #[non_exhaustive]
pub enum Reason { Startup, NoData, DataArriving, Discontinuity, Reprime }   // why we are not playing
#[derive(Clone, Copy, PartialEq, Eq)] #[non_exhaustive]
pub enum Hint { InputDegraded, InputRecovered, SyncLoss, PcrDiscontinuity, Reconnected }
#[derive(Clone, Copy, PartialEq, Eq)] #[non_exhaustive]
pub enum Event {
    /// First fill complete. Caller: setRate(1, time: media_27mhz); media_27mhz = oldest buffered media position.
    StartPlaying { media_27mhz: Ticks27, lead_us: Micros },
    /// Underrun or re-prime. Caller MUST set the renderer rate to 0 and freeze the clock at hold_at_27mhz.
    Pause { hold_at_27mhz: Ticks27, reason: Reason, depth_us: Micros, resume_target_us: Micros },
    /// Refilled to resume_target. Caller: setRate(1, time: media_27mhz) with EXACTLY this position.
    Resume { media_27mhz: Ticks27, lead_us: Micros },
    TargetChanged { target_us: Micros },
    /// Estimated need exceeds max_us: target is clamped, the caller may want to tell the user (stall message).
    OverMax { need_us: Micros, max_us: Micros },
    Rebaselined,                                   // PCR discontinuity absorbed
    EventsDropped { count: u32 },
}
#[non_exhaustive] pub struct Stats {
    pub state: State, pub reason: Reason, pub lead_us: i64, pub target_us: Micros,
    pub stalls: u32, pub reprimes: u32, pub last_stall_depth_us: Micros, pub longest_freeze_us: Micros,
    pub est: JitterEstimate, pub clock_regressions: u32, pub discontinuities: u32,
    pub hints_seen: u32, pub events_dropped: u32,
}
```

`Event`/`State`/`Reason`/`Hint`/`SampleOutcome` carry `name() -> &'static str` and `impl_spec_display!` (label convention #204; `label_coverage.rs` guard). `Error` is a SKIP-list entry (error type) unless it also gets a label.

### 2.4 Decision contract (what the caller must do; this is the part zenith-tv's measurement made mandatory)

`lead_us = media_last_arrived - playback_27mhz` as a **signed** distance (`pcr_signed_distance`), so a clock that has run ahead of the media reads negative.

1. `StartPlaying{media_27mhz}`: set the renderer clock to `media_27mhz` and rate 1 (e.g. `setRate(1, time:)`). `media_27mhz` is the oldest media position still buffered, so no PTS is ever behind the clock at start.
2. `Pause{hold_at_27mhz}`: **immediately** set rate 0 and hold the clock at `hold_at_27mhz` (= `min(playback_27mhz, media_last_arrived)`: the last position for which media exists; never ahead of data even if the clock overran before the tick noticed). Keep demuxing and queueing every chunk that arrives **while paused**: it is ahead of the held clock and must NOT be dropped, flushed or rendered; this is the refill.
3. `Resume{media_27mhz}`: set the clock to exactly `media_27mhz` (= `hold_at_27mhz`, or the oldest queued media if the queue starts later, never earlier than queued data) and rate 1 in one call (`setRate(1, time: media_27mhz)`). It is not enough to set rate 1 without the time: late-arriving PTS would already be behind a free-running clock, which is the measured permanent-starvation failure.
4. While `State != Playing` the caller keeps calling `on_tick` (with the held position or `None`) at a steady period; recommended <= 50 ms (matches zenith-tv's poll), required <= `starve_threshold_us`-order so starvation is noticed within one tick. Detection latency is bounded by that period and shows up as `hold_at` being clamped to the data.
5. The controller never reads wall time; if the caller stops ticking, nothing changes.

State machine (inputs: `T` = tick with lead `L`, target `G`; `C` = chunk; `H` = hint). Hysteresis keyed on the previous state (Shaka's SATISFIED/STARVING pattern; dash.js's single `stallThreshold` flaps): the enter threshold is `starve_threshold_us` (20 ms), the exit threshold is the full resume target, so a lead hovering between them cannot oscillate.

| state \ input | T: `L <= th` (clock advancing) | T: `L >= exit target` | T: otherwise | C | H |
|---|---|---|---|---|---|
| Buffering (Startup) | stay | -> Playing, `StartPlaying` (target = initial or seed-raised) | stay | update jitter, retarget | `Reconnected`/`PcrDiscontinuity`: restart fill |
| Playing | -> Starved, `Pause` (reason NoData if now - last chunk >= `nodata_after_us` else DataArriving), stall memory updated | n/a | stay; if `lead < reprime_floor` for `reprime_hold` and floor != 0 -> Starved(reason Reprime), `Pause` | update jitter, retarget (slewed), `TargetChanged` if changed | `InputDegraded`: no state change (non-authoritative: only sets the Pause reason hint and raises `Stats.hints_seen`); `SyncLoss`/`PcrDiscontinuity`: -> Buffering(reason Discontinuity), `Pause`, `Rebaselined` |
| Starved | stay (clock frozen by caller) | -> Playing, `Resume{media_27mhz}`, target after the stall | stay | as above; if estimate > max: `OverMax` | `InputRecovered`: informational |

Startup suppression: the first fill (Buffering/Startup -> Playing) is never counted in `stalls`, never raises `after_rebuffer`, and never updates stall memory. A tick with `playback_27mhz == None` while Buffering is the normal case. A zero-lead start (clock not started) is not an underrun.

Hints are **non-authoritative**: `transmux`-side `InputDegraded`/`SyncLoss` change the reported reason and (for `SyncLoss`) trigger a re-baseline, but starvation is only ever declared from lead + clock, never from a hint alone (a hint can lag or be wrong; the lead measurement is the truth).

PCR discontinuity: a chunk whose media delta exceeds its arrival delta by more than `discontinuity_us`, or runs backwards by more than it (default 1 s, `[sourced]` GStreamer rtpjitterbuffer resync threshold, idea only; late data such as the tf-1 gap, where arrival advances 1433 ms but media 35 ms, is a gap and never a jump) or an explicit hint triggers `reset_baseline` (min window + histogram cleared, **stall memory kept**) and Buffering(Discontinuity). Clock drift between the PCR and arrival clocks (TA: 33 ppm = 2 ms/min, scatter +-50 ppm over 60 s) is absorbed by the 2 s windowed minimum (never a running minimum); no explicit drift estimator is shipped (non-goal), but `Stats` exposes the windowed minimum's slope for diagnostics only if the extra channels show > 100 ppm.

## 3. Algorithms

All arithmetic is integer: `u64`/`i64` microseconds and Q8/Q15 fixed point; widening to `u128` only inside one helper `mul_div_q`. No division by a runtime value that can be zero (validated at construction).

### 3.1 Sample definition (RFC 5481 s4.2, s7.1.2)

For each PCR-carrying chunk: `media_us = unroll(media_27mhz) / 27` (the unroll uses `clock33::pcr_signed_distance` against the previous media time, accumulated in `i128`; the crate keeps only `D` differences, so absolute wrap never matters), `D = arrival_us - media_us` (relative transit; the constant clock offset cancels), `Dmin = min(D over the trailing min_window_us)` (monotonic deque, amortised O(1), capped at `max_samples`: over-cap drops the oldest, shrinking the window, counted), `PDV = D - Dmin`. RFC 5481 s7.1.2 states PDV (D minus the minimum), not the mean-based J, is the right statistic for sizing a de-jitter buffer. A 2 s window `[tuned]`: TA table "PDV": with 2 s the tf-1 p90 is 5.3 ms, with 10 s it is 1356 ms (the gap stays in the window and masks the baseline); t18 PDV p99.9 is 15.3 ms (2 s) vs 17.5 (10 s) so a longer window buys nothing. Drift: 100 ppm x 2 s = 0.2 ms, negligible. Window source idea: GStreamer's 2 s skew window (LGPL: idea only, no code).

### 3.2 Burst-aware per-interval maximum (NetEq-style)

Samples are grouped into fixed `interval_us` (500 ms `[sourced]` libwebrtc NetEq's 500 ms; T=250 and 500 ms give identical decisions on both traces, TA estimator table) intervals; only the **maximum PDV per interval** is fed to the histogram. A burst of N chunks after a stall then counts once (not N times), so a catch-up burst cannot flood the histogram and a stall cannot be diluted by the fast samples around it. Intervals with no samples feed nothing (a stall interval is represented by the first post-stall sample's interval).

### 3.3 Two estimators and their combination

1. **Baseline** = quantile of a forgetting histogram of interval maxima: `buckets` x `bucket_us` (400 x 10 ms = 0..4 s, `[tuned]`: 10 ms resolves the 15-20 ms clean-link PDV band, 4 s > the 3 s ceiling), counts in Q15 weights. Per interval: `w[i] = w[i]*forget_q15 >> 15` for all i, then `w[bucket(max)] += 32768 - forget_q15`. A ramp `forget_eff = min(forget, k/(k+1))` (k = intervals seen) makes early intervals a plain average (NetEq's start-weight idea, simplified, `[guess]`). `forget = 0.98` `[sourced]` (NetEq 0.983) . Quantile = smallest bucket upper edge whose cumulative weight `>= q * total` (integer compare `cum*1000 >= total*q_permille`). **q = 0.99 `[tuned]`**: TA shows q95 holds a single 1.4 s event for ~1 interval only (new-interval weight 0.02 < 1-q = 0.05) while q99 holds it analytically 34 intervals (17 s at 500 ms) and 12.6+ s in the replay. Memory O(buckets); cost O(buckets) per interval (400 ops / 500 ms).
2. **Stall memory** = max-hold of the interval-max PDV with a hold then linear decay: `peak` is replaced when a larger interval max arrives; value = `peak` while age <= `hold_us` (60 s), then `peak * (1 - (age-hold)/decay)` down to 0 over `decay_us` (60 s). `[sourced]` Shaka dynamicTargetLatency stabilityThreshold 60 s; the trace cannot validate it (ends 12.6 s after the gap), so `[guess]` until synthetic + extra channels. Why a second estimator at all: TA finding 5: histogram quantile forgets in seconds while stalls recur on a minutes scale; EWMA+3 sigma (Media3 style) reads 98 ms after a 1.4 s stall; RFC 3550 J reads 84 ms (6% of the stall) and is back to 3 ms after 2 s.
3. **RFC 3550 A.8 J** (`J += (|D_i - D_{i-1}| - J) / 16`, integer truncating form) is computed and reported (`rfc3550_jitter_us`) as the interoperable statistic, never used for sizing.

### 3.4 Policy

```
base      = baseline_us                         (hist q99)
need      = max( k1*base + margin , k2*stall_peak )          // Q8 multiplies
raw       = clamp( min_us, need, max_us )
target    = follower(raw)                       // rise instantly; fall <= max_step_down per second
resume    = clamp( min_us, max( target, after_rebuffer*last_stall_depth ), max_us )   // x1.5, last_stall_depth = stall_peak at the starve instant
start     = max( initial_us, target_from_seed )  // first fill only
```

- **Asymmetric follower** (`[sourced]` libwebrtc VCMTiming: increase fast, decrease limited to 100 ms/s; Media3 live offset: rise instantly, decay by a bounded step): `target = max(raw, prev - step_down*dt)`.
- **Capped, decaying stall increment** (avoid the ratchet): the stall adds to the target only through `stall_peak`, which is capped by `max_us` and decays after `hold+decay` stall-free (Media3 +500 ms/rebuffer never decays; Shaka resets after a 60 s stability window; hls.js caps padding at one target duration `[sourced]`). zenith-tv's current "2 s for the rest of the connection after one stall" is the ratchet pitfall this removes.
- **Resume target** (`after_rebuffer`, `[guess]` 1.5): needs `> 1x last stall depth` to survive a repeat (TA: the 1.4 s gap needs 1430 ms; no catch-up happens, so the buffer after resume is exactly the resume target) and `< 2x` (RFC 6298 doubles the RTO on a repeated timeout, so a second consecutive gap is ~2x: a resume at 2.1 s survives 1.5x repeats, not a 2x one; `max_us` 3 s covers 2.1x of 1.4 s).
- **Over-max report**: if `need > max_us` the target is clamped and `Event::OverMax` is raised once per excursion (hysteresis: re-armed when `need <= max_us*3/4`).
- **No buffer-top-up by speed-up**: when the lead sits below the target in Playing there is no corrective action except the optional voluntary re-prime (`reprime_floor_us`, `[guess]` default OFF: TA finding 3 shows a surviving buffer is permanently depleted; a deliberate freeze to rebuild it is a UX decision for the owner, section 8).

### 3.5 Memory, complexity, overflow

- State: deque of <= `max_samples` (`(u64,i64)`, 16 B each, 64 KiB worst case at 4096; typical 2 s window at one PCR per ~35 ms = ~60 entries), histogram `buckets` x `u32`, ~20 scalars, event queue 16 x ~32 B. `alloc` is used once at construction; no allocation after `new` (deque pre-sized, hence O(1) and bounded).
- Per `on_chunk`: amortised O(1) (monotonic deque push/pop); per interval close: O(buckets); per `on_tick`: O(1). No loops over unbounded input.
- Overflow: `arrival_us` up to 2^64 us (584 000 years); `D` is `i64`/`i128` difference (values > 2^63 us are clamped via saturating ops and counted); media unrolling accumulates in `i128`; Q8/Q15 products are `u64` x `u16` (<2^80 fits `u128` helper); histogram weights <= 32768 * 2 (geometric sum) per bucket in `u32`. The `max_us <= buckets*bucket_us` rule bounds every index. Zero-length windows/empty histograms return 0 and never divide.

## 4. Default parameter table

| parameter | default | tag | number / source |
|---|---|---|---|
| `min_window_us` | 2 s | `[tuned]` | TA: tf-1 PDV p90 5.3 ms (2 s) vs 1356 ms (10 s, gap masks baseline); t18 no gain from 10 s |
| `interval_us` | 500 ms | `[sourced]` | NetEq 500 ms; 250 vs 500 ms identical on both traces |
| `bucket_us` x `buckets` | 10 ms x 400 | `[tuned]` | clean PDV band 15-20 ms (t18 p99.9 15.3 ms); range 4 s > `max_us` |
| `forget_q15` | 0.98 (32113) | `[sourced]` | NetEq 0.983; with q99 holds one event 17 s (analytic) |
| `quantile_permille` | 990 | `[tuned]` | q95 retains a 1.4 s event ~1 interval, q99 17 s analytic (TA) |
| `hold_us` / `decay_us` | 60 s / 60 s | `[sourced]`+`[guess]` | Shaka stabilityThreshold 60 s; RFC 6298 inter-timeout scale; trace too short to test (12.6 s after the gap) |
| `k1`, `margin_us` | 2.0, 100 ms | `[guess]` | does **not bind** on either trace: t18/tf-1 pre-gap baseline 20 ms -> 140 ms < `min_us`. Binds only if baseline > 100 ms; needs the other channels |
| `k2` | 1.2 | `[tuned]` | 1.2 x 1402 ms = 1.68 s >= the 1.43 s needed to survive the gap with 0.25 s slack |
| `min_us` | 300 ms | `[tuned]`+`[sourced]` | clean-link zero-stall minimum 40 ms (TA) + 20 ms starve threshold + 150-200 ms measured Apple start hold + 35 ms PCR/32 ms AC-3 frame granularity |
| `initial_us` | 500 ms | `[guess]` | decision for the owner: TA stall-model rows. 500 ms stalls on tf-1; the smallest zero-stall start is 1430 ms, 3x the zap cost; per-channel seeding (section 7) lets a lossy channel start at 1.7 s on the next zap without penalising clean ones |
| `max_us` | 3 s | `[tuned]` | covers a 2x-RTO repeat of the tf-1 gap (1.4 s x 2 = 2.8 s) and 2.1 s resume; ExoPlayer default min/max buffer is far larger (50 s) but VOD |
| `after_rebuffer_q8` | 1.5 (384) | `[guess]` | `> 1x` needed to survive a repeat (1.43 s), `< 2x` RTO doubling; resume 2.1 s -> freeze 3.0 s on the tf-1 gap (TA) |
| `max_step_down_us_per_s` | 100 ms/s | `[sourced]` | libwebrtc VCMTiming |
| `starve_threshold_us` | 20 ms | `[sourced]` | zenith-tv Apple-side figure (0.02 s) |
| `nodata_after_us` | 200 ms | `[tuned]` | 5x the PCR-chunk spacing p99 of 41 ms (TA), above t18/tf-1 non-gap max chunk gap 36 ms |
| `discontinuity_us` | 1 s | `[sourced]`+`[tuned]` | GStreamer resync-jump idea. Rule: a PCR jump is media moving **forward faster than arrival** by > 1 s, or **backwards** by > 1 s. Late data (tf-1 gap: arrival +1433 ms, media +35 ms) is a gap, NOT a discontinuity; clean PDV is < 20 ms |
| `reprime_floor_us` / hold | 0 (off) / 2 s | `[guess]` | open decision |
| `stability_us` | 60 s | `[sourced]` | Shaka |

Honest summary: on the two traces the **binding** parameters are `min_us`, `initial_us`, `quantile`, `k2`, `after_rebuffer`, `max_us`; `k1`/`margin`/histogram shape are untested until a link with 100 ms+ continuous jitter is traced (synthetic ramp scenario covers the mechanics, not the values).

## 5. Test plan

**Unit, with independent oracles** (no test compares the crate to itself):
- Windowed minimum vs a brute-force scan of the same window; max-hold vs brute force; quantile vs sort of the weight-expanded multiset (tolerance: one bucket).
- RFC 3550 A.8: J against the reference C algorithm in `specs/rfc3550_rtp.txt` s A.8 (hand-ported in the test, not in `src`), with worked vectors for constant, alternating and ramping transit.
- RFC 5481 PDV definition: D - Dmin against a direct implementation on a 1 M-sample random trace.
- **Python parity (independent oracle)**: `docs/live-buffer/analysis/*.py` is a second implementation of PDV, estimators and the playback model; the Rust golden tests must match its published numbers (PDV p90/p99.9, J, estimator outputs, the 1430 ms minimum) within one bucket / 1 ms.
- Integer exactness: Q15 forgetting equals an exact rational reference to <= 1 LSB per interval; no float in `src` (grep gate).

**Real-trace golden tests** (`test_fixtures::private("live-buffer/tf-1.csv")`, `t18.csv`; skip loudly only when `private/` is not checked out, panic when it is but the file is missing): replay at chunk granularity; assert golden decisions: t18 never leaves Playing, zap = 500 ms + <= 35 ms; tf-1 with `initial=500` stalls at the gap with `Pause`, `Resume` after `resume=2.1 s`, the second gap simulated by re-injection survives; tf-1 with seed `peak=1402 ms` starts at 1.7 s and never stalls; estimator values match TA at the listed checkpoints. Traces are never copied to this public repo (PROVENANCE.md); the public CI therefore relies on the synthetic suite.

**Synthetic generator** (`tests/synth.rs`, deterministic seeded; each scenario asserts exact event sequences): (1) RFC 6298 RTO gaps 1.0/2.0/4.0 s, singly and consecutively; (2) periodic loss every N min (N = 1, 5) to exercise hold/decay; (3) ramping jitter 0 -> 300 ms PDV to exercise k1/margin and the follower slew; (4) clock drift +-100 ppm over 30 min (target must not creep; PDV baseline stays flat); (5) PCR wrap (base 2^33 x 300 boundary crossing) and PCR discontinuity (jump +-5 s, backwards) -> `Rebaselined`, memory kept; (6) 1.5 s gap **while paused**: no permanent starvation (see below); (7) a catch-up burst of 10x rate after a stall (must not inflate the baseline: burst-aware max); (8) non-monotonic arrival clock and `u64::MAX` timestamps.

**Contract test (the zenith-tv failure)**: a renderer model with a free-running clock. Model A (ignores `Pause`/`Resume`, clock runs through the gap) reproduces the measured permanent starvation (lead stays <= 0 after data resumes: asserts the test *bites*); Model B (obeys 2.4: freeze at `hold_at`, `setRate(1,time:)` at `Resume.media_27mhz`) ends the 1.5 s gap with lead >= resume target and zero further `Pause` events. Also asserts media queued while paused is never behind the resumed clock.

**Property tests** (`proptest`, dev-dep, std-only test crate path): no panic on arbitrary `(now, arrival, media, bytes)` sequences incl. non-monotonic, `u64` extremes, zero-length windows; `target_us` always within `[min, max]`; target never falls faster than `max_step_down`; target never exceeds `max_us`; state machine never `Playing -> Playing` with a `Resume`; every `Pause` is followed by at most one `Resume`; events bounded by queue capacity; determinism (same input -> same output, run twice).

**Biting mutations** (each must fail >= 1 named test): q99 -> q95; drop the forgetting (`forget`=1); remove the hold (decay immediately); remove the per-interval max (feed every sample); use a running instead of windowed minimum (drift scenario must fail); swap the hysteresis (single threshold) -> flap test; skip the startup suppression (first fill counted as stall); resume at `hold_at - 1` / without time; ignore `Pause` reason; treat `InputDegraded` as authoritative; `after_rebuffer` 1.0 (second-gap scenario must fail); remove the over-max hysteresis; replace `pcr_signed_distance` with `abs_diff` (wrap scenario). Mutation proof must show recompilation (workspace rule).

**Fuzz**: `fuzz/fuzz_targets/live_buffer.rs`: arbitrary op stream (chunk/tick/hint) against a tracked invariant set; nightly fuzz-build gate. **Examples** (>= 2, `std::fs` at runtime): `examples/replay_trace.rs` (CSV in, decisions out), `examples/live_loop.rs` (skeleton of a caller loop with a mock renderer implementing 2.4). **Guards**: `tests/label_coverage.rs` (every `pub enum`), `tests/clock33_guard.rs` (no `abs_diff`/`1<<33`; allowlist empty), `tests/no_float_guard.rs` (no `f32`/`f64` outside `#[cfg(test)]`), no-hand-roll tripwire not applicable (no protocol text; documented). Builds: `--no-default-features`, `thumbv7em-none-eabi`, MSRV 1.95, miri (core logic) as a stretch.

## 6. Acceptance-bar mapping ([`CRATE-ACCEPTANCE.md`](../CRATE-ACCEPTANCE.md))

| item | status |
|---|---|
| Symmetric Parse/Serialize, byte-identical round-trip, no `self.raw`, reserved-bit policy | **N/A**: no wire format (as `broadcast-loudness`: the compliance traces and the Python oracle are the hard gate instead) |
| No raw-byte public API, no magic numbers, `#[non_exhaustive]` | applies; every default is a named `const` in `DEFAULT` blocks, enforced by review + a literal guard |
| Spec-cited module docs; verified source | **partial: spec-posture departure requested.** The sizing half has no spec. Cited: RFC 3550 6.4.1/A.8 (J), RFC 5481 (PDV), RFC 6298 (RTO), RFC 9002 s5 (RTT estimation, for the min-filter idea), and permissive players (ExoPlayer/Media3 Apache-2.0, hls.js Apache-2.0, Shaka Apache-2.0, dash.js BSD-3, libwebrtc BSD-3). Heuristics are marked `implementation-defined` in the README, with the `[tuned]/[sourced]/[guess]` table. **Owner approval of this departure is requested**; LGPL GStreamer is idea-only (no code read into the crate) |
| Committed real fixture | private only (`private/fixtures/live-buffer/`, PROVENANCE exists); public CI relies on synthetic + the Python parity numbers; request: owner confirms whether zenith-tv may release a redacted public excerpt |
| Independent oracle per format | brute-force window, RFC 3550 A.8 reference, RFC 5481 definition, Python analysis scripts |
| Tests bite; >=2 entities | mutation list above; >= 2 channels/sessions: two `BufferController`s fed interleaved traces must not interact; two stalls in one trace |
| 6-gate CI, MSRV, thumbv7em | applies |
| #204 labels | applies (`State`, `Reason`, `Hint`, `Event`, `SampleOutcome`, `ConfigField`) |
| Fuzz target, >=2 examples | applies (5) |
| CLI | none (examples only) |
| RELEASE-DOCS | CHANGELOG, `docs/release-notes/<ver>.md`, README (with the implementation-defined notice), crate-root `//!`, `[package.metadata.docs.rs]`, GitHub Release |

Release plumbing (later PRs, not this one): `docs/crates-index.toml` + `docs/CRATES.md` entry (domain `live`, `standards = "RFC 3550, RFC 5481, RFC 6298"`, `lifecycle = "active"`; checked by `tools/gen-crate-index.py --check`), CLAUDE.md crate list, `.github/workflows/release-live-buffer.yml` cloned from `release-srt-runtime.yml` (independently versioned, tag `live-buffer-v*`, **no publish without owner sign-off**), crate README standards table (RFCs + "implementation-defined" row), workspace member + `clock33_guard`.

## 7. Integration plan (zenith-tv, tvOS)

Facts from zenith-tv: playback clock = `AVSampleBufferRenderSynchronizer.currentTime()`; starved when last demuxed video PTS - `currentTime` <= 0.02 s; no Apple stall callback; `delaysRateChangeUntilHasSufficientMediaData` only gates the start (it did hold the clock ~0.15-0.2 s at start with 1.40 s buffered); the clock keeps running through an underrun (measured, 1.5 s gap); today's policy is 1 s start / 2 s after the first stall for the rest of the connection; UX: 5 s stall message, 30 s reconnect, 15 s start timeout.

Wrapper (C ABI mirror; every field `u32`/`u64`/`i64`, no floats, opaque handle):

```c
typedef struct lb_ctrl lb_ctrl;
typedef struct { uint64_t jitter_min_window_us, interval_us; uint32_t bucket_us; uint16_t buckets, forget_q15, quantile_permille;
                 uint64_t min_us, initial_us, max_us, margin_us, stall_hold_us; uint16_t k1_q8, k2_q8, after_rebuffer_q8;
                 uint32_t max_step_down_us_per_s; uint64_t starve_threshold_us, nodata_after_us, reprime_floor_us; } lb_config;
typedef struct { uint32_t kind; uint32_t reason; uint64_t media_27mhz; uint64_t a_us; uint64_t b_us; } lb_event;  // kind: 1 Start 2 Pause 3 Resume 4 Target 5 OverMax 6 Rebaselined 7 Dropped
lb_ctrl *lb_new(const lb_config *, const uint64_t *seed_peak_us, int32_t *err);
void lb_on_chunk(lb_ctrl *, uint64_t arrival_us, uint8_t has_pcr, uint64_t pcr_27mhz, uint32_t bytes);
void lb_on_tick(lb_ctrl *, uint64_t now_us, uint8_t has_pos, uint64_t playback_27mhz);
int  lb_poll_event(lb_ctrl *, lb_event *out);   // 1 = event written
void lb_free(lb_ctrl *);
```

Loop: `on_chunk` from the HTTP read callback with `DispatchTime` uptime as `arrival_us` and the demuxer's last PCR per chunk (zenith-tv already records exactly these three columns); `on_tick` every 50 ms with `currentTime()` mapped into the PCR domain (their PTS->PCR offset); drain events: `StartPlaying`/`Resume` -> `synchronizer.setRate(1, time:)`, `Pause` -> `setRate(0, time: hold_at)` and **keep appending samples while paused**. Persist `StallMemory` per channel (lossy channel `tf-1` would start at 1.7 s next zap, clean channels at 0.5 s); on reconnect call `reset(keep_memory: true)`; the 5 s stall message maps to `Starved` age, the 30 s reconnect to `Starved` age or `OverMax`.

`transmux` exposure (**not changed in this PR**): it already unrolls PCR and knows continuity/sync. A future additive API would expose the last PCR per read chunk and `InputDegraded` (continuity errors / TEI) / `SyncLoss` as `Hint` values via a small adapter in the *caller* (the two crates stay independent; `live-buffer` depends on neither).

## 8. Risks, open decisions, data to request

1. **n=1 evidence.** One stall, one lossy channel. The adaptive parameters that only bind under heavier jitter (`k1`, `margin`, hist shape, hold/decay) are untested by real data. Ask zenith-tv for the other 6 channels, **plus long (>= 30 min) recordings** on a lossy link (stall rate, repeat spacing, RTO doubling) and a wired/Wi-Fi comparison, and the server-side explanation of why the sender never catches up (TA finding 3): if the server could burst after a stall, the buffer would refill and the whole depletion argument softens.
2. **Initial target vs zap time.** 500 ms (default) stalls once on tf-1 (freeze 3.0 s at resume 2.1 s; fixed 1 s/2 s: freeze 2.4 s); 1.43 s is the smallest survivor at +0.93 s zap on every zap. Owner decides: low default + per-channel seed, or a 1.5 s default. Also whether `after_rebuffer` 1.5 (longer freeze, repeat-safe) or 1.0 (shorter freeze, repeat stalls again).
3. **Voluntary re-prime** (`reprime_floor_us`): after a gap survived with 135 ms left (initial 1.5 s) the stream is fragile and only a deliberate freeze rebuilds it. Default OFF; owner call.
4. **Spec-posture departure** (section 6): approve implementation-defined policy documentation.
5. Sample-and-hold media time (<= 35 ms quantisation), PTS->PCR offset ownership, tick-rate-bound detection latency, `AVSampleBufferRenderSynchronizer` seek-to-time cost on resume (unmeasured; if setRate(1,time:) flushes, add a measured `resume_lead_extra_us`).
6. PCR-less transport (a stream with no PCR PID) is unsupported at first; follow-up: PTS-based media time at 90 kHz (same API, `pts*300`).
7. Private traces: cannot be public. Public CI must trust the synthetic suite and Python parity numbers.

## 9. Estimated size

| task | est. |
|---|---|
| 1. crate skeleton, config + validation + errors + labels | S |
| 2. `DeliveryJitter` (min window, interval max, Q15 histogram, max-hold, J) + unit oracles | M |
| 3. policy + follower + stall memory | S |
| 4. state machine + events + contract test (renderer models A/B) | M |
| 5. synthetic generator + property tests | M |
| 6. real-trace golden + Python parity | S |
| 7. fuzz target, 2 examples, guards, thumbv7em | S |
| 8. docs (README standards table, CHANGELOG, release note), index entries, release lane | S |
| 9. C ABI sketch crate in zenith-tv (outside this repo) | S |

S ~ half a day, M ~ a day; about 5 working days plus review/audit rounds and the extra-channel data loop.
