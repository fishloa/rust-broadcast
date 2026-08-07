# transmux 0.21.1

**Release date:** 2026-07-30

Patch release to floor the `mpeg-ts` dependency to 0.3.1, preventing a consumer from resolving two `broadcast-common` majors into one dependency graph (#858).

## What's fixed

- Floor `mpeg-ts` to `0.3.1`. The `^0.3` bucket also contains 0.3.0, which is built against `broadcast-common` 8, so a consumer could resolve two `broadcast-common` majors into one graph and hit trait-resolution errors.

## Migration

No breaking changes.
