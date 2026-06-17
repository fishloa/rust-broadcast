## bandwidth_reservation() — §9.7.5, Table 12, PDF p. 54

Provided for reserving bandwidth in a multiplex (e.g. keeping a PID present
at an intended repetition rate). Differs from splice_null() so receiving
equipment can handle it uniquely (e.g. remove it from the multiplex).
Descriptors sent with this command cannot be expected to be carried through
the entire transmission chain and should be private descriptors used only by
the bandwidth reservation process.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `bandwidth_reservation() {` |  |  |
| `}` |  |  |

