# timed-metadata 0.4.1

**Release date:** 2026-07-27

Moves the `roundtrip` example from `transmux` into this crate (where it belongs) and requires `broadcast-common` 9 to stay epoch-pure within the `^0.4` caret bucket (#858).

## What's new

- `roundtrip` example (previously lived in `transmux`, now properly in its own crate).

## What changed

- Requires `broadcast-common` 9 (epoch-pure floor).

## Migration

No breaking changes. Consumers already on `broadcast-common` 9 need no action.
