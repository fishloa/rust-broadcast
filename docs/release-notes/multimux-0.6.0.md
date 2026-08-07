# multimux 0.6.0

Released 2026-08-02.

### Added

- **Runtime admin API** (#749): `GET/POST/DELETE /admin/routes`,
  `POST /admin/reload` — add/remove/list routes and hot-reload the config
  file without restarting.
- **Signed-URL egress auth** (#747).
- **Classic MPEG-TS HLS output** (#887): `OutputKind::TsHls` serving `.ts`
  segments instead of fMP4.
