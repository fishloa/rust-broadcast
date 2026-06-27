## splice_time() — §9.8.1, Table 14, PDF pp. 56-57

The splice_time() structure, when modified by `pts_adjustment`, specifies
the time of the splice event.

| Syntax | Bits | Mnemonic |
|---|---|---|
| `splice_time() {` |  |  |
| &nbsp;&nbsp;time_specified_flag | 1 | bslbf |
| &nbsp;&nbsp;`if(time_specified_flag == 1) {` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 6 | bslbf |
| &nbsp;&nbsp;&nbsp;&nbsp;pts_time | 33 | uimsbf |
| &nbsp;&nbsp;`}` |  |  |
| &nbsp;&nbsp;`else` |  |  |
| &nbsp;&nbsp;&nbsp;&nbsp;reserved | 7 | bslbf |
| `}` |  |  |

- **time_specified_flag** — 1-bit flag; when '1', indicates the presence of
  the `pts_time` field and associated reserved bits.
- **pts_time** — 33 bits, time in ticks of the program's 90 kHz clock. When
  modified by `pts_adjustment`, represents the time of the intended Splice
  Point: the Splice Point shall be the first PES packet in the bit stream
  with a PTS time greater than or equal to `pts_time` as adjusted by
  `pts_adjustment`. Coding constraints for Splice Points are documented in
  SCTE 172.

