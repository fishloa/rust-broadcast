## L1-post signalling — overall structure
_§7.2.3.0/§7.2.3.3/§7.2.3.5, PDF pp. 66–73_

The L1-post signalling block is, in wire order:

1. **Configurable** part (Figure 27) — once.
2. **Dynamic** part for the current T2-frame (Figure 28) — once.
3. **Dynamic** part for the *next* T2-frame (Figure 28 again) — present **iff** the L1-pre `L1_REPETITION_FLAG = '1'` (§7.2.3.3). The L1-post does not change size within a super-frame, so both dynamic instances carry the same PLP/AUX loop counts.
4. **L1-post extension** field (one or more extension blocks, Table 37) — present **iff** the L1-pre `L1_POST_EXTENSION = '1'` (§7.2.3.4). Blocks follow contiguously and exactly fill the extension field.
5. **CRC_32** (32 bits) — over the configurable + both dynamic parts (if repeated) + the extension field. Located via `L1_POST_INFO_SIZE` (annex F).
6. **L1 padding** — variable, zero-valued, to align LDPC blocks (§7.2.3.6).

The information part (everything before the CRC) is `L1_POST_INFO_SIZE` bits; `Kpost_ex_pad = L1_POST_INFO_SIZE + 32`.

