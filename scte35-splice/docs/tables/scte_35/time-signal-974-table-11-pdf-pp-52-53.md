## time_signal() — §9.7.4, Table 11, PDF pp. 52-53

Provides a time synchronized data delivery mechanism; the unique payload of
the message is carried in the descriptor loop (when used to signal splice
events it shall carry one or more segmentation descriptors). If
`time_specified_flag` is 0 (no `pts_time` in the message) the command shall
be interpreted as an immediate command (with an unspecified amount of
accuracy error).

| Syntax | Bits | Mnemonic |
|---|---|---|
| `time_signal() {` |  |  |
| &nbsp;&nbsp;splice_time() |  |  |
| `}` |  |  |

