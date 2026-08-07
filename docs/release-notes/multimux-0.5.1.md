# multimux 0.5.1

**Release date:** 2026-07-30

Fixes a Smooth Streaming pull-source panic and a DASH output 503 on the first segment request. Adds `#[non_exhaustive]` to public config/event enums.

## What's fixed

- Smooth-pull input: fix panic when the manifest contains zero audio tracks.
- DASH output: return `Await` instead of `503` when the first segment is still being written.

## What's new

- `#[non_exhaustive]` on `InputScheme`, `OutputScheme`, `RouteEvent`.

## Migration

No breaking changes (`#[non_exhaustive]` is additive for match-with-wildcard consumers).
