//! Data-carousel module reassembly — collects [`DownloadDataBlock`]s into
//! complete modules per the DII's `moduleSize`/`blockSize` announcement
//! (`docs/iso_13818_6_carousel.md`, "Module reassembly").

use std::collections::HashMap;

use super::messages::{Dii, DownloadDataBlock};

/// Identifies one module instance on the carousel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ModuleKey {
    /// downloadId from the DII / DDB headers.
    pub download_id: u32,
    /// moduleId from the DII module entry.
    pub module_id: u16,
    /// moduleVersion — a version bump restarts collection.
    pub module_version: u8,
}

/// A fully reassembled module.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Module {
    /// Identity of the completed module.
    pub key: ModuleKey,
    /// The `moduleSize` bytes, in order.
    pub data: Vec<u8>,
}

/// Internal map key: one slot per module instance, version held in the
/// [`Slot`] — so the stale-version check in `note_dii` is a single keyed
/// lookup instead of a scan over every in-progress module.
type SlotKey = (u32, u16); // (download_id, module_id)

/// Per-module collection state. `received` is a bitset (one bit per block) so
/// a hostile `blockSize = 1` DII costs ~1/8 of the module size in tracking
/// overhead instead of one `bool` per byte.
struct Slot {
    module_version: u8,
    block_size: usize,
    data: Vec<u8>,
    received: Vec<u64>,
    n_blocks: usize,
    remaining: usize,
}

impl Slot {
    fn is_received(&self, n: usize) -> bool {
        (self.received[n >> 6] >> (n & 63)) & 1 != 0
    }
    fn mark_received(&mut self, n: usize) {
        self.received[n >> 6] |= 1 << (n & 63);
    }
}

/// Default cap on a single module's announced `moduleSize`.
pub const DEFAULT_MAX_MODULE_SIZE: u32 = 64 * 1024 * 1024;
/// Default cap on the TOTAL bytes held across all in-progress modules — a
/// hostile carousel rotating downloadId/moduleId/moduleVersion can otherwise
/// multiply the per-module cap without bound.
pub const DEFAULT_MAX_TOTAL_BYTES: usize = 256 * 1024 * 1024;
/// Default cap on the number of in-progress module slots. The byte budget
/// alone does not model the per-slot map entry + bitset overhead, so many
/// distinct *small*-module announcements must be bounded separately. 16 384
/// slots is far above any real carousel (a DII announces at most 65 535
/// modules per `numberOfModules`, real ones tens) while capping worst-case
/// map overhead at a few MiB.
pub const DEFAULT_MAX_SLOTS: usize = 16 * 1024;

/// Collects DDB blocks into complete modules.
///
/// Usage: call [`note_dii`](Self::note_dii) for every DII (repeats are
/// idempotent; a changed `moduleVersion` restarts that module), then feed
/// every DDB through [`feed_ddb`](Self::feed_ddb). DDBs for modules not yet
/// announced by a DII are ignored — carousels repeat, so the block comes
/// round again after the DII has been seen.
///
/// Memory bounds: each announced module is capped at `max_module_size`, the
/// aggregate of all in-progress module buffers at `max_total_bytes`, and the
/// number of in-progress slots at `max_slots` (the byte budget alone does not
/// model per-slot map/bitset overhead, so many distinct small modules are
/// bounded separately) — announcements that would exceed any cap are skipped
/// until completed modules free space, and `moduleSize == 0` announcements
/// are rejected outright. Block tracking is a bitset
/// (~`moduleSize/blockSize/8` bytes), so the worst-case overhead for a
/// `blockSize = 1` announcement is ~12.5% on top of the data buffer.
///
/// Liveness on lossy streams: the skip-until-space policy means that when the
/// budget is held by large modules that never complete (sustained loss),
/// later small announcements are starved until the large slots complete or
/// are version-bumped. This is by design — drop and recreate the reassembler
/// to recover from a wedged stream.
pub struct ModuleReassembler {
    slots: HashMap<SlotKey, Slot>,
    max_module_size: u32,
    max_total_bytes: usize,
    max_slots: usize,
    total_bytes: usize,
}

impl Default for ModuleReassembler {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleReassembler {
    /// New reassembler with [`DEFAULT_MAX_MODULE_SIZE`] and
    /// [`DEFAULT_MAX_TOTAL_BYTES`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_MODULE_SIZE, DEFAULT_MAX_TOTAL_BYTES)
    }

    /// New reassembler with a custom per-module size cap (aggregate budget
    /// stays at [`DEFAULT_MAX_TOTAL_BYTES`]).
    #[must_use]
    pub fn with_max_module_size(max_module_size: u32) -> Self {
        Self::with_limits(max_module_size, DEFAULT_MAX_TOTAL_BYTES)
    }

    /// New reassembler with explicit per-module and aggregate byte caps
    /// (slot-count cap stays at [`DEFAULT_MAX_SLOTS`]).
    #[must_use]
    pub fn with_limits(max_module_size: u32, max_total_bytes: usize) -> Self {
        Self {
            slots: HashMap::new(),
            max_module_size,
            max_total_bytes,
            max_slots: DEFAULT_MAX_SLOTS,
            total_bytes: 0,
        }
    }

    /// Replace the in-progress slot-count cap (default
    /// [`DEFAULT_MAX_SLOTS`]). Announcements past the cap are skipped until
    /// completed modules free slots — the same skip-until-space policy as the
    /// byte budget.
    #[must_use]
    pub fn with_max_slots(mut self, max_slots: usize) -> Self {
        self.max_slots = max_slots;
        self
    }

    /// Register the modules announced by a DII. Skipped: `moduleSize == 0`
    /// (nothing to reassemble), modules over the per-module cap,
    /// `blockSize == 0`, and modules that would push the aggregate budget
    /// over `max_total_bytes` or the slot count to `max_slots`.
    /// Re-announcement of an in-progress (same-version) module is a no-op; a
    /// new version replaces the old slot (freeing its budget first).
    pub fn note_dii(&mut self, dii: &Dii<'_>) {
        for m in &dii.modules {
            if m.module_size == 0 || m.module_size > self.max_module_size || dii.block_size == 0 {
                continue;
            }
            let key: SlotKey = (dii.download_id, m.module_id);
            if let Some(existing) = self.slots.get(&key) {
                if existing.module_version == m.module_version {
                    continue; // carousel repeat — keep accumulated blocks
                }
                // Older version — drop it, releasing its budget.
                let s = self.slots.remove(&key).expect("just found");
                self.total_bytes -= s.data.len();
            }
            let size = m.module_size as usize;
            if self.total_bytes + size > self.max_total_bytes || self.slots.len() >= self.max_slots
            {
                continue; // budget or slot cap exhausted — skip until space frees
            }
            let block_size = dii.block_size as usize;
            let n_blocks = size.div_ceil(block_size).max(1);
            self.total_bytes += size;
            self.slots.insert(
                key,
                Slot {
                    module_version: m.module_version,
                    block_size,
                    data: vec![0u8; size],
                    received: vec![0u64; n_blocks.div_ceil(64)],
                    n_blocks,
                    remaining: n_blocks,
                },
            );
        }
    }

    /// Feed one DDB. Returns the completed [`Module`] when this block was the
    /// last missing piece. Blocks for unknown (downloadId, moduleId, version)
    /// triples, out-of-range block numbers, repeats, and blocks whose length
    /// disagrees with the DII geometry are ignored.
    pub fn feed_ddb(&mut self, ddb: &DownloadDataBlock<'_>) -> Option<Module> {
        let key: SlotKey = (ddb.download_id, ddb.module_id);
        let slot = self.slots.get_mut(&key)?;
        let n = ddb.block_number as usize;
        if slot.module_version != ddb.module_version || n >= slot.n_blocks || slot.is_received(n) {
            return None;
        }
        let offset = n * slot.block_size;
        let expected = (slot.data.len() - offset).min(slot.block_size);
        if ddb.block_data.len() != expected {
            return None; // disagrees with the announced geometry — corrupt
        }
        slot.data[offset..offset + expected].copy_from_slice(ddb.block_data);
        slot.mark_received(n);
        slot.remaining -= 1;
        if slot.remaining > 0 {
            return None;
        }
        let slot = self.slots.remove(&key).expect("slot exists");
        self.total_bytes -= slot.data.len();
        Some(Module {
            key: ModuleKey {
                download_id: ddb.download_id,
                module_id: ddb.module_id,
                module_version: slot.module_version,
            },
            data: slot.data,
        })
    }

    /// Number of modules currently being collected.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.slots.len()
    }

    /// Total announced `moduleSize` bytes currently held by in-progress
    /// module buffers — the quantity charged against `max_total_bytes`.
    ///
    /// This counts data-buffer bytes only, not the per-slot map entry and
    /// block-bitset overhead, so it understates true retained memory (the
    /// slot-count cap bounds that overhead instead). Do not use it as a
    /// memory-pressure signal; use [`pending`](Self::pending) × expected
    /// module size for a rough upper bound.
    #[must_use]
    pub fn pending_bytes(&self) -> usize {
        self.total_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::super::messages::DiiModule;
    use super::*;

    fn dii(download_id: u32, block_size: u16, modules: Vec<DiiModule<'static>>) -> Dii<'static> {
        Dii {
            transaction_id: 0x8000_0002,
            adaptation: &[],
            download_id,
            block_size,
            window_size: 0,
            ack_period: 0,
            t_c_download_window: 0,
            t_c_download_scenario: 0,
            compatibility_descriptor: &[],
            modules,
            private_data: &[],
        }
    }

    fn module(module_id: u16, module_size: u32, module_version: u8) -> DiiModule<'static> {
        DiiModule {
            module_id,
            module_size,
            module_version,
            module_info: &[],
        }
    }

    fn ddb(
        download_id: u32,
        module_id: u16,
        module_version: u8,
        block_number: u16,
        block_data: &[u8],
    ) -> DownloadDataBlock<'_> {
        DownloadDataBlock {
            download_id,
            adaptation: &[],
            module_id,
            module_version,
            block_number,
            block_data,
        }
    }

    #[test]
    fn two_block_module_completes() {
        let mut r = ModuleReassembler::new();
        r.note_dii(&dii(1, 4, vec![module(7, 6, 0)]));
        assert!(r.feed_ddb(&ddb(1, 7, 0, 0, &[1, 2, 3, 4])).is_none());
        let m = r.feed_ddb(&ddb(1, 7, 0, 1, &[5, 6])).expect("complete");
        assert_eq!(m.key.module_id, 7);
        assert_eq!(m.data, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(r.pending(), 0);
    }

    #[test]
    fn out_of_order_blocks_complete() {
        let mut r = ModuleReassembler::new();
        r.note_dii(&dii(1, 2, vec![module(1, 4, 0)]));
        assert!(r.feed_ddb(&ddb(1, 1, 0, 1, &[3, 4])).is_none());
        let m = r.feed_ddb(&ddb(1, 1, 0, 0, &[1, 2])).expect("complete");
        assert_eq!(m.data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn ddb_before_dii_is_ignored() {
        let mut r = ModuleReassembler::new();
        assert!(r.feed_ddb(&ddb(1, 1, 0, 0, &[1, 2])).is_none());
        // After the DII arrives, the carousel repeat completes it.
        r.note_dii(&dii(1, 2, vec![module(1, 2, 0)]));
        assert!(r.feed_ddb(&ddb(1, 1, 0, 0, &[1, 2])).is_some());
    }

    #[test]
    fn version_mismatch_ignored_and_new_version_restarts() {
        let mut r = ModuleReassembler::new();
        r.note_dii(&dii(1, 2, vec![module(1, 4, 0)]));
        assert!(r.feed_ddb(&ddb(1, 1, 0, 0, &[1, 2])).is_none());
        // DDB with a different version is not accepted into the v0 slot.
        assert!(r.feed_ddb(&ddb(1, 1, 3, 1, &[9, 9])).is_none());
        // A DII announcing v3 replaces the v0 slot entirely.
        r.note_dii(&dii(1, 2, vec![module(1, 4, 3)]));
        assert_eq!(r.pending(), 1);
        assert!(r.feed_ddb(&ddb(1, 1, 3, 0, &[5, 6])).is_none());
        let m = r.feed_ddb(&ddb(1, 1, 3, 1, &[7, 8])).expect("complete");
        assert_eq!(m.key.module_version, 3);
        assert_eq!(m.data, vec![5, 6, 7, 8]);
    }

    #[test]
    fn repeated_dii_keeps_progress() {
        let mut r = ModuleReassembler::new();
        let d = dii(1, 2, vec![module(1, 4, 0)]);
        r.note_dii(&d);
        assert!(r.feed_ddb(&ddb(1, 1, 0, 0, &[1, 2])).is_none());
        r.note_dii(&d); // carousel repeat
        let m = r.feed_ddb(&ddb(1, 1, 0, 1, &[3, 4])).expect("complete");
        assert_eq!(m.data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn duplicate_and_out_of_range_blocks_ignored() {
        let mut r = ModuleReassembler::new();
        r.note_dii(&dii(1, 2, vec![module(1, 4, 0)]));
        assert!(r.feed_ddb(&ddb(1, 1, 0, 0, &[1, 2])).is_none());
        assert!(r.feed_ddb(&ddb(1, 1, 0, 0, &[1, 2])).is_none()); // dup
        assert!(r.feed_ddb(&ddb(1, 1, 0, 9, &[9, 9])).is_none()); // range
        assert_eq!(r.pending(), 1);
    }

    #[test]
    fn wrong_block_length_ignored() {
        let mut r = ModuleReassembler::new();
        r.note_dii(&dii(1, 4, vec![module(1, 6, 0)]));
        // Block 0 must be exactly blockSize (4); block 1 exactly 2.
        assert!(r.feed_ddb(&ddb(1, 1, 0, 0, &[1, 2, 3])).is_none());
        assert!(r.feed_ddb(&ddb(1, 1, 0, 1, &[5, 6, 7])).is_none());
        assert_eq!(r.pending(), 1);
        assert!(r.feed_ddb(&ddb(1, 1, 0, 0, &[1, 2, 3, 4])).is_none());
        assert!(r.feed_ddb(&ddb(1, 1, 0, 1, &[5, 6])).is_some());
    }

    #[test]
    fn oversize_module_skipped() {
        let mut r = ModuleReassembler::with_max_module_size(8);
        r.note_dii(&dii(1, 4, vec![module(1, 9, 0), module(2, 8, 0)]));
        assert_eq!(r.pending(), 1); // only module 2 within the cap
    }

    #[test]
    fn zero_block_size_skipped() {
        let mut r = ModuleReassembler::new();
        r.note_dii(&dii(1, 0, vec![module(1, 4, 0)]));
        assert_eq!(r.pending(), 0);
    }

    /// `moduleSize == 0` announcements are rejected: a module with no data has
    /// nothing to reassemble, and zero-size slots cost zero budget — a hostile
    /// carousel rotating ids could otherwise grow the slot map without bound.
    #[test]
    fn zero_size_module_announcement_ignored() {
        let mut r = ModuleReassembler::new();
        r.note_dii(&dii(1, 4, vec![module(1, 0, 0)]));
        assert_eq!(r.pending(), 0);
        assert!(r.feed_ddb(&ddb(1, 1, 0, 0, &[])).is_none());
    }

    /// The slot-count cap bounds the map itself: data bytes alone don't model
    /// the per-slot map/bitset overhead, so many distinct small-module
    /// announcements must also be bounded. Completing a module frees its slot
    /// for later announcements.
    #[test]
    fn slot_count_capped() {
        let mut r = ModuleReassembler::new().with_max_slots(3);
        let modules: Vec<_> = (0..5).map(|i| module(i, 1, 0)).collect();
        r.note_dii(&dii(1, 4, modules));
        assert_eq!(r.pending(), 3); // first three announcements; rest skipped
                                    // Completing one frees a slot...
        assert!(r.feed_ddb(&ddb(1, 0, 0, 0, &[0xAA])).is_some());
        assert_eq!(r.pending(), 2);
        // ...so a re-announcement of a skipped module now fits.
        r.note_dii(&dii(1, 4, vec![module(4, 1, 0)]));
        assert_eq!(r.pending(), 3);
    }

    /// The aggregate budget bounds rotating-key amplification: announcements
    /// past `max_total_bytes` are skipped, and completing a module frees its
    /// budget for later announcements.
    #[test]
    fn aggregate_budget_bounds_total_memory() {
        let mut r = ModuleReassembler::with_limits(8, 10);
        r.note_dii(&dii(1, 4, vec![module(1, 8, 0)]));
        assert_eq!(r.pending_bytes(), 8);
        // A second module would exceed the 10-byte aggregate budget — skipped,
        // even though it is within the per-module cap.
        r.note_dii(&dii(2, 4, vec![module(1, 8, 0)]));
        assert_eq!(r.pending(), 1);
        assert_eq!(r.pending_bytes(), 8);
        // Completing the first frees its budget...
        assert!(r.feed_ddb(&ddb(1, 1, 0, 0, &[0; 4])).is_none());
        assert!(r.feed_ddb(&ddb(1, 1, 0, 1, &[0; 4])).is_some());
        assert_eq!(r.pending_bytes(), 0);
        // ...so the repeat announcement now fits.
        r.note_dii(&dii(2, 4, vec![module(1, 8, 0)]));
        assert_eq!(r.pending(), 1);
        assert_eq!(r.pending_bytes(), 8);
    }

    /// A version bump releases the old slot's budget before charging the new.
    #[test]
    fn version_replacement_releases_budget() {
        let mut r = ModuleReassembler::with_limits(8, 8);
        r.note_dii(&dii(1, 4, vec![module(1, 8, 0)]));
        assert_eq!(r.pending_bytes(), 8);
        r.note_dii(&dii(1, 4, vec![module(1, 8, 1)]));
        assert_eq!(r.pending(), 1);
        assert_eq!(r.pending_bytes(), 8); // replaced, not doubled
    }

    /// blockSize=1 tracking is a bitset — the structure stays usable and the
    /// dup-guard still holds at single-byte granularity.
    #[test]
    fn block_size_one_uses_bitset() {
        let mut r = ModuleReassembler::new();
        r.note_dii(&dii(1, 1, vec![module(1, 130, 0)]));
        for i in 0..129u16 {
            assert!(r.feed_ddb(&ddb(1, 1, 0, i, &[i as u8])).is_none());
            // duplicate of the same block is ignored
            assert!(r.feed_ddb(&ddb(1, 1, 0, i, &[i as u8])).is_none());
        }
        let m = r.feed_ddb(&ddb(1, 1, 0, 129, &[0x81])).expect("complete");
        assert_eq!(m.data.len(), 130);
        assert_eq!(m.data[129], 0x81);
    }
}
