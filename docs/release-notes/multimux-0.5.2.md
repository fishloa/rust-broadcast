# multimux 0.5.2

**Release date:** 2026-07-30

Dependency-floor fix to prevent split-bucket trait-resolution errors (#858).

## What's fixed

- Floor `media-plane` to 0.1.1. The `^0.1` bucket also contains 0.1.0 (built against `transmux` 0.20), so a consumer could resolve two `transmux` minors into one graph and hit trait-resolution errors.

## Migration

No breaking changes.
