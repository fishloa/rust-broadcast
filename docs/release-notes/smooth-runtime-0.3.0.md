# smooth-runtime 0.3.0

_Released 2026-11-17._

Minor-class release (0.2 -> 0.3) from the audit 20 bounds bundle. It adds byte and length caps for held fragment bodies, the manifest, a fragment body and a stream plan, and it resolves a fragment URL through `resolve_http` so a non-web scheme or an `https` downgrade is never fetched. The new bounds are config fields with defaults; existing callers keep working.

## Upgrading

- `media-plane` 0.12 -> 0.13 (the requirement is now `0.13.0`).

## Added

- `SmoothClientConfig::max_held_bytes` (default `DEFAULT_MAX_HELD_BYTES`, 32 MiB, set with `with_max_held_bytes`): fragment bodies are held, across every stream, until all of them have announced their local track id, and an origin that serves one stream nothing usable could make the healthy streams' fragments accumulate without bound. Past the bound the streams that have not announced are abandoned (`Dropped::StreamLost`) so the rest are announced and their held fragments released, and `SmoothError::AnnounceStalled { max }` ends the session when nothing is left to abandon (#2816).
- `SmoothClientConfig::{max_manifest_bytes, max_fragment_bytes, max_plan_len}` (defaults `DEFAULT_MAX_MANIFEST_BYTES` 4 MiB, `DEFAULT_MAX_FRAGMENT_BYTES` 16 MiB, `DEFAULT_MAX_PLAN_LEN` 10 000, set with the matching `with_*` setters), so a remote origin cannot exhaust memory through the manifest, a fragment body, or a live refresh that appends chunks faster than they are fetched (#2823):
  - a manifest body past `max_manifest_bytes` is the new `ManifestFault::TooLarge`, retried and then fatal like any other manifest fault, and `max_manifest_bytes` is the document bound the manifest parser is given;
  - a fragment body past `max_fragment_bytes` ends the session with the new `SmoothError::FragmentTooLarge { stream, bytes, max }`;
  - a stream's plan past `max_plan_len` after a refresh drops its oldest chunks and reports `Output::Discontinuity`.
  `broadcast-client` reports `SmoothError::FragmentTooLarge` as `ErrorKind::SegmentFetchFailed { status: None }`.

## Fixed

- A fragment URL resolved from the manifest's `Url` template must now be an `http`/`https` URL reached without an `https`-to-`http` downgrade (`broadcast_common::url_ref::resolve_http`): a template naming `file:`, `ftp:` or downgrading the connection is `Dropped::FragmentUrl` and is never requested, instead of becoming a fetch the host performs (#2853).

---

Published from tag `smooth-runtime-v0.3.0`.
