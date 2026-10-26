The verbatim spec transcriptions are in the private repo (`private/specs-verified/captions/cc-data/decode/`); this is the public own-words summary. The public-domain 47 CFR 79.102 conformance model stays here: [cea708-conformance.md](cea708-conformance.md).

# cc-data `decode` feature: the caption decode model

The `decode` feature (on by default) adds `Cea608Decoder` and `Cea708Decoder` above the `cc_data()` carriage. Both are one-way (bytes to caption state) and written to ignore malformed, truncated or over-long input without panicking (each has a no-panic test on arbitrary bytes). Source: `src/decode/{cea608,cea708,screen}.rs`. Clause numbers refer to ANSI/CTA-608-E and ANSI/CTA-708-E.

## CEA-608 (line 21)

Input is byte pairs from `cc_type` 0 (field 1) and 1 (field 2), via `push_triplets` (invalid triplets are skipped) or `push_pair(field2, b1, b2)`. The top bit (parity) is masked off before classification; the decoder does not check parity.

Pair classification, in order:

- 0x00 0x00 is filler and ignored.
- On field 2, a first byte in 0x01..=0x0F is XDS. The decoder marks an XDS span open (0x0F closes it), swallows its payload pairs, and never turns XDS into caption text.
- A first byte in 0x10..=0x1F is a control code. A caption control code arriving inside an open XDS span ends the skip and is processed normally.
- Anything else is two displayable characters (bytes of 0x20 or above, mapped through the standard set).

Doubling: a control pair identical to the previous control pair on the same field is treated as the repeat and dropped once. Any displayable character resets the memory of the last control pair.

Channels: four logical channels CC1-CC4 (field 1 gives CC1/CC2, field 2 gives CC3/CC4). The first byte picks channel 1 or 2 (0x10..=0x17 versus 0x18..=0x1F). On field 2 the miscellaneous-control codes use a shifted first byte (0x15/0x1D), handled specially (issue #1043). Plain characters carry no channel, so they go to the channel of the most recent control code on that field. Each channel has its own displayed memory, non-displayed memory, mode, cursor, and pen. Screens are 15 rows by 32 columns.

Modes and the miscellaneous controls:

- Pop-on: RCL selects the mode and moves the cursor home but does not erase the back buffer. Characters go to non-displayed memory, and EOC swaps the two memories. ENM clears the back buffer, EDM clears the front.
- Roll-up: RU2/RU3/RU4 select a 2-4 row window with the base on row 15. Characters go straight to displayed memory. CR scrolls the window up one row within its rows and blanks the base row.
- Paint-on: RDC selects the mode and characters go straight to displayed memory.
- Text mode: TR (clears and homes) and RTD set the mode to text; the state is tracked, with no separate text-mode screen model beyond that.
- BS erases the previous cell, DER clears from the cursor to end of row, tab offsets 1-3 advance the cursor. FON (flash on) is accepted and does nothing.

Addressing and style: a PAC sets row (rows grouped in pairs by first byte, row 11 alone), then either an indent in steps of 4 columns (up to 28) or a colour/italics attribute, plus underline. Mid-row codes set colour or italics and underline from the cursor and occupy one cell (a styled space). Each stored cell remembers its colour, italics and underline (`Cea608StyledChar`).

Character sets: standard set, special set (including the music note), and two extended Western-European blocks. An extended character first backspaces over the fallback character the sender put before it. Background/foreground attribute codes are recognised and ignored, so there is no background-colour or opacity model.

Outputs: `screen(channel)`, `mode(channel)`, `channel_text(channel)`, per-row `text()` and `styled_cells()`.

## CEA-708 (DTVCC)

Pipeline: `cc_data` triplets (`Dtvcc708Start`/`Dtvcc708Data`) feed a packet buffer, which becomes service blocks, which feed the command interpreter.

- Packet reassembly: a Start triplet flushes the previous packet and opens a new one. Data with no open packet is dropped. The buffer persists across `push_triplets` calls and is decoded only when the length declared by the header's size code has arrived (code 0 means 127 data bytes, otherwise `2*code - 1`) or the next Start arrives (fixed in issue #1042). `push_packet` takes a complete packet directly.
- Sequence number: the 2-bit header sequence must advance by 1 modulo 4; a gap resets every service (packet-loss recovery). `reset()` does the same on demand.
- Service blocks: header carries a 3-bit service number and 5-bit size. Service number 7 escapes to an extended header with a 6-bit number. An all-zero header ends the packet. A truncated final block is processed with what is present. Only services 1-6 are tracked (per 47 CFR 79.102(c)); other numbers are ignored.
- Code spaces: C0 (0x00-0x1F), G0 (0x20-0x7F, ASCII with 0x7F as the music note), C1 (0x80-0x9F), G1 (0xA0-0xFF, Latin-1 by code point). The EXT1 prefix (0x10) reaches C2, G2, C3 and G3. G2 maps a subset (quotes, bullet, ellipsis, fractions, trademark, some accented letters); unmapped G2 and G3 values become an underscore, and G3's one defined icon is a placeholder glyph. C2 and C3 commands are length-skipped, not interpreted. The P16 command and undefined C0 codes are skipped by their defined lengths.
- C0 controls acting on the current window: BS, FF (clear), CR (next row, scrolling the grid when at the bottom), HCR (erase row, go to column 0). ETX and NUL are no-ops.

Windows and pens (per service, 8 windows DF0-DF7, at most 12 rows by 42 columns):

- DefineWindow (DF0-DF7) sets priority, visibility, locks, anchor point and anchor coordinates (relative or absolute), row/column counts, and window/pen style preset IDs. On creation it applies preset window and pen styles (preset 0 treated as 1), allocates the grid, and clears text; on update a non-zero preset is re-applied and text stays. It also makes that window current.
- CW0-CW7 select the current window (only if defined). CLW, DSW, HDW, TGW and DLW take a bitmap and clear, show, hide, toggle or delete the chosen windows.
- SetWindowAttributes, SetPenAttributes, SetPenColor and SetPenLocation update justification, direction, word wrap, fill/border/edge, font, size, offset, italics, underline, colours, opacities and the cursor position. RST resets the service.
- Predefined window styles and pen styles (IDs 1-7) are applied through lookup helpers at the end of `cea708.rs`.

Outputs: `windows(service)` exposes `Window` records (state, geometry, all attributes, text via `text()`), and `service_text(service)` joins the text of visible windows ordered by priority then window ID.

## Delay commands

DLY (command plus a tenths-of-seconds byte) and DLC (cancel) are consumed with the correct length but have no effect: the decoder has no clock and applies no timing.

## Implemented versus not

Implemented: everything described above. Not implemented: XDS decoding (skipped only); 608 background/attribute codes, flash and separate text-mode display; any 708 delay or timing behaviour; C2/C3 command semantics; rendering or layout of windows (geometry is recorded, nothing is drawn); services beyond 6 and the extended-service content; most G2/G3 glyphs (placeholders). Pen offset and size are stored per window, not per character, so mixed-style runs inside one window are not kept.
