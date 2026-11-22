# webrtc-runtime 0.10.0

_Released 2026-11-22._

Minor for a 0.x crate. One real fix: a STUN gather whose Binding transaction exhausts its retransmissions now settles instead of being rescheduled forever. The breaking part is the epoch move (`broadcast-auth` 0.9, `broadcast-common` 14, `rtp-packet` 0.9).

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-auth` 0.8 -> 0.9, `broadcast-common` 13 -> 14, `rtp-packet` 0.8 -> 0.9). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each at the jiff 1.0 epoch.

## Fixed

- `media::gather` now closes its server-reflexive gather when the STUN Binding transaction exhausts its retransmissions (RFC 8489 §6.2.1): `handle_timeout` drains the terminal event, so a STUN server that never answers settles the gather instead of leaving `MediaTransport` to schedule it forever (audit 22 finding 37).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `webrtc-runtime-v0.10.0`.
