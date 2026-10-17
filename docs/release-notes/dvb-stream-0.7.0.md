# dvb-stream 0.7.0

_Released 2026-10-17._

**Breaking release.** Own breaks: the T2-MI stream constructors take a typed `mpeg_ts::pid::Pid` instead of a `u16`, and the crate now builds against new epochs of `broadcast-common` (10), `dvb-si`/`dvb-t2mi` (12), `mpeg-ts` (0.6) and `rtp-packet` (0.5). It also fixes a framing bug: `TsDecoder::datagram` (and so `UdpSectionStream` and `UdpT2miStream`) now unwraps RTP-encapsulated MPEG-2 TS datagrams (RFC 3550 section 5.1 header, CSRCs, header extension and padding skipped), where previously a `0x47` byte inside the RTP header could lock the decoder onto misaligned bytes.

## Why this version

- **Proposed version:** 0.6.0 -> **0.7.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (`T2miEventStream::new` and `UdpT2miStream::{from_socket,bind,bind_multicast}` take `Pid`); plus the RTP datagram fix.
- **cargo-semver-checks vs the live release (all features):** none flagged. (Lint-based: it misses type changes of existing fields and arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** also forced by the sibling epochs it depends on: `broadcast-common` 9 -> 10, `dvb-si` 11 -> 12, `dvb-t2mi` 11 -> 12, `mpeg-ts` 0.5 -> 0.6, `rtp-packet` 0.4 -> 0.5.
- **Caret epoch:** 0.6 -> 0.7; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-dvb-si = "11.1"
+dvb-si = "12.0"
-dvb-t2mi = "11.1"
+dvb-t2mi = "12.0"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
-rtp-packet = "0.4"
+rtp-packet = "0.5"
```

## Breaking changes and migration

```rust
use mpeg_ts::pid::Pid;

// before
let s = T2miEventStream::new(reader, 0x1000);
let u = UdpT2miStream::bind(&multicast_config, 0x1000).await?;
// after
let s = T2miEventStream::new(reader, Pid::try_new(0x1000)?);
let u = UdpT2miStream::bind(&multicast_config, Pid::try_new(0x1000)?).await?;
```

The same `Pid` argument applies to `UdpT2miStream::from_socket` and `UdpT2miStream::bind_multicast`. `Pid::try_new` rejects values above `0x1FFF`, so an out-of-range PID is now a construction-time error instead of a silently never-matching filter. `SectionStream`, `UdpSectionStream` and `TsDecoder` signatures are unchanged.

New dependencies: `rtp-packet` and `broadcast-common` (for the RTP unwrap in `TsDecoder::datagram`). Because `dvb-si`, `dvb-t2mi` and `mpeg-ts` also move epoch, resolve them all at the versions above in one dependency graph; `Serialize::to_bytes` was removed from `broadcast-common` 10 (use `try_to_bytes`).

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `dvb-si` 11 -> 12, `dvb-t2mi` 11 -> 12, `mpeg-ts` 0.5 -> 0.6, `rtp-packet` 0.4 -> 0.5). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Fixed
- `TsDecoder::datagram` (and so `UdpSectionStream` / `UdpT2miStream`) now
  unwraps RTP-encapsulated MPEG-2 TS datagrams (RFC 3550 §5.1 header, CSRCs,
  header extension and padding skipped; payload must be whole 188-byte
  packets starting on `0x47`). Previously the TS lattice was hunted for
  through the RTP header bytes, so a `0x47` in the header (sequence number,
  timestamp or SSRC) 188 bytes before a `0x47` in the first packet latched
  the decoder onto misaligned bytes (#1099, r11-W-DS-2). Adds `rtp-packet`
  and `broadcast-common` dependencies.

#### Changed (BREAKING)

- **BREAKING (#1159):** `T2miEventStream::new` and `UdpT2miStream::{from_socket,bind,bind_multicast}` take `mpeg_ts::pid::Pid` instead of `u16`.

## Merged changes since `dvb-stream-v0.6.0`

- docs: correct false 'not vendored' / 'edition unverified' claims (#1311)
- docs: cluster A crate READMEs to the crate-README standard (what/not, standards table, features, doctested quick start, API, design notes) (#1289)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- Fix confirmed transport/ancillary review defects (supersedes #1277) (#1280)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
- Audit systemic wave 1: reserved-bit spec cleanup, push mux, T9 clock33, rtcp profile extensions (release set: lockstep 11.1.0 + cascade) (#1173)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
