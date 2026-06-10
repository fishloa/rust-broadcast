# Architecture Decision Records

Short, append-only records of significant, hard-to-reverse design decisions in
the `rust-dvb` workspace — the *why*, not the *how*. Format: lightweight
[Nygard ADRs](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions).

- One file per decision: `NNNN-kebab-title.md`, numbered sequentially.
- Status is one of `Proposed` / `Accepted` / `Superseded by NNNN` / `Deprecated`.
- Don't edit the decision of an accepted ADR; supersede it with a new one.

| ADR | Title | Status |
|-----|-------|--------|
| [0001](0001-macro-driven-drift-tested-dispatch.md) | Macro-driven, drift-tested dispatch for all tag/type enums | Accepted |
| [0002](0002-keep-per-crate-dispatch-macros-separate.md) | Keep the per-crate dispatch macros separate (don't hoist into `dvb-common`) | Accepted |
