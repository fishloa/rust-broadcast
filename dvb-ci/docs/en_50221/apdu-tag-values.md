# Application Object Tags (APDU tag values)

_Source: EN 50221 §8.8.2, Table 58 + Figure 16 (PDF pp. 56-57), render-verified_

The coding of the `apdu_tag` follows the ASN.1 rules. Each `apdu_tag` is coded on
**three bytes**. Of the 24 bits, 10 are fixed by the ASN.1 rules (Figure 16); only
primitive tags are used.

## Primitive tag coding (Figure 16, p. 56)

| Byte 1 (b24..b17) | Byte 2 (b16..b9) | Byte 3 (b8..b1) |
|-------------------|------------------|-----------------|
| `1 0 0 1 1 1 1 1` | `1 x x x x x x x` | `0 x x x x x x x` |

So every public `apdu_tag` begins with byte `0x9F`, the second byte has its MSB set
(`1xxxxxxx`), and the third byte has its MSB clear (`0xxxxxxx`).

## Table 58 — Application object tag values (full list)

Direction column: `--->` = host to app/module, `<---` = app/module to host,
`<-->` = either direction.

| apdu_tag | tag value (hex) | Resource | Direction (host <-> app) |
|----------|-----------------|----------|--------------------------|
| Tprofile_enq             | `9F 80 10` | resource mgr.      | `<-->` |
| Tprofile                 | `9F 80 11` | resource mgr.      | `<-->` |
| Tprofile_change          | `9F 80 12` | resource mgr.      | `<-->` |
| Tapplication_info_enq    | `9F 80 20` | application info.  | `--->` |
| Tapplication_info        | `9F 80 21` | application info.  | `<---` |
| Tenter_menu              | `9F 80 22` | application info.  | `--->` |
| Tca_info_enq             | `9F 80 30` | CA Support         | `--->` |
| Tca_info                 | `9F 80 31` | CA Support         | `<---` |
| Tca_pmt                  | `9F 80 32` | CA Support         | `--->` |
| Tca_pmt_reply            | `9F 80 33` | CA Support         | `<---` |
| Ttune                    | `9F 84 00` | Host Control       | `<---` |
| Treplace                 | `9F 84 01` | Host Control       | `<---` |
| Tclear_replace           | `9F 84 02` | Host Control       | `<---` |
| Task_release             | `9F 84 03` | Host Control       | `--->` |
| Tdate_time_enq           | `9F 84 40` | Date-time          | `<---` |
| Tdate_time               | `9F 84 41` | Date-time          | `--->` |
| Tclose_mmi               | `9F 88 00` | MMI                | `--->` |
| Tdisplay_control         | `9F 88 01` | MMI                | `<---` |
| Tdisplay_reply           | `9F 88 02` | MMI                | `--->` |
| Ttext-last               | `9F 88 03` | MMI                | `<---` |
| Ttext-more               | `9F 88 04` | MMI                | `<---` |
| Tkeypad_control          | `9F 88 05` | MMI                | `<---` |
| Tkeypress                | `9F 88 06` | MMI                | `--->` |
| Tenq                     | `9F 88 07` | MMI                | `<---` |
| Tansw                    | `9F 88 08` | MMI                | `--->` |
| Tmenu_last               | `9F 88 09` | MMI                | `<---` |
| Tmenu_more               | `9F 88 0A` | MMI                | `<---` |
| Tmenu_answ               | `9F 88 0B` | MMI                | `--->` |
| Tlist_last               | `9F 88 0C` | MMI                | `<---` |
| Tlist_more               | `9F 88 0D` | MMI                | `<---` |
| Tsubtitle_segment_last   | `9F 88 0E` | MMI                | `<---` |
| Tsubtitle_segment_more   | `9F 88 0F` | MMI                | `--->` |
| Tdisplay_message         | `9F 88 10` | MMI                | `<---` |
| Tscene_end_mark          | `9F 88 11` | MMI                | `<---` |
| Tscene_done              | `9F 88 12` | MMI                | `<---` |
| Tscene_control           | `9F 88 13` | MMI                | `--->` |
| Tsubtitle_download_last  | `9F 88 14` | MMI                | `<---` |
| Tsubtitle_download_more  | `9F 88 15` | MMI                | `--->` |
| Tflush_download          | `9F 88 16` | MMI                | `<---` |
| Tdownload_reply          | `9F 88 17` | MMI                | `<---` |
| Tcomms_cmd               | `9F 8C 00` | low-speed comms.   | `<---` |
| Tconnection_descriptor   | `9F 8C 01` | low-speed comms.   | `<---` |
| Tcomms_reply             | `9F 8C 02` | low-speed comms.   | `--->` |
| Tcomms_send_last         | `9F 8C 03` | low-speed comms.   | `<---` |
| Tcomms_send_more         | `9F 8C 04` | low-speed comms.   | `<---` |
| Tcomms_rcv_last          | `9F 8C 05` | low-speed comms.   | `--->` |
| Tcomms_rcv_more          | `9F 8C 06` | low-speed comms.   | `--->` |

## Notes

- The high byte of byte 2 selects the resource family: `80` = resource manager /
  application info / CA support, `84` = host control / date-time, `88` = MMI,
  `8C` = low-speed communications. (This is a transcription observation, not an
  explicit spec rule.)
- The `..._last` / `..._more` and `..._send_last` / `..._send_more` pairs are the
  `L_apdu_tag` / `M_apdu_tag` chaining pair (last block vs. more-to-follow) of the
  APDU chaining mechanism — see `apdu-coding.md`.
- All values above are render-verified directly from the Table 58 image (pp. 56-57).
