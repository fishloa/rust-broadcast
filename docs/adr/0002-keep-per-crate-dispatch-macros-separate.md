# ADR-0002: Keep the per-crate dispatch macros separate (do not hoist into `dvb-common`)

- **Status:** Accepted (2026-06-10)
- **Scope:** all `dvb-*` crates
- **Relates to:** [ADR-0001](0001-macro-driven-drift-tested-dispatch.md)

## Context

[ADR-0001](0001-macro-driven-drift-tested-dispatch.md) put every
wire-discriminant dispatch enum behind a macro-driven, drift-tested pattern,
and noted as a **future optional** follow-up: hoisting the four near-identical
`declare_*!` macros into one shared definition in `dvb-common` so
`AnyDescriptor`, `AnyTableSection`, `AnyPayload`, and the extension bodies
share one macro instead of four copies. That follow-up "would need its own
follow-up ADR if taken." This is that ADR — and the decision is **not** to take
it.

After completing the extension-level migration (ADR-0001 stage 2), the four
macros were compared head to head. They share only a skeleton (build an enum
from a list, `From` impls, a `name()`, a dispatcher, a drift test). On nearly
every concrete axis they diverge, and the divergence is essential to each
crate's wire contract, not incidental:

| Axis | `declare_descriptors!` | `declare_tables!` | `declare_payloads!` | `declare_extension_bodies!` |
|---|---|---|---|---|
| Discriminant | scalar `0xNN` | **range list** `[lo..=hi, …]` | scalar `0xNN` | scalar `0xNN` |
| Enum derives | `Debug` | `Debug` | `Debug` | `Debug, Clone, PartialEq, Eq` |
| serde | `rename_all = "camelCase"` | camelCase | camelCase | **none** (PascalCase keys) |
| yoke | `Yokeable` | `Yokeable` | `Yokeable` | **none** |
| Fall-through | `Other { Box<dyn …> }` **and** `Unknown { tag, body }` | `Unknown { table_id, raw }` | `Unknown { packet_type, body }` | `Raw(&[u8])` |
| `@no_dispatch` section | yes | yes | no | no |
| `*Def` const | `TAG: u8` | `TABLE_ID_RANGES: &[(u8,u8)]` | `PACKET_TYPE: u8` | `TAG_EXTENSION: u8` |
| Dispatcher | `dispatch(tag, full) -> Option<Result>` | `parse(bytes) -> Result` (inline `Unknown`) | `dispatch(ptype, bytes) -> Option<Result>` | free `parse_body(tag_ext, sel) -> Result` (inline `Raw`) |
| Macro-only extras | registry `Other`, `DISPATCHED_TAGS` | `parse_as`, `DISPATCHED_RANGES`, disjoint-range test | `DISPATCHED_TYPES` | `selector_len`/`write_selector` serialize delegation, `kind()` coupling |

The discriminant alone (scalar vs inclusive-range) forces different fragment
matchers, different dispatcher arm syntax, and different drift assertions. The
extension-body macro additionally carries serialize delegation and `ExtensionTag`
coupling that no other dispatch has, and deliberately omits the serde/yoke
derives the others require.

## Decision

**Keep the four `declare_*!` macros separate, each next to the dispatch enum it
generates. Do not hoist them into a shared `dvb-common` macro.**

A single shared macro would have to be parameterised over discriminant shape,
four distinct fall-through layouts (one with a `Box<dyn>` registry), the derive
and serde/yoke attribute set, four dispatcher signatures, and the
extension-only serialize/`kind()` extras — an ~8-parameter macro thick with
optional sections. That is harder to read than the four focused, self-documenting
macros; it saves almost no lines (each crate's per-type invocation list stays
exactly as long); and it couples three crates' dispatch evolution to one fragile
definition in the foundation crate. The duplication here is shallow structural
rhyme, not shared logic worth centralising.

The drift-tested *pattern* from ADR-0001 remains mandatory; only the
single-shared-macro *implementation* is rejected.

## Consequences

- **+** Each macro stays readable and local to its crate; changing one crate's
  dispatch (e.g. adding a registry, a new fall-through, a serialize hook) never
  touches the others or `dvb-common`.
- **+** No macro added to the public surface of the foundation crate.
- **−** The four macro bodies keep their shallow structural similarity; a future
  reader sees four ~150-line macros that "look alike." This is accepted as the
  cheaper, clearer state.
- If a fifth dispatch enum appears that is genuinely shape-identical to an
  existing one, copy the closest macro rather than generalising — and only
  revisit this decision if three or more become exactly congruent.
