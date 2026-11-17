# media-doctor 0.17.0

_Released 2026-11-17._

### Fixed
- `watch --metrics-max-conns` must be at least 1 and `--metrics-io-timeout-ms` must be non-zero; both are rejected at parse time with the flag named, instead of starting a server that refuses every scrape or drops every connection.
- BREAKING: `metrics_server::channel` returns a `watch::Receiver<Bytes>` (was `Receiver<Arc<str>>`), and the exposition is served from that published buffer instead of being copied into a fresh `Vec` on every scrape; `MetricsPublisher` publishes a `Bytes` body accordingly.
- `WatchState`'s PAT/PMT reconciliation indexes each elementary PID's declaring programs (`pid -> program -> stream_type`), so a PAT cycle costs O(programs + PIDs) instead of scanning every program per released PID and every program per PAT entry; a forged fast-cycling PAT can no longer stall ingest quadratically (#2856).

---

Published from tag `media-doctor-v0.17.0`.
