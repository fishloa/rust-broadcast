//! Data-carousel module reassembly — collects [`DownloadDataBlock`]s into
//! complete modules per the DII's `moduleSize`/`blockSize` announcement
//! (`docs/iso_13818_6_carousel.md`, "Module reassembly").

use std::collections::HashMap;

use super::messages::{Dii, DownloadDataBlock};

/// Identifies one module instance on the carousel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Module {
    /// Identity of the completed module.
    pub key: ModuleKey,
    /// The `moduleSize` bytes, in order.
    pub data: Vec<u8>,
}

/// Per-module collection state.
struct Slot {
    block_size: usize,
    data: Vec<u8>,
    received: Vec<bool>,
    remaining: usize,
}

/// Default cap on a single module's announced `moduleSize` — a hostile DII
/// could otherwise make the reassembler allocate gigabytes.
pub const DEFAULT_MAX_MODULE_SIZE: u32 = 64 * 1024 * 1024;

/// Collects DDB blocks into complete modules.
///
/// Usage: call [`note_dii`](Self::note_dii) for every DII (repeats are
/// idempotent; a changed `moduleVersion` restarts that module), then feed
/// every DDB through [`feed_ddb`](Self::feed_ddb). DDBs for modules not yet
/// announced by a DII are ignored — carousels repeat, so the block comes
/// round again after the DII has been seen.
pub struct ModuleReassembler {
    slots: HashMap<ModuleKey, Slot>,
    max_module_size: u32,
}

impl Default for ModuleReassembler {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleReassembler {
    /// New reassembler with [`DEFAULT_MAX_MODULE_SIZE`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
            max_module_size: DEFAULT_MAX_MODULE_SIZE,
        }
    }

    /// New reassembler with a custom per-module size cap.
    #[must_use]
    pub fn with_max_module_size(max_module_size: u32) -> Self {
        Self {
            slots: HashMap::new(),
            max_module_size,
        }
    }

    /// Register the modules announced by a DII. Modules over the size cap or
    /// with `blockSize == 0` are skipped. Re-announcement of an in-progress
    /// (same-version) module is a no-op; a new version replaces the old slot.
    pub fn note_dii(&mut self, dii: &Dii<'_>) {
        for m in &dii.modules {
            if m.module_size > self.max_module_size || dii.block_size == 0 {
                continue;
            }
            let key = ModuleKey {
                download_id: dii.download_id,
                module_id: m.module_id,
                module_version: m.module_version,
            };
            // Drop any older version of the same module.
            self.slots.retain(|k, _| {
                !(k.download_id == key.download_id
                    && k.module_id == key.module_id
                    && k.module_version != key.module_version)
            });
            if self.slots.contains_key(&key) {
                continue; // carousel repeat — keep accumulated blocks
            }
            let size = m.module_size as usize;
            let block_size = dii.block_size as usize;
            let n_blocks = size.div_ceil(block_size).max(1);
            self.slots.insert(
                key,
                Slot {
                    block_size,
                    data: vec![0u8; size],
                    received: vec![false; n_blocks],
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
        let key = ModuleKey {
            download_id: ddb.download_id,
            module_id: ddb.module_id,
            module_version: ddb.module_version,
        };
        let slot = self.slots.get_mut(&key)?;
        let n = ddb.block_number as usize;
        if n >= slot.received.len() || slot.received[n] {
            return None;
        }
        let offset = n * slot.block_size;
        let expected = (slot.data.len() - offset).min(slot.block_size);
        if ddb.block_data.len() != expected {
            return None; // disagrees with the announced geometry — corrupt
        }
        slot.data[offset..offset + expected].copy_from_slice(ddb.block_data);
        slot.received[n] = true;
        slot.remaining -= 1;
        if slot.remaining > 0 {
            return None;
        }
        let slot = self.slots.remove(&key).expect("slot exists");
        Some(Module {
            key,
            data: slot.data,
        })
    }

    /// Number of modules currently being collected.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.slots.len()
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
}
