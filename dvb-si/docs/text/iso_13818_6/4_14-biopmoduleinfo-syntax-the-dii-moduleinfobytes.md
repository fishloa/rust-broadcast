## Table 4.14 — BIOP::ModuleInfo syntax (the DII `moduleInfoBytes`)
_§4.7.5.1, PDF p. 42_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `ModuleTimeOut` | 32 | + | µs to time out acquisition of all blocks |
| `BlockTimeOut` | 32 | + | µs to time out the next block |
| `MinBlockTime` | 32 | + | min µs between two blocks |
| `taps_count` | 8 | N1 | ≥ 1 (≥ one BIOP_OBJECT_USE tap) |
| per tap: `Id` | 16 | `0x0000` | user private |
| per tap: `Use` | 16 | `0x0017` | BIOP_OBJECT_USE |
| per tap: `association_tag` | 16 | + | ES on which the modules are broadcast |
| per tap: `selector_length` | 8 | `0x00` | (zero-length selector) |
| `UserInfoLength` | 8 | N2 | |
| `userInfo_data_byte` × N2 | 8 each | + | descriptor loop (incl. NUL terminators) |

The `userInfo` loop carries Data-Carousel module descriptors. DVB receivers must
support the **`compressed_module_descriptor` (tag `0x09`)**, which signals that
the module is transmitted zlib-compressed (see below).

