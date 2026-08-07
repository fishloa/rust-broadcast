# multimux 0.6.0

**Release date:** 2026-08-02

Adds classic MPEG-TS HLS output, a runtime admin API for live route management, and signed-URL egress authentication. The HLS-pull source now uses `broadcast-hls` directly instead of reaching through `transmux`.

## What's new

- **Classic MPEG-TS HLS output** (`OutputKind::TsHls`, config `"ts_hls"`, #887): whole `.ts` segments with no `#EXT-X-MAP`, no low-latency parts. Container is per-route — mutually exclusive with fMP4-based outputs on the same route.
- **Runtime admin API** (#749): add, remove, list routes and reload config without restarting the origin. Opt-in via `Config::admin` with mandatory auth. Endpoints: `GET/POST/DELETE /admin/routes[/{name}]`, `POST /admin/reload`.
- **Signed-URL output auth** (`OutputAuthSpec::SignedUrl`, #747): HMAC-SHA256 signed URLs with rotatable keys, path-bound tokens, optional IP binding, and constant-time signature comparison.
- `serve_config_file` / `serve_config_file_with_registry` entry points for config-file-aware serving (enables `POST /admin/reload`).
- `Route`/`InputSpec`/`AuthSpec`/`OutputKind` now derive `PartialEq` (used by the admin reload diff).

## Migration

Requires `multimux` 0.6 (pre-1.0 caret boundary `^0.5` -> `^0.6`). No API-level breaking changes beyond the version bump.
