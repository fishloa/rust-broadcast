# The media plane — project-wide ingress/egress architecture

> **Revision 2**, 2026-07-26. Rev 1 was synthesised from three commissioned architectures over a
> survey of the *muxing* crates only. A subsequent audit against the other ~20 crates returned
> **6 blocking findings and 14 gaps**. Rev 2 incorporates all of them. The rev-1 core (one log,
> N cursors, honest egress asymmetry) survives; its layering, timing model and egress taxonomy
> did not.
>
> Owner directive: *"I don't mind breaking things if the end game is better / more beautiful.
> Do not go with an option because it doesn't break."*

## 0. What rev 1 got wrong

| # | Rev-1 claim | Reality |
|---|---|---|
| B1 | One time model: absolute `i64` in track timescale | **Six domains.** GPS-epoch UTC (`splice_schedule.utc_splice_time`) and segment-relative `emsg` v0 cannot be expressed as either. |
| B2 | Analysis is a `PushEgress` reader class | **TR 101 290 has 19 indicators; a demuxed IR preserves 2.** `ConformanceMonitor::feed(pkt, t: Duration)` needs one 188-byte packet plus wall-clock arrival. Roadmap #737 (T-STD) is impossible under rev 1. |
| B3 | `Dialer → IngestSession → [Transform]* → Timeline` | **No byte→byte stage exists**, and CAM descramble, `ts-fix`, T2-MI/BBFrame inner-TS recovery all are one. |
| B4 | "No multi-input; SSAI is the only near-join" | **Already false.** ST 2022-7 hitless (2-input) and RIST bonding (N-input) are on the roadmap — and rev 1 *cited* 2022-7 to justify one-writer-per-Timeline. |
| B5 | One connection → one Timeline | **MPTS and T2-MI multi-PLP break it.** `parse_pat` flattens all programs; `program_number` appears nowhere in the IR. |
| B6 | Make the Stpp/Wvtt skip an error | **`CodecConfig` has no subtitle variant** — that converts "works" into "fails". AC-4 misdiagnosed: the variant exists, the demux arm doesn't. |

## 1. The shape (revised — four layers, not three)

```
Dialer|Listener  ──►  [ByteStage]*  ──►  IngestSession  ──►  [IrTransform]*  ──►  TrunkWriter
   (N sources)         byte→byte           demux               IR→IR                   │
        │                  │                                                           │
        └─ ByteMerge ──────┘   ByteTap ─► conformance / media-doctor / ts-fix probes    │
           (2022-7, RIST)      (Wire | PostTransform, carries arrival Instant)          │
                                                                                       ▼
                                                                        ┌──────── Trunk ────────┐
                                                                        │ sample ring           │
                                                                        │ segment log           │
                                                                        │ EVENT log (90 kHz)    │
                                                                        └───────────────────────┘
                                        subscribe() ─► SampleCursor  ─► PushEgress    (WHEP, RTMP-out, SRT-out, loudness)
                                        subscribe() ─► SegmentCursor ─► SegmentEgress (DVR, MABR, ROUTE, Smooth-push)
                                        resolve()   ─────────────────► ServedEgress   (LL-HLS, DASH, catch-up)
```

Renamed from rev 1: **`Trunk`**, not `Timeline` — `timed_metadata::Timeline` is published and does
33-bit wrap-unrolling. Two `Timeline` types in one dependency graph is a documentation mess.

### 1.1 The byte layer (resolves B2, B3, B4, most of B5, G5)

```rust
/// byte → byte, pre-demux. Clock-taking, deadline-driven.
pub trait ByteStage: Send {
    fn feed(&mut self, input: &[u8], now: Instant) -> Result<(), StageError>;
    fn poll(&mut self) -> Option<Bytes>;
    fn next_deadline(&self) -> Option<Instant>;
    fn on_deadline(&mut self, now: Instant);
}
```
Implementors: `dvb_ci_runtime::CaDescrambler` (CAM descramble), `ts_fix::TsFix` (continuity/PCR
repair, PID filter, PSI regen), `dvb_t2mi::T2miPump` + `InnerTsRecovery` (inner-TS recovery),
program-split (see §1.3).

```rust
/// The ONE bounded multi-input primitive. N byte sources → 1 byte stream.
/// The IR layer above stays strictly single-input.
pub struct ByteMerge { policy: MergePolicy /* Hitless2022_7 | FirstArrival | Failover */ }
```
This is rev 2 conceding B4 honestly: there is a graph, with exactly two node shapes, at the byte
layer only. That is cheaper than pretending the topology is a line.

```rust
/// Positional tap for analysis. Yields bytes AS RECEIVED, with arrival time,
/// including bytes the demuxer will reject (bad CRC, TEI set, unaligned).
pub enum TapPoint { Wire, PostTransform }
pub struct ByteTap { /* bounded ring, own Lagged */ }
impl ByteTap { pub fn poll(&mut self) -> Option<(Bytes, Timestamp)>; }
```
`dvb-conformance`, `media-doctor watch`, and `ts-fix` verification are `ByteTap` consumers, not
`PushEgress`. This is what makes TR 101 290 (all 19 indicators) and #737 (T-STD, which needs
arrival timing) possible at all.

(`Timestamp`, not `std::time::Instant` — the plane is `no_std`-capable; see §4's
`broadcast_common::Timestamp(u64 nanos)`.)

**Verified 2026-07-26 — the existing consumer already fits this shape with no adaptation.**
`dvb_conformance::ConformanceMonitor::feed(&mut self, ts_packet: &[u8], t: Duration) -> &[ConformanceEvent]`
(`dvb-conformance/src/lib.rs:649`) is *already* a per-packet streaming API returning the events
raised by that packet, and `media-doctor`'s `watch.rs:217` already drives it live off a UDP
ingest. It emits `Continuity_count_error` (TR 101 290 indicator 1.4), `SyncByteError`, and
`TransportError` (from `header.tei`). `dvb-conformance` is `no_std`-capable
(`#![cfg_attr(not(feature = "std"), no_std)]`), so the tap can carry it. `ByteTap`'s
`(Bytes, Timestamp)` maps onto `feed`'s two arguments directly — `Timestamp` → `Duration`.

**Consequence: loss/degradation detection is NOT a `DemuxEvent` concern** (issue #778 rescoped
accordingly). `transmux::StreamingTsDemux` reads only `pid` and `pusi` of the seven `TsHeader`
fields and discards `tei`/`continuity_counter`/`scrambling` — that is *correct layering*, not a
defect: a container demuxer's job is framing, and TR 101 290 conformance is a tap's job on the
same bytes. Adding an `InputDegraded` variant to `DemuxEvent` would have duplicated subtle
already-shipped logic (legal CC duplicates, `discontinuity_indicator` exclusion, non-payload
packets not advancing CC — `media-doctor/src/diagnostics/cc_anomaly.rs:83-95`) *and* coupled
independently-versioned `transmux` to the lockstep `dvb-conformance` (8.6.0). The tap avoids
both. `DemuxEvent` keeps only what the demuxer itself observes as a framing fact
(sender-signalled `discontinuity_indicator`, PCR, track lifecycle).

### 1.2 Trunk, cursors, and the event log

```rust
pub struct Trunk { /* sample ring + segment log + event log + track set + notify */ }

impl Trunk {
    pub fn writer(&self) -> Option<TrunkWriter>;              // exactly once, never blocks
    pub fn subscribe_samples(&self, at: SeekPoint, p: ReaderPolicy) -> SampleCursor;
    pub fn subscribe_segments(&self, at: SeekPoint, p: ReaderPolicy) -> SegmentCursor;
    pub fn events(&self) -> EventLogView<'_>;
    pub fn tracks(&self) -> TrackSetSnapshot;                  // generation-versioned
}
```

**The event log (resolves B1c/B1d, G1, G9).** Reference clock is **90 kHz absolute**, matching
`timed_metadata::MediaTime` — *not* per-track timescale. It carries `timed_metadata::TimedEvent`
(owned, lossless, `#[non_exhaustive]`, already published) rather than a new type; `EmsgBox<'a>` is
borrowed and cannot be stored, and `SourcePayload::Emsg` is already its owned form. Entries are
addressable **by media time and by segment**, because `emsg` v0 is segment-relative and cannot be
finalised until the segmenter owns a boundary. Scheduled events carry an optional **UTC anchor**
(`timed_metadata::TimeAnchor`) and stay in GPS/UTC until an anchor exists.

⇒ `timed-metadata` becomes a **real** dependency of the plane, not a dev-dep (see §6 for the
publish cycle this creates).

**Sparse vs timed tracks (G10).** A SCTE-35 section PID emits one untimed sample every N minutes.
Two retention classes in the ring:
- `Timed { pts, dts }` — count/duration bounded, `Lagged` permitted.
- `Sparse { arrival: Watermark }` — retention **slaved to the trunk window**, never dropped
  independently, and `Lagged` on a sparse reader escalates to `RouteHealth::Degraded` (a lost cue
  is a correctness failure, not a QoS blip).

`Sample.pts/dts` therefore stay **`Option`al** for section-carried tracks rather than being
fabricated — rev 1's mandatory absolute pair was wrong for this class.

### 1.3 Program dimension (resolves B5)

Program splitting is a **`ByteStage`**, reusing `ts_fix`'s shipped
`filter_pids(PidFilter::program(n))`. One program → one `IngestSession` → one `Trunk`. Writer
identity is `(session, program)`. An MPTS route fans one socket into N byte-stages, N trunks, and
per-service egress — the broadcast-headend case rev 1 could not express.

## 2. Ingress

`Dialer` (dial out) and `Listener` (accept, N sessions, `max_sessions` bound) as in rev 1 — that
part held. `IngestSession` gains the clock parameters that `Stage` needs anyway (G4):

```rust
pub trait IngestSession: Send {
    fn tracks(&self) -> &TrackSet;
    fn feed(&mut self, input: &[u8], now: Instant) -> Result<(), IngestError>;
    fn poll(&mut self, out: &mut TrackBatch) -> SessionPoll;   // …| NewProgram(ProgramId, TrackSet) | Ended
    fn poll_transmit(&mut self) -> Option<Transmit>;
    fn next_deadline(&self) -> Option<Instant>;
    fn on_deadline(&mut self, now: Instant);
}
```

**CAM ordering (G5), stated because rev 1 hid it.** `CaDescrambler::feed_ts` filters to
`descramble_pids ∪ ca_pids ∪ emm_pids ∪ PCR` and **does not carry PAT/PMT** (the CAM gets the PMT
via the `ca_pmt` APDU). Feeding its output straight to `TsDemux` finds *no tracks*. Order is:
SI-demux → CAM control (`add_service(&PmtSection)`, `set_cat(&CatSection)` — parsed `dvb-si`
structs, never bytes) → descrambled bytes → PSI regen (`ts-fix` or `mpeg_ts::mux::SiMux`) → media
demux. That is a control feedback loop, and `dvb-si` is **not currently a dependency of transmux or
multimux at all**. `Driver::pump` blocks on a char device ⇒ blocking pool. One slot = one TS path;
multi-tuner is N byte stages into a remuxer.

## 3. Egress — three shapes, not two (resolves G6)

```rust
/// Pull. N consumers at N positions. Renders from the Trunk.
pub trait ServedEgress: Send + Sync + 'static { /* resolve(req, &Trunk) -> EgressResponse */ }

/// Sample push. Owns one SampleCursor.
pub trait PushEgress: Send {
    fn negotiate(&mut self, tracks: &TrackSet) -> Result<TrackSelection, EgressError>;
    /// Track sets change mid-stream (PMT version bump; config not knowable until the
    /// first sample — `refine_legacy_config`). Rev 1's once-only negotiate was wrong. (G11)
    fn renegotiate(&mut self, tracks: &TrackSet) -> Result<TrackSelection, EgressError>;
    async fn send(&mut self, batch: &TrackBatch) -> Result<(), EgressError>;
}

/// Segment/object push. Owns one SegmentCursor. DVR, DVB-MABR, ATSC ROUTE, Smooth-push.
pub trait SegmentEgress: Send {
    async fn on_segment(&mut self, seg: &SegmentRef) -> Result<(), EgressError>;
    async fn on_manifest(&mut self, m: &ManifestSnapshot) -> Result<(), EgressError>;
}
```

`SegmentRef` fields are derived from what the existing packagers actually emit (G7): segment
number, sequence number, is-segment-start, keyframe alignment, per-track SDP, availability time.
A FLUTE/ROUTE sender additionally needs a **carousel repeat schedule with per-object deadlines**,
TOI allocation and FDT expiry — none of which is a `TrackBatch`. (`dvb-flute` is headers only: no
FDT XML, no object reassembly, no FEC, no scheduler.)

**Whole-asset writers are tools, not runtime egress.** `ProgressiveMux` needs the complete `Media`
(two passes, `stco`→`co64` promotion on absolute offsets); MXF OP1a/AS-11 (#754) writes its Footer
Partition and Index Tables after the essence. These stay `Package`-over-a-bounded-`Media`, stated
explicitly so #754 isn't shoehorned into a runtime trait.

### 3.1 Per-viewer cursors do NOT scale — high-fan-out egress needs a relay

Rev 1 claimed "the viewer *is* a cursor, so N viewers at N positions falls out for free". The
benchmark refutes it: writer cost is O(N) in cursor count, so one cursor per WHEP viewer puts
hundreds of readers on the trunk lock (~100 readers extrapolates to ~60 us mean publish, over
budget for a high-rate route).

**Rule:** a cursor is for a *distinct consumer of the stream* (segmenter, DVR writer, analysis tap,
one push relay) — **not** for each peer of a one-to-many protocol. `PushEgress` implementations that
serve many peers (WHEP, and any future RTP multicast/SSM egress) take **one** cursor and fan out to
their peers themselves, where per-peer state (SRTP context, congestion window, pacing epoch) already
has to live anyway. Supported trunk reader count is therefore **single-digit by design**; peers are
unbounded but live behind a relay.

**Rev-1 claim corrected (G7):** removing the three `String` manifest renderers does **not** collapse
`Package::Output` to `Vec<u8>`. Four composite outputs remain — `Vec<Chunk>`, `RtpOutput`,
`SmoothOutput`, `TsHlsOutput` — and their per-unit metadata is exactly what `SegmentRef` must model.

**TS-out needs DVB SI (G8).** `transmux::ts_mux` emits one PAT + one PMT with hardcoded
`PMT_PID = 0x1000`, `PROGRAM_NUMBER = 1`, and no SDT/NIT/EIT/TDT/TOT. An SPTS without SI is not
deliverable to a DVB network. Wire `mpeg_ts::mux::SiMux` (rate-scheduled, TR 101 211 intervals) as
the SI scheduler with `dvb-si` building the tables; PIDs and `program_number` become route config.
This is deadline-driven — hence `now` on `Stage`/`ByteStage`.

## 4. The IR break

```rust
pub struct Sample {
    pub data: Bytes,                 // shareable, sliceable (packetisation), slab-recyclable
    pub dts: Option<i64>,            // absolute, track timescale; None for section-carried tracks
    pub pts: Option<i64>,
    pub duration: Option<u32>,
    pub flags: SampleFlags,
}
```
- `Bytes` over `Arc<Frame>` — payload sharing **and** zero-copy subrange slicing (every RTP
  packetiser would otherwise copy per packet). `mpeg-ts` already depends on `bytes` unconditionally
  at `default-features = false`, so the `no_std`+`alloc` posture holds.
- **Cost rev 1 omitted (G12):** `Bytes` is immutable/shared, so in-place rewrite paths
  (`cenc_encrypt`, `sample_aes`, `Sample::from_annexb`, `TsMux`'s Annex-B inverse) gain an
  allocation unless refcount == 1. Mitigation: `try_into_mut` fast path, or encrypt before publish
  while the writer holds the only reference. Net copies on the encrypt path may go *up*; the
  fan-out and packetisation wins still dominate.
- **Add `CodecConfig::Subtitle { format }`** (stpp/TTML, wvtt/WebVTT, DVB-bitmap, teletext) and the
  missing **`ac-4` demux arm** in the same break. **Only then** is turning the silent skip into an
  error safe (B6).
- Consolidate `media.rs` + `pipeline.rs` into `transmux::ir`; `#[non_exhaustive]` on `Media`/`Track`;
  TS provenance off the neutral spec.

**Derived tracks (G2).** Captions live *inside* video samples (`cc_data()` in picture user_data);
ANC is frame-locked beside video (ST 2038); `dvb-subtitle` has no PTS at all. Model them as
`DerivedFrom { source: TrackId, extractor: ExtractorId }` — materialised lazily by the first cursor
that asks, cached in the ring, leaving the video track byte-identical. This gives `negotiate()`
something to see (a real HLS `#EXT-X-MEDIA` subtitle rendition) and follows the shape the codebase
already chose: `Cea608CueExtractor::push_frame(pts33, &[CcTriplet])`. ST 2038 ANC shares the video
sample's PTS and must not be independently retimed by a rebase transform.

## 5. Corrected invariants

**"We never decode" is already false (G3).** `cc-data`'s `decode` feature is *in default*
(CEA-608/708 window/pen interpreters); `timed-metadata`'s `teletext` does EN 300 706 Hamming FEC;
transmux parses SPS/AC-3/DTS/VP8/Vorbis/MPEG-2 headers pervasively and `refine_legacy_config` reads
*inside* sample data; `from_annexb`/`TsMux` rewrite NAL framing. Restated precisely:

> **No audio or video *essence* (PCM/pixel) decoder in the media plane. Bitstream-header parsing
> and non-A/V-essence decode (captions, teletext, ANC) are in scope.**

Caption decode is therefore a **derived track**, not a "quarantined tap".

**ST 337 defeats `negotiate()` (G14).** AC-3/Dolby E wrapped in AES3 signals as *linear PCM*, so a
WHEP sink's "reject AC-3 loudly" accepts it and ships noise, and the loudness tap confidently
measures a bitstream as PCM. Rule: probe the first N frames for the `Pa`/`Pb` sync pattern before
believing a PCM claim; loudness refuses unverified PCM. (`st337` has no `data_type`→codec map — ST 338
was unverifiable — so the rule can only be "it's wrapped, don't trust the claim".)

## 6. Migration (revised)

Order and blast radius as rev 1, plus the corrections the audit forced:

1. `broadcast-common` **8.7.0** additive — `Stage` **with `now: Instant` + deadlines** (G4). Zero cascade.
2. `transmux` **0.20.0** breaking — `ir`, `Bytes`, optional absolute timing, `CodecConfig::Subtitle`,
   `ac-4` arm, silent-drops→errors. **Publish-cycle hazard (G13):** `transmux` dev-deps
   `media-doctor` which normal-deps `transmux`; same for `timed-metadata`. Publish `transmux` 0.20
   with those dev-deps pinned to the prior release, then `media-doctor`. Move `timed-metadata`'s
   SEI-caption fixture test out before the plane depends on it normally.
3. **`media-plane` 0.1.0** NEW — `Trunk`, cursors, event log, `ByteStage`/`ByteTap`/`ByteMerge`,
   `Dialer`/`Listener`/`IngestSession`, three egress traits, retention, registry. Own release lane.
4. `ll-hls-runtime` **0.2.0** breaking — renders from `&Trunk`; `server/store.rs` deleted.
5. `multimux` **0.5.0** breaking — deletes `Output`/`SampleSource`/`SourceConnector`/`run_pipeline`/
   `store.rs`; `multimux::http` is the sole axum adapter. **`acap-multimux` must be re-verified on
   the ARTPEC-6 camera before tagging — a gate.**
6. Deprecate **`dvb-stream`** (F4): zero reverse deps, duplicate resync, duplicate multicast bind —
   strictly subsumed by a `udp://` `Dialer` + `ByteStage`. Fold its `SectionStream`/`T2miEventStream`
   in as example stage compositions.
7. Roadmap: each protocol is one `Dialer`/`Listener`/`ByteStage`/`PushEgress`/`SegmentEgress`/
   `ServedEgress` impl plus a registry line. **`dvb-simulcrypt`** (ECMG/EMMG/C(P)SIG — head-end
   scrambling) belongs beside CENC encrypt, using the section-injection hook `ts_mux` already has
   plus rate scheduling.

Explicitly out of scope, named so nobody wonders (F1): **`ule`** yields IP datagrams, not media —
a datagram plane, like `st2110`.

## 7. Still-open weaknesses

- **SSAI is a two-timeline operation and rev 2 stops calling it a `Transform`** (B4). It is an
  IR-level two-cursor composition. Rev 1's claim that absolute timing alone makes it work was wrong
  (G9): the modular 33-bit PTS lives *inside an opaque section payload*, so resolving a cue needs a
  SCTE-35 parser plus `timed_metadata::Timeline`'s epoch tracker in the plane — and transmux has no
  runtime `scte35-splice` dependency today. Nothing owns this yet.
- **Two planes (main + `st2110`) will drift.** Unchanged from rev 1; still true.
- **`ByteMerge` admits a graph.** Bounded to the byte layer with three named policies. If a third
  multi-input shape appears at the IR layer, this design is wrong and a real DAG is the answer.
- **`Trunk` at rate is MEASURED (`spikes/trunk-bench`, 2026-07-26): PASS at the specced scale, with
  one claim corrected.** 200-track MPTS x 6 readers, ~1 Gbit/s aggregate: 999.97/1000 Mbit/s
  sustained, publish mean 5.6 us / p99 44.3 us / max 144 us against a ~111 us inter-arrival budget,
  no reader starvation, `Bytes` no-copy fan-out confirmed by pointer identity + allocator counts.
  **But writer cost is cheap O(N) in reader count, NOT O(1)** (956 ns -> 9.98 us across 1 -> 16
  readers): writer and readers contend on one shared `Mutex`. It reads as flat at N=6 only because
  the per-op cost is small against the budget. Consequence in SS3.1. Mitigation if reader counts
  grow: shard the lock per track, or serve reads from an `ArcSwap` snapshot. Re-measure before
  raising the supported reader count.
- **`st377-1` uses `Package`/`Track`/`Sequence` MXF vocabulary** — naming collision with the IR,
  worth one disambiguating sentence.
