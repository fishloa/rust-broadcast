# Adding a parser crate to the workspace

This guide is for authoring a **new sibling crate** (e.g. one like `scte35-splice`)
that plugs its own wire types into the existing `dvb-si` / `dvb-t2mi` dispatch
without forking or patching them.

The headline: **integrating a new crate requires zero breaking change to the
existing crates** — only additive work. The registries, the open `*Def` traits,
`Parse`/`Serialize`, the push-iterator pumps, and the `broadcast-common` primitives
are the integration surface, and they are deliberately designed for exactly this.

## The integration model (and its two honest nuances)

The four dispatch enums (`dvb_si::tables::AnyTableSection`,
`dvb_si::descriptors::AnyDescriptor`, the descriptor-extension bodies, and
`dvb_t2mi::payload::AnyPayload`) are generated in-tree from a drift-tested macro
list — see [ADR-0001](adr/0001-macro-driven-drift-tested-dispatch.md) and
[ADR-0002](adr/0002-keep-per-crate-dispatch-macros-separate.md). A separate
crate extends them through **runtime registries**, not by editing that list. Two
consequences are worth stating plainly:

1. **Runtime registration, not compile-time auto-wiring.** A new crate's types
   flow in via `registry.register::<T>()` plus the matching `*_with` seam — a
   few lines of caller setup — not "link the crate and it's automatically in
   `AnyTableSection`." This is deliberate: no global mutable state, and no
   `inventory`/`linkme`-style link-time registration dependency.

2. **`Other { value }` + downcast, not a first-class enum variant.** A separate
   crate cannot add a variant to `dvb-si`'s `AnyTableSection` (that is an
   in-tree-only macro edit). Instead its sections surface through the
   type-erased `Other` arm and are recovered with `downcast_ref`. This is the
   correct trade for zero coupling — the new crate stays fully decoupled from
   `dvb-si`'s release cadence.

Neither is a limitation in practice: a registered type round-trips, decodes, and
(under the `serde` feature) serializes through the same seams as a built-in.

## What a new crate implements

Each registry is keyed off a small open trait the new type implements. Implement
the trait, register the type, and call the `*_with` seam instead of the default:

| You want to add… | Implement | Register on | Drive with |
|---|---|---|---|
| A private/custom **table** (own `table_id`) | `dvb_si::traits::TableDef` | `TableRegistry::register::<T>()` | `AnyTableSection::parse_with(&reg, bytes)` or `SectionEvent::table_section_with(&reg)` |
| A private **descriptor** (own tag, optionally PDS-scoped) | `dvb_si::traits::DescriptorDef` | `DescriptorRegistry::register::<T>()` / `register_for_pds::<T>(pds)` | `DescriptorLoop::iter_with(&reg)` |
| A private **extended descriptor** (tag `0x7F`, own `descriptor_tag_extension`) | `dvb_si::descriptors::extension::ExtensionBodyDef` | `ExtensionRegistry::register::<T>()` | `DescriptorLoop::iter_with_extensions(&desc_reg, &ext_reg)` → `ExtIterItem::CustomExtension` |
| A private **T2-MI payload** (own `packet_type`) | `dvb_t2mi::traits::PayloadDef` | `PayloadRegistry::register::<T>()` | `AnyPayload::dispatch_with(ptype, bytes, &reg)` or `T2miEvent::payload_with(&reg)` |

All four `*Def` traits are **un-sealed** — external crates may implement them.
The type must also be `Debug + Any + Send + Sync` (the `*Object` bound the
blanket impl supplies); recover the concrete type from the `Other` arm with the
inherent `downcast_ref` on the `dyn` object.

Every registered type still owes the project's hard invariants: a symmetric
`Serialize` for its `Parse`, a byte-identical round-trip test, a spec citation in
its module doc, no magic numbers outside `#[cfg(test)]`, and a clean
`--no-default-features` build.

## Worked examples live next to the registries

Each registry's module doc carries a complete, compiling example — read those for
the exact signatures rather than copying from here:

- Tables — `dvb-si/src/tables/registry.rs` (and the `table_section_with` test in
  `dvb-si/src/demux.rs`).
- Descriptors (incl. PDS scoping) — `dvb-si/src/descriptors/registry.rs`.
- Descriptor extensions (`0x7F`) — `dvb-si/src/descriptors/extension/registry.rs`.
- T2-MI payloads — `dvb-t2mi/src/payload/registry.rs`.

The end-to-end extensibility audit in this repo's history built an external
scratch crate implementing **all four** `*Def` traits and drove every registry
and pump seam on both the all-features and `--no-default-features` arms — so the
pattern above is validated, not aspirational.

## Wiring the crate into the workspace

When the new crate is ready to become a workspace member:

1. Add its path to `members` in the root `Cargo.toml` (each crate lives in its
   own top-level directory).
2. Depend on `broadcast-common` (and `dvb-si`/`dvb-t2mi` if it extends their dispatch)
   with a workspace-pinned version.
3. Follow the same release discipline as the rest of the workspace — see the
   lockstep version + tag-driven publish rules in `CLAUDE.md`.

No existing crate needs a breaking change to accommodate the addition; at most a
future `pub` accessor is added, never a break.
