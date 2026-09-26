# multimux-cli 0.9.0 — 2026-09-26

Rebuild on `multimux` 0.11.0. **Upgrade if you run the `multimux` binary with WHIP, WHEP, push
outputs, RTP/UDP inputs, or any authenticated route.** That release carries the security fixes
for GHSA-jwfh-m4vx-fhwx, GHSA-48qq-7p78-2jvj, GHSA-6cpc-jqv3-qcj3, GHSA-c5v7-p4jv-2fhc and
GHSA-2w4r-qf2x-pqm6 — the last covers the new single-publisher rule (a route now rejects a
second, concurrent RTMP/WHIP publisher), startup warnings when an ingest route runs with no
authentication configured, and Digest challenges that report `stale=true` on an expired nonce
instead of silently re-prompting for credentials. See the multimux 0.11.0 release note for
details.

The command line, flags and JSON config are unchanged. The version is a minor only because the
`multimux` dependency moved from 0.10 to 0.11.

MSRV 1.95.0.
