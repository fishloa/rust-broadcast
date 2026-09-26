# ttml-subtitle 0.2.1

Security patch. **Upgrade if you parse TTML/IMSC documents you do not control.** Drop-in for
0.2.x.

## Security

**GHSA-9gp9-h275-mjjh** — before this release, the parser and serializer recursed once per
nesting level of `<span>` and metadata elements, with no limit. About 3,000 nested spans (about
30 KB of input) overflowed the stack and aborted the process.

## Changes

- **Parsing:** span and metadata nesting deeper than `MAX_NESTING_DEPTH` (64) is rejected with
  a constraint-violation error. Real subtitle documents nest only a few levels.
- **Serializing:** `to_xml` walks nested inline content and metadata with an explicit stack
  instead of recursing, so even a document built in code with 100,000 nested spans serializes.
  Its output is byte-identical to the previous serializer, which is checked by a test that
  compares against the old recursive implementation.

## Known limitation

Dropping a document built in code with extreme nesting (tens of thousands of levels) can still
overflow the stack, because Rust's generated `Drop` for the boxed element tree recurses. Parsed
documents can't reach that depth. This is tracked separately.

MSRV 1.95.0.
