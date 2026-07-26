# Media-plane implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL — superpowers:subagent-driven-development.
> Spec: `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` (rev 2).
> Steps are numbered to match the spec's §6 migration.

**Goal:** replace the ad-hoc ingress/egress surfaces with the four-layer media plane
(byte layer → demux → IR transforms → Trunk → three egress shapes).

**Gate on the whole plan:** Step 3 (`media-plane 0.1`) MUST NOT start until the Trunk fan-out
benchmark (`spikes/trunk-bench`) returns PASS or PASS-WITH-CHANGES. A FAIL invalidates §1.2 of the
spec and the plan is rewritten, not patched.

## Global constraints
- MSRV **1.86**, edition 2024, `--locked`. `cargo nextest`, never two cargo commands at once.
- `no_std` + `alloc` preserved for `broadcast-common`, `transmux`, and every codec crate. The core
  abstractions must not force `std` or tokio downward.
- Every wire type keeps symmetric parse/serialize + a byte-identical round-trip test.
- **Untrusted input:** every constructor that consumes wire bytes takes a mandatory bound. No
  `new()`, no `Default`, no unbounded buffer, anywhere. (Four remote alloc-DoS vectors have already
  shipped in this workspace; the architecture makes that class a type error.)
- Real fixtures, mutation-verified tests, per-task review. `#[non_exhaustive]` on every growable
  public type. #204 label pair (`name()` + `impl_spec_display!`) on every public spec/field enum.

---

## Step 1 — `broadcast-common` 8.6.0 → 8.7.0 (additive, zero cascade)

**Files:** create `broadcast-common/src/stage.rs`; modify `src/lib.rs`, `src/mux.rs` (docs only), `CHANGELOG.md`.

**Interfaces produced:**
```rust
pub trait Stage {
    type Out;
    type Error;
    fn feed(&mut self, input: &[u8], now: Instant) -> Result<(), Self::Error>;
    fn poll(&mut self) -> Option<Self::Out>;
    fn finish(&mut self) -> Result<(), Self::Error>;
    fn next_deadline(&self) -> Option<Instant>;
    fn on_deadline(&mut self, now: Instant);
    fn demand(&self) -> Demand;
}
#[non_exhaustive] pub struct Demand { pub want_bytes: usize, pub saturated: bool }
```
- `now`/deadlines are on the trait from the start — the audit (G4) showed `ConformanceMonitor::feed(pkt, t)`,
  `SiMux::poll_into(now, out)` and `WatchState::feed_datagram(payload, clock)` all need a clock, and a
  clockless `Stage` would cover only the container-demux family.
- `Instant` is `std`-only ⇒ gate as `#[cfg(feature = "std")]`, or take a `Timestamp(u64 nanos)` newtype so
  `no_std` implementors work. **Decide in Step 1 and write it down**; prefer the newtype.
- `mux.rs`: doc-scope `Package`/`Unpackage` as the batch/whole-file contract. **No `#[deprecated]`** — it
  would fire under CI's `-D warnings` across ~30 transmux test files for no user benefit.

**Steps:** write the trait + a doctest implementor → `cargo build -p broadcast-common --all-features --locked`
and `--no-default-features` → verify no dependent broke (`cargo build --workspace --locked`) → CHANGELOG → commit.

## Step 2 — `transmux` 0.19.0 → 0.20.0 (breaking; ships alone)

Split into reviewable tasks; each ends green.

**2a — consolidate the IR.** Merge `media.rs` + `pipeline.rs` into `transmux::ir` (today `media.rs`
imports its own leaves back). No semantic change yet. `#[non_exhaustive]` on `Media`/`Track`.

**2b — `Sample.data: Vec<u8>` → `Bytes`.** Add the `try_into_mut` fast path for the in-place rewrite
paths the audit flagged (G12): `cenc_encrypt`, `sample_aes`, `Sample::from_annexb`, `TsMux`'s Annex-B
inverse. **Test that the encrypt path does not regress in allocation count** — the spec admits net
copies there may rise; measure it rather than assume.

**2c — timing.** `pts`/`dts` become `Option<i64>`, absolute, in the track timescale, rollover unwrapped
once at the demux edge. Populate them in the FLV/WebM/PS/RTMP/RTP demuxers that leave `start_decode_time`
at 0 today. Delete write-only `SourceTiming`; keep original stamps in `Provenance`. **Section-carried
tracks stay `None` — do not fabricate.** Byte-identical round-trip fixtures are the gate (absolute
timestamps carry strictly more information than running sums, so TS→IR→TS and fMP4→IR→fMP4 must still
match byte-for-byte).

**2d — codec coverage before strictness.** Add `CodecConfig::Subtitle { format: SubtitleFormat }`
(stpp/TTML, wvtt/WebVTT, DVB-bitmap, teletext) and the missing **`ac-4` demux arm**. ONLY THEN turn
`Fmp4Demux`'s silent Stpp/Wvtt skip and `CmafMux`'s silent `CodecConfig::Data` filter into errors.
Ordering is mandatory — reversing it regresses working CMAF-with-subtitles inputs (audit B6).

**2e — `Stage` adoption.** Implement `Stage` for `StreamingTsDemux`, `StreamingFlvDemux`,
`ProgressiveDemux`, `RtpStreamDepacketiser`, and the five segmenters. `DemuxEvent` moves out of
`ts_demux.rs` into a neutral module and de-TS-ifies (`Pcr`, `Discontinuity{pid}` → provenance-carrying
variants). Each stage keeps its own `Out` — do NOT force one universal event enum (the TS-only variants
in `DemuxEvent` are precisely what forced the RTMP first-Sample workaround).

**2f — publish-cycle fix (audit G13).** `transmux` dev-deps `media-doctor` which normal-deps `transmux`;
same shape for `timed-metadata`. Pin those dev-deps to the prior release for the 0.20 publish, then
release `media-doctor`. Move `timed-metadata`'s SEI-caption fixture test out of that crate before
`media-plane` takes a normal dependency on it.

**Blast radius (verified):** 6 in-workspace path-deps recompile in the same PR. `acap-multimux` pins
transmux `"0.18"` and `bindings/transmux-py` pins `"0.15"` — **both unaffected.** Fold in the already-open
#720 binding ripple.

## Step 3 — `media-plane` 0.1.0 (NEW) — **GATED ON THE BENCHMARK**

Only start when `spikes/trunk-bench` reports PASS or PASS-WITH-CHANGES; adopt any named fix from that
report before writing `Trunk`.

**3a — byte layer.** `ByteStage`, `ByteTap { TapPoint::{Wire, PostTransform} }` yielding `(Bytes, Instant)`
including bytes the demuxer rejects, and `ByteMerge { Hitless2022_7 | FirstArrival | Failover }`. This is
the layer that unblocks conformance/T-STD, CAM descramble, `ts-fix`, T2-MI inner-TS recovery, and
ST 2022-7/RIST. Build it first — five of the audit's six blockers resolve here.

**3b — `Trunk` + cursors.** Sample ring (with `Timed` vs `Sparse` retention classes), segment log, event
log at 90 kHz carrying `timed_metadata::TimedEvent` with an optional UTC anchor and segment-addressable
entries (`emsg` v0 is segment-relative). Single `TrunkWriter`, never blocks. `SampleCursor`/`SegmentCursor`
with `Lagged` charged to the slow reader; `Lagged` on a `Sparse` reader escalates to `Degraded`.

**3c — ingress traits.** `Dialer`, `Listener { max_sessions }`, `IngestSession` (with `NewProgram` for the
program dimension), generic `run_dial`/`run_listen` drivers, supervision with EOF ≠ error so
`HealthState::Failed` becomes producible.

**3d — egress traits.** `ServedEgress` (with `EgressResponse::Await` for blocking reload, no axum in the
trait), `PushEgress` (with `renegotiate` for mid-stream track-set changes), `SegmentEgress`.

**3e — retention + stores.** `Retention` (`Tiered { hot, cold, cold_window }`), `SegmentSink`,
`ArchiveOverrun { Gap (default) | StallIngest | Terminate }`.

**3f — acceptance furniture.** `docs/CRATE-ACCEPTANCE.md` in full: ≥2 examples, fuzz targets over every
byte-consuming entry point, `tests/label_coverage.rs`, real fixtures, `release-media-plane.yml` lane.

## Step 4 — `ll-hls-runtime` 0.1.1 → 0.2.0 (breaking)
Render playlists and blocking-reload decisions from `&Trunk`. Delete `server/store.rs`. Implement
`ServedEgress`. Blast radius: `multimux` only.

## Step 5 — `multimux` 0.4.0 → 0.5.0 (breaking)
Delete `Output`, `SampleSource`, `SourceConnector`, `run_pipeline`, `store.rs` and the ~180 lines of
forwarding. Port the 9 sources to `Dialer`/`Listener`; the 3 outputs to `ServedEgress` behind a single
concrete `multimux::http` axum adapter. Per-route policy (kills the process-wide `IngestTimeouts` and
the hardcoded `MOVIE_TIMESCALE = 90_000`). Symmetric registry — third parties can register ingress,
byte stages, all three egress shapes, transforms and auth.
**Gate: `acap-multimux` must be re-verified on the ARTPEC-6 camera before this is tagged.**

## Step 6 — cleanup + roadmap
Deprecate `dvb-stream` (zero reverse deps; duplicate resync and multicast bind; strictly subsumed).
Name `ule` as out of scope (datagram plane). Then each roadmap protocol is one impl + one registry line:
#744 re-egress, #742 Smooth, #743 WHEP, #740 WHIP, #741 RIST, #746 DVR, #757 loudness (derived-track
caption decode, essence decode quarantined), #745 DRM/CPIX, #755 MABR + #750 ROUTE (`SegmentEgress`),
#751/#752 (separate `st2110` plane), #753 TTML.

**The test this plan must pass:** Step 6 touches no trait defined in Steps 1–5.
