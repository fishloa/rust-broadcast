# dvb-si 14.0.0

_Released 2026-11-14._

Breaking-class release (13 -> 14): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 13.0.0 -> 14.0.0.

## Changed
- BREAKING: the long-form table constructors `AitSection::new`, `CitSection::new`, `EitSection::new`, `IntSection::new`, `MpeIfecSection::new`, `NitSection::new`, `PmtSection::new`, `RctSection::new`, `RntSection::new`, `SatSection::new`, `SdtSection::new` and `UntSection::new` take one `tables::SectionVersion` (`version_number`, `current_next_indicator`, `section_number`, `last_section_number`; build it with `SectionVersion::new`) in place of those four positional arguments (#2612).
- The carousel CRC32 descriptor module doc now states that its `CRC_32` is the same CRC-32/MPEG-2 as `broadcast_common::crc32_mpeg2` and that the check calls it; the code is unchanged (c6 C6-12).

## Added
- `skills/dvb-si-basics/SKILL.md`: a Symposium agent skill shipped in the published package, so an AI coding agent working in a project that depends on `dvb-si` gets usage guidance covering the `SiDemux` loop, descriptor loops, the symmetric parse/serialize contract, carousel reassembly, features and the common pitfalls.
- `TableId`, `DescriptorTag` and `ExtensionTag` implement the workspace label convention (`name()` + `Display` via `broadcast_common::impl_spec_display!`).

## Fixed
- The Satellite Access Table's bit-packed body reader reports a short input in bytes, matching every other `BufferTooShort` in the workspace, instead of absolute bits; its serializer is now `broadcast_common::bits::BitWriter` rather than a second hand-rolled writer, keeping the field-width check the SAT relies on (issue #2461).
- `ModuleReassembler`: the completed-module FIFO no longer grows by one entry per completed module version (it is now removed together with its record and never duplicated), so a long-lived carousel that cycles versions stays bounded and a stale entry cannot evict a live record.
- `ModuleReassembler::note_dii`: a DII repeat that announces the same module version with a different `module_size` or `block_size` replaces the slot instead of leaving DDBs validated against stale geometry.
- `SiDemux` discards a legally retransmitted (duplicate continuity_counter) continuation packet instead of appending it to the section being reassembled, which failed the CRC and dropped the whole section. Requires the `mpeg-ts` release that adds `SectionReassembler::feed_with_continuity`.
- `SiDemux` PAT-follow: a PAT spanning several sections no longer unwatches the PMT PIDs named only by other sections; the watch set is the union over every section of the current version and is pruned only once all sections of a new version are seen.
- `SiDemux` PAT-follow: a `current_next_indicator = 0` PAT is no longer followed, and a `table_id` 0 section on a PID other than 0 no longer rewrites the watch set.
- `SiDemux` PAT-follow: a `current_next_indicator = 0` PAT no longer clears or fills the PAT section accumulator (its generation is now keyed on `current_next_indicator`), so a multi-section current PAT keeps all its PMT PIDs followed.
- `SiDemux` version gate: the gate key now includes `current_next_indicator`, so a current and a next version of the same section (NIT/SDT/EIT) are no longer re-emitted on every carousel cycle.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `mpeg-ts` 0.7 -> 0.8.

---

Published from tag `v14.0.0`.
