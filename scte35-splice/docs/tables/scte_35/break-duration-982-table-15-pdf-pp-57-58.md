## break_duration() — §9.8.2, Table 15, PDF pp. 57-58

Specifies the duration of the commercial Break(s); may be used to give the
splicer an indication of when the Break will be over and when the network In
Point will occur.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `break_duration() {` |  |  |
| &nbsp;&nbsp;auto_return | 1 | bslbf |
| &nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;duration | 33 | uimsbf |
| `}` |  |  |

- **auto_return** — 1-bit flag; when '1' (Auto Return Mode, §9.9.2.2), the
  duration shall be used by the splicing device to know when the return to
  the network feed (end of Break) is to take place; a splice_insert()
  command with `out_of_network_indicator` set to 0 is not intended to be
  sent to end this Break (though one may be sent to terminate it early, and
  shall always override the running duration). When '0', the duration field,
  if present, is not required to end the Break because a new splice_insert()
  command will be sent; its presence acts as a safety mechanism in the event
  that the end-of-Break splice_insert() command is lost.
- **duration** — 33 bits; elapsed time in ticks of the program's 90 kHz
  clock.

