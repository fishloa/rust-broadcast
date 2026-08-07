# multimux 0.2.2

**Release date:** 2026-07-18

Fixes two LL-HLS preload-hint part regressions: the final part of a segment
was evicted from `live_parts` the instant the segment closed (causing a
per-segment 404 + latency bump), and requests for not-yet-produced hinted
parts returned 404 instead of blocking per RFC 8216bis §6.2.2.

## What's fixed

- **Final part 404 at segment boundary:** `add_segment` now moves a closed
  segment's parts into a bounded `recent_parts` buffer so the hinted final
  part is served (HTTP 200) instead of 404ing.
- **Pre-produced part 404:** Requests for a `#EXT-X-PRELOAD-HINT` part the
  origin hasn't produced yet now block until the part becomes available,
  matching the RFC 8216bis blocking-reload contract.
