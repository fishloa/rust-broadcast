# ADTS frame layout with error check (CRC) and multiple raw_data_blocks

Source: ISO/IEC 13818-7:2004(E) (MPEG-2 AAC), downloaded from the SRS/ossrs spec
mirror (`ISO_IEC_13818-7-AAC-2004-67b015c6ddfc9a4af83665738477124a.pdf`). The PDF
has a text layer; the tables below were transcribed from `pdftotext -layout`
output of §6.2 (Tables 4-10, pages 20-22) and §8.1.1 semantics (pages 37-41),
and every width/condition was read from that text. Not value-checked against a
second edition of the standard (ISO/IEC 14496-3 Table 1.A.5 repeats the same
syntax by reference but was not consulted).

## Table 5 - adts_frame()

```
adts_frame() {
    adts_fixed_header();
    adts_variable_header();
    if (number_of_raw_data_blocks_in_frame == 0) {
        adts_error_check();
        raw_data_block();
    } else {
        adts_header_error_check();
        for (i = 0; i <= number_of_raw_data_blocks_in_frame; i++) {
            raw_data_block();
            adts_raw_data_block_error_check();
        }
    }
}
```

## Table 6 - adts_header_error_check()

```
if (protection_absent == '0') {
    for (i = 1; i <= number_of_raw_data_blocks_in_frame; i++)
        raw_data_block_position[i];      16 bits  uimsbf
    crc_check;                           16 bits  rpchof
}
```

## Table 7 - adts_raw_data_block_error_check()

```
if (protection_absent == '0')
    crc_check;                           16 bits  rpchof
```

## Table 10 - adts_error_check()

```
if (protection_absent == '0')
    crc_check;                           16 bits  rpchof
```

Fixed header (Table 8) is 28 bits and variable header (Table 9) is 28 bits
(`copyright_identification_bit` 1, `copyright_identification_start` 1,
`frame_length` 13, `adts_buffer_fullness` 11, `number_of_raw_data_blocks_in_frame` 2);
together 7 bytes.

## Semantics (§8.1.1)

- `number_of_raw_data_blocks_in_frame`: the frame carries this value + 1 blocks.
- `frame_length`: length of the frame **including headers and error_check**, in bytes.
- `raw_data_block_position[i]`: start position of the i-th `raw_data_block()` in
  the `adts_frame()`, **measured as a byte offset from the start of the first
  `raw_data_block()`** in the frame (so block 0 is implicitly at 0; the table has
  n entries for blocks 1..n).
- `crc_check`: CRC as described in ISO/IEC 11172-3 §2.4.3.1 (not transcribed
  here; transmux does not validate it).
- Header-level CRC covers: all bits of the fixed header, the variable header and
  every `raw_data_block_position[i]`. Per-block CRCs cover the first 192 bits of
  each SCE/CPE/CCE/LFE etc. (not needed for stripping).
- When `protection_absent == 1` there is NO position table and no CRC: blocks are
  back to back with no markers (boundaries are only findable by decoding).

## Resulting byte layout, protection_absent == 0, n > 0

```
7 B header | 2n B positions | 2 B crc | blk0 | 2 B crc | blk1 | 2 B crc | ... | blkn | 2 B crc
```

Position offsets include the interleaved CRC words (they are offsets within the
frame). Assumption flagged: the spec text says "offset in bytes from the start of
the first raw_data_block()" without spelling out that the intervening crc_check
words are counted; transmux treats them as counted (physical offsets) and falls
back to "strip header-level CRC only" when the table is not self-consistent.
