---
status: DRAFT
---
# dvb-si 13.1.0

_Released 2026-11-10._

Lockstep minor (13.0.0 -> 13.1.0). No breaking changes. The shared summary and migration notes are in the lockstep note [v13.1.0](v13.1.0.md); this note lists what changed in `dvb-si`.

## Added
- `TableId`, `DescriptorTag` and `ExtensionTag` implement the workspace label convention (`name()` + `Display` via `broadcast_common::impl_spec_display!`).

## Changed
- The carousel CRC32 descriptor module doc now states that its `CRC_32` is the same CRC-32/MPEG-2 as `broadcast_common::crc32_mpeg2` and that the check calls it; the code is unchanged.

## Fixed
- `ModuleReassembler`: the completed-module FIFO no longer grows by one entry per completed module version (it is now removed together with its record and never duplicated), so a long-lived carousel that cycles versions stays bounded and a stale entry cannot evict a live record.
- `ModuleReassembler::note_dii`: a DII repeat that announces the same module version with a different `module_size` or `block_size` replaces the slot instead of leaving DDBs validated against stale geometry.
- `SiDemux` discards a legally retransmitted (duplicate continuity_counter) continuation packet instead of appending it to the section being reassembled, which failed the CRC and dropped the whole section. Requires the `mpeg-ts` release that adds `SectionReassembler::feed_with_continuity`.
- `SiDemux` PAT-follow: a PAT spanning several sections no longer unwatches the PMT PIDs named only by other sections; the watch set is the union over every section of the current version and is pruned only once all sections of a new version are seen.
- `SiDemux` PAT-follow: a `current_next_indicator = 0` PAT is no longer followed, and a `table_id` 0 section on a PID other than 0 no longer rewrites the watch set.
- `SiDemux` PAT-follow: a `current_next_indicator = 0` PAT no longer clears or fills the PAT section accumulator (its generation is now keyed on `current_next_indicator`), so a multi-section current PAT keeps all its PMT PIDs followed.
- `SiDemux` version gate: the gate key now includes `current_next_indicator`, so a current and a next version of the same section (NIT/SDT/EIT) are no longer re-emitted on every carousel cycle.

## Dependencies
- `mpeg-ts` 0.7.0 -> 0.7.1.

---

Published from tag `v13.1.0`.
