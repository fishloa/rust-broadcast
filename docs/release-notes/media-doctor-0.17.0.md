# media-doctor 0.17.0

_Released 2026-11-17._

Minor-class release (0.16 -> 0.17) from the audit 20 bounds bundle. The metrics endpoint clamps its connection count and I/O timeout at parse time, serves a published `Bytes` buffer instead of copying a `Vec` per scrape, and the PAT/PMT reconciliation is indexed so a fast-cycling PAT cannot stall ingest quadratically.

## Upgrading

- BREAKING: `metrics_server::channel` returns `watch::Receiver<Bytes>` instead of `Receiver<Arc<str>>`, and `MetricsPublisher` publishes `Bytes`; update the receiver type and anything that reads the exposition as `Arc<str>`.

## Fixed

- `watch --metrics-max-conns` must be at least 1 and `--metrics-io-timeout-ms` must be non-zero; both are rejected at parse time with the flag named, instead of starting a server that refuses every scrape or drops every connection.
- BREAKING: `metrics_server::channel` returns a `watch::Receiver<Bytes>` (was `Receiver<Arc<str>>`), and the exposition is served from that published buffer instead of being copied into a fresh `Vec` on every scrape; `MetricsPublisher` publishes a `Bytes` body accordingly.
- `WatchState`'s PAT/PMT reconciliation indexes each elementary PID's declaring programs (`pid -> program -> stream_type`), so a PAT cycle costs O(programs + PIDs) instead of scanning every program per released PID and every program per PAT entry; a forged fast-cycling PAT can no longer stall ingest quadratically (#2856).

---

Published from tag `media-doctor-v0.17.0`.
