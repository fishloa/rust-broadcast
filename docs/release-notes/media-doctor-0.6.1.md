# media-doctor 0.6.1

**Release date:** 2026-07-30

Dependency floor fix: floors dependencies to epoch-pure versions to prevent split-bucket trait-resolution errors (#858).

## What's fixed

- Floor `mpeg-ts` to `0.3.1` (epoch-pure within the `^0.3` caret bucket).
- Floor `timed-metadata` to `0.4.1` (epoch-pure within the `^0.4` caret bucket).

## Migration

No breaking changes.
