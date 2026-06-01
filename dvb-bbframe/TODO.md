# dvb_bbframe

DVB-S2/DVB-T2 Base-Band Frame parser and builder.

## TODO

### Extract raw BBFrame fixtures

- [ ] Extract raw BBFrame binary files from existing TS `.ts` captures in `tests/fixtures/`
  - Strip the outer TS section headers (`00 80 00 [slen] [count]`)
  - Save as `.bbframe` files (e.g. `tnt-5w-12732v-bbframe.bbframe`)
  - Each file = complete concatenated BBFrames, 10-byte header + data
- [ ] Add integration tests that parse raw `.bbframe` files directly
  - No section extraction needed, just walk the binary and validate CRC + parse headers
  - This decouples BBFRAME crate tests from TS section format (which is only the capture container, not the protocol)
- [ ] Same for Rai file — extract outer BBFrames or inner T2-MI HEM payloads to `.bbframe`

### Fix test duplication between TNT and Rai

- Both files currently use the same format (outer BBFrame on PID 0x010E in TS sections)
- The Rai file should test HEM mode but needs either:
  - Extraction of T2-MI inner stream BBFrames from inside the outer stream, OR
  - A separate capture of raw DVB-T2 HEM BBFrames
