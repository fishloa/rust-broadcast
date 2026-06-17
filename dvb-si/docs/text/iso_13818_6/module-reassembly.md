## Module reassembly

A module is complete when, for the (downloadId, moduleId, moduleVersion) triple
announced by a DII entry, every block `0..ceil(moduleSize / blockSize)` has
been received; the final block carries `moduleSize − (nBlocks−1)×blockSize`
bytes. Implemented by `carousel::ModuleReassembler`.

