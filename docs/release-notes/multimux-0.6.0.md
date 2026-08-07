# multimux 0.6.0

**Release date:** 2026-08-02

Adds an admin API (`/admin/routes`, `/admin/programs`, `/admin/metrics`), HMAC signed-URL output auth scheme, and TS-HLS output alongside the existing CMAF-HLS. Requires `broadcast-auth` 0.2.1 for signed-URL support and `transmux` 0.22 / `broadcast-hls` 0.1 for the HLS playlist extraction.

## What's new

- **Admin API** — `GET /admin/routes` (route list + status), `GET /admin/programs` (active programs per route), `GET /admin/metrics` (Prometheus text format). Protected by a separate auth config (`admin_auth`).
- **Signed-URL output auth** — HMAC-SHA256 query-string tokens with configurable expiry, via `broadcast-auth` 0.2.1's `SignedUrl` verifier.
- **TS-HLS output** — MPEG-2 TS segments in HLS (alongside existing CMAF-HLS). Configured via `OutputScheme::TsHls`.

## What changed

- Requires `broadcast-auth` 0.2.1, `transmux` 0.22, `broadcast-hls` 0.1, `media-plane` 0.2.

## Migration

Requires the listed dependency versions. No breaking API changes to existing routes or config; the admin API and TS-HLS output are additive.
